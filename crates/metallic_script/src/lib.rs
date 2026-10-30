//! Sandboxed Luau scripting for the Metallic Engine.
//!
//! [`ScriptVm`] wraps one Luau virtual machine (via `mlua`) with the engine's sandbox
//! rules: no `io`, `debug`, `require`, `load`/`loadstring` or `getfenv`/`setfenv`, only
//! `os.clock` and `os.time`, `collectgarbage("count")` only, a 0.1 s execution budget per
//! resume and a 256 MB memory cap. Scripts run as coroutines, so `task.wait`,
//! `task.spawn` and `task.delay` work; the host drives them by calling [`ScriptVm::step`].
//!
//! [`ScriptHost`] sits on top: it runs a world's `Script`s and `LocalScript`s in a server
//! and a client VM and gives them the Roblox-named API (`game`, `Instance`, services,
//! signals, `RemoteEvent`; see `docs/luau-api.md`).

#![warn(missing_docs)]

mod api;
mod host;
mod pattern;
mod task_lib;
mod vm;

pub use api::{
    BoundResult, InputActionEvent, InputDevice, InputEventKind, InputValue, KEY_CODES,
    MAX_ACTION_EVENTS_PER_STEP, USER_INPUT_TYPES,
};
pub use host::{LOCAL_PLAYER_NAME, ScriptHost};
pub use vm::{
    Budget, Context, MEMORY_LIMIT_BYTES, OutputSink, RESUME_BUDGET, STEP_BUDGET_SEC, ScriptError,
    ScriptVm,
};
