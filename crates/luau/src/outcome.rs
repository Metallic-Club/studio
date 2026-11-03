//! How a resume ended, as the host sees it: finished values, the reason of a yield, or a [`ScriptFailure`] with
//! the script's line and traceback.

use metallic_script::{Resume, ScriptFailure, YieldReason};
use mlua::thread::ThreadStatus;
use mlua::{Error, Lua, MultiValue, Thread, Value};

use crate::ffi::{self, SignalHandle};
use crate::shared::Shared;

/// The first value of a yield from `task.wait` (`[marker, WAIT, seconds]`) or `signal:Wait()`
/// (`[marker, WAIT_SIGNAL, signal]`); see the prelude in `library.rs`.
pub(crate) const WAIT: f64 = 1.0;
/// See [`WAIT`].
pub(crate) const WAIT_SIGNAL: f64 = 2.0;

/// The [`Resume`] for a thread whose resume returned `result`. Call it while the call's frame is published: an
/// error needs the chunk name.
pub(crate) fn outcome(lua: &Lua, shared: &Shared, thread: &Thread, result: mlua::Result<MultiValue>) -> Resume {
    match result {
        Ok(values) => match thread.status() {
            ThreadStatus::Resumable => Resume::Yielded(yield_reason(lua, &values)),
            _ => Resume::Finished(ffi::from_lua(lua, shared, values).unwrap_or_default()),
        },
        Err(error) => Resume::Errored(failure(shared, &error)),
    }
}

/// Why a thread yielded: the marked yields of `task.wait` and `signal:Wait()`, or [`YieldReason::Other`] for a
/// plain `coroutine.yield`.
fn yield_reason(lua: &Lua, values: &MultiValue) -> YieldReason {
    let mut values = values.iter();
    match values.next() {
        Some(Value::LightUserData(marker)) if *marker == ffi::yield_marker() => {}
        _ => return YieldReason::Other,
    }
    match number(values.next()) {
        Some(WAIT) => YieldReason::Wait {
            seconds: number(values.next()).unwrap_or(0.0),
        },
        Some(WAIT_SIGNAL) => match values.next().and_then(|v| ffi::payload_of::<SignalHandle>(lua, v)) {
            Some(signal) => YieldReason::WaitSignal {
                target: signal.target,
                signal: signal.signal,
            },
            None => YieldReason::Other,
        },
        _ => YieldReason::Other,
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// A Luau error as a [`ScriptFailure`]: a timeout if the interrupt raised it, out of memory, or a runtime error
/// with the chunk position taken off the message and the line kept.
pub(crate) fn failure(shared: &Shared, error: &Error) -> ScriptFailure {
    if let Some(line) = shared.timed_out.take() {
        return ScriptFailure::Timeout { line };
    }
    if is_memory(error) {
        return ScriptFailure::OutOfMemory;
    }
    let (message, traceback) = describe(error);
    let chunk = ffi::with_ctx(|ctx| ctx.chunk_name().to_owned()).ok();
    let (line, message) = match strip_position(&message, chunk.as_deref()) {
        Some((line, rest)) => (Some(line), rest.to_owned()),
        None => (
            traceback.as_deref().and_then(|t| traceback_line(t, chunk.as_deref())),
            message,
        ),
    };
    ScriptFailure::Runtime {
        message,
        line,
        traceback,
    }
}

fn is_memory(error: &Error) -> bool {
    match error {
        Error::MemoryError(_) => true,
        Error::CallbackError { cause, .. } | Error::WithContext { cause, .. } => is_memory(cause),
        _ => false,
    }
}

/// The message and the traceback (if any) of an error.
fn describe(error: &Error) -> (String, Option<String>) {
    match error {
        Error::RuntimeError(text) => match text.split_once("\nstack traceback:\n") {
            Some((message, traceback)) => (message.to_owned(), Some(traceback.to_owned())),
            None => (text.clone(), None),
        },
        Error::CallbackError { traceback, cause } => {
            let (message, inner) = describe(cause);
            let traceback = traceback.strip_prefix("stack traceback:\n").unwrap_or(traceback);
            (message, inner.or_else(|| Some(traceback.to_owned())))
        }
        Error::WithContext { cause, .. } => describe(cause),
        other => (other.to_string(), None),
    }
}

/// `chunk:12: message` → `(12, "message")`, when the position names the running chunk.
fn strip_position<'m>(message: &'m str, chunk: Option<&str>) -> Option<(u32, &'m str)> {
    let rest = message.strip_prefix(chunk?)?.strip_prefix(':')?;
    let (line, rest) = rest.split_once(':')?;
    let line = line.parse().ok()?;
    Some((line, rest.strip_prefix(' ').unwrap_or(rest)))
}

/// The first line of the running chunk in a traceback (`\tchunk:12: in function ...`).
fn traceback_line(traceback: &str, chunk: Option<&str>) -> Option<u32> {
    let chunk = chunk?;
    traceback.lines().find_map(|frame| {
        let at = frame.find(chunk)?;
        let rest = frame[at + chunk.len()..].strip_prefix(':')?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_taken_off_messages() {
        let chunk = "Scripts/Door.luau#4.1";
        assert_eq!(
            strip_position("Scripts/Door.luau#4.1:12: boom: bang", Some(chunk)),
            Some((12, "boom: bang"))
        );
        assert_eq!(strip_position("other:12: boom", Some(chunk)), None);
        assert_eq!(strip_position("boom", Some(chunk)), None);
    }

    #[test]
    fn traceback_lines_are_found() {
        let traceback = "\t[C]: in ?\n\tScripts/Door.luau#4.1:7: in function 'f'\n\tScripts/Door.luau#4.1:9: in ?";
        assert_eq!(traceback_line(traceback, Some("Scripts/Door.luau#4.1")), Some(7));
    }
}
