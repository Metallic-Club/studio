//! `require(moduleScript)`.
//!
//! A `ModuleScript` runs once per VM; its single return value is cached and handed to every
//! caller. The cache is scoped to the VM, so the server and the client side of a host each run
//! a module once.
//!
//! The loading is split so the module's code runs as ordinary Luau (it may yield, and it runs
//! under the requiring thread's deadline): [`prepare`] finds the cache entry or compiles the
//! chunk, a small Luau function runs it under `pcall`, and [`finish`] checks the result and
//! records it. A finished module is `done` or `failed`; a failure is cached too, so a module
//! that errored errors the same way for every caller instead of running its side effects again.
//!
//! While a module loads, the thread loading it is remembered (`RequireState`). A second
//! `require` of it from another thread **waits** (the Luau side polls with `task.wait()` until
//! the module is done, like Roblox's require, which yields until the module finished); it is a
//! cycle only when the requiring thread is the loading thread, or the loader is itself waiting
//! (directly or through other waits) on a module the requiring thread is loading. A loading
//! script that is stopped or times out outside the module's `pcall` releases what it was
//! loading, so the next `require` loads it afresh; a module that raised an error is cached as
//! failed. A client VM refuses modules that live under `ServerScriptService` (the container is
//! not visible to clients).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use metallic_core::Value as CoreValue;
use mlua::{Function, Lua, Table, Thread, UserDataRef, Value};

use super::instance::LuaInstance;
use super::{ApiState, errors};
use crate::vm::{Context, script_env};

/// Entry kinds `prepare` answers with.
const CACHED_VALUE: i64 = 0;
const RUN_CHUNK: i64 = 1;
const CACHED_ERROR: i64 = 2;
const WAIT: i64 = 3;

/// Containers whose ModuleScripts a client VM cannot require.
const SERVER_ONLY: &[&str] = &["ServerScriptService", "ServerStorage"];

/// Who is loading a module.
struct Loader {
    /// The Luau thread running the module's chunk.
    thread: Thread,
    /// Its pointer, the identity used for cycle detection.
    ptr: usize,
    /// The script that owns that thread (0 for none).
    owner: u64,
}

/// A thread waiting for a module that another thread is loading.
struct Waiting {
    key: String,
    owner: u64,
}

/// Loading bookkeeping of one VM.
#[derive(Default)]
pub(crate) struct RequireState {
    loading: RefCell<HashMap<String, Loader>>,
    waiting: RefCell<HashMap<usize, Waiting>>,
}

impl RequireState {
    /// Forgets what a stopped script was loading or waiting for.
    pub(crate) fn release_owner(&self, owner: u64) {
        if owner == 0 {
            return;
        }
        self.loading.borrow_mut().retain(|_, l| l.owner != owner);
        self.waiting.borrow_mut().retain(|_, w| w.owner != owner);
    }

    /// True when `me` waiting for `key` would wait on itself: the loader of `key` is `me`, or is
    /// waiting for a module that `me` (or a thread waiting on `me`) is loading.
    fn is_cycle(&self, me: usize, key: &str) -> bool {
        let loading = self.loading.borrow();
        let waiting = self.waiting.borrow();
        let mut next = loading.get(key).map(|l| l.ptr);
        for _ in 0..64 {
            let Some(thread) = next else { return false };
            if thread == me {
                return true;
            }
            next = waiting
                .get(&thread)
                .and_then(|w| loading.get(&w.key))
                .map(|l| l.ptr);
        }
        false
    }
}

const LUA: &str = r##"
local prepare, finish, pcall, select, error, wait = ...
local function done(module, ok, ...)
    local good, result = finish(module, ok, select("#", ...), (...))
    if good then return result end
    error(result, 0)
end
return function(module)
    local kind, value = prepare(module)
    while kind == 3 do
        wait()
        kind, value = prepare(module)
    end
    if kind == 0 then return value end
    if kind == 2 then error(value, 0) end
    return done(module, pcall(value))
end
"##;

/// The cache key of a module: its id as text (a Lua number cannot hold every id exactly).
fn entry_key(module: &LuaInstance) -> String {
    module.id.to_bits().to_string()
}

/// Installs `require` into the VM (as a library global).
pub(crate) fn install(
    lua: &Lua,
    state: &Rc<ApiState>,
    lib: Table,
    shared_g: Table,
) -> mlua::Result<Function> {
    let entries = lua.create_table()?;

    let st = state.clone();
    let tbl = entries.clone();
    let prepare = errors::guarded_fn(
        lua,
        move |lua, module: Value| -> mlua::Result<(i64, Value)> {
            let invalid =
                || mlua::Error::runtime("Attempted to call require with invalid argument(s).");
            let Value::UserData(ud) = module else {
                return Err(invalid());
            };
            let module = ud.borrow::<LuaInstance>().map_err(|_| invalid())?;
            module
                .expect_class("ModuleScript", "require")
                .map_err(|_| invalid())?;
            let key = entry_key(&module);
            let current = lua.current_thread();
            let me = current.to_pointer() as usize;
            st.require.waiting.borrow_mut().remove(&me);
            if st.context == Context::Client && server_only(&*st.world()?, module.id) {
                return Err(mlua::Error::runtime(
                    "Attempted to require a ModuleScript that is only available on the server",
                ));
            }
            if let Value::Table(entry) = tbl.get::<Value>(key.as_str())? {
                return match entry.get::<String>("state")?.as_str() {
                    "done" => Ok((CACHED_VALUE, entry.get("value")?)),
                    _ => Ok((CACHED_ERROR, entry.get("value")?)),
                };
            }
            // A loader whose thread died (it timed out or failed outside the module's own pcall)
            // never reports back: the module is failed, as in Roblox.
            let dead = st
                .require
                .loading
                .borrow()
                .get(&key)
                .is_some_and(|l| l.thread.is_finished() || l.thread.is_error());
            if dead {
                st.require.loading.borrow_mut().remove(&key);
                let entry = lua.create_table()?;
                let message = Value::String(lua.create_string(format!(
                    "{}: Requested module experienced an error while loading",
                    st.world()?.full_name(module.id)
                ))?);
                entry.set("state", "failed")?;
                entry.set("value", message.clone())?;
                tbl.set(key.as_str(), entry)?;
                return Ok((CACHED_ERROR, message));
            }
            if st.require.loading.borrow().contains_key(&key) {
                if st.require.is_cycle(me, &key) {
                    return Err(mlua::Error::runtime(
                        "Requested module was required recursively",
                    ));
                }
                let owner = st.budget.current_owner();
                st.require
                    .waiting
                    .borrow_mut()
                    .insert(me, Waiting { key, owner });
                return Ok((WAIT, Value::Nil));
            }
            let (name, source) = {
                let world = st.world()?;
                let source = match world.get(module.id, "Source") {
                    Ok(CoreValue::String(s)) => s,
                    _ => String::new(),
                };
                (world.full_name(module.id), source)
            };
            let entry = lua.create_table()?;
            let fail = |message: String| -> mlua::Result<(i64, Value)> {
                entry.set("state", "failed")?;
                let message = Value::String(lua.create_string(&message)?);
                entry.set("value", message.clone())?;
                tbl.set(key.as_str(), entry.clone())?;
                Ok((CACHED_ERROR, message))
            };
            let script = st.wrap(lua, module.id)?;
            let env = script_env(
                lua,
                &lib,
                &shared_g,
                vec![("script", Value::UserData(script))],
            )?;
            match lua
                .load(source.as_str())
                .set_name(format!("={name}"))
                .set_environment(env)
                .into_function()
            {
                Ok(function) => {
                    let owner = st.budget.current_owner();
                    st.require.loading.borrow_mut().insert(
                        key,
                        Loader {
                            thread: current,
                            ptr: me,
                            owner,
                        },
                    );
                    Ok((RUN_CHUNK, Value::Function(function)))
                }
                Err(e) => fail(crate::vm::format_error(&e)),
            }
        },
    )?;

    let st = state.clone();
    let tbl = entries;
    let finish = errors::guarded_fn(
        lua,
        move |lua,
              (module, ok, count, first): (UserDataRef<LuaInstance>, bool, usize, Value)|
              -> mlua::Result<(bool, Value)> {
            let key = entry_key(&module);
            st.require.loading.borrow_mut().remove(&key);
            let entry = lua.create_table()?;
            tbl.set(key.as_str(), entry.clone())?;
            if !ok {
                // The module's own error, untouched: it already names the module and line.
                entry.set("state", "failed")?;
                entry.set("value", first.clone())?;
                return Ok((false, first));
            }
            if count != 1 {
                let message = Value::String(lua.create_string(format!(
                    "{}: Module code did not return exactly one value",
                    st.world()?.full_name(module.id)
                ))?);
                entry.set("state", "failed")?;
                entry.set("value", message.clone())?;
                return Ok((false, message));
            }
            entry.set("state", "done")?;
            entry.set("value", first.clone())?;
            Ok((true, first))
        },
    )?;

    let globals = lua.globals();
    let chunk: Function = lua.load(LUA).set_name("=[engine]").into_function()?;
    let require: Function = chunk.call((
        prepare,
        finish,
        globals.get::<Function>("pcall")?,
        globals.get::<Function>("select")?,
        globals.get::<Function>("error")?,
        globals.get::<Table>("task")?.get::<Function>("wait")?,
    ))?;
    Ok(require)
}

/// True when `id` lives under a container clients cannot see.
fn server_only(world: &metallic_core::World, id: metallic_core::InstanceId) -> bool {
    let mut cur = world.parent(id);
    while let Some(c) = cur {
        if world
            .class_name(c)
            .is_some_and(|n| SERVER_ONLY.contains(&n))
        {
            return true;
        }
        cur = world.parent(c);
    }
    false
}
