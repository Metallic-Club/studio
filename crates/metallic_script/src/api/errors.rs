//! Makes errors raised by Rust API functions look like Luau errors.
//!
//! An error returned by a Rust callback reaches the script as an opaque userdata whose text
//! carries a Rust-side traceback. A script that catches it with `pcall` would see that
//! userdata. Each API function is therefore wrapped in a small Luau function that turns
//! such an error into a plain string `<script name>:<line>: <message>` and raises it again.

use mlua::{FromLuaMulti, Function, IntoLuaMulti, Lua, Value};

use crate::vm::{callback_message, script_location};

/// The Luau `wrap(f)` helper, kept in the Lua state's app data.
struct ErrorWrapper(Function);

/// Luau source of the wrapper factory (see the module docs).
const WRAPPER: &str = r#"
local pcall, error, callback, location = ...
local function finish(ok, ...)
    if ok then return ... end
    local e = ...
    local message = callback(e)
    if message ~= nil then error(location() .. message, 0) end
    error(e, 0)
end
return function(f)
    return function(...) return finish(pcall(f, ...)) end
end
"#;

/// Creates the wrapper helper. Call once per VM, before any other API setup.
pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let location = lua.create_function(|lua, ()| Ok(script_location(lua, 0)))?;
    let callback = lua.create_function(|_, error: Value| Ok(callback_message(&error)))?;
    let wrap: Function = lua.load(WRAPPER).set_name("=[engine]").call((
        globals.get::<Function>("pcall")?,
        globals.get::<Function>("error")?,
        callback,
        location,
    ))?;
    lua.set_app_data(ErrorWrapper(wrap));
    Ok(())
}

/// Wraps `f` so that its errors become plain script errors.
pub(crate) fn guarded(lua: &Lua, f: Function) -> mlua::Result<Function> {
    let Some(wrapper) = lua.app_data_ref::<ErrorWrapper>() else {
        return Err(mlua::Error::runtime("error wrapper is not installed"));
    };
    wrapper.0.call(f)
}

/// Creates a guarded Luau function from a Rust closure.
pub(crate) fn guarded_fn<A, R, F>(lua: &Lua, f: F) -> mlua::Result<Function>
where
    A: FromLuaMulti,
    R: IntoLuaMulti,
    F: Fn(&Lua, A) -> mlua::Result<R> + 'static,
{
    guarded(lua, lua.create_function(f)?)
}

/// Guards a native iterator factory and each returned iterator, without retaining a strong Lua handle.
pub(crate) fn guarded_iterator_factory(lua: &Lua, factory: Function) -> mlua::Result<Function> {
    let factory = guarded(lua, factory)?;
    let wrap = lua.create_function(|lua, iterator: Function| guarded(lua, iterator))?;
    lua.load("local factory, wrap = ... return function(...) return wrap(factory(...)) end")
        .set_name("=[engine]")
        .call((factory, wrap))
}
