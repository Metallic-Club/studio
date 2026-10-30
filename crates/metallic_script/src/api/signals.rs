//! Signals (`:Connect`, `RBXScriptConnection:Disconnect`) and how the host fires them.
//!
//! A signal is identified by an instance and an [`EventKind`]. Each VM keeps its own
//! [`Hub`] of connected Lua functions. Firing takes a snapshot of the live connections
//! first, so a `Connect` made by a handler does not run in the same dispatch, and a
//! `Disconnect` made by a handler stops later handlers of that dispatch from running.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use metallic_core::InstanceId;
use mlua::{Function, Lua, MultiValue, UserData, UserDataFields, UserDataMethods, Value};

use super::ApiState;
use crate::vm::format_error;

/// The events scripts can connect to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EventKind {
    /// `BasePart.Touched(hit)`
    Touched,
    /// `Humanoid.Died()`
    Died,
    /// `Player.CharacterAdded(model)`
    CharacterAdded,
    /// `Players.PlayerAdded(player)`
    PlayerAdded,
    /// `RunService.Heartbeat(dt)`
    Heartbeat,
    /// `RemoteEvent.OnServerEvent(player, ...)`
    OnServerEvent,
    /// `RemoteEvent.OnClientEvent(...)`
    OnClientEvent,
}

/// One signal: an event of one instance.
pub(crate) type SignalKey = (InstanceId, EventKind);

struct Connection {
    id: u64,
    func: Function,
    alive: Rc<Cell<bool>>,
}

/// The connections of one VM.
#[derive(Default)]
pub(crate) struct Hub {
    connections: RefCell<HashMap<SignalKey, Vec<Connection>>>,
    next_id: Cell<u64>,
}

impl Hub {
    fn connect(&self, key: SignalKey, func: Function) -> (u64, Rc<Cell<bool>>) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let alive = Rc::new(Cell::new(true));
        self.connections
            .borrow_mut()
            .entry(key)
            .or_default()
            .push(Connection {
                id,
                func,
                alive: alive.clone(),
            });
        (id, alive)
    }

    fn disconnect(&self, key: SignalKey, id: u64) {
        if let Some(list) = self.connections.borrow_mut().get_mut(&key) {
            list.retain(|c| c.id != id);
        }
    }

    fn snapshot(&self, key: SignalKey) -> Vec<(Function, Rc<Cell<bool>>)> {
        self.connections
            .borrow()
            .get(&key)
            .map(|list| {
                list.iter()
                    .map(|c| (c.func.clone(), c.alive.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl ApiState {
    /// Calls every handler connected to `key`. `args` builds the arguments for one handler
    /// (called once per handler, so tables can be copied afresh). Each handler runs in its
    /// own thread under the execution budget; its errors go to the output sink.
    pub(crate) fn fire(&self, key: SignalKey, args: &dyn Fn(&Lua) -> mlua::Result<Vec<Value>>) {
        let handlers = self.hub.snapshot(key);
        if handlers.is_empty() {
            return;
        }
        let Some(lua) = self.lua.try_upgrade() else {
            return;
        };
        for (func, alive) in handlers {
            if !alive.get() {
                continue;
            }
            let values = match args(&lua) {
                Ok(values) => values,
                Err(e) => {
                    self.sink.error(&format_error(&e));
                    continue;
                }
            };
            let mut all = Vec::with_capacity(values.len() + 1);
            all.push(Value::Function(func));
            all.extend(values);
            if let Err(e) = self
                .budget
                .call::<MultiValue>(&self.spawn, MultiValue::from_vec(all))
            {
                self.sink.error(&e.message);
            }
        }
    }
}

/// The Luau `RBXScriptSignal`.
pub(crate) struct LuaSignal {
    pub(crate) state: Rc<ApiState>,
    pub(crate) key: SignalKey,
}

impl UserData for LuaSignal {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(mlua::MetaMethod::Type, "RBXScriptSignal");
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("Connect", |lua, this, func: Function| {
            let (id, alive) = this.state.hub.connect(this.key, func);
            lua.create_userdata(LuaConnection {
                state: this.state.clone(),
                key: this.key,
                id,
                alive,
            })
        });
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, _, ()| {
            Ok("RBXScriptSignal".to_string())
        });
    }
}

/// The Luau `RBXScriptConnection`.
pub(crate) struct LuaConnection {
    state: Rc<ApiState>,
    key: SignalKey,
    id: u64,
    alive: Rc<Cell<bool>>,
}

impl UserData for LuaConnection {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(mlua::MetaMethod::Type, "RBXScriptConnection");
        fields.add_field_method_get("Connected", |_, this| Ok(this.alive.get()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("Disconnect", |_, this, ()| {
            this.alive.set(false);
            this.state.hub.disconnect(this.key, this.id);
            Ok(())
        });
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, _, ()| {
            Ok("RBXScriptConnection".to_string())
        });
    }
}
