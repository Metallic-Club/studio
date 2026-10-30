//! Sandboxed Luau scripting for the Metallic Engine.
//!
//! [`ScriptVm`] wraps one Luau virtual machine (via `mlua`) with the engine's sandbox
//! rules: no `io`, `debug`, `require`, `load`/`loadstring` or `getfenv`/`setfenv`, only
//! `os.clock` and `os.time`, `collectgarbage("count")` only, a 0.1 s execution budget per
//! resume and a 256 MB memory cap. Scripts run as coroutines, so `task.wait`,
//! `task.spawn` and `task.delay` work; the host drives them by calling [`ScriptVm::step`].

#![warn(missing_docs)]

mod task_lib;
mod vm;

pub use vm::{Context, MEMORY_LIMIT_BYTES, OutputSink, RESUME_BUDGET, ScriptError, ScriptVm};
