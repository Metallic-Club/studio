//! `require(moduleScript)`.
//!
//! A `ModuleScript` runs once per VM; its single return value is cached and handed to every
//! caller. The cache is scoped to the VM, so the server and the client side of a host each run
//! a module once.
//!
//! The loading is split so the module's code runs as ordinary Luau (it may yield, and it runs
//! under the requiring thread's deadline): [`prepare`] finds the cache entry or compiles the
//! chunk, a small Luau function runs it under `pcall`, and [`finish`] checks the result and
//! records it. States of an entry: loading (a second `require` of it is a cycle), done, failed.
//! A failure is cached too, so a module that errored errors the same way for every caller
//! instead of running its side effects again.

use std::rc::Rc;

use metallic_core::Value as CoreValue;
use mlua::{Function, Lua, Table, UserDataRef, Value};

use super::instance::LuaInstance;
use super::{ApiState, errors};
use crate::vm::script_env;

/// Entry kinds `prepare` answers with.
const CACHED_VALUE: i64 = 0;
const RUN_CHUNK: i64 = 1;
const CACHED_ERROR: i64 = 2;

const LUA: &str = r##"
local prepare, finish, pcall, select, error = ...
local function done(module, ok, ...)
    local good, result = finish(module, ok, select("#", ...), (...))
    if good then return result end
    error(result, 0)
end
return function(module)
    local kind, value = prepare(module)
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
            if let Value::Table(entry) = tbl.get::<Value>(key.as_str())? {
                return match entry.get::<String>("state")?.as_str() {
                    "loading" => Err(mlua::Error::runtime(
                        "Requested module was required recursively",
                    )),
                    "done" => Ok((CACHED_VALUE, entry.get("value")?)),
                    _ => Ok((CACHED_ERROR, entry.get("value")?)),
                };
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
                    entry.set("state", "loading")?;
                    tbl.set(key.as_str(), entry)?;
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
            let entry: Table = tbl.get(key.as_str())?;
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
    ))?;
    Ok(require)
}
