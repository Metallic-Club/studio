//! The Luau backend: the first implementation of the `ScriptLanguage` trait from `metallic_script`.
//!
//! Owns: the Luau VM, native wrappers of the engine's data types, compiling and resuming Luau threads.
//!
//! Must not depend on: anything but `metallic_script` and `metallic_core`. A second language is a new crate next to
//! this one. See `docs/adr/0003-scripting.md`.
//!
//! # The VM
//!
//! [`LuauLanguage`] owns one Luau VM (through mlua), so an engine makes one per context: one on the server, one on
//! each client. The VM is sandboxed (see the `library` module) and every script runs in its own environment table,
//! whose `__index` is the read-only globals and which holds `script`, `game` and `workspace` (Luau's sandbox
//! model). Sources are compiled at optimisation level 2; the bytecode is compiled once per file by the host's
//! chunk cache and loaded per script, named after the script's chunk name, so errors read
//! `Scripts/Door.luau#12.1:4: message` inside the VM and `Workspace.Door:4: message` in the output.
//!
//! # Bindings come from the registry
//!
//! No class is written here. An instance is a tagged userdata holding its id; a member name resolves through the
//! registry to a property, method or signal, once per (class, name), and is cached per class under the name's
//! Luau atom. Methods run through their `MethodDesc`, signals through the host's connections, `Enum` is built
//! from the registry's enums.
//!
//! # Destroyed instances
//!
//! As in Roblox, a script can still read an instance it destroyed (`print(part.Name, "was destroyed")`): its
//! properties keep their last values, `Parent` is nil, tree queries come back empty, and writes fail with
//! `cannot set X: the instance was destroyed` (the `instance` module has the table). Each `Instance` userdata pins its
//! instance in the data model while it lives, and Luau's garbage collector gives the pin back, so the data model keeps
//! a destroyed instance's last state exactly as long as some script value refers to it. The pins go back at the
//! start and end of each call into the backend, or at once with [`LuauLanguage::collect_garbage`]. A backend runs
//! against one data model at a time; pins are kept per data model (`ffi`'s module docs explain the mechanism).
//!
//! # Threads and the host
//!
//! Each script thread, listener call and `task` thread is a Luau coroutine with a [`ThreadId`]. `task.wait` and
//! `signal:Wait()` yield the coroutine with a marker the backend turns into a [`YieldReason`]; `task.spawn`,
//! `defer`, `delay` and `cancel` go through [`HostContext`], so the host's scheduler decides when everything
//! runs (`metallic_script`'s crate docs give the order).
//!
//! `require` (see the `library` module) yields the same way while its module loads; a module runs in a thread of
//! its own and its one return value stays in the VM, so every requirer gets the same table. A method that invokes
//! a callback (`BindableFunction:Invoke`) is a C function that yields to the host, and resumes with the callback's
//! results; a function assigned to a callback member (`OnInvoke`) is kept as a registry reference under the key
//! the host chose. Errors the host raises in a waiting thread (a failed module or callback, a destroyed
//! instance) go through `Thread::resume_error`, so `pcall` catches them.
//!
//! # Budgets
//!
//! Luau's interrupt (a plain C callback, not an mlua closure) polls the call's [`Deadline`] at every safepoint;
//! `Deadline::poll` reads the clock once per 256 polls, so a hot loop pays a decrement and a branch. A thread past
//! its deadline fails with [`ScriptFailure::Timeout`] and the VM stays usable. The memory limit is Luau's.
//!
//! # Native code (`jit` feature)
//!
//! `--features jit` compiles Luau's native code generator in, and every loaded chunk is compiled to machine code.
//! The feature only takes effect on x86-64 and aarch64 Windows, Linux, macOS and Android: the dependency that turns
//! it on is target-specific (`Cargo.toml`), so iOS and consoles, which forbid generating code at run time, always
//! use the interpreter (ADR 0003).
//!
//! # `unsafe`
//!
//! The hot paths use the Luau C API directly, because mlua's safe callbacks cost more than the budget for a
//! property read. All of it is in the `ffi` module (`#![allow(unsafe_code)]` there, a `// SAFETY:` comment on each
//! block); the rest of the crate is safe code on top of it.
//!
//! # Panic strategy: `unwind` only
//!
//! Script errors are C++ exceptions that unwind through Rust frames (this crate's and mlua's), which Rust only
//! allows with `panic = "unwind"`. Under `panic = "abort"` the first script error would abort the process, so such a
//! build fails to compile here instead. `examples/error_smoke.rs` exercises every error path in the shipped `dist`
//! profile; `ffi`'s module docs explain the mechanism.
//!
//! [`Deadline`]: metallic_script::Deadline
//! [`YieldReason`]: metallic_script::YieldReason

#[cfg(panic = "abort")]
compile_error!(
    "metallic_luau needs `panic = \"unwind\"`: Luau script errors are C++ exceptions that unwind through Rust \
     frames, and with `panic = \"abort\"` the first one would abort the process (see crates/luau/src/ffi.rs)"
);

mod atoms;
mod convert;
mod ffi;
mod instance;
mod library;
mod outcome;
mod shared;
mod types;

use std::sync::Arc;

use metallic_core::{DataModel, Id, ListenerKey, Registry, ScriptValue};
use metallic_script::{ChunkId, CompileError, HostContext, ModuleKey, Resume, ScriptFailure, ScriptLanguage, ThreadId};
use mlua::chunk::{ChunkMode, Compiler};
use mlua::thread::ThreadStatus;
use mlua::{Lua, LuaOptions, MultiValue, StdLib, Table, Thread};
use thiserror::Error;

use crate::ffi::{FrameGuard, Push};
use crate::shared::Shared;

/// The compiler settings every chunk uses.
fn compiler() -> Compiler {
    Compiler::new()
        .set_optimization_level(2)
        .set_debug_level(1)
        // `Vector3.new(x, y, z)` compiles to the native vector constructor.
        .set_vector_ctor("Vector3.new")
        .set_vector_type("Vector3")
        // Our `typeof` knows the engine's types; the builtin would answer "userdata" and "vector".
        .set_disabled_builtins(["typeof"])
}

/// Why a Luau backend could not be created.
#[derive(Debug, Error)]
#[error("could not set up the Luau VM: {0}")]
pub struct LuauError(String);

impl From<mlua::Error> for LuauError {
    fn from(error: mlua::Error) -> Self {
        Self(error.to_string())
    }
}

/// The Luau scripting backend: one sandboxed Luau VM, with the engine's API built from a class registry.
///
/// Register it with a `ScriptHost`; it runs `.luau` files.
pub struct LuauLanguage {
    lua: Lua,
    shared: Shared,
    compiler: Compiler,
    env_meta: Table,
    /// Compiled chunks, by [`ChunkId`].
    chunks: Vec<Vec<u8>>,
}

impl LuauLanguage {
    /// A VM whose bindings come from `registry`, which must be the registry of the data models it will run
    /// scripts against.
    pub fn new(registry: Arc<Registry>) -> Result<Self, LuauError> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())?;
        let compiler = compiler();
        lua.set_compiler(compiler.clone());
        let mut shared = Shared::new(registry);
        let library = library::install(&lua, &mut shared)?;
        Ok(Self {
            lua,
            shared,
            compiler,
            env_meta: library.env_meta,
            chunks: Vec::new(),
        })
    }

    /// Turns the time budget off (`false`) or back on. Off, Luau runs without an interrupt and a runaway loop is
    /// never stopped: only for measuring the interrupt's cost and for trusted tools.
    pub fn set_time_budget_enabled(&mut self, enabled: bool) -> Result<(), LuauError> {
        Ok(ffi::set_interrupt(&self.lua, enabled)?)
    }

    /// How many threads the backend holds (scripts, listeners and tasks not yet released).
    pub fn thread_count(&self) -> usize {
        self.shared.threads.borrow().len()
    }

    /// Runs a full garbage collection, then gives `dm` back the pins of the instances no script value refers to any
    /// more, so the snapshots of destroyed ones are freed now rather than at the next call into the backend (which
    /// gives pins back when it starts and ends). `dm` must be the data model the scripts run against.
    pub fn collect_garbage(&mut self, dm: &mut DataModel) -> Result<(), LuauError> {
        self.lua.gc_collect()?;
        ffi::release_pins(dm);
        Ok(())
    }

    /// A new environment for the running script: `script`, `game` and `workspace` (`instances`, in that order),
    /// then the globals. Called inside a frame, so the three are pinned like any script value.
    fn environment(&self, instances: [Option<Id>; 3]) -> mlua::Result<Table> {
        let env = self.lua.create_table()?;
        env.set_metatable(Some(self.env_meta.clone()))?;
        let pushes = instances.map(|id| id.map_or(Push::Nil, Push::Instance));
        let mut values = ffi::pushes_to_lua(&self.lua, Some(&self.shared), Vec::from(pushes))?.into_iter();
        for name in ["script", "game", "workspace"] {
            env.raw_set(name, values.next().unwrap_or(mlua::Value::Nil))?;
        }
        env.set_safeenv(true);
        Ok(env)
    }

    /// Registers a new thread for `function`.
    fn new_thread(&self, function: mlua::Function) -> Result<ThreadId, ScriptFailure> {
        let thread = self.lua.create_thread(function).map_err(|e| start_failure(&e))?;
        Ok(self.shared.threads.borrow_mut().add(thread))
    }

    /// The function of `chunk`, loaded for the running script (or module) in a new environment.
    fn chunk_function(&self, chunk: ChunkId, ctx: &mut HostContext<'_>) -> Result<mlua::Function, ScriptFailure> {
        if !Arc::ptr_eq(ctx.registry(), &self.shared.registry) {
            return Err(ScriptFailure::runtime(
                "this Luau backend was built for another class registry",
                None,
            ));
        }
        let Some(bytecode) = usize::try_from(chunk.0).ok().and_then(|i| self.chunks.get(i)) else {
            return Err(ScriptFailure::runtime("unknown chunk", None));
        };
        let dm = &*ctx.dm;
        let workspace = dm.registry().class_id("Workspace").and_then(|class| dm.service(class));
        let instances = [Some(ctx.script()), dm.root(), workspace];
        let name = format!("={}", ctx.chunk_name());
        let _frame = FrameGuard::enter(ctx, &self.shared);
        let env = self.environment(instances).map_err(|e| start_failure(&e))?;
        self.lua
            .load(bytecode.as_slice())
            .set_name(name)
            .set_mode(ChunkMode::Binary)
            .set_environment(env)
            .into_function()
            .map_err(|e| start_failure(&e))
    }

    /// How a resume of `thread` ended. A module's thread that returned keeps its one value for `require` instead of
    /// handing values to the host.
    fn settle(&self, thread: ThreadId, lua_thread: &Thread, result: mlua::Result<MultiValue>) -> Resume {
        let module = self.shared.threads.borrow().module_of(thread);
        if let Some(key) = module
            && lua_thread.status() != ThreadStatus::Resumable
        {
            self.shared.threads.borrow_mut().clear_module(thread);
            if let Ok(values) = &result {
                return match values.len() {
                    1 => {
                        let value = values.front().cloned().unwrap_or(mlua::Value::Nil);
                        self.shared.modules.borrow_mut().insert(key, value);
                        Resume::Finished(Vec::new())
                    }
                    _ => Resume::Errored(ScriptFailure::runtime(
                        "Module code did not return exactly one value",
                        None,
                    )),
                };
            }
        }
        outcome::outcome(&self.lua, &self.shared, lua_thread, result)
    }
}

/// A failure to create a thread.
fn start_failure(error: &mlua::Error) -> ScriptFailure {
    match error {
        mlua::Error::MemoryError(_) => ScriptFailure::OutOfMemory,
        other => ScriptFailure::runtime(other.to_string(), None),
    }
}

impl Drop for LuauLanguage {
    fn drop(&mut self) {
        // Thread and function handles go before the VM they point into. Callback references live in the VM's
        // registry and go with it.
        self.shared.threads.borrow_mut().clear();
        self.shared.listeners.borrow_mut().clear();
        self.shared.modules.borrow_mut().clear();
        self.shared.callbacks.borrow_mut().clear();
    }
}

impl ScriptLanguage for LuauLanguage {
    fn name(&self) -> &str {
        "luau"
    }

    fn extensions(&self) -> &[&str] {
        &["luau"]
    }

    fn compile(&mut self, source: &str, chunk_name: &str) -> Result<ChunkId, CompileError> {
        match self.compiler.compile(source) {
            Ok(bytecode) => {
                self.chunks.push(bytecode);
                Ok(ChunkId(self.chunks.len() as u64 - 1))
            }
            Err(error) => Err(compile_error(&error, chunk_name)),
        }
    }

    fn spawn(&mut self, chunk: ChunkId, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        let function = self.chunk_function(chunk, ctx)?;
        self.new_thread(function)
    }

    fn spawn_listener(&mut self, listener: ListenerKey, _ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        let function = self.shared.listeners.borrow().get(&listener).cloned();
        let function = match function {
            Some(function) => function,
            None => {
                let reference = self.shared.callbacks.borrow().get(&listener).copied();
                let Some(reference) = reference else {
                    return Err(ScriptFailure::runtime("unknown listener", None));
                };
                ffi::referenced_function(&self.lua, reference).map_err(|e| start_failure(&e))?
            }
        };
        self.new_thread(function)
    }

    fn spawn_module(
        &mut self,
        chunk: ChunkId,
        key: ModuleKey,
        ctx: &mut HostContext<'_>,
    ) -> Result<ThreadId, ScriptFailure> {
        let function = self.chunk_function(chunk, ctx)?;
        let thread = self.new_thread(function)?;
        self.shared.threads.borrow_mut().set_module(thread, key);
        Ok(thread)
    }

    fn release_module(&mut self, key: ModuleKey) {
        self.shared.modules.borrow_mut().remove(&key);
    }

    fn resume(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        let (lua_thread, pending) = {
            let mut threads = self.shared.threads.borrow_mut();
            (threads.get(thread), threads.take_pending(thread))
        };
        let Some(lua_thread) = lua_thread else {
            return Resume::Errored(ScriptFailure::runtime("cannot resume a dead thread", None));
        };
        self.shared.timed_out.set(None);
        let _frame = FrameGuard::enter(ctx, &self.shared);
        let args = match pending {
            Some(args) => Ok(args),
            None => ffi::to_lua(&self.lua, &self.shared, args.to_vec()),
        };
        let result = args.and_then(|args| resume_thread(&lua_thread, args));
        self.settle(thread, &lua_thread, result)
    }

    fn resume_error(&mut self, thread: ThreadId, message: &str, ctx: &mut HostContext<'_>) -> Resume {
        let lua_thread = {
            let mut threads = self.shared.threads.borrow_mut();
            // Arguments kept for a first resume are not wanted any more.
            threads.take_pending(thread);
            threads.get(thread)
        };
        let Some(lua_thread) = lua_thread else {
            return Resume::Errored(ScriptFailure::runtime("cannot resume a dead thread", None));
        };
        self.shared.timed_out.set(None);
        let _frame = FrameGuard::enter(ctx, &self.shared);
        let result = lua_thread.resume_error::<MultiValue>(message);
        self.settle(thread, &lua_thread, result)
    }

    fn release_thread(&mut self, thread: ThreadId) {
        self.shared.threads.borrow_mut().remove(thread);
    }

    fn release_listener(&mut self, listener: ListenerKey) {
        self.shared.listeners.borrow_mut().remove(&listener);
        let reference = self.shared.callbacks.borrow_mut().remove(&listener);
        if let Some(reference) = reference {
            ffi::unreference(&self.lua, reference);
        }
    }

    fn memory_used(&self) -> usize {
        self.lua.used_memory()
    }

    fn set_memory_limit(&mut self, bytes: Option<usize>) {
        // Zero is mlua's "no limit". A limit below what is in use takes effect at the next allocation.
        let _ = self.lua.set_memory_limit(bytes.unwrap_or(0));
    }
}

fn resume_thread(thread: &Thread, args: MultiValue) -> mlua::Result<MultiValue> {
    thread.resume::<MultiValue>(args)
}

/// A compiler error (`:12: message`) as a [`CompileError`].
fn compile_error(error: &mlua::Error, chunk: &str) -> CompileError {
    let text = match error {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let rest = text.strip_prefix(':').unwrap_or(&text);
    let (line, message) = match rest.split_once(':') {
        Some((line, message)) if line.parse::<u32>().is_ok() => {
            (line.parse().unwrap_or(1), message.trim_start().to_owned())
        }
        _ => (1, text.clone()),
    };
    CompileError {
        chunk: chunk.to_owned(),
        line,
        column: None,
        message,
    }
}
