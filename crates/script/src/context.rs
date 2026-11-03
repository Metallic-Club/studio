//! `HostContext`: everything a backend sees during one call, and the requests it leaves for the host.

use std::sync::Arc;

use metallic_core::{
    Args, ConnectionId, DataModel, DmError, Id, ListenerKey, MethodCall, PropFlags, Registry, ScriptError, ScriptValue,
};

use crate::host::{ListenerRecord, Listeners, ScriptRecord};
use crate::{Deadline, LanguageId, OutputSink, OutputSource, ThreadId};

/// Something a backend asks the host to do after the current call returns. The scheduler (#45) adds the `task`
/// library's requests here; until then there is one.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HostRequest {
    /// The backend created another thread for the running script (a coroutine, a new task...). The host tracks
    /// it from now on, so [`ScriptHost::reap`](crate::ScriptHost::reap) releases it with the script.
    AdoptThread(ThreadId),
}

/// A connection made through [`HostContext::connect`]: the key the backend stores its function under, and the
/// data model's connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The key the host chose. Deliveries name it, and [`ScriptLanguage::spawn_listener`] receives it.
    ///
    /// [`ScriptLanguage::spawn_listener`]: crate::ScriptLanguage::spawn_listener
    pub listener: ListenerKey,
    /// The data model's connection, for `signal:Wait()` ([`YieldReason::WaitSignal`]).
    ///
    /// [`YieldReason::WaitSignal`]: crate::YieldReason::WaitSignal
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
        let listener = self.listeners.next_key();
        let connection = self.dm.connect(target, signal, listener).map_err(dm_error)?;
        Ok(self.adopt(listener, connection))
    }

    /// Connects to changes of the property `prop` of `target` (`GetPropertyChangedSignal(prop):Connect` in
    /// scripts). Otherwise like [`HostContext::connect`].
    pub fn connect_property_changed(&mut self, target: Id, prop: &str) -> Result<Connection, ScriptError> {
        let prop = self.dm.prop_id(target, prop).map_err(dm_error)?;
        let listener = self.listeners.next_key();
        let connection = self
            .dm
            .connect_property_changed(target, prop, listener)
            .map_err(dm_error)?;
        Ok(self.adopt(listener, connection))
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
    fn adopt(&mut self, listener: ListenerKey, connection: ConnectionId) -> Connection {
        self.listeners.map.insert(
            listener,
            ListenerRecord {
                script: self.script,
                language: self.language,
                connection,
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

/// A data model error as a script sees it.
fn dm_error(error: DmError) -> ScriptError {
    ScriptError::runtime(error.to_string())
}
