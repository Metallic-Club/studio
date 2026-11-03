//! `HostContext`: everything a backend sees during one call, and the requests it leaves for the host.

use std::sync::Arc;

use metallic_core::{
    Args, ConnectionId, DataModel, DmError, Id, ListenerKey, MethodCall, PropFlags, Registry, ScriptError, ScriptValue,
};

use crate::host::{ListenerRecord, Listeners, ScriptRecord};
use crate::{
    Deadline, LanguageId, MAX_DEFER_DEPTH, OutputSink, OutputSource, Resume, SignalRef, ThreadId, YieldReason,
};

/// The connection of a `task.spawn`ed thread that yielded in `signal:Wait()` during a call, made at once so events
/// the caller fires later in the same call reach it. The host's waiter takes it over after the call.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EarlyWait {
    pub(crate) thread: ThreadId,
    pub(crate) listener: ListenerKey,
    pub(crate) connection: ConnectionId,
}

/// Something a backend asks the host to do after the current call returns, in the order asked. The `task` library
/// lands here; [`HostContext`] has a method for each request, which backends should prefer.
///
/// Every thread named here belongs to the backend being called. One the host does not track yet is adopted as the
/// running script's; one it already tracks first leaves whatever it was waiting for (a sleep, the deferred queue,
/// a signal).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HostRequest {
    /// The backend created another thread for the running script (a coroutine...). The host tracks it from now
    /// on, so [`ScriptHost::reap`](crate::ScriptHost::reap) releases it with the script. It is never resumed on
    /// the host's own.
    AdoptThread(ThreadId),
    /// `task.spawn`: the backend already resumed `thread` nested in this call, and it ended with `resume`. The host
    /// treats that like its own resume: a finished thread is released, a failed one reported and released, a
    /// yielded one scheduled by its [`YieldReason`](crate::YieldReason).
    Spawned {
        /// The spawned thread.
        thread: ThreadId,
        /// How its nested resume ended.
        resume: Resume,
    },
    /// `task.defer`: resume `thread` with `args` at the end of the current resumption point.
    Defer {
        /// The thread to resume.
        thread: ThreadId,
        /// What to resume it with.
        args: Vec<ScriptValue>,
    },
    /// `task.delay`: resume `thread` with `args` after `seconds` of game time, at the earliest at the next
    /// `Heartbeat`.
    Delay {
        /// The thread to resume.
        thread: ThreadId,
        /// How long to wait. Zero, negative and NaN mean "at the next `Heartbeat`".
        seconds: f64,
        /// What to resume it with.
        args: Vec<ScriptValue>,
    },
    /// `task.cancel`: unschedule and release `thread`. It never runs again.
    Cancel(ThreadId),
}

/// A connection made through [`HostContext::connect`] or [`HostContext::once`]: the key the backend stores its
/// function under, and the data model's connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The key the host chose. Deliveries name it, and [`ScriptLanguage::spawn_listener`] receives it.
    ///
    /// [`ScriptLanguage::spawn_listener`]: crate::ScriptLanguage::spawn_listener
    pub listener: ListenerKey,
    /// The data model's connection.
    pub connection: ConnectionId,
}

/// What a backend sees during one call (`spawn`, `spawn_listener` or `resume`): the data model, the running
/// script, the call's deadline, the output, and the host's connection and request tables.
///
/// It lives only for that call. A backend keeps [`Id`]s, [`ListenerKey`]s and its own handles between calls, never
/// a reference taken from here.
pub struct HostContext<'a> {
    /// The data model scripts run against. Property reads and writes, `MethodCall`s and instance creation all go
    /// through it. Connections must go through [`HostContext::connect`] instead, so the host can reap them.
    pub dm: &'a mut DataModel,
    /// When this call must stop. Poll it at safepoints with [`Deadline::poll`].
    pub deadline: Deadline,
    /// The script the running code belongs to.
    script: Id,
    /// The backend being called.
    language: LanguageId,
    /// The running script's entry in the host.
    record: &'a mut ScriptRecord,
    /// Every listener of every script, to add and remove connections.
    listeners: &'a mut Listeners,
    /// Requests for the host, handled after the call returns.
    pending: &'a mut Vec<HostRequest>,
    /// Where `print` and `warn` go.
    output: &'a mut dyn OutputSink,
    /// How many deferred resumes deep the running code is (0 outside the deferred queue).
    defer_depth: u32,
    /// Waiter connections made during this call.
    early_waits: &'a mut Vec<EarlyWait>,
}

impl<'a> HostContext<'a> {
    /// Builds the context of one call. Only the host makes these.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        dm: &'a mut DataModel,
        deadline: Deadline,
        script: Id,
        language: LanguageId,
        record: &'a mut ScriptRecord,
        listeners: &'a mut Listeners,
        pending: &'a mut Vec<HostRequest>,
        output: &'a mut dyn OutputSink,
        defer_depth: u32,
        early_waits: &'a mut Vec<EarlyWait>,
    ) -> Self {
        Self {
            dm,
            deadline,
            script,
            language,
            record,
            listeners,
            pending,
            output,
            defer_depth,
            early_waits,
        }
    }

    /// The script instance the running code belongs to (`script` in Luau).
    pub fn script(&self) -> Id {
        self.script
    }

    /// The script's full name when it started, such as `Workspace.Door.Script`.
    pub fn script_name(&self) -> &str {
        &self.record.name
    }

    /// The script's chunk name, `path#uid` (such as `Scripts/Door.luau#12.1`). Backends name the script's
    /// function after it, so tracebacks and the profiler can tell two scripts sharing a file apart.
    pub fn chunk_name(&self) -> &str {
        &self.record.chunk_name
    }

    /// The class registry: classes, properties, methods, signals and enums, for bindings.
    pub fn registry(&self) -> &Arc<Registry> {
        self.dm.registry()
    }

    /// Prints `message` to the output, as the running script.
    pub fn print(&mut self, message: &str) {
        let from = source(self.record);
        self.output.print(&from, message);
    }

    /// Prints `message` to the output as a warning, as the running script.
    pub fn warn(&mut self, message: &str) {
        let from = source(self.record);
        self.output.warn(&from, message);
    }

    /// Asks the host for something it handles after this call returns.
    pub fn request(&mut self, request: HostRequest) {
        self.pending.push(request);
    }

    /// `task.spawn`: reports that the backend resumed `thread` nested in this call and how that ended
    /// ([`HostRequest::Spawned`]).
    ///
    /// A thread that yielded in `signal:Wait()` is connected at once, so an event the caller fires later in this
    /// call wakes it, as in Roblox. (A `Spawned` pushed through [`HostContext::request`] is connected only after
    /// the call.)
    pub fn spawned(&mut self, thread: ThreadId, resume: Resume) {
        if let Resume::Yielded(YieldReason::WaitSignal { target, signal }) = resume {
            let listener = self.listeners.next_key();
            // A failure is reported when the host connects again after the call.
            if let Ok(connection) = connect_signal(self.dm, target, signal, listener) {
                self.early_waits.push(EarlyWait {
                    thread,
                    listener,
                    connection,
                });
            }
        }
        self.pending.push(HostRequest::Spawned { thread, resume });
    }

    /// `task.defer`: resumes `thread` with `args` at the end of the current resumption point
    /// ([`HostRequest::Defer`]).
    ///
    /// Refuses when the running code is itself [`MAX_DEFER_DEPTH`] deferred resumes deep (deferred work that keeps
    /// deferring more would never end); the backend raises the error in the calling thread.
    pub fn defer(&mut self, thread: ThreadId, args: Vec<ScriptValue>) -> Result<(), ScriptError> {
        if self.defer_depth >= MAX_DEFER_DEPTH {
            return Err(ScriptError::runtime(defer_depth_message()));
        }
        self.pending.push(HostRequest::Defer { thread, args });
        Ok(())
    }

    /// `task.delay`: resumes `thread` with `args` after `seconds` of game time ([`HostRequest::Delay`]).
    pub fn delay(&mut self, seconds: f64, thread: ThreadId, args: Vec<ScriptValue>) {
        self.pending.push(HostRequest::Delay { thread, seconds, args });
    }

    /// `task.cancel`: unschedules and releases `thread` ([`HostRequest::Cancel`]). The backend treats it as dead
    /// from now on.
    pub fn cancel(&mut self, thread: ThreadId) {
        self.pending.push(HostRequest::Cancel(thread));
    }

    /// Calls the method `name` on `this` through the class's `MethodDesc`, the path every language uses.
    pub fn call_method(&mut self, this: Id, name: &str, args: Args) -> Result<ScriptValue, ScriptError> {
        let class = self.dm.class_of(this).map_err(dm_error)?;
        let Some(method) = self.dm.registry().method(class, name) else {
            let class = self.dm.class_name(this).map_err(dm_error)?;
            return Err(ScriptError::runtime(format!("{name} is not a valid member of {class}")));
        };
        let mut call = MethodCall {
            this,
            args,
            dm: &mut *self.dm,
        };
        (method.func)(&mut call)
    }

    /// Reads the property `name` of `id` as a script sees it.
    pub fn get_property(&self, id: Id, name: &str) -> Result<ScriptValue, ScriptError> {
        let prop = self.dm.prop_id(id, name).map_err(dm_error)?;
        self.dm.get(id, prop).map(ScriptValue::from).map_err(dm_error)
    }

    /// Writes the property `name` of `id` from a script, converting `value` to the property's type. Read-only
    /// properties refuse.
    pub fn set_property(&mut self, id: Id, name: &str, value: &ScriptValue) -> Result<(), ScriptError> {
        let prop = self.dm.prop_id(id, name).map_err(dm_error)?;
        let class = self.dm.class_of(id).map_err(dm_error)?;
        let Some(resolved) = self.dm.registry().prop(class, prop) else {
            return Err(ScriptError::runtime(format!("{name} is not a valid property")));
        };
        let meta = resolved.meta;
        if meta.flags.contains(PropFlags::READ_ONLY) {
            return Err(ScriptError::runtime(format!("{} is read-only", meta.name)));
        }
        let value = value.to_value(&meta.ty)?;
        self.dm.set(id, prop, value).map_err(dm_error)
    }

    /// Connects the signal `signal` of `target` (`Connect` in scripts). The host picks the listener key and
    /// records that the running script owns it; the backend stores its function under
    /// [`Connection::listener`].
    pub fn connect(&mut self, target: Id, signal: &str) -> Result<Connection, ScriptError> {
        let signal = self.dm.signal_id(target, signal).map_err(dm_error)?;
        self.connect_to(target, SignalRef::Signal(signal), false)
    }

    /// Connects to changes of the property `prop` of `target` (`GetPropertyChangedSignal(prop):Connect` in
    /// scripts). Otherwise like [`HostContext::connect`].
    pub fn connect_property_changed(&mut self, target: Id, prop: &str) -> Result<Connection, ScriptError> {
        let prop = self.dm.prop_id(target, prop).map_err(dm_error)?;
        self.connect_to(target, SignalRef::PropertyChanged(prop), false)
    }

    /// Connects the signal `signal` of `target` for one event only (`Once` in scripts). The host disconnects it
    /// before running the listener, and releases the listener right after spawning its thread. Otherwise like
    /// [`HostContext::connect`]; it can be disconnected before it fires.
    pub fn once(&mut self, target: Id, signal: &str) -> Result<Connection, ScriptError> {
        let signal = self.dm.signal_id(target, signal).map_err(dm_error)?;
        self.connect_to(target, SignalRef::Signal(signal), true)
    }

    /// Connects `signal` of `target`, already resolved, for every event or (`once`) only the next one.
    pub fn connect_to(&mut self, target: Id, signal: SignalRef, once: bool) -> Result<Connection, ScriptError> {
        let listener = self.listeners.next_key();
        let connection = connect_signal(self.dm, target, signal, listener).map_err(dm_error)?;
        Ok(self.adopt(listener, connection, once))
    }

    /// Ends the connection of `listener` (`Disconnect` in scripts) and forgets it. The backend drops its own
    /// function for the key; the host will not call [`ScriptLanguage::release_listener`] for it. Returns `false`
    /// when the key is unknown, already disconnected, or belongs to another language.
    ///
    /// [`ScriptLanguage::release_listener`]: crate::ScriptLanguage::release_listener
    pub fn disconnect(&mut self, listener: ListenerKey) -> bool {
        if !self
            .listeners
            .map
            .get(&listener)
            .is_some_and(|r| r.language == self.language)
        {
            return false;
        }
        let Some(record) = self.listeners.map.remove(&listener) else {
            return false;
        };
        // A connection object may travel to another script of the same language; only the owner lists the key.
        if record.script == self.script {
            self.record.listeners.remove(&listener);
        }
        self.dm.disconnect(record.connection)
    }

    /// Records a new connection as the running script's.
    fn adopt(&mut self, listener: ListenerKey, connection: ConnectionId, once: bool) -> Connection {
        self.listeners.map.insert(
            listener,
            ListenerRecord {
                script: self.script,
                language: self.language,
                connection,
                once,
            },
        );
        self.record.listeners.insert(listener);
        Connection { listener, connection }
    }
}

/// The output source for lines a running script writes itself.
fn source(record: &ScriptRecord) -> OutputSource<'_> {
    OutputSource {
        script: &record.name,
        chunk: &record.chunk_name,
        line: None,
    }
}

/// Connects `listener` to `signal` of `target` in the data model.
pub(crate) fn connect_signal(
    dm: &mut DataModel,
    target: Id,
    signal: SignalRef,
    listener: ListenerKey,
) -> Result<ConnectionId, DmError> {
    match signal {
        SignalRef::Signal(signal) => dm.connect(target, signal, listener),
        SignalRef::PropertyChanged(prop) => dm.connect_property_changed(target, prop, listener),
    }
}

/// The error of a `task.defer` past [`MAX_DEFER_DEPTH`], worded as Roblox words it.
pub(crate) fn defer_depth_message() -> String {
    format!("maximum re-entrancy depth ({MAX_DEFER_DEPTH}) exceeded calling task.defer")
}

/// A data model error as a script sees it.
fn dm_error(error: DmError) -> ScriptError {
    ScriptError::runtime(error.to_string())
}
