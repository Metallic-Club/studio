//! The sandboxed Luau virtual machine.

use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Error, Function, Lua, LuaOptions, MultiValue, StdLib, Value, VmState};

use crate::task_lib::{self, Scheduler};

/// Execution budget of one resume of a script thread (0.1 s).
pub const RESUME_BUDGET: Duration = Duration::from_millis(100);

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

/// Turns an `mlua` error into the engine's `chunk:line: message` text.
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

/// One sandboxed Luau VM with its own thread scheduler.
pub struct ScriptVm {
    lua: Lua,
    sched: Rc<Scheduler>,
    context: Context,
    setup_error: Option<String>,
}

impl ScriptVm {
    /// Creates a VM that prints to stdout/stderr.
    pub fn new(context: Context) -> ScriptVm {
        Self::with_output(context, Rc::new(StdioSink))
    }

    /// Creates a VM whose `print`/`warn` and thread errors go to `output`.
    ///
    /// If sandbox setup fails (a bug in this crate, not a script error) the VM refuses
    /// to run anything: every [`ScriptVm::run_script`] returns the setup error.
    pub fn with_output(context: Context, output: Rc<dyn OutputSink>) -> ScriptVm {
        let lua = match Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()) {
            Ok(lua) => lua,
            Err(_) => Lua::new(),
        };
        let sched = Rc::new(Scheduler::new(output));
        let setup_error = Self::setup(&lua, &sched)
            .err()
            .map(|e| format!("script VM setup failed: {e}"));
        ScriptVm {
            lua,
            sched,
            context,
            setup_error,
        }
    }

    fn setup(lua: &Lua, sched: &Rc<Scheduler>) -> mlua::Result<()> {
        let globals = lua.globals();
        let tostring: Function = globals.get("tostring")?;

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
        let real_os: mlua::Table = globals.get("os")?;
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

        // print / warn go to the sink.
        let join = Rc::new(move |args: MultiValue| -> mlua::Result<String> {
            let mut parts = Vec::with_capacity(args.len());
            for v in args {
                parts.push(tostring.call::<String>(v)?);
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

        task_lib::install(lua, sched)?;

        // Execution budget: abort the running thread once its deadline has passed.
        let s = sched.clone();
        lua.set_interrupt(move |_| match s.deadline.get() {
            Some(d) if Instant::now() >= d => Err(Error::runtime(
                "Script timeout: exhausted allowed execution time",
            )),
            _ => Ok(VmState::Continue),
        });

        lua.set_memory_limit(MEMORY_LIMIT_BYTES)?;
        lua.sandbox(true)
    }

    /// Which side this VM runs on.
    pub fn context(&self) -> Context {
        self.context
    }

    /// The underlying `mlua::Lua`, for host APIs (task 7) and the debugger (task 15),
    /// which reaches the raw `lua_State` through `mlua::ffi`.
    ///
    /// The execution budget uses `Lua::set_interrupt`; Luau's debugger callbacks
    /// (`lua_callbacks`: `debugbreak`, `debugstep`) are separate and do not conflict.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Compiles and runs `source` as a new script thread named `chunk_name` (the
    /// instance's full name). Returns when the script finishes or first yields
    /// (e.g. in `task.wait`); the rest runs from [`ScriptVm::step`].
    pub fn run_script(&mut self, chunk_name: &str, source: &str) -> Result<(), ScriptError> {
        if let Some(message) = &self.setup_error {
            return Err(ScriptError {
                message: message.clone(),
            });
        }
        let to_err = |e: Error| ScriptError {
            message: format_error(&e),
        };
        let function = self
            .lua
            .load(source)
            .set_name(format!("={chunk_name}"))
            .into_function()
            .map_err(to_err)?;
        let thread = self.lua.create_thread(function).map_err(to_err)?;
        let result = self.sched.resume_with_budget(&thread, Vec::new());
        if result.is_err() {
            // Release whatever the failed script allocated (e.g. after the memory cap).
            let _ = self.lua.gc_collect();
        }
        result.map_err(to_err)
    }

    /// Advances the VM clock to `now` (seconds) and resumes every due thread. Errors in
    /// those threads are reported to the output sink.
    pub fn step(&mut self, now: f64) {
        self.sched.step(now);
    }

    /// Drops the VM together with every pending `task.wait`/`task.delay`.
    pub fn shutdown(self) {
        self.sched.clear();
    }
}
