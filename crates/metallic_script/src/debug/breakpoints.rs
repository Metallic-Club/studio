//! Breakpoints: kept per script owner, patched into each loaded script's bytecode with
//! `lua_breakpoint`, and checked (conditions, logpoints) when a patched line runs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_int;

use mlua::{Function, Lua, Table, Value, ffi};

use super::Breakpoint;
use super::inspect;
use crate::task_lib::Scheduler;
use crate::vm::format_error;

/// A compiled condition or log message and the metatable of its environment proxy.
type Check = (Function, Table);

/// One breakpoint of one script.
struct Bp {
    /// The line asked for.
    line: u32,
    kind: Breakpoint,
    /// The line Luau patched (the next line with code), per load; None if none.
    actual: Option<c_int>,
    /// Compiled condition or log expressions, with the proxy table their environment's
    /// `__index` is pointed at for each hit.
    check: Option<Check>,
    /// Text parts of a log message, around the expression values.
    parts: Vec<String>,
}

/// A loaded script: its main function (to patch) and environment (to attribute frames).
struct Loaded {
    owner: u64,
    env_ptr: usize,
    _env: Table,
    main: Function,
}

/// The breakpoints of one VM and the scripts they apply to.
#[derive(Default)]
pub(crate) struct BreakpointSet {
    by_owner: RefCell<HashMap<u64, Vec<Bp>>>,
    loaded: RefCell<Vec<Loaded>>,
}

/// Patches (or restores) `line` of `main` and the functions in it; the patched line.
fn patch(lua: &Lua, main: &Function, line: c_int, enable: bool) -> Option<c_int> {
    let mut actual = -1;
    // SAFETY: the stack holds exactly the Lua function `main`; it is popped again.
    let ok = unsafe {
        lua.exec_raw::<()>(main.clone(), |st| {
            actual = ffi::lua_breakpoint(st, -1, line, c_int::from(enable));
            ffi::lua_settop(st, 0);
        })
    };
    (ok.is_ok() && actual >= 0).then_some(actual)
}

/// Splits a log message into text and `{expression}` parts.
fn split_log(message: &str) -> (Vec<String>, Vec<String>) {
    let mut parts = vec![String::new()];
    let mut exprs = Vec::new();
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        if let Some(last) = parts.last_mut() {
            last.push_str(&rest[..open]);
        }
        exprs.push(rest[open + 1..open + close].to_string());
        parts.push(String::new());
        rest = &rest[open + close + 1..];
    }
    if let Some(last) = parts.last_mut() {
        last.push_str(rest);
    }
    (parts, exprs)
}

impl BreakpointSet {
    pub(crate) fn is_empty(&self) -> bool {
        self.by_owner.borrow().values().all(Vec::is_empty)
    }

    pub(crate) fn clear(&self) {
        self.by_owner.borrow_mut().clear();
        self.loaded.borrow_mut().clear();
    }

    /// The owner of the script whose environment is at `env_ptr`.
    pub(crate) fn owner_of_env(&self, env_ptr: usize) -> Option<u64> {
        if env_ptr == 0 {
            return None;
        }
        self.loaded
            .borrow()
            .iter()
            .find(|l| l.env_ptr == env_ptr)
            .map(|l| l.owner)
    }

    /// Whether the function at `fn_ptr` is the body of a loaded script.
    pub(crate) fn is_main(&self, fn_ptr: usize) -> bool {
        fn_ptr != 0
            && self
                .loaded
                .borrow()
                .iter()
                .any(|l| l.main.to_pointer() as usize == fn_ptr)
    }

    /// Records a script that was just compiled and patches its breakpoints.
    pub(crate) fn on_load(&self, lua: &Lua, owner: u64, main: &Function, env: &Table) {
        self.loaded.borrow_mut().push(Loaded {
            owner,
            env_ptr: env.to_pointer() as usize,
            _env: env.clone(),
            main: main.clone(),
        });
        if let Some(bps) = self.by_owner.borrow_mut().get_mut(&owner) {
            for bp in bps {
                bp.actual = patch(lua, main, bp.line as c_int, true);
            }
        }
    }

    /// Replaces the breakpoints of `owner`. Returns warnings for conditions or log
    /// expressions that do not compile (those breakpoints pause plainly).
    pub(crate) fn set_owner(
        &self,
        lua: &Lua,
        owner: u64,
        list: &[(u32, Breakpoint)],
    ) -> Vec<String> {
        let mains: Vec<Function> = self
            .loaded
            .borrow()
            .iter()
            .filter(|l| l.owner == owner)
            .map(|l| l.main.clone())
            .collect();
        let old = self
            .by_owner
            .borrow_mut()
            .remove(&owner)
            .unwrap_or_default();
        for bp in &old {
            if let Some(actual) = bp.actual {
                for main in &mains {
                    patch(lua, main, actual, false);
                }
            }
        }
        let mut warnings = Vec::new();
        let mut bps = Vec::new();
        for (line, kind) in list {
            let (check, parts) = match compile(lua, kind) {
                Ok(c) => c,
                Err(e) => {
                    warnings.push(format!("breakpoint at line {line}: {e}"));
                    (None, Vec::new())
                }
            };
            let mut actual = None;
            for main in &mains {
                actual = patch(lua, main, *line as c_int, true);
            }
            bps.push(Bp {
                line: *line,
                kind: kind.clone(),
                actual,
                check,
                parts,
            });
        }
        if !bps.is_empty() {
            self.by_owner.borrow_mut().insert(owner, bps);
        }
        warnings
    }

    /// The line Luau put the breakpoint of `owner` at `line` on, once its script loaded.
    pub(crate) fn actual_line(&self, owner: u64, line: u32) -> Option<u32> {
        self.by_owner
            .borrow()
            .get(&owner)?
            .iter()
            .find(|bp| bp.line == line)?
            .actual
            .and_then(|a| u32::try_from(a).ok())
    }
}

/// Compiles a condition (`return (expr)`) or log message (`return (e1), (e2), ...`) with
/// a proxy environment.
fn compile(lua: &Lua, kind: &Breakpoint) -> mlua::Result<(Option<Check>, Vec<String>)> {
    let (source, parts) = match kind {
        Breakpoint::Plain => return Ok((None, Vec::new())),
        Breakpoint::Condition(expr) => (format!("return ({expr})"), Vec::new()),
        Breakpoint::Log(message) => {
            let (parts, exprs) = split_log(message);
            if exprs.is_empty() {
                return Ok((None, parts));
            }
            let list: Vec<String> = exprs.iter().map(|e| format!("({e})")).collect();
            (format!("return {}", list.join(", ")), parts)
        }
    };
    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    proxy.set_metatable(Some(meta.clone()))?;
    let f = lua
        .load(source)
        .set_name("=[breakpoint]")
        .set_environment(proxy)
        .into_function()?;
    Ok((Some((f, meta)), parts))
}

/// A breakpoint of the script running at level 0 of `l` was hit on `line`: runs its
/// condition or log message and says whether to pause.
///
/// # Safety
/// Called from the `debugbreak` hook of `l`.
pub(crate) unsafe fn check_hit(
    lua: &Lua,
    s: &Scheduler,
    l: *mut ffi::lua_State,
    line: c_int,
) -> bool {
    let d = &s.debug;
    let Some(owner) = d
        .breakpoints
        .owner_of_env(unsafe { inspect::env_pointer(l, 0) })
    else {
        return false;
    };
    let hit = {
        let by_owner = d.breakpoints.by_owner.borrow();
        by_owner.get(&owner).and_then(|bps| {
            bps.iter()
                .find(|bp| bp.actual == Some(line))
                .map(|bp| (bp.kind.clone(), bp.check.clone(), bp.parts.clone()))
        })
    };
    let Some((kind, check, parts)) = hit else {
        return false;
    };
    let run = |check: &Check| -> mlua::Result<Vec<Value>> {
        let vars = lua.create_table()?;
        if let Some(values) = unsafe { inspect::frame_values(lua, l, 0) } {
            for (name, value) in values.upvalues.into_iter().chain(values.locals) {
                vars.raw_set(name, value)?;
            }
            let meta = lua.create_table()?;
            match values.env {
                Some(env) => meta.raw_set("__index", env)?,
                None => meta.raw_set("__index", lua.globals())?,
            }
            vars.set_metatable(Some(meta))?;
        }
        check.1.raw_set("__index", vars)?;
        let out = d.quiet(|| check.0.call::<mlua::MultiValue>(()));
        check.1.raw_set("__index", Value::Nil)?;
        Ok(out?.into_iter().collect())
    };
    match kind {
        Breakpoint::Plain => true,
        Breakpoint::Condition(expr) => match check.as_ref().map(run) {
            Some(Ok(values)) => values
                .first()
                .is_some_and(|v| !matches!(v, Value::Nil | Value::Boolean(false))),
            Some(Err(e)) => {
                s.sink.warn(&format!(
                    "breakpoint condition `{expr}` failed: {}",
                    format_error(&e)
                ));
                true
            }
            None => true,
        },
        Breakpoint::Log(_) => {
            let values = match check.as_ref().map(run) {
                Some(Ok(values)) => values,
                Some(Err(e)) => {
                    s.sink
                        .warn(&format!("logpoint failed: {}", format_error(&e)));
                    return false;
                }
                None => Vec::new(),
            };
            let mut text = String::new();
            for (i, part) in parts.iter().enumerate() {
                text.push_str(part);
                if i + 1 < parts.len() {
                    match values.get(i) {
                        Some(Value::String(v)) => text.push_str(&v.to_string_lossy()),
                        Some(v) => text.push_str(&inspect::display(s, v)),
                        None => text.push_str("nil"),
                    }
                }
            }
            s.sink.print(&text);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::split_log;

    #[test]
    fn log_message_splits_into_text_and_expressions() {
        let (parts, exprs) = split_log("x = {x}, y = {t.y}!");
        assert_eq!(parts, ["x = ", ", y = ", "!"]);
        assert_eq!(exprs, ["x", "t.y"]);
        let (parts, exprs) = split_log("plain {unclosed");
        assert_eq!(parts, ["plain {unclosed"]);
        assert!(exprs.is_empty());
    }
}
