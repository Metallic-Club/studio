//! The `task` library (`task.wait`, `task.spawn`, `task.delay`), the scheduler that runs
//! script threads, and the execution budget.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::time::Instant;

use mlua::{Error, Function, IntoLuaMulti, LightUserData, Lua, MultiValue, Result, Thread, Value};

use crate::vm::{OutputSink, RESUME_BUDGET, STEP_BUDGET_SEC, format_error, lossy};

/// Text of the error raised when a thread runs out of budget.
pub(crate) const TIMEOUT_MSG: &str = "Script timeout: exhausted allowed execution time";

/// Markers (light userdata) that script threads use to talk to the scheduler. A script
/// cannot forge them: it has no way to create light userdata.
static TAGS: [u8; 3] = [0; 3];

fn tag(i: usize) -> Value {
    Value::LightUserData(LightUserData(&TAGS[i] as *const u8 as *mut c_void))
}

fn is_tag(v: Option<&Value>, i: usize) -> bool {
    matches!(v, Some(Value::LightUserData(p)) if p.0 == &TAGS[i] as *const u8 as *mut c_void)
}

/// Yield marker of `task.wait`: `yield(SLEEP, seconds)`.
const SLEEP: usize = 0;
/// First result of a guarded thread body that finished normally.
const OK: usize = 1;
/// First result of a guarded thread body that raised an error (second result: message).
const ERR: usize = 2;

/// A suspended coroutine waiting for its wake-up time.
struct Pending {
    wake: f64,
    seq: u64,
    /// Time at which the thread went to sleep (to compute what `task.wait` returns).
    started: f64,
    /// `true` for `task.wait` (resume with elapsed time), `false` for `task.delay`.
    is_wait: bool,
    thread: Thread,
    args: Vec<Value>,
}

/// State shared between the VM, the `task` functions and the interrupt hook.
pub(crate) struct Scheduler {
    pub(crate) now: Cell<f64>,
    pub(crate) deadline: Cell<Option<Instant>>,
    /// Set by the interrupt hook when the budget ran out: `"<chunk>:<line>: "` of the
    /// running code (possibly empty).
    pub(crate) timeout_loc: RefCell<Option<String>>,
    pending: RefCell<Vec<Pending>>,
    /// Thread (by address) -> the owner id of the script it belongs to. Removed when the
    /// thread finishes. Owner 0 means "no script".
    owners: RefCell<HashMap<usize, u64>>,
    /// The owner of the code that is running now; new threads and connections inherit it.
    current: Cell<u64>,
    seq: Cell<u64>,
    pub(crate) sink: Rc<dyn OutputSink>,
    /// Lua function `guard(f, ...)`: runs `f` under `xpcall` with the error normaliser.
    guard: RefCell<Option<Function>>,
}

impl Scheduler {
    pub(crate) fn new(sink: Rc<dyn OutputSink>) -> Self {
        Scheduler {
            now: Cell::new(0.0),
            deadline: Cell::new(None),
            timeout_loc: RefCell::new(None),
            pending: RefCell::new(Vec::new()),
            owners: RefCell::new(HashMap::new()),
            current: Cell::new(0),
            seq: Cell::new(0),
            sink,
            guard: RefCell::new(None),
        }
    }

    /// Forgets every sleeping thread.
    pub(crate) fn clear(&self) {
        self.pending.borrow_mut().clear();
        self.owners.borrow_mut().clear();
        *self.guard.borrow_mut() = None;
    }

    pub(crate) fn guard(&self) -> Result<Function> {
        self.guard
            .borrow()
            .clone()
            .ok_or_else(|| Error::runtime("script VM is shut down"))
    }

    /// Creates a guarded thread that will run `f`; the first resume must pass `f` first.
    pub(crate) fn new_thread(
        &self,
        lua: &Lua,
        f: Function,
        args: Vec<Value>,
    ) -> Result<(Thread, Vec<Value>)> {
        let thread = lua.create_thread(self.guard()?)?;
        let owner = self.current.get();
        if owner != 0 {
            self.owners
                .borrow_mut()
                .insert(thread.to_pointer() as usize, owner);
        }
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(Value::Function(f));
        all.extend(args);
        Ok((thread, all))
    }

    /// The owner id of the script a thread belongs to.
    pub(crate) fn owner_of(&self, thread: &Thread) -> Option<u64> {
        self.owners
            .borrow()
            .get(&(thread.to_pointer() as usize))
            .copied()
    }

    /// The owner of the code that is running now (0 outside any script).
    pub(crate) fn current_owner(&self) -> u64 {
        self.current.get()
    }

    /// Runs `run` with `owner` as the current owner, restoring the previous one after.
    pub(crate) fn as_owner<T>(&self, owner: u64, run: impl FnOnce() -> T) -> T {
        let prev = self.current.replace(owner);
        let out = run();
        self.current.set(prev);
        out
    }

    /// Forgets the sleeping threads of `owner`; they are never resumed again.
    pub(crate) fn cancel_owner(&self, owner: u64) {
        let owners = self.owners.borrow();
        self.pending.borrow_mut().retain(|p| {
            owners
                .get(&(p.thread.to_pointer() as usize))
                .is_none_or(|o| *o != owner)
        });
        drop(owners);
        self.owners.borrow_mut().retain(|_, o| *o != owner);
    }

    fn push(&self, wake: f64, is_wait: bool, thread: Thread, args: Vec<Value>) {
        let seq = self.seq.get();
        self.seq.set(seq + 1);
        let started = self.now.get();
        self.pending.borrow_mut().push(Pending {
            wake,
            seq,
            started,
            is_wait,
            thread,
            args,
        });
    }

    /// Interprets the outcome of one resume: queue sleepers, report guarded errors.
    fn after_resume(
        &self,
        thread: &Thread,
        raw: Result<MultiValue>,
    ) -> std::result::Result<(), String> {
        if thread.status() != mlua::thread::ThreadStatus::Resumable {
            self.owners
                .borrow_mut()
                .remove(&(thread.to_pointer() as usize));
        }
        let values = raw.map_err(|e| format_error(&e))?;
        if thread.status() == mlua::thread::ThreadStatus::Resumable {
            if is_tag(values.front(), SLEEP) {
                let secs = match values.get(1) {
                    Some(Value::Number(n)) => *n,
                    Some(Value::Integer(n)) => *n as f64,
                    _ => 0.0,
                };
                self.push(
                    self.now.get() + secs.max(0.0),
                    true,
                    thread.clone(),
                    Vec::new(),
                );
            }
            // Any other yield parks the thread; its owner resumes it later.
            return Ok(());
        }
        if is_tag(values.front(), ERR) {
            return Err(match values.get(1) {
                Some(Value::String(s)) => lossy(&s.as_bytes()),
                _ => "unknown script error".to_string(),
            });
        }
        Ok(())
    }

    /// Runs `run` with a time budget. A nested call keeps the outer deadline if it is
    /// earlier. Returns the timeout error text if the budget ran out.
    pub(crate) fn budgeted<T>(&self, run: impl FnOnce() -> T) -> (T, Option<String>) {
        let prev = self.deadline.get();
        if prev.is_none() {
            *self.timeout_loc.borrow_mut() = None;
        }
        let mine = Instant::now() + RESUME_BUDGET;
        self.deadline.set(Some(prev.map_or(mine, |p| p.min(mine))));
        let out = run();
        self.deadline.set(prev);
        let loc = if prev.is_none() {
            self.timeout_loc.borrow_mut().take()
        } else {
            self.timeout_loc.borrow().clone()
        };
        (out, loc.map(|l| format!("{l}{TIMEOUT_MSG}")))
    }

    /// Resumes `thread` under a fresh (or the enclosing, if earlier) budget.
    pub(crate) fn resume_with_budget(
        &self,
        thread: &Thread,
        args: Vec<Value>,
    ) -> std::result::Result<(), String> {
        let owner = self.owner_of(thread).unwrap_or(0);
        let (raw, timeout) = self.as_owner(owner, || {
            self.budgeted(|| thread.resume::<MultiValue>(MultiValue::from_vec(args)))
        });
        if let Some(msg) = timeout {
            return Err(msg);
        }
        self.after_resume(thread, raw)
    }

    /// Resumes `thread` inside the caller's budget (used by `task.spawn`).
    fn resume_inline(&self, thread: &Thread, args: Vec<Value>) -> std::result::Result<(), String> {
        let owner = self.owner_of(thread).unwrap_or(self.current.get());
        let raw = self.as_owner(owner, || {
            thread.resume::<MultiValue>(MultiValue::from_vec(args))
        });
        self.after_resume(thread, raw)
    }

    /// Calls `f(args)` under the budget, through the error normaliser.
    pub(crate) fn call_guarded(
        &self,
        lua: &Lua,
        owner: u64,
        f: &Function,
        args: impl IntoLuaMulti,
    ) -> std::result::Result<MultiValue, String> {
        let guard = self.guard().map_err(|e| format_error(&e))?;
        let mut all = vec![Value::Function(f.clone())];
        all.extend(args.into_lua_multi(lua).map_err(|e| format_error(&e))?);
        let (raw, timeout) = self.as_owner(owner, || {
            self.budgeted(|| guard.call::<MultiValue>(MultiValue::from_vec(all)))
        });
        if let Some(msg) = timeout {
            return Err(msg);
        }
        let mut values = raw.map_err(|e| format_error(&e))?;
        if is_tag(values.front(), ERR) {
            return Err(match values.get(1) {
                Some(Value::String(s)) => lossy(&s.as_bytes()),
                _ => "unknown script error".to_string(),
            });
        }
        values.pop_front();
        Ok(values)
    }

    /// Resumes due threads, stopping early once [`STEP_BUDGET_SEC`] is used up; the rest
    /// stay due and run on the next step. Errors go to the sink.
    pub(crate) fn step(&self, now: f64) {
        self.now.set(now);
        let mut due = {
            let mut pending = self.pending.borrow_mut();
            let (due, later): (Vec<_>, Vec<_>) = pending.drain(..).partition(|p| p.wake <= now);
            *pending = later;
            due
        };
        due.sort_by(|a, b| a.wake.total_cmp(&b.wake).then(a.seq.cmp(&b.seq)));
        let started = Instant::now();
        let mut due = due.into_iter();
        while let Some(p) = due.next() {
            if started.elapsed().as_secs_f64() > STEP_BUDGET_SEC {
                let mut pending = self.pending.borrow_mut();
                pending.push(p);
                pending.extend(due);
                break;
            }
            let args = if p.is_wait {
                vec![Value::Number(now - p.started)]
            } else {
                p.args
            };
            if let Err(e) = self.resume_with_budget(&p.thread, args) {
                self.sink.error(&e);
            }
        }
    }
}

/// Registers the global `task` table and the thread guard.
pub(crate) fn install(lua: &Lua, sched: &Rc<Scheduler>) -> Result<()> {
    let globals = lua.globals();
    let xpcall: Value = globals.get("xpcall")?;
    let type_fn: Value = globals.get("type")?;
    let tostring: Value = globals.get("tostring")?;
    let sub: Value = globals.get::<mlua::Table>("string")?.get("sub")?;
    let find: Value = globals.get::<mlua::Table>("string")?.get("find")?;
    let info: Value = globals.get::<mlua::Table>("debug")?.get("info")?;
    let yield_fn: Value = globals.get::<mlua::Table>("coroutine")?.get("yield")?;

    // The guard runs every script thread. Its handler turns any error value into a
    // string without running script code (no `__tostring` of script tables) and adds the
    // `<chunk>:<line>:` of the nearest Lua frame when the message has none.
    let guard: Function = lua
        .load(
            r#"
local xpcall, type, tostring, sub, info, okTag, errTag, find = ...
local function describe(e)
    local t = type(e)
    if t == "string" then return e end
    if t == "nil" or t == "number" or t == "boolean" then return tostring(e) end
    if t == "userdata" then
        local s = tostring(e)
        if sub(s, 1, 15) == "runtime error: " then s = sub(s, 16) end
        local cut = find(s, "\nstack traceback", 1, true)
        if cut then s = sub(s, 1, cut - 1) end
        return s
    end
    return "<" .. t .. ">"
end
local function handler(e)
    local msg = describe(e)
    local level = 2
    while true do
        local src, line = info(level, "sl")
        if src == nil then return msg end
        if line ~= nil and line >= 0 and src ~= "[C]" and src ~= "[engine]" then
            if sub(msg, 1, #src + 1) == src .. ":" then return msg end
            return src .. ":" .. line .. ": " .. msg
        end
        level += 1
    end
end
local function finish(ok, ...)
    if ok then return okTag, ... end
    return errTag, ...
end
return function(f, ...) return finish(xpcall(f, handler, ...)) end
"#,
        )
        .set_name("=[engine]")
        .call((
            xpcall,
            type_fn,
            tostring,
            sub,
            info,
            tag(OK),
            tag(ERR),
            find,
        ))?;
    *sched.guard.borrow_mut() = Some(guard);

    let task = lua.create_table()?;

    // `task.wait` yields `(SLEEP, seconds)` to the scheduler.
    let wait: Function = lua
        .load("local yield, sleep = ... return function(n) return yield(sleep, n or 0) end")
        .call((yield_fn, tag(SLEEP)))?;
    task.set("wait", wait)?;

    let s = sched.clone();
    task.set(
        "spawn",
        lua.create_function(move |lua, (f, args): (Value, MultiValue)| {
            let args: Vec<Value> = args.into_iter().collect();
            let (thread, first) = match f {
                Value::Function(f) => s.new_thread(lua, f, args)?,
                Value::Thread(t) => (t, args),
                other => return Err(bad_target(&other)),
            };
            if let Err(e) = s.resume_inline(&thread, first) {
                // A timeout is reported once, by the resume that owns the budget.
                if s.timeout_loc.borrow().is_none() {
                    s.sink.error(&e);
                }
            }
            Ok(thread)
        })?,
    )?;

    let s = sched.clone();
    task.set(
        "delay",
        lua.create_function(move |lua, (secs, f, args): (f64, Value, MultiValue)| {
            let args: Vec<Value> = args.into_iter().collect();
            let (thread, first) = match f {
                Value::Function(f) => s.new_thread(lua, f, args)?,
                Value::Thread(t) => (t, args),
                other => return Err(bad_target(&other)),
            };
            s.push(s.now.get() + secs.max(0.0), false, thread.clone(), first);
            Ok(thread)
        })?,
    )?;

    globals.set("task", task)
}

fn bad_target(v: &Value) -> Error {
    Error::runtime(format!(
        "invalid argument (function or thread expected, got {})",
        v.type_name()
    ))
}
