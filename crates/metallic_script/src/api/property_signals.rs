//! `Instance.Changed` and `Instance:GetPropertyChangedSignal(name)` (#410).
//!
//! Nothing runs inside `World::set`: a handler there would re-enter the world borrow and could
//! recurse without bound. Instead the host reads the world's change journal
//! ([`World::property_changes_since`](metallic_core::World::property_changes_since)) once per
//! `step` and fires the signals through the normal dispatch (budget, deferral, per-owner
//! cleanup). A write made by a handler lands in the journal after the cursor of that delivery,
//! so it fires on the next `step`.
//!
//! The cost is paid only for watched instances: a VM watches an instance from the moment one of
//! its scripts reads `inst.Changed` or calls `inst:GetPropertyChangedSignal`, until the
//! instance is destroyed. A VM that watches nothing does not read the journal at all. The
//! signals themselves are plain [`LuaSignal`]s keyed by [`EventKind::Changed`] and
//! [`EventKind::PropertyChanged`], so every signal method (`Connect`, `Disconnect`, and
//! whatever else `LuaSignal` offers) works on them.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use metallic_core::classes::{find_property, is_subclass};
use metallic_core::{ChangeCursor, InstanceId};
use mlua::{Function, Lua, UserDataRef, Value};

use super::instance::LuaInstance;
use super::signals::{EventKind, LuaSignal};
use super::{ApiState, lua_error};

/// Which instances one VM watches, and where its last delivery left the journal.
#[derive(Default)]
pub(crate) struct PropWatch {
    /// Watched instances with the journal position when watching began: changes made
    /// before it are not delivered.
    watched: RefCell<HashMap<InstanceId, ChangeCursor>>,
    /// Where the last delivery stopped reading; `None` while nothing is watched.
    cursor: Cell<Option<ChangeCursor>>,
    /// Journal entries read for delivery since the VM started (a test hook for the cost).
    scanned: Cell<u64>,
}

impl ApiState {
    /// Starts watching the properties of `id` for this VM (a no-op when already watched).
    pub(crate) fn watch_properties(&self, id: InstanceId) -> mlua::Result<()> {
        let now = self.world()?.change_cursor();
        self.props.watched.borrow_mut().entry(id).or_insert(now);
        if self.props.cursor.get().is_none() {
            self.props.cursor.set(Some(now));
        }
        Ok(())
    }

    /// How many journal entries property delivery has read so far.
    pub(crate) fn property_changes_scanned(&self) -> u64 {
        self.props.scanned.get()
    }

    /// Fires `GetPropertyChangedSignal(name)` and then `Changed(name)` for every real change
    /// of a watched, non-hidden property since the last call, oldest first.
    pub(crate) fn deliver_property_changes(self: &Rc<Self>) {
        let batch: Vec<(InstanceId, &'static str)> = {
            let Ok(world) = self.world() else { return };
            let mut watched = self.props.watched.borrow_mut();
            watched.retain(|&id, _| world.is_alive(id));
            let next = (!watched.is_empty()).then(|| world.change_cursor());
            let Some(start) = self.props.cursor.replace(next) else {
                return;
            };
            if watched.is_empty() {
                return;
            }
            let Some(changes) = world.property_changes_since(start) else {
                self.sink.warn(
                    "too many property changes in one step; some Changed events were dropped",
                );
                return;
            };
            let mut scanned = 0u64;
            let mut batch = Vec::new();
            for change in changes {
                scanned += 1;
                // Engine bookkeeping never reaches scripts (`PropDef::hidden`).
                if change.prop.hidden {
                    continue;
                }
                if watched
                    .get(&change.id)
                    .is_some_and(|since| change.at > *since)
                {
                    batch.push((change.id, change.prop.name));
                }
            }
            self.props.scanned.set(self.props.scanned.get() + scanned);
            batch
        };
        for (id, name) in batch {
            // A handler earlier in this batch may have destroyed it.
            if !self.world().is_ok_and(|w| w.is_alive(id)) {
                continue;
            }
            self.fire((id, EventKind::PropertyChanged(name)), &|_| Ok(Vec::new()));
            self.fire((id, EventKind::Changed), &|lua| {
                Ok(vec![Value::String(lua.create_string(name)?)])
            });
        }
    }
}

/// `Instance:GetPropertyChangedSignal(name)`.
pub(crate) fn methods(lua: &Lua) -> mlua::Result<Vec<(&'static str, Function)>> {
    let get = lua.create_function(|lua, (this, name): (UserDataRef<LuaInstance>, String)| {
        let class = this.class()?;
        let def = match find_property(class, &name) {
            Some(def) if !def.hidden => def,
            _ if name == "Parent" || (name == "CFrame" && is_subclass(class, "BasePart")) => {
                return Err(lua_error(format!(
                    "GetPropertyChangedSignal(\"{name}\") is not supported yet"
                )));
            }
            _ => return Err(lua_error(format!("{name} is not a valid property name."))),
        };
        this.state.watch_properties(this.id)?;
        lua.create_userdata(LuaSignal {
            state: this.state.clone(),
            key: (this.id, EventKind::PropertyChanged(def.name)),
        })
    })?;
    Ok(vec![("GetPropertyChangedSignal", get)])
}
