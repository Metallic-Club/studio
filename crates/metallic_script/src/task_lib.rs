//! The `task` library (`task.wait`, `task.spawn`, `task.delay`) and its scheduler.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use mlua::{Error, Function, Lua, MultiValue, Result, Thread, Value};

use crate::vm::{OutputSink, RESUME_BUDGET, format_error};

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
    pending: RefCell<Vec<Pending>>,
    seq: Cell<u64>,
    pub(crate) sink: Rc<dyn OutputSink>,
}

impl Scheduler {
    pub(crate) fn new(sink: Rc<dyn OutputSink>) -> Self {
        Scheduler {
            now: Cell::new(0.0),
            deadline: Cell::new(None),
            pending: RefCell::new(Vec::new()),
            seq: Cell::new(0),
            sink,
        }
    }

    /// Forgets every sleeping thread.
    pub(crate) fn clear(&self) {
        self.pending.borrow_mut().clear();
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

    /// Resumes `thread` once. If it yields, it is queued again (`task.wait` semantics).
    pub(crate) fn resume(&self, thread: &Thread, args: Vec<Value>) -> Result<()> {
        let yielded: MultiValue = thread.resume(MultiValue::from_vec(args))?;
        if thread.status() == mlua::thread::ThreadStatus::Resumable {
            let secs = match yielded.front() {
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
        Ok(())
    }

    /// Starts a fresh time budget, then resumes `thread`.
    pub(crate) fn resume_with_budget(&self, thread: &Thread, args: Vec<Value>) -> Result<()> {
        self.deadline.set(Some(Instant::now() + RESUME_BUDGET));
        let result = self.resume(thread, args);
        self.deadline.set(None);
        result
    }

    /// Resumes every thread whose wake-up time is `<= now`. Errors go to the sink.
    pub(crate) fn step(&self, now: f64) {
        self.now.set(now);
        let mut due = {
            let mut pending = self.pending.borrow_mut();
            let (due, later): (Vec<_>, Vec<_>) = pending.drain(..).partition(|p| p.wake <= now);
            *pending = later;
            due
        };
        due.sort_by(|a, b| a.wake.total_cmp(&b.wake).then(a.seq.cmp(&b.seq)));
        for p in due {
            let args = if p.is_wait {
                vec![Value::Number(now - p.started)]
            } else {
                p.args
            };
            if let Err(e) = self.resume_with_budget(&p.thread, args) {
                self.sink.error(&format_error(&e));
            }
        }
    }
}

/// Registers the global `task` table.
pub(crate) fn install(lua: &Lua, sched: &Rc<Scheduler>) -> Result<()> {
    let task = lua.create_table()?;

    // `task.wait` yields to the scheduler; the first yielded value is the delay.
    let wait: Function = lua
        .load("local yield = coroutine.yield\nreturn function(n) return yield(n or 0) end")
        .eval()?;
    task.set("wait", wait)?;

    let s = sched.clone();
    task.set(
        "spawn",
        lua.create_function(move |lua, (f, args): (Value, MultiValue)| {
            let thread = to_thread(lua, f)?;
            if let Err(e) = s.resume(&thread, args.into_iter().collect()) {
                s.sink.error(&format_error(&e));
            }
            Ok(thread)
        })?,
    )?;

    let s = sched.clone();
    task.set(
        "delay",
        lua.create_function(move |lua, (secs, f, args): (f64, Value, MultiValue)| {
            let thread = to_thread(lua, f)?;
            s.push(
                s.now.get() + secs.max(0.0),
                false,
                thread.clone(),
                args.into_iter().collect(),
            );
            Ok(thread)
        })?,
    )?;

    lua.globals().set("task", task)
}

fn to_thread(lua: &Lua, value: Value) -> Result<Thread> {
    match value {
        Value::Function(f) => lua.create_thread(f),
        Value::Thread(t) => Ok(t),
        other => Err(Error::runtime(format!(
            "invalid argument #1 (function or thread expected, got {})",
            other.type_name()
        ))),
    }
}
