//! Makes errors raised by Rust API functions look like Luau errors.
//!
//! An error returned by a Rust callback reaches the script as an opaque userdata whose text
//! carries a Rust-side traceback. A script that catches it with `pcall` would see that
//! userdata. Each API function is therefore wrapped in a small Luau function that turns
//! such an error into a plain string `<script name>:<line>: <message>` and raises it again.

use mlua::{FromLuaMulti, Function, IntoLuaMulti, Lua, Table};

use crate::vm::script_location;

/// The Luau `wrap(f)` helper, kept in the Lua state's app data.
struct ErrorWrapper(Function);

/// Luau source of the wrapper factory (see the module docs).
const WRAPPER: &str = r#"
local pcall, error, type, tostring, find, sub, location = ...
local function clean(e)
    local s = tostring(e)
    if sub(s, 1, 15) == "runtime error: " then s = sub(s, 16) end
    local cut = find(s, "\nstack traceback", 1, true)
    if cut then s = sub(s, 1, cut - 1) end
    return s
end
local function finish(ok, ...)
    if ok then return ... end
    local e = ...
    if type(e) == "userdata" then error(location() .. clean(e), 0) end
    error(e, 0)
end
return function(f)
    return function(...) return finish(pcall(f, ...)) end
end
"#;

/// Creates the wrapper helper. Call once per VM, before any other API setup.
pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let string: Table = globals.get("string")?;
    let location = lua.create_function(|lua, ()| Ok(script_location(lua, 0)))?;
    let wrap: Function = lua.load(WRAPPER).set_name("=[engine]").call((
        globals.get::<Function>("pcall")?,
        globals.get::<Function>("error")?,
        globals.get::<Function>("type")?,
        globals.get::<Function>("tostring")?,
        string.get::<Function>("find")?,
        string.get::<Function>("sub")?,
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
