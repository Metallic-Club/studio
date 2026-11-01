//! Reading paused frames: the call stack, locals and upvalues, the variables tree, watch
//! expressions and writing locals.

use std::ffi::{CStr, c_int};

use mlua::{Function, Lua, MultiValue, Table, Value, ffi};

use super::{DebugCore, Scope, VarHandle, VarNode};
use crate::api::LuaInstance;
use crate::task_lib::Scheduler;
use crate::vm::{ScriptError, format_error};

/// At most this many locals, upvalues or table entries are read per frame or table.
const MAX_VARS: c_int = 200;
/// Strings longer than this are cut in the variables tree.
const MAX_TEXT: usize = 200;

/// The displayable part of one frame.
#[derive(Clone, Debug)]
pub(crate) struct FrameInfo {
    /// The script that owns the frame's environment, when known.
    pub(crate) owner: Option<u64>,
    pub(crate) chunk: String,
    pub(crate) function: String,
    pub(crate) line: u32,
}

/// One script frame of a paused thread, with its variables as they were at the pause.
pub(crate) struct FrameSnap {
    pub(crate) info: FrameInfo,
    pub(crate) state: *mut ffi::lua_State,
    pub(crate) level: c_int,
    pub(crate) locals: Vec<(String, Value)>,
    pub(crate) upvalues: Vec<(String, Value)>,
    pub(crate) env: Option<Table>,
}

/// The variables of one frame, read from the live thread.
pub(crate) struct FrameValues {
    pub(crate) locals: Vec<(String, Value)>,
    pub(crate) upvalues: Vec<(String, Value)>,
    pub(crate) env: Option<Table>,
}

fn c_text(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: Luau names and sources are NUL-terminated.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Reads the locals, upvalues and environment of stack `level` of `l`.
///
/// # Safety
/// `l` is a live thread that is paused or running a debug hook, on this VM.
pub(crate) unsafe fn frame_values(
    lua: &Lua,
    l: *mut ffi::lua_State,
    level: c_int,
) -> Option<FrameValues> {
    let values: MultiValue = unsafe {
        lua.exec_raw((), |st| {
            if st == l || ffi::lua_checkstack(st, 12) == 0 || ffi::lua_checkstack(l, 4) == 0 {
                return;
            }
            ffi::lua_createtable(st, 0, 0);
            let names = ffi::lua_gettop(st);
            ffi::lua_createtable(st, 0, 0);
            let vals = names + 1;
            let mut n = 1;
            while n <= MAX_VARS {
                let name = ffi::lua_getlocal(l, level, n);
                if name.is_null() {
                    break;
                }
                ffi::lua_xmove(l, st, 1);
                ffi::lua_rawseti_(st, vals, n);
                ffi::lua_pushstring_(st, name);
                ffi::lua_rawseti_(st, names, n);
                n += 1;
            }
            ffi::lua_pushinteger_(st, n - 1);
            ffi::lua_createtable(st, 0, 0);
            let unames = ffi::lua_gettop(st);
            ffi::lua_createtable(st, 0, 0);
            let uvals = unames + 1;
            let mut ar = std::mem::zeroed::<ffi::lua_Debug>();
            if ffi::lua_getinfo(l, level, c"f".as_ptr(), &mut ar) == 0 {
                ffi::lua_pushinteger_(st, 0);
                ffi::lua_pushnil(st);
                return;
            }
            let mut m = 1;
            while m <= MAX_VARS {
                let name = ffi::lua_getupvalue(l, -1, m);
                if name.is_null() {
                    break;
                }
                ffi::lua_xmove(l, st, 1);
                ffi::lua_rawseti_(st, uvals, m);
                ffi::lua_pushstring_(st, name);
                ffi::lua_rawseti_(st, unames, m);
                m += 1;
            }
            ffi::lua_pushinteger_(st, m - 1);
            ffi::lua_getfenv(l, -1);
            ffi::lua_xmove(l, st, 1);
            ffi::lua_pop(l, 1);
        })
    }
    .ok()?;
    let v: Vec<Value> = values.into_iter().collect();
    let pairs = |names: Option<&Value>, vals: Option<&Value>, count: Option<&Value>| {
        let (Some(Value::Table(names)), Some(Value::Table(vals))) = (names, vals) else {
            return Vec::new();
        };
        let count = match count {
            Some(Value::Integer(n)) => *n,
            Some(Value::Number(n)) => *n as i64,
            _ => 0,
        };
        (1..=count)
            .filter_map(|i| {
                let name: String = names.raw_get(i).ok()?;
                let value: Value = vals.raw_get(i).unwrap_or(Value::Nil);
                (!name.starts_with('(')).then_some((name, value))
            })
            .collect::<Vec<_>>()
    };
    let locals = pairs(v.first(), v.get(1), v.get(2));
    let upvalues = pairs(v.get(3), v.get(4), v.get(5));
    let env = match v.get(6) {
        Some(Value::Table(t)) => Some(t.clone()),
        _ => None,
    };
    Some(FrameValues {
        locals,
        upvalues,
        env,
    })
}

/// The pointer of the environment of the function at `level` of `l` (0 if none).
///
/// # Safety
/// `l` is a live thread that is paused or running a debug hook.
pub(crate) unsafe fn env_pointer(l: *mut ffi::lua_State, level: c_int) -> usize {
    unsafe {
        if ffi::lua_checkstack(l, 2) == 0 {
            return 0;
        }
        let mut ar = std::mem::zeroed::<ffi::lua_Debug>();
        if ffi::lua_getinfo(l, level, c"f".as_ptr(), &mut ar) == 0 {
            return 0;
        }
        ffi::lua_getfenv(l, -1);
        let p = ffi::lua_topointer(l, -1) as usize;
        ffi::lua_pop(l, 2);
        p
    }
}

/// The pointer of the function at `level` of `l` (0 if none).
///
/// # Safety
/// `l` is a live thread that is paused or running a debug hook.
pub(crate) unsafe fn function_pointer(l: *mut ffi::lua_State, level: c_int) -> usize {
    unsafe {
        if ffi::lua_checkstack(l, 1) == 0 {
            return 0;
        }
        let mut ar = std::mem::zeroed::<ffi::lua_Debug>();
        if ffi::lua_getinfo(l, level, c"f".as_ptr(), &mut ar) == 0 {
            return 0;
        }
        let p = ffi::lua_topointer(l, -1) as usize;
        ffi::lua_pop(l, 1);
        p
    }
}

/// The script frames of `l`, innermost first, with their variables.
///
/// # Safety
/// `l` is a live thread running a debug hook or C function of this VM.
pub(crate) unsafe fn capture(lua: &Lua, d: &DebugCore, l: *mut ffi::lua_State) -> Vec<FrameSnap> {
    let mut frames = Vec::new();
    let depth = unsafe { ffi::lua_stackdepth(l) };
    for level in 0..depth {
        let mut ar = unsafe { std::mem::zeroed::<ffi::lua_Debug>() };
        if unsafe { ffi::lua_getinfo(l, level, c"sln".as_ptr(), &mut ar) } == 0 {
            break;
        }
        let chunk = c_text(ar.short_src);
        if chunk == "[C]" || chunk == "[engine]" || chunk.is_empty() {
            continue;
        }
        let what = c_text(ar.what);
        let function = match c_text(ar.name) {
            name if !name.is_empty() => name,
            _ if what == "main" || d.breakpoints.is_main(unsafe { function_pointer(l, level) }) => {
                "(main)".to_string()
            }
            _ => "(anonymous)".to_string(),
        };
        let owner = d.breakpoints.owner_of_env(unsafe { env_pointer(l, level) });
        let values = unsafe { frame_values(lua, l, level) };
        let (locals, upvalues, env) = match values {
            Some(v) => (v.locals, v.upvalues, v.env),
            None => (Vec::new(), Vec::new(), None),
        };
        frames.push(FrameSnap {
            info: FrameInfo {
                owner,
                chunk,
                function,
                line: u32::try_from(ar.currentline).unwrap_or(0),
            },
            state: l,
            level,
            locals,
            upvalues,
            env,
        });
    }
    frames
}

/// Writes `value` into the innermost local called `name` at `level` of `l`.
///
/// # Safety
/// `l` is a live thread that is paused or running a debug hook, on this VM.
pub(crate) unsafe fn write_local(
    lua: &Lua,
    l: *mut ffi::lua_State,
    level: c_int,
    name: &str,
    value: Value,
) -> bool {
    let mut done = false;
    let ran = unsafe {
        lua.exec_raw::<()>(value, |st| {
            if ffi::lua_checkstack(l, 2) == 0 {
                ffi::lua_settop(st, 0);
                return;
            }
            let mut found = 0;
            let mut n = 1;
            while n <= MAX_VARS {
                let local = ffi::lua_getlocal(l, level, n);
                if local.is_null() {
                    break;
                }
                ffi::lua_pop(l, 1);
                if CStr::from_ptr(local).to_bytes() == name.as_bytes() {
                    found = n;
                }
                n += 1;
            }
            if found > 0 && st != l {
                ffi::lua_xmove(st, l, 1);
                done = !ffi::lua_setlocal(l, level, found).is_null();
            }
            ffi::lua_settop(st, 0);
        })
    };
    ran.is_ok() && done
}

/// A short text for `value`. Tables are not asked for `__tostring` (script code).
pub(crate) fn display(s: &Scheduler, value: &Value) -> String {
    match value {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Number(n) => number_text(*n),
        Value::String(text) => {
            let mut t = text.to_string_lossy();
            if t.chars().count() > MAX_TEXT {
                t = t.chars().take(MAX_TEXT).collect::<String>() + "...";
            }
            format!("{t:?}")
        }
        Value::Table(t) => {
            let n = t.clone().pairs::<Value, Value>().count();
            format!("table ({n} {})", if n == 1 { "entry" } else { "entries" })
        }
        Value::Function(f) => match f.info().name {
            Some(name) => format!("function {name}"),
            None => "function".into(),
        },
        Value::Thread(_) => "thread".into(),
        Value::UserData(ud) => {
            if let Ok(inst) = ud.borrow::<LuaInstance>() {
                let class = inst
                    .state
                    .world()
                    .ok()
                    .and_then(|w| w.class_name(inst.id))
                    .unwrap_or("destroyed");
                return format!("{} ({class})", inst.name());
            }
            let text = s
                .lua()
                .ok()
                .and_then(|lua| lua.globals().get::<Function>("tostring").ok())
                .and_then(|f| {
                    s.debug.suppress.set(s.debug.suppress.get() + 1);
                    let r = f.call::<String>(value.clone()).ok();
                    s.debug.suppress.set(s.debug.suppress.get() - 1);
                    r
                });
            text.unwrap_or_else(|| "userdata".into())
        }
        Value::Vector(v) => format!("{}, {}, {}", v.x(), v.y(), v.z()),
        Value::Buffer(_) => "buffer".into(),
        Value::LightUserData(_) => "userdata".into(),
        Value::Error(e) => format_error(e),
        _ => value.type_name().into(),
    }
}

fn number_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// A row for `name = value`, with a handle when the value can be expanded.
fn node(s: &Scheduler, name: String, value: Value) -> VarNode {
    let display = display(s, &value);
    let expandable = match &value {
        Value::Table(_) => true,
        Value::UserData(ud) => ud.borrow::<LuaInstance>().is_ok(),
        _ => false,
    };
    let children = expandable.then(|| {
        let mut handles = s.debug.handles.borrow_mut();
        handles.push(value);
        VarHandle(u32::try_from(handles.len() - 1).unwrap_or(u32::MAX))
    });
    VarNode {
        name,
        display,
        children,
    }
}

/// Keeps the last of several variables with the same name (the innermost one in scope).
fn innermost(vars: &[(String, Value)]) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for (name, value) in vars {
        out.retain(|(n, _)| n != name);
        out.push((name.clone(), value.clone()));
    }
    out
}

impl DebugCore {
    /// Runs `f` with breaks suppressed.
    pub(crate) fn quiet<R>(&self, f: impl FnOnce() -> R) -> R {
        self.suppress.set(self.suppress.get() + 1);
        let r = f();
        self.suppress.set(self.suppress.get() - 1);
        r
    }

    fn with_frame<R>(&self, frame: usize, f: impl FnOnce(&FrameSnap) -> R) -> Option<R> {
        let paused = self.paused.borrow();
        let snap = paused.iter().flat_map(|b| b.frames.iter()).nth(frame)?;
        Some(f(snap))
    }

    /// The variables of `scope` in paused frame `frame` (0 = innermost).
    pub(crate) fn variables(&self, s: &Scheduler, frame: usize, scope: Scope) -> Vec<VarNode> {
        let vars = self.with_frame(frame, |snap| match scope {
            Scope::Locals => innermost(&snap.locals),
            Scope::Upvalues => snap.upvalues.clone(),
            Scope::Globals => snap
                .env
                .as_ref()
                .map(|env| {
                    let mut all: Vec<(String, Value)> = env
                        .clone()
                        .pairs::<Value, Value>()
                        .flatten()
                        .filter_map(|(k, v)| match k {
                            Value::String(k) => Some((k.to_string_lossy(), v)),
                            _ => None,
                        })
                        .collect();
                    all.sort_by(|a, b| a.0.cmp(&b.0));
                    all
                })
                .unwrap_or_default(),
        });
        vars.unwrap_or_default()
            .into_iter()
            .map(|(name, value)| node(s, name, value))
            .collect()
    }

    /// The children of an expanded table (raw entries, keys sorted) or Instance
    /// (`Name`, `ClassName`, `Parent`, then its children by name).
    pub(crate) fn expand(&self, s: &Scheduler, handle: VarHandle) -> Vec<VarNode> {
        let value = self.handles.borrow().get(handle.0 as usize).cloned();
        match value {
            Some(Value::Table(t)) => {
                let mut entries: Vec<(Value, Value)> = t
                    .pairs::<Value, Value>()
                    .flatten()
                    .take(MAX_VARS as usize)
                    .collect();
                entries.sort_by(|(a, _), (b, _)| key_order(a, b));
                entries
                    .into_iter()
                    .map(|(k, v)| node(s, key_text(&k), v))
                    .collect()
            }
            Some(Value::UserData(ud)) => {
                let Ok(inst) = ud.borrow::<LuaInstance>() else {
                    return Vec::new();
                };
                let Ok(lua) = s.lua() else { return Vec::new() };
                let (id, state, own_name) = (inst.id, inst.state.clone(), inst.name());
                drop(inst);
                let Ok(world) = state.world() else {
                    return Vec::new();
                };
                if !world.is_alive(id) {
                    return Vec::new();
                }
                let mut rows = vec![
                    VarNode {
                        name: "Name".into(),
                        display: format!("{own_name:?}"),
                        children: None,
                    },
                    VarNode {
                        name: "ClassName".into(),
                        display: format!("{:?}", world.class_name(id).unwrap_or("")),
                        children: None,
                    },
                ];
                let parent = world.parent(id);
                let children: Vec<_> = world.children(id).to_vec();
                drop(world);
                let parent = state.wrap_opt(&lua, parent).unwrap_or(Value::Nil);
                rows.push(node(s, "Parent".into(), parent));
                for child in children {
                    if let Ok(ud) = state.wrap(&lua, child) {
                        let name = match ud.borrow::<LuaInstance>() {
                            Ok(i) => i.name(),
                            Err(_) => continue,
                        };
                        rows.push(node(s, name, Value::UserData(ud)));
                    }
                }
                rows
            }
            _ => Vec::new(),
        }
    }

    /// Builds the environment a watch expression of `frame` runs in: its upvalues and
    /// locals, then the script's globals.
    fn frame_env(&self, lua: &Lua, frame: usize) -> Result<Table, ScriptError> {
        let to_err = |e: mlua::Error| ScriptError {
            message: format_error(&e),
        };
        let (locals, upvalues, env) = self
            .with_frame(frame, |snap| {
                (
                    innermost(&snap.locals),
                    snap.upvalues.clone(),
                    snap.env.clone(),
                )
            })
            .ok_or_else(|| ScriptError {
                message: "no such frame".into(),
            })?;
        let vars = lua.create_table().map_err(to_err)?;
        for (name, value) in upvalues.into_iter().chain(locals) {
            vars.raw_set(name, value).map_err(to_err)?;
        }
        let meta = lua.create_table().map_err(to_err)?;
        match env {
            Some(env) => meta.raw_set("__index", env).map_err(to_err)?,
            None => meta.raw_set("__index", lua.globals()).map_err(to_err)?,
        }
        vars.set_metatable(Some(meta)).map_err(to_err)?;
        Ok(vars)
    }

    /// Evaluates the Luau expression `expr` in paused frame `frame`, under the execution
    /// budget; returns its value.
    pub(crate) fn evaluate_value(
        &self,
        s: &Scheduler,
        frame: usize,
        expr: &str,
    ) -> Result<Value, ScriptError> {
        let to_err = |e: mlua::Error| ScriptError {
            message: format_error(&e),
        };
        let lua = s.lua().map_err(to_err)?;
        let env = self.frame_env(&lua, frame)?;
        let owner = self
            .with_frame(frame, |snap| snap.info.owner)
            .flatten()
            .unwrap_or(0);
        let f = lua
            .load(format!("return ({expr})"))
            .set_name("=[watch]")
            .set_environment(env)
            .into_function()
            .map_err(to_err)?;
        let values = self
            // Run as the frame's script: what the watch creates counts against its quotas.
            .quiet(|| s.call_guarded(&lua, owner, &f, ()))
            .map_err(|message| ScriptError { message })?;
        Ok(values.into_iter().next().unwrap_or(Value::Nil))
    }

    /// Like [`DebugCore::evaluate_value`], as text.
    pub(crate) fn evaluate(
        &self,
        s: &Scheduler,
        frame: usize,
        expr: &str,
    ) -> Result<String, ScriptError> {
        let value = self.evaluate_value(s, frame, expr)?;
        Ok(display(s, &value))
    }

    /// Sets local `name` of paused frame `frame` to the value of `expr`. The write to the
    /// innermost frame of a thread paused at an instruction happens when it resumes.
    pub(crate) fn set_local(
        &self,
        s: &Scheduler,
        frame: usize,
        name: &str,
        expr: &str,
    ) -> Result<(), ScriptError> {
        let value = self.evaluate_value(s, frame, expr)?;
        let lua = s.lua().map_err(|e| ScriptError {
            message: format_error(&e),
        })?;
        let mut paused = self.paused.borrow_mut();
        let mut index = frame;
        for broken in paused.iter_mut() {
            if index >= broken.frames.len() {
                index -= broken.frames.len();
                continue;
            }
            let snap = &mut broken.frames[index];
            if !snap.locals.iter().any(|(n, _)| n == name) {
                return Err(ScriptError {
                    message: format!("{name} is not a local of this frame"),
                });
            }
            if let Some(slot) = snap.locals.iter_mut().rev().find(|(n, _)| n == name) {
                // Luau locals are dynamically typed, so a new type is allowed; a warning says
                // so, because it is usually a slip (a string typed for a number).
                let luau_type = |v: &Value| match v {
                    Value::Integer(_) => "number",
                    v => v.type_name(),
                };
                let (was, now) = (luau_type(&slot.1), luau_type(&value));
                if was != now && !slot.1.is_nil() && !value.is_nil() {
                    s.sink.warn(&format!(
                        "debugger: {name} changed type from {was} to {now}"
                    ));
                }
                slot.1 = value.clone();
            }
            let (state, level) = (snap.state, snap.level);
            if matches!(broken.how, super::How::Hook { .. }) && level == 0 {
                broken.writes.push((level, name.to_string(), value));
                return Ok(());
            }
            // SAFETY: the paused thread is alive while it is in the chain.
            return if unsafe { write_local(&lua, state, level, name, value) } {
                Ok(())
            } else {
                Err(ScriptError {
                    message: format!("cannot set {name}"),
                })
            };
        }
        Err(ScriptError {
            message: "no such frame".into(),
        })
    }
}

fn key_text(k: &Value) -> String {
    match k {
        Value::String(s) => {
            let s = s.to_string_lossy();
            let ident = s
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && s.chars().all(|c| c.is_alphanumeric() || c == '_');
            if ident { s } else { format!("[{s:?}]") }
        }
        Value::Integer(n) => format!("[{n}]"),
        Value::Number(n) => format!("[{}]", number_text(*n)),
        Value::Boolean(b) => format!("[{b}]"),
        other => format!("[{}]", other.type_name()),
    }
}

fn key_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let num = |v: &Value| match v {
        Value::Integer(n) => Some(*n as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    };
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => key_text(a).cmp(&key_text(b)),
    }
}
