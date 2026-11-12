//! The globals scripts see, built once per VM before it is sandboxed: the data type libraries, `Instance`,
//! `Enum`, `typeof`, `print`/`warn`, `tick`/`time`, the `task` library, and the metatables of every engine type.
//!
//! # Sandbox
//!
//! Luau's standard libraries are the safe set (no `io`, no `os.execute`/`getenv`/`exit`, no `package`). On top of
//! that this removes `getfenv`/`setfenv` (they would reach other scripts' environments and turn off `safeenv`),
//! `loadstring` and `require` (mlua's file-based one), `collectgarbage`, and every `debug` function but
//! `traceback` and `info`. Then `luaL_sandbox` makes the globals and every library table read-only. `_G` and
//! `shared` are writable tables shared by the VM's scripts, as in Roblox.
//!
//! # `require`
//!
//! Our own `require(moduleScript)` takes a `ModuleScript` instance, never a path. It asks the host
//! (`HostContext::require`): a loaded module's value is returned at once (the same value every time, kept in
//! `Shared::modules`); otherwise the thread yields to the host, which loads the module (or waits for a load in
//! progress) and resumes it, and it asks again. Errors (the module failed, a cycle, another language) are raised in
//! the requiring thread. The module itself runs in its own thread (`LuauLanguage::spawn_module`), with `script` set
//! to the `ModuleScript`, and must return exactly one value.
//!
//! # Time
//!
//! - `time()` is game time ([`HostContext::now`](metallic_script::HostContext::now)): seconds of the engine's
//!   clock since the game started, the same clock `task.wait` uses.
//! - `tick()` is the wall clock, seconds since the Unix epoch (Roblox's `tick`).
//! - `os.clock()` is Luau's own: a high-resolution monotonic clock, for measuring durations.
//! - `os.time()`, `os.date()` and `os.difftime()` are Luau's (wall clock, no side effects).

use std::ffi::c_int;
use std::time::{SystemTime, UNIX_EPOCH};

use metallic_core::{CFrame, Color3, Id, Vector2, Vector3};
use mlua::{Function, Lua, MultiValue, Table, Thread, Value, Variadic};

use crate::ffi::{self, ConnectionHandle, EnumRef, Push, Raise, SignalHandle, tag, with_ctx, with_shared};
use crate::outcome::{REQUIRE, WAIT, WAIT_SIGNAL, outcome};
use crate::shared::{GET_PROPERTY_CHANGED_SIGNAL, Shared};
use crate::{atoms, instance, types};

/// The functions that yield the running thread to the host. They are Luau code because only Luau code can yield
/// across a call; the host tells them apart from a script's own `coroutine.yield` by the marker.
const PRELUDE: &str = r#"
local marker, wait_kind, signal_kind, require_kind, requireNow = ...
local yield = coroutine.yield
local typeof = typeof
local function wait(seconds)
    return yield(marker, wait_kind, seconds)
end
local function signalWait(signal)
    if typeof(signal) ~= "RBXScriptSignal" then
        error("Expected ':' not '.' calling member function Wait", 2)
    end
    return yield(marker, signal_kind, signal)
end
local function require(module)
    local value, loaded = requireNow(module)
    while not loaded do
        -- The host loads the module (or waits for its load) and resumes this thread, or raises its error here.
        yield(marker, require_kind, module)
        value, loaded = requireNow(module)
    end
    return value
end
return wait, signalWait, require
"#;

/// The error of `require` with something that is not a `ModuleScript`, as Roblox words it.
const INVALID_REQUIRE: &str = "Attempted to call require with invalid argument(s).";

/// What the backend keeps from building the library.
pub(crate) struct Library {
    /// The metatable of every script's environment: `__index` is the read-only globals.
    pub(crate) env_meta: Table,
}

fn lua_error(raise: Raise) -> mlua::Error {
    mlua::Error::runtime(raise.0)
}

/// A metatable whose entries are C functions, locked against scripts.
fn metatable(lua: &Lua, type_name: &str, entries: &[(&str, ffi::lua_CFunction)]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, f) in entries {
        table.raw_set(*name, ffi::c_function(lua, *f)?)?;
    }
    table.raw_set("__type", type_name)?;
    table.raw_set("__metatable", "The metatable is locked")?;
    Ok(table)
}

/// A table of method closures (each with its name as upvalue) for a data type, anchored in the registry.
fn methods(lua: &Lua, f: ffi::lua_CFunction, names: &[&str]) -> mlua::Result<c_int> {
    let table = lua.create_table()?;
    for name in names {
        table.raw_set(*name, ffi::c_closure(lua, f, name)?)?;
    }
    table.set_readonly(true);
    ffi::registry_ref(lua, table)
}

/// A read-only library table.
fn library(lua: &Lua, functions: &[(&str, ffi::lua_CFunction)], values: Vec<(&str, Value)>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (name, f) in functions {
        table.raw_set(*name, ffi::c_function(lua, *f)?)?;
    }
    for (name, value) in values {
        table.raw_set(name, value)?;
    }
    Ok(table)
}

/// One pushed value as an mlua value.
fn pushed(lua: &Lua, push: Push) -> mlua::Result<Value> {
    let mut values = ffi::pushes_to_lua(lua, None, vec![push])?;
    Ok(values.pop_front().unwrap_or(Value::Nil))
}

/// Builds the globals and metatables, fills `shared.refs`, and sandboxes the VM.
pub(crate) fn install(lua: &Lua, shared: &mut Shared) -> mlua::Result<Library> {
    ffi::install_callbacks(lua)?;
    let mut extra = vec![GET_PROPERTY_CHANGED_SIGNAL];
    extra.extend_from_slice(&["Connect", "Once", "Wait", "Disconnect", "Connected"]);
    atoms::register(&shared.registry, &extra);

    let globals = lua.globals();
    for name in ["getfenv", "setfenv", "loadstring", "require", "collectgarbage"] {
        globals.raw_set(name, Value::Nil)?;
    }
    if let Ok(debug) = globals.get::<Table>("debug") {
        let safe = lua.create_table()?;
        for name in ["traceback", "info"] {
            safe.raw_set(name, debug.get::<Value>(name)?)?;
        }
        globals.raw_set("debug", safe)?;
    }

    install_types(lua, shared, &globals)?;
    install_instance(lua, shared, &globals)?;
    install_enums(lua, shared, &globals)?;
    install_functions(lua, &globals)?;
    install_signals(lua, &globals)?;

    let g = lua.create_table()?;
    let shared_table = lua.create_table()?;
    ffi::sandbox(lua, &[("_G", &g), ("shared", &shared_table)])?;

    let env_meta = lua.create_table()?;
    env_meta.raw_set("__index", globals)?;
    env_meta.raw_set("__metatable", "The metatable is locked")?;
    env_meta.set_readonly(true);
    Ok(Library { env_meta })
}

/// `Vector3`, `Vector2`, `CFrame`, `Color3` and their metatables.
fn install_types(lua: &Lua, shared: &mut Shared, globals: &Table) -> mlua::Result<()> {
    let vector_meta = metatable(
        lua,
        "Vector3",
        &[("__index", types::vector3_index), ("__namecall", types::vector3_method)],
    )?;
    vector_meta.set_readonly(true);
    ffi::set_vector_metatable(lua, &vector_meta)?;
    shared.refs.vector3_methods = methods(lua, types::vector3_method, types::VECTOR3_METHODS)?;
    let v3 = |v: Vector3| Value::Vector(mlua::Vector::new(v.x, v.y, v.z));
    globals.raw_set(
        "Vector3",
        library(
            lua,
            &[("new", types::vector3_new)],
            vec![
                ("zero", v3(Vector3::ZERO)),
                ("one", v3(Vector3::ONE)),
                ("xAxis", v3(Vector3::X)),
                ("yAxis", v3(Vector3::Y)),
                ("zAxis", v3(Vector3::Z)),
            ],
        )?,
    )?;

    let v2_meta = metatable(
        lua,
        "Vector2",
        &[
            ("__index", types::vector2_index),
            ("__namecall", types::vector2_method),
            ("__add", types::vector2_add),
            ("__sub", types::vector2_sub),
            ("__mul", types::vector2_mul),
            ("__div", types::vector2_div),
            ("__unm", types::vector2_unm),
            ("__eq", types::vector2_eq),
            ("__tostring", types::vector2_to_string),
        ],
    )?;
    v2_meta.set_readonly(true);
    ffi::set_metatable(lua, tag::VECTOR2, &v2_meta)?;
    shared.refs.vector2_methods = methods(lua, types::vector2_method, types::VECTOR2_METHODS)?;
    let v2 = |v: Vector2| pushed(lua, Push::Vector2(v));
    globals.raw_set(
        "Vector2",
        library(
            lua,
            &[("new", types::vector2_new)],
            vec![
                ("zero", v2(Vector2::ZERO)?),
                ("one", v2(Vector2::ONE)?),
                ("xAxis", v2(Vector2::X)?),
                ("yAxis", v2(Vector2::Y)?),
            ],
        )?,
    )?;

    let cf_meta = metatable(
        lua,
        "CFrame",
        &[
            ("__index", types::cframe_index),
            ("__namecall", types::cframe_method),
            ("__mul", types::cframe_mul),
            ("__add", types::cframe_add),
            ("__sub", types::cframe_sub),
            ("__eq", types::cframe_eq),
            ("__tostring", types::cframe_to_string),
        ],
    )?;
    cf_meta.set_readonly(true);
    ffi::set_metatable(lua, tag::CFRAME, &cf_meta)?;
    shared.refs.cframe_methods = methods(lua, types::cframe_method, types::CFRAME_METHODS)?;
    globals.raw_set(
        "CFrame",
        library(
            lua,
            &[
                ("new", types::cframe_new),
                ("Angles", types::cframe_angles),
                ("fromEulerAnglesXYZ", types::cframe_angles),
                ("fromOrientation", types::cframe_from_orientation),
                ("fromEulerAnglesYXZ", types::cframe_from_orientation),
                ("lookAt", types::cframe_look_at),
                ("fromAxisAngle", types::cframe_from_axis_angle),
            ],
            vec![("identity", pushed(lua, Push::CFrame(CFrame::IDENTITY))?)],
        )?,
    )?;

    let color_meta = metatable(
        lua,
        "Color3",
        &[
            ("__index", types::color3_index),
            ("__namecall", types::color3_method),
            ("__eq", types::color3_eq),
            ("__tostring", types::color3_to_string),
        ],
    )?;
    color_meta.set_readonly(true);
    ffi::set_metatable(lua, tag::COLOR3, &color_meta)?;
    shared.refs.color3_methods = methods(lua, types::color3_method, types::COLOR3_METHODS)?;
    let _ = Color3::WHITE;
    globals.raw_set(
        "Color3",
        library(
            lua,
            &[
                ("new", types::color3_new),
                ("fromRGB", types::color3_from_rgb),
                ("fromHex", types::color3_from_hex),
                ("fromHSV", types::color3_from_hsv),
            ],
            Vec::new(),
        )?,
    )?;
    Ok(())
}

/// The `Instance` metatable, its identity cache and `Instance.new`.
fn install_instance(lua: &Lua, shared: &mut Shared, globals: &Table) -> mlua::Result<()> {
    let meta = metatable(
        lua,
        "Instance",
        &[
            ("__index", instance::index),
            ("__newindex", instance::new_index),
            ("__namecall", instance::namecall),
            ("__eq", instance::eq),
            ("__tostring", instance::to_string),
        ],
    )?;
    meta.set_readonly(true);
    ffi::set_metatable(lua, tag::INSTANCE, &meta)?;

    let cache = lua.create_table()?;
    let weak = lua.create_table()?;
    weak.raw_set("__mode", "v")?;
    cache.set_metatable(Some(weak))?;
    shared.refs.instances = ffi::registry_ref(lua, cache)?;

    globals.raw_set("Instance", library(lua, &[("new", instance::new)], Vec::new())?)?;
    Ok(())
}

/// `Enum.<Type>.<Item>`, built from the registry: one userdata per item, cached so `==` compares identity.
fn install_enums(lua: &Lua, shared: &mut Shared, globals: &Table) -> mlua::Result<()> {
    let meta = metatable(
        lua,
        "EnumItem",
        &[
            ("__index", types::enum_item_index),
            ("__eq", types::enum_item_eq),
            ("__tostring", types::enum_item_to_string),
        ],
    )?;
    meta.set_readonly(true);
    ffi::set_metatable(lua, tag::ENUM_ITEM, &meta)?;

    let flat = lua.create_table()?;
    let root = lua.create_table()?;
    for (enum_index, desc) in shared.enums.descs().iter().enumerate() {
        let Ok(enum_index) = u16::try_from(enum_index) else {
            break;
        };
        let table = lua.create_table()?;
        let list = lua.create_table()?;
        for item_index in 0..shared.enums.items(enum_index).len() {
            let Ok(item_index) = u16::try_from(item_index) else {
                break;
            };
            let item = EnumRef { enum_index, item_index };
            let value = pushed(lua, Push::Enum(item))?;
            if let (Some(flat_index), Some(name)) =
                (shared.enums.flat_index(item), desc.items.get(usize::from(item_index)))
            {
                flat.raw_set(flat_index, value.clone())?;
                table.raw_set(*name, value.clone())?;
                list.raw_push(value)?;
            }
        }
        let name = desc.name;
        let get_items = lua.create_function(move |lua, _: MultiValue| {
            let copy = lua.create_table()?;
            for value in list.sequence_values::<Value>() {
                copy.raw_push(value?)?;
            }
            Ok(copy)
        })?;
        let enum_meta = lua.create_table()?;
        enum_meta.raw_set(
            "__index",
            lua.create_function(move |_, (_, key): (Value, String)| match key.as_str() {
                "GetEnumItems" => Ok(get_items.clone()),
                _ => Err(mlua::Error::runtime(format!(
                    "{key} is not a valid member of \"Enum.{name}\""
                ))),
            })?,
        )?;
        enum_meta.raw_set("__tostring", lua.create_function(move |_, _: Value| Ok(name))?)?;
        enum_meta.raw_set("__metatable", "The metatable is locked")?;
        table.set_metatable(Some(enum_meta))?;
        table.set_readonly(true);
        root.raw_set(name, table)?;
    }
    shared.refs.enum_items = ffi::registry_ref(lua, flat)?;
    let root_meta = lua.create_table()?;
    root_meta.raw_set(
        "__index",
        lua.create_function(|_, (_, key): (Value, String)| -> mlua::Result<Value> {
            Err(mlua::Error::runtime(format!("{key} is not a valid member of \"Enum\"")))
        })?,
    )?;
    root_meta.raw_set("__metatable", "The metatable is locked")?;
    root.set_metatable(Some(root_meta))?;
    globals.raw_set("Enum", root)?;
    Ok(())
}

ffi::lua_fn!(typeof_fn => typeof_body);

fn typeof_body(call: &ffi::Call<'_>) -> ffi::Ret {
    if call.args.len() < 1 {
        return Err(Raise::from("missing argument #1 to 'typeof'"));
    }
    Ok(ffi::Out::One(Push::Static(call.args.type_name(1))))
}

/// `typeof`, `print`, `warn`, `tick`, `time` and the `task` library.
fn install_functions(lua: &Lua, globals: &Table) -> mlua::Result<()> {
    globals.raw_set("typeof", ffi::c_function(lua, typeof_fn)?)?;

    let tostring: Function = globals.get("tostring")?;
    let join = move |values: Variadic<Value>| -> mlua::Result<String> {
        let mut line = String::new();
        for (i, value) in values.into_iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            match value {
                Value::String(s) => line.push_str(&s.to_string_lossy()),
                other => line.push_str(&tostring.call::<mlua::LuaString>(other)?.to_string_lossy()),
            }
        }
        Ok(line)
    };
    let print_join = join.clone();
    globals.raw_set(
        "print",
        lua.create_function(move |_, values: Variadic<Value>| {
            let line = print_join(values)?;
            with_ctx(|ctx| ctx.print(&line)).map_err(lua_error)
        })?,
    )?;
    globals.raw_set(
        "warn",
        lua.create_function(move |lua, values: Variadic<Value>| {
            let line = join(values)?;
            // The line of the code that called `warn` (level 1; 0 is `warn` itself), for the output to go to (#544).
            let at = lua
                .inspect_stack(1, |debug| debug.current_line())
                .flatten()
                .and_then(|line| u32::try_from(line).ok());
            with_ctx(|ctx| ctx.warn_at(&line, at)).map_err(lua_error)
        })?,
    )?;
    globals.raw_set(
        "tick",
        lua.create_function(|_, ()| {
            Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64()))
        })?,
    )?;
    globals.raw_set(
        "time",
        lua.create_function(|_, ()| with_ctx(|ctx| ctx.now()).map_err(lua_error))?,
    )?;

    let task = lua.create_table()?;
    task.raw_set("spawn", lua.create_function(task_spawn)?)?;
    task.raw_set("defer", lua.create_function(task_defer)?)?;
    task.raw_set("delay", lua.create_function(task_delay)?)?;
    task.raw_set("cancel", lua.create_function(task_cancel)?)?;
    let require_now = lua.create_function(require_now)?;
    let (wait, signal_wait, require): (Function, Function, Function) =
        lua.load(PRELUDE)
            .set_name("=task")
            .call((ffi::yield_marker(), WAIT, WAIT_SIGNAL, REQUIRE, require_now))?;
    task.raw_set("wait", wait)?;
    globals.raw_set("task", task)?;
    globals.raw_set("require", require)?;
    // Kept for `install_signals`, which runs next.
    lua.set_named_registry_value("metallic.signal_wait", signal_wait)?;
    Ok(())
}

/// `require`'s question to the host: `(value, true)` when the module is loaded, `(nil, false)` when the caller must
/// yield for it; an error when it failed, is not a `ModuleScript`, or is in another language.
fn require_now(lua: &Lua, module: Value) -> mlua::Result<(Value, bool)> {
    let Some(module) = ffi::payload_of::<Id>(lua, &module) else {
        return Err(mlua::Error::runtime(INVALID_REQUIRE));
    };
    let key = with_ctx(|ctx| ctx.require(module))
        .map_err(lua_error)?
        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
    let Some(key) = key else {
        return Ok((Value::Nil, false));
    };
    let value = with_shared(|s| s.modules.borrow().get(&key).cloned()).map_err(lua_error)?;
    Ok((value.unwrap_or(Value::Nil), true))
}

/// The thread for `task.spawn`/`defer`/`delay`'s first argument: a new one for a function, the same for a thread.
fn task_thread(lua: &Lua, target: Value, name: &str) -> mlua::Result<Thread> {
    match target {
        Value::Function(f) => lua.create_thread(f),
        Value::Thread(thread) => Ok(thread),
        other => Err(mlua::Error::runtime(format!(
            "invalid argument #1 to '{name}' (function or thread expected, got {})",
            other.type_name()
        ))),
    }
}

/// `task.spawn(f, ...)`: runs the thread now, nested in the caller, then hands it to the host.
fn task_spawn(lua: &Lua, (target, args): (Value, MultiValue)) -> mlua::Result<Thread> {
    let thread = task_thread(lua, target, "spawn")?;
    let id = with_shared(|s| s.threads.borrow_mut().add(thread.clone())).map_err(lua_error)?;
    let result = thread.resume::<MultiValue>(args);
    let resume = with_shared(|s| outcome(lua, s, &thread, result)).map_err(lua_error)?;
    with_ctx(|ctx| ctx.spawned(id, resume)).map_err(lua_error)?;
    Ok(thread)
}

/// `task.defer(f, ...)`: resumes the thread at the end of the current resumption point.
fn task_defer(lua: &Lua, (target, args): (Value, MultiValue)) -> mlua::Result<Thread> {
    let thread = task_thread(lua, target, "defer")?;
    let id = with_shared(|s| {
        let mut threads = s.threads.borrow_mut();
        let id = threads.add(thread.clone());
        if !args.is_empty() {
            threads.set_pending(id, args);
        }
        id
    })
    .map_err(lua_error)?;
    let deferred = with_ctx(|ctx| ctx.defer(id, Vec::new())).map_err(lua_error)?;
    if let Err(error) = deferred {
        let _ = with_shared(|s| s.threads.borrow_mut().take_pending(id));
        return Err(mlua::Error::runtime(error.to_string()));
    }
    Ok(thread)
}

/// `task.delay(seconds, f, ...)`: resumes the thread after `seconds` of game time.
fn task_delay(lua: &Lua, (seconds, target, args): (Option<f64>, Value, MultiValue)) -> mlua::Result<Thread> {
    let thread = task_thread(lua, target, "delay")?;
    let id = with_shared(|s| {
        let mut threads = s.threads.borrow_mut();
        let id = threads.add(thread.clone());
        if !args.is_empty() {
            threads.set_pending(id, args);
        }
        id
    })
    .map_err(lua_error)?;
    with_ctx(|ctx| ctx.delay(seconds.unwrap_or(0.0), id, Vec::new())).map_err(lua_error)?;
    Ok(thread)
}

/// `task.cancel(thread)`: the thread never runs again. The running thread cannot cancel itself.
fn task_cancel(_: &Lua, thread: Thread) -> mlua::Result<()> {
    if matches!(
        thread.status(),
        mlua::thread::ThreadStatus::Running | mlua::thread::ThreadStatus::Normal
    ) {
        return Err(mlua::Error::runtime("cannot cancel a thread that is running"));
    }
    let id = with_shared(|s| s.threads.borrow().id_of(&thread)).map_err(lua_error)?;
    if let Some(id) = id {
        with_ctx(|ctx| ctx.cancel(id)).map_err(lua_error)?;
    }
    Ok(())
}

/// The signal and connection objects: `Connect`, `Once`, `Wait`, `Disconnect`, `Connected`.
fn install_signals(lua: &Lua, _globals: &Table) -> mlua::Result<()> {
    let signal_wait: Function = lua.named_registry_value("metallic.signal_wait")?;
    lua.unset_named_registry_value("metallic.signal_wait")?;

    let connect = |once: bool| {
        lua.create_function(move |lua, (this, f): (Value, Function)| {
            let name = if once { "Once" } else { "Connect" };
            let signal = ffi::payload_of::<SignalHandle>(lua, &this)
                .ok_or_else(|| mlua::Error::runtime(format!("Expected ':' not '.' calling member function {name}")))?;
            let connection = with_ctx(|ctx| ctx.connect_to(signal.target, signal.signal, once))
                .map_err(lua_error)?
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            with_shared(|s| {
                s.listeners.borrow_mut().insert(connection.listener, f);
                let handle = ConnectionHandle {
                    listener: connection.listener,
                };
                ffi::pushes_to_lua(lua, Some(s), vec![Push::Connection(handle)])
            })
            .map_err(lua_error)?
        })
    };
    let signal_methods = lua.create_table()?;
    signal_methods.raw_set("Connect", connect(false)?)?;
    signal_methods.raw_set("Once", connect(true)?)?;
    signal_methods.raw_set("Wait", signal_wait)?;
    signal_methods.set_readonly(true);
    let signal_meta = lua.create_table()?;
    signal_meta.raw_set("__index", signal_methods)?;
    signal_meta.raw_set("__tostring", lua.create_function(|_, _: Value| Ok("Signal"))?)?;
    signal_meta.raw_set("__type", "RBXScriptSignal")?;
    signal_meta.raw_set("__metatable", "The metatable is locked")?;
    signal_meta.set_readonly(true);
    ffi::set_metatable(lua, tag::SIGNAL, &signal_meta)?;

    let disconnect = lua.create_function(|lua, this: Value| {
        let handle = ffi::payload_of::<ConnectionHandle>(lua, &this)
            .ok_or_else(|| mlua::Error::runtime("Expected ':' not '.' calling member function Disconnect"))?;
        let removed = with_ctx(|ctx| ctx.disconnect(handle.listener)).map_err(lua_error)?;
        if removed {
            with_shared(|s| s.listeners.borrow_mut().remove(&handle.listener)).map_err(lua_error)?;
        }
        Ok(())
    })?;
    let connection_meta = lua.create_table()?;
    connection_meta.raw_set(
        "__index",
        lua.create_function(move |lua, (this, key): (Value, String)| match key.as_str() {
            "Disconnect" => Ok(Value::Function(disconnect.clone())),
            "Connected" => {
                let handle = ffi::payload_of::<ConnectionHandle>(lua, &this);
                let connected = handle
                    .is_some_and(|h| with_shared(|s| s.listeners.borrow().contains_key(&h.listener)).unwrap_or(false));
                Ok(Value::Boolean(connected))
            }
            _ => Err(mlua::Error::runtime(format!(
                "{key} is not a valid member of RBXScriptConnection"
            ))),
        })?,
    )?;
    connection_meta.raw_set("__tostring", lua.create_function(|_, _: Value| Ok("Connection"))?)?;
    connection_meta.raw_set("__type", "RBXScriptConnection")?;
    connection_meta.raw_set("__metatable", "The metatable is locked")?;
    connection_meta.set_readonly(true);
    ffi::set_metatable(lua, tag::CONNECTION, &connection_meta)?;
    Ok(())
}
