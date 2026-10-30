//! The sandboxed Luau virtual machine.

use std::borrow::Cow;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{
    Error, FromLuaMulti, Function, IntoLuaMulti, Lua, LuaOptions, MultiValue, StdLib, Table,
    Thread, Value, VmState, WeakLua,
};

use crate::task_lib::{self, Scheduler, TIMEOUT_MSG};

/// Execution budget of one resume of a script thread (0.1 s).
pub const RESUME_BUDGET: Duration = Duration::from_millis(100);

/// Total time one [`ScriptVm::step`] may spend resuming threads (0.25 s). Threads still
/// due when it is used up run on the next step.
pub const STEP_BUDGET_SEC: f64 = 0.25;

/// Memory cap of one VM (256 MB).
pub const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;

/// Which side of the game a VM belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// The authoritative server VM (runs `Script`s).
    Server,
    /// A player's client VM (runs `LocalScript`s).
    Client,
}

/// A script error, formatted as `<Instance full name>:<line>: <message>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    /// The formatted error text.
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Receives everything scripts print, warn and (for failing threads) report as errors.
pub trait OutputSink {
    /// A `print(...)` line.
    fn print(&self, line: &str);
    /// A `warn(...)` line.
    fn warn(&self, line: &str);
    /// An error raised by a script thread outside `run_script` (e.g. in a delayed task).
    fn error(&self, line: &str);
}

/// Default sink: prints to stdout, warnings and errors to stderr.
struct StdioSink;

impl OutputSink for StdioSink {
    fn print(&self, line: &str) {
        println!("{line}");
    }
    fn warn(&self, line: &str) {
        eprintln!("Warning: {line}");
    }
    fn error(&self, line: &str) {
        eprintln!("{line}");
    }
}

/// Converts script bytes to text, replacing invalid UTF-8.
pub(crate) fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Turns an `mlua` error into text, dropping tracebacks.
pub(crate) fn format_error(e: &Error) -> String {
    let text = match e {
        Error::CallbackError { cause, .. } => return format_error(cause),
        Error::RuntimeError(m) | Error::MemoryError(m) => m.clone(),
        Error::SyntaxError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    match text.find("\nstack traceback") {
        Some(i) => text[..i].trim_end().to_string(),
        None => text,
    }
}

fn setup_failed(e: Error) -> ScriptError {
    ScriptError {
        message: format!("script VM setup failed: {e}"),
    }
}

/// Lua that bounds `string.rep` (results over 64 MB are rejected, also when the count is
/// a numeric string). The pattern functions are bounded in `pattern.rs`.
const STRING_LIMITS: &str = r#"
local string, type, error, tonumber, tostring = string, type, error, tonumber, tostring
local rep = string.rep
local MAX_REP = 67108864
string.rep = function(s, n)
    local count = tonumber(n)
    if count ~= nil and (type(s) == "string" or type(s) == "number") and #tostring(s) * count > MAX_REP then
        error("string.rep: result too large (limit 64 MB)", 2)
    end
    return rep(s, n)
end
"#;

/// Lua that bounds `table.move`, whose element loop runs in C where the time budget's
/// interrupt cannot fire (#164): moves of more than 2^24 elements (also a numeric-string,
/// fractional, huge or NaN range) are rejected before the C loop starts. Arguments it
/// cannot read as numbers go to the original, which raises its usual error.
const TABLE_LIMITS: &str = r#"
local table, error, tonumber, floor, ceil = table, error, tonumber, math.floor, math.ceil
local move = table.move
local MAX_MOVE = 16777216
local function trunc(x)
    if x >= 0 then return floor(x) end
    return ceil(x)
end
table.move = function(a1, f, e, t, a2)
    local nf, ne = tonumber(f), tonumber(e)
    if nf ~= nil and ne ~= nil and not (trunc(ne) - trunc(nf) < MAX_MOVE) then
        error("table.move: too many elements to move (limit 16777216)", 2)
    end
    return move(a1, f, e, t, a2)
end
"#;

/// Sets process-wide Luau flags once, before the first VM exists.
fn init_luau_flags() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // Makes `table.move` over a sparse range cost the size of the tables, not of the
        // range (off by default in luau740). Best effort: `TABLE_LIMITS` bounds it anyway.
        let _ = Lua::set_fflag("LuauTableMoveTimeoutFix", true);
    });
}

/// A cloneable handle for running Lua functions under the VM's execution budget (see
/// [`ScriptVm::budget`]). It holds the Lua state weakly, so closures registered on the VM
/// may capture it without keeping the VM alive; once the VM is gone its methods return an
/// error.
#[derive(Clone)]
pub struct Budget {
    lua: WeakLua,
    sched: Rc<Scheduler>,
}

impl Budget {
    fn lua(&self) -> Result<Lua, ScriptError> {
        self.lua.try_upgrade().ok_or_else(|| ScriptError {
            message: "script VM no longer exists".to_string(),
        })
    }

    /// Calls `f(args)` with a 0.1 s budget (or the enclosing budget's remainder when
    /// called from inside a running script, whichever ends first). Errors, including
    /// timeouts, are returned as [`ScriptError`] with the `<chunk>:<line>:` prefix.
    pub fn call<R: FromLuaMulti>(
        &self,
        f: &Function,
        args: impl IntoLuaMulti,
    ) -> Result<R, ScriptError> {
        self.call_as(self.sched.current_owner(), f, args)
    }

    /// Like [`Budget::call`], with `owner` (a script's owner id, 0 for none) as the owner
    /// of the threads and connections the call creates.
    pub(crate) fn call_as<R: FromLuaMulti>(
        &self,
        owner: u64,
        f: &Function,
        args: impl IntoLuaMulti,
    ) -> Result<R, ScriptError> {
        let lua = self.lua()?;
        let values = self
            .sched
            .call_guarded(&lua, owner, f, args)
            .map_err(|message| ScriptError { message })?;
        R::from_lua_multi(values, &lua).map_err(|e| ScriptError {
            message: format_error(&e),
        })
    }

    /// The owner id of the script a thread belongs to.
    pub(crate) fn owner_of(&self, thread: &Thread) -> Option<u64> {
        self.sched.owner_of(thread)
    }

    /// The owner id of the script whose code is running now (0 outside any script).
    pub(crate) fn current_owner(&self) -> u64 {
        self.sched.current_owner()
    }

    /// Cancels the sleeping threads of a script.
    pub(crate) fn cancel_owner(&self, owner: u64) {
        self.sched.cancel_owner(owner);
    }

    /// Resumes a thread that parked itself with a plain `coroutine.yield` (for example
    /// a future `Event:Wait()`), under the same budget. If it yields again it is
    /// parked or put to sleep as usual.
    pub fn resume(&self, thread: &Thread, args: impl IntoLuaMulti) -> Result<(), ScriptError> {
        let lua = self.lua()?;
        let args = args.into_lua_multi(&lua).map_err(|e| ScriptError {
            message: format_error(&e),
        })?;
        self.sched
            .resume_with_budget(thread, args.into_iter().collect())
            .map_err(|message| ScriptError { message })
    }
}

/// One sandboxed Luau VM with its own thread scheduler.
pub struct ScriptVm {
    lua: Lua,
    sched: Rc<Scheduler>,
    context: Context,
    /// The read-only library table every script environment inherits from.
    lib: Table,
    /// The writable `_G` shared by all scripts of this VM.
    shared_g: Table,
}

impl ScriptVm {
    /// Creates a VM that prints to stdout/stderr.
    ///
    /// # Errors
    /// Returns an error if the Lua state or its sandbox cannot be set up.
    pub fn new(context: Context) -> Result<ScriptVm, ScriptError> {
        Self::with_output(context, Rc::new(StdioSink))
    }

    /// Creates a VM whose `print`/`warn` and thread errors go to `output`.
    ///
    /// # Errors
    /// Returns an error if the Lua state or its sandbox cannot be set up.
    pub fn with_output(
        context: Context,
        output: Rc<dyn OutputSink>,
    ) -> Result<ScriptVm, ScriptError> {
        init_luau_flags();
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()).map_err(setup_failed)?;
        let sched = Rc::new(Scheduler::new(output));
        let lib = Self::setup(&lua, &sched).map_err(setup_failed)?;
        let shared_g = lua.create_table().map_err(setup_failed)?;
        Ok(ScriptVm {
            lua,
            sched,
            context,
            lib,
            shared_g,
        })
    }

    fn setup(lua: &Lua, sched: &Rc<Scheduler>) -> mlua::Result<Table> {
        let globals = lua.globals();
        let tostring: Function = globals.get("tostring")?;

        // Needs `debug`, `xpcall`, `coroutine`: must run before they are removed.
        task_lib::install(lua, sched)?;
        crate::pattern::install(lua, sched)?;
        lua.load(STRING_LIMITS).set_name("=[engine]").exec()?;
        lua.load(TABLE_LIMITS).set_name("=[engine]").exec()?;

        // Remove everything the sandbox forbids.
        for name in [
            "io",
            "debug",
            "loadstring",
            "load",
            "getfenv",
            "setfenv",
            "require",
            "dofile",
            "loadfile",
            "package",
            "newproxy",
        ] {
            globals.set(name, Value::Nil)?;
        }
        let os = lua.create_table()?;
        let real_os: Table = globals.get("os")?;
        for name in ["clock", "time"] {
            os.set(name, real_os.get::<Value>(name)?)?;
        }
        globals.set("os", os)?;

        let real_collect: Function = globals.get("collectgarbage")?;
        globals.set(
            "collectgarbage",
            lua.create_function(move |_, option: Option<String>| match option.as_deref() {
                Some("count") => real_collect.call::<f64>("count"),
                _ => Err(Error::runtime("collectgarbage: only \"count\" is allowed")),
            })?,
        )?;

        // print / warn go to the sink (invalid UTF-8 is replaced).
        let join = Rc::new(move |args: MultiValue| -> mlua::Result<String> {
            let mut parts = Vec::with_capacity(args.len());
            for v in args {
                let s: mlua::LuaString = tostring.call(v)?;
                parts.push(lossy(&s.as_bytes()));
            }
            Ok(parts.join(" "))
        });
        let (s, j) = (sched.clone(), join.clone());
        globals.set(
            "print",
            lua.create_function(move |_, args: MultiValue| {
                s.sink.print(&j(args)?);
                Ok(())
            })?,
        )?;
        let s = sched.clone();
        globals.set(
            "warn",
            lua.create_function(move |_, args: MultiValue| {
                s.sink.warn(&join(args)?);
                Ok(())
            })?,
        )?;

        // Execution budget: abort the running thread once its deadline has passed. The
        // position is remembered so the timeout error can carry `<chunk>:<line>:`.
        let s = sched.clone();
        lua.set_interrupt(move |lua| match s.deadline.get() {
            Some(d) if Instant::now() >= d => {
                if s.timeout_loc.borrow().is_none() {
                    let loc = script_location(lua, 0);
                    *s.timeout_loc.borrow_mut() = Some(loc);
                }
                Err(Error::runtime(TIMEOUT_MSG))
            }
            _ => Ok(VmState::Continue),
        });

        lua.set_memory_limit(MEMORY_LIMIT_BYTES)?;

        // The string metatable is shared by every string: make it read-only.
        let string_mt: Table = lua.load("return getmetatable('')").eval()?;
        string_mt.set_readonly(true);

        let lib = globals.clone();
        lua.sandbox(true)?;
        Ok(lib)
    }

    /// Which side this VM runs on.
    pub fn context(&self) -> Context {
        self.context
    }

    /// The underlying `mlua::Lua`, for host APIs (task 7) and the debugger (task 15),
    /// which reaches the raw `lua_State` through `mlua::ffi`.
    ///
    /// Calls made directly through this handle run **without** an execution budget; use
    /// [`ScriptVm::call_with_budget`] / [`ScriptVm::budget`] for anything that runs
    /// script code. The budget uses `Lua::set_interrupt`; Luau's debugger callbacks
    /// (`lua_callbacks`: `debugbreak`, `debugstep`) are separate and do not conflict.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Sets how long one resume may run before it times out ([`RESUME_BUDGET`] by default).
    /// The budget is wall-clock time, so test rigs on loaded machines raise it; tests of the
    /// timeout itself keep the default.
    pub fn set_resume_budget(&self, budget: Duration) {
        self.sched.resume_budget.set(budget);
    }

    /// Adds (or replaces) a global visible to every script of this VM, e.g. `game` or
    /// `Instance`. The library table is read-only to scripts; only the host can write it.
    pub fn set_global(&self, name: &str, value: impl mlua::IntoLua) -> Result<(), ScriptError> {
        let to_err = |e: Error| ScriptError {
            message: format_error(&e),
        };
        self.lib.set_readonly(false);
        let result = self.lib.set(name, value).map_err(to_err);
        self.lib.set_readonly(true);
        result
    }

    /// A handle for calling functions and resuming parked threads under the budget. It
    /// can be moved into host callbacks registered on [`ScriptVm::lua`].
    pub fn budget(&self) -> Budget {
        Budget {
            lua: self.lua.weak(),
            sched: self.sched.clone(),
        }
    }

    /// Calls `f(args)` under the execution budget; see [`Budget::call`].
    pub fn call_with_budget<R: FromLuaMulti>(
        &self,
        f: &Function,
        args: impl IntoLuaMulti,
    ) -> Result<R, ScriptError> {
        self.budget().call(f, args)
    }

    /// Compiles and runs `source` as a new script thread named `chunk_name` (the
    /// instance's full name). Returns when the script finishes or first yields
    /// (e.g. in `task.wait`); the rest runs from [`ScriptVm::step`].
    ///
    /// Each script gets its own global environment (assignments to globals stay local
    /// to the script); library tables are shared and read-only, and `_G` is one
    /// writable table shared by all scripts of this VM.
    pub fn run_script(&mut self, chunk_name: &str, source: &str) -> Result<(), ScriptError> {
        self.run_script_with(chunk_name, source, Vec::new())
    }

    /// Like [`ScriptVm::run_script`], and also defines `globals` (for example `script`) in
    /// this script's own environment only.
    pub fn run_script_with(
        &mut self,
        chunk_name: &str,
        source: &str,
        globals: Vec<(&str, Value)>,
    ) -> Result<(), ScriptError> {
        self.run_script_owned(chunk_name, source, globals, 0)
    }

    /// Like [`ScriptVm::run_script_with`]. Every thread and connection the script creates
    /// carries `owner` (not 0), so the host can stop exactly this script later even when
    /// other scripts share its chunk name.
    pub fn run_script_owned(
        &mut self,
        chunk_name: &str,
        source: &str,
        globals: Vec<(&str, Value)>,
        owner: u64,
    ) -> Result<(), ScriptError> {
        let to_err = |e: Error| ScriptError {
            message: format_error(&e),
        };
        let env = self.lua.create_table().map_err(to_err)?;
        let meta = self.lua.create_table().map_err(to_err)?;
        meta.set("__index", self.lib.clone()).map_err(to_err)?;
        env.set_metatable(Some(meta)).map_err(to_err)?;
        env.set("_G", self.shared_g.clone()).map_err(to_err)?;
        for (name, value) in globals {
            env.set(name, value).map_err(to_err)?;
        }
        let function = self
            .lua
            .load(source)
            .set_name(format!("={chunk_name}"))
            .set_environment(env)
            .into_function()
            .map_err(to_err)?;
        let (thread, args) = self
            .sched
            .as_owner(owner, || {
                self.sched.new_thread(&self.lua, function, Vec::new())
            })
            .map_err(to_err)?;
        let result = self.sched.resume_with_budget(&thread, args);
        if result.is_err() {
            // Release whatever the failed script allocated (e.g. after the memory cap).
            let _ = self.lua.gc_collect();
        }
        result.map_err(|message| ScriptError { message })
    }

    /// Advances the VM clock to `now` (seconds) and resumes due threads (at most
    /// [`STEP_BUDGET_SEC`] of work per call). Errors in those threads are reported to
    /// the output sink.
    pub fn step(&mut self, now: f64) {
        self.sched.step(now);
    }

    /// Drops the VM together with every pending `task.wait`/`task.delay`.
    pub fn shutdown(self) {
        self.sched.clear();
    }
}

/// `"<chunk>:<line>: "` of the nearest script frame at or above stack `level`, skipping C
/// and engine frames; empty if there is none.
pub(crate) fn script_location(lua: &Lua, level: usize) -> String {
    for l in level..level + 8 {
        let found = lua.inspect_stack(l, |dbg| {
            let src = dbg
                .source()
                .short_src
                .map(Cow::into_owned)
                .unwrap_or_default();
            (src, dbg.current_line())
        });
        match found {
            None => break,
            Some((src, Some(line))) if !src.starts_with('[') && !src.is_empty() => {
                return format!("{src}:{line}: ");
            }
            Some(_) => {}
        }
    }
    String::new()
}
