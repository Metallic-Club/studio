//! Signals: connections, the built-in events, firing from class code, and deferred delivery (ADR 0002 §5).
//!
//! # Language-neutral connections
//!
//! Core never stores a closure. A connection is an instance, a signal (or one property, for
//! `GetPropertyChangedSignal`) and an opaque [`ListenerKey`] chosen by the caller: the script host maps keys to
//! script functions of any language, and tests use plain numbers. Delivering an event means handing the caller
//! `(connection, listener, source, args)`; running the listener is the caller's job.
//!
//! # What an event costs when nothing listens
//!
//! Every change the data model makes reports once to `DataModel::notify`, which turns it into events here. Each
//! slot carries a "watched" flag, set while the instance has at least one connection, so a change to an unwatched
//! instance costs that one check. The events that fire on every ancestor (`DescendantAdded`,
//! `DescendantRemoving`) or on every descendant (`AncestryChanged`) also keep a count of their connections in the
//! whole data model, and skip the walk entirely while it is zero.
//!
//! # Derived properties
//!
//! A computed property declared with `PropMeta::derived_from` (such as `Position`, derived from `CFrame`) fires
//! with its sources: when a stored property changes on a watched instance, its own `Changed` and property signal
//! are queued first, then those of each computed property derived from it, in declaration order. A write to
//! `CFrame` therefore fires `CFrame`, `Position`, `Orientation`, through both `Changed` and the property signals.
//!
//! **No double firing:** writing a computed property fires nothing by itself. Its setter writes the stored
//! properties, and only those writes fire, each with its dependents. Writing `Position` writes `CFrame` once, so it
//! fires exactly what a `CFrame` write fires, each property once; a write that leaves `CFrame` unchanged fires
//! nothing. A setter that writes two sources of the same computed property fires it once per changed source.
//! Dependents fire whenever a source changes, without comparing their own value: writing `Orientation` also fires
//! `Position` even though the position stayed. Comparing would mean running every getter on every watched write.
//!
//! # Value objects
//!
//! A class declared with `ClassDesc::changed_passes_value("Value")` (in Roblox, `ValueBase`: `IntValue`,
//! `NumberValue`, `ObjectValue`, ...) fires `Changed(newValue)` instead of `Changed(propertyName)`, and only when
//! that property changes. The new value is passed as a script sees it: an integer as a number, a reference as the
//! instance or nil. Every other change on such an instance, renames and moves included, fires no `Changed`, as in
//! Roblox, where `ValueBase.Changed` replaces `Instance.Changed`; each property's own property-changed signal
//! (`GetPropertyChangedSignal("Name")`) still fires as usual, `Value`'s included.
//!
//! # Deferred delivery
//!
//! An event is queued only when a connection listens to that signal on that instance, with its arguments, in fire
//! order. [`DataModel::drain_deliveries`] turns the queue into one [`Delivery`] per connection that existed when the
//! event fired, in connection order, and empties it. A connection made after an event fired does not receive it,
//! and one disconnected before the drain receives nothing.
//!
//! **Re-entrancy:** the drain only hands deliveries out; the caller runs the listeners after it returns. A listener
//! that changes the data model queues new events for the *next* drain, so delivery never recurses and a listener
//! never sees the data model half-way through another change.
//!
//! # Destroyed instances ("graves")
//!
//! An instance destroyed while something is connected anywhere leaves a grave: its class name and name, readable
//! through [`DataModel::grave`] while the events about it are delivered, so a handler can still print `part.Name`.
//! A grave lasts until the drain after the one that delivered those events begins. The instance's connections
//! receive the events queued up to and including its destruction (`Destroying`, `AncestryChanged`, ...), then the
//! drain that delivers them drops the connections.
//!
//! # Dropped connections
//!
//! Every connection ends exactly once, in one of two ways, so a caller that keeps something per connection (the
//! script host's listener records and the backend's function references) can release it exactly once:
//!
//! - The caller ends it: [`DataModel::disconnect`] returns `true`. Nothing else reports it.
//! - The data model drops it on its own: today, when its instance is destroyed. The drain that delivers the
//!   instance's last events drops the connection and records `(ConnectionId, ListenerKey)`;
//!   [`DataModel::drain_dropped_connections`] hands the records out. A connection made and destroyed before any
//!   drain is reported too, and so is every connection in a destroyed subtree.
//!
//! Disconnecting a connection whose instance is destroyed but not yet drained still returns `true`, and the drain
//! then does not report it. The intended frame loop is: `drain_deliveries`, run the listeners (their last events
//! included), then `drain_dropped_connections` and release what it lists. The records wait in the data model until
//! they are drained, so nothing is lost if a frame skips the call; a host that never calls it keeps them all.

use std::sync::Arc;

use super::{DataModel, DmError, Notice};
use crate::registry::hash::FxMap;
use crate::{ClassId, Id, PropId, ScriptValue, SignalId};

/// The signals the data model fires itself, from the changes it makes. Classes declare them (on `Object` and
/// `Instance`) under exactly these names; a class that does not declare one simply never fires it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BuiltinSignal {
    /// `Changed(propertyName: string)`: a property was written or the instance renamed. Attribute changes are not
    /// included; they have [`BuiltinSignal::AttributeChanged`]. Value objects pass the new value instead (see the
    /// module docs).
    Changed,
    /// `ChildAdded(child: Instance)`, on the new parent.
    ChildAdded,
    /// `ChildRemoved(child: Instance)`, on the old parent, also when the child is destroyed.
    ChildRemoved,
    /// `DescendantAdded(descendant: Instance)`, on every new ancestor, once for the moved instance and once for each
    /// of its descendants.
    DescendantAdded,
    /// `DescendantRemoving(descendant: Instance)`, on every old ancestor, once for the moved instance and once for
    /// each of its descendants.
    DescendantRemoving,
    /// `AncestryChanged(child: Instance, parent: Instance?)`, on the moved instance and each of its descendants:
    /// `child` is the instance that moved, `parent` its new parent (nil when it left the tree).
    AncestryChanged,
    /// `AttributeChanged(attributeName: string)`: an attribute was set, changed or removed.
    AttributeChanged,
    /// `Destroying()`: the instance is about to be destroyed.
    Destroying,
}

impl BuiltinSignal {
    /// Every built-in signal.
    pub const ALL: [Self; 8] = [
        Self::Changed,
        Self::ChildAdded,
        Self::ChildRemoved,
        Self::DescendantAdded,
        Self::DescendantRemoving,
        Self::AncestryChanged,
        Self::AttributeChanged,
        Self::Destroying,
    ];

    /// How many there are.
    pub const COUNT: usize = Self::ALL.len();

    /// The name scripts connect to.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Changed => "Changed",
            Self::ChildAdded => "ChildAdded",
            Self::ChildRemoved => "ChildRemoved",
            Self::DescendantAdded => "DescendantAdded",
            Self::DescendantRemoving => "DescendantRemoving",
            Self::AncestryChanged => "AncestryChanged",
            Self::AttributeChanged => "AttributeChanged",
            Self::Destroying => "Destroying",
        }
    }

    /// The index of the whole-model connection counter that gates this signal's tree walk, if it has one.
    const fn walk(self) -> Option<usize> {
        match self {
            Self::DescendantAdded => Some(0),
            Self::DescendantRemoving => Some(1),
            Self::AncestryChanged => Some(2),
            _ => None,
        }
    }
}

/// What a connection listens to, chosen by the caller and opaque to the data model. The script host uses it to
/// find the script function (a Luau function reference, or another language's equivalent); tests use numbers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ListenerKey(pub u64);

/// A connection made with [`DataModel::connect`] or [`DataModel::connect_property_changed`]. Like an [`Id`], it
/// carries a generation, so a disconnected connection's id never names a newer one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ConnectionId {
    index: u32,
    generation: u32,
}

/// One event for one connection, from [`DataModel::drain_deliveries`]: call `listener` with `args`.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    /// The connection being delivered to.
    pub connection: ConnectionId,
    /// The listener the connection was made with.
    pub listener: ListenerKey,
    /// The instance the signal belongs to. It may be destroyed by now; [`DataModel::grave`] still names it.
    pub source: Id,
    /// The signal's arguments.
    pub args: Vec<ScriptValue>,
}

/// What a connection listens to on its instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    /// A declared signal.
    Signal(SignalId),
    /// The changed signal of one property (`GetPropertyChangedSignal`).
    Property(PropId),
}

#[derive(Debug)]
struct Connection {
    source: Id,
    key: Key,
    listener: ListenerKey,
    /// When it was made: it receives events fired after this.
    serial: u64,
    /// The whole-model counter it adds to, for the signals that walk the tree.
    walk: Option<usize>,
}

/// One entry of the connection table. A free entry keeps its generation, so old connection ids stay stale.
#[derive(Debug)]
struct ConnectionSlot {
    generation: u32,
    connection: Option<Connection>,
}

/// A queued event. Its arguments are `args[start..end]` of the shared argument list, so queueing allocates
/// nothing once the lists have grown.
#[derive(Debug)]
struct Event {
    source: Id,
    key: Key,
    /// Connections made before this (with a lower serial) receive it.
    serial: u64,
    start: usize,
    end: usize,
}

/// What stays readable about a destroyed instance while its events are delivered.
#[derive(Debug)]
struct Grave {
    class: &'static str,
    name: Arc<str>,
    /// The drain count when it was destroyed.
    epoch: u64,
}

/// Every connection, the event queue and the graves.
#[derive(Debug, Default)]
pub(super) struct Signals {
    connections: Vec<ConnectionSlot>,
    /// Free connection-table indices.
    free: Vec<u32>,
    /// Each watched instance's connections, in connection order. Entries of destroyed instances stay until the
    /// drain that delivers their last events.
    by_source: FxMap<Id, Vec<u32>>,
    /// How many connections exist.
    live: usize,
    /// Connections to `DescendantAdded`, `DescendantRemoving` and `AncestryChanged` (see [`BuiltinSignal::walk`]).
    walkers: [usize; 3],
    next_serial: u64,
    events: Vec<Event>,
    args: Vec<ScriptValue>,
    /// Shared copies of property names, so `Changed` allocates no string once a name has been seen.
    names: FxMap<&'static str, Arc<str>>,
    graves: FxMap<Id, Grave>,
    /// Watched instances destroyed since the last drain, whose connections that drain drops.
    dead: Vec<Id>,
    /// Connections dropped with their destroyed instances, until `drain_dropped_connections` hands them out.
    dropped: Vec<(ConnectionId, ListenerKey)>,
    /// How many drains have run.
    epoch: u64,
}

impl Signals {
    /// Whether a connection on `id` listens to `key`.
    fn listens(&self, id: Id, key: Key) -> bool {
        self.by_source.get(&id).is_some_and(|list| {
            list.iter().any(|&i| {
                self.connections
                    .get(i as usize)
                    .and_then(|slot| slot.connection.as_ref())
                    .is_some_and(|c| c.key == key)
            })
        })
    }

    /// Queues an event on `source` with `args`, for the connections that exist now.
    fn push(&mut self, source: Id, key: Key, args: impl IntoIterator<Item = ScriptValue>) {
        let start = self.args.len();
        self.args.extend(args);
        self.events.push(Event {
            source,
            key,
            serial: self.next_serial,
            start,
            end: self.args.len(),
        });
    }

    /// The shared copy of a property name.
    fn intern(&mut self, name: &'static str) -> Arc<str> {
        Arc::clone(self.names.entry(name).or_insert_with(|| Arc::from(name)))
    }

    /// Removes the connection `index` from the table and returns it; its id goes stale. Does not touch
    /// `by_source`.
    fn free_connection(&mut self, index: u32) -> Option<Connection> {
        let slot = self.connections.get_mut(index as usize)?;
        let connection = slot.connection.take()?;
        // A table entry whose generation would wrap is retired instead of reused, so an id is never handed out twice.
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(index);
        }
        self.live -= 1;
        if let Some(counter) = connection.walk.and_then(|w| self.walkers.get_mut(w)) {
            *counter -= 1;
        }
        Some(connection)
    }
}

impl DataModel {
    /// The id of the instance's visible signal `name`.
    pub fn signal_id(&self, id: Id, name: &str) -> Result<SignalId, DmError> {
        let class = self.slot(id)?.class;
        self.registry
            .signal_id(class, name)
            .ok_or_else(|| DmError::UnknownSignal {
                class: self.class_label(class),
                signal: name.to_owned(),
            })
    }

    /// Connects `listener` to the instance's signal `signal` (`Connect` in scripts). Every event the signal fires
    /// from now on is delivered by [`DataModel::drain_deliveries`] until [`DataModel::disconnect`], or until the
    /// instance is destroyed. Connecting the same listener twice makes two connections, each delivered.
    ///
    /// Refuses a stale id and a signal the class does not have (or hides).
    pub fn connect(&mut self, id: Id, signal: SignalId, listener: ListenerKey) -> Result<ConnectionId, DmError> {
        let class = self.slot(id)?.class;
        let visible = self
            .registry
            .class(class)
            .and_then(|c| c.signal_by_id(signal))
            .is_some_and(|s| !s.hidden);
        if !visible {
            return Err(DmError::UnknownSignal {
                class: self.class_label(class),
                signal: format!("#{}", signal.raw()),
            });
        }
        let walk = BuiltinSignal::ALL
            .into_iter()
            .find(|&b| self.builtin(id, b) == Some(signal))
            .and_then(BuiltinSignal::walk);
        Ok(self.add_connection(id, Key::Signal(signal), listener, walk))
    }

    /// Connects `listener` to the changes of one property of the instance (`GetPropertyChangedSignal(name)` in
    /// scripts). The event has no arguments. It fires whenever `Changed` fires with this property's name.
    ///
    /// Refuses a stale id and a property the class does not have (or hides).
    pub fn connect_property_changed(
        &mut self,
        id: Id,
        prop: PropId,
        listener: ListenerKey,
    ) -> Result<ConnectionId, DmError> {
        let class = self.slot(id)?.class;
        if !self.registry.prop(class, prop).is_some_and(|p| !p.hidden) {
            return Err(DmError::UnknownProperty {
                class: self.class_label(class),
                prop: format!("#{}", prop.raw()),
            });
        }
        Ok(self.add_connection(id, Key::Property(prop), listener, None))
    }

    fn add_connection(&mut self, id: Id, key: Key, listener: ListenerKey, walk: Option<usize>) -> ConnectionId {
        let s = &mut self.signals;
        let connection = Connection {
            source: id,
            key,
            listener,
            serial: s.next_serial,
            walk,
        };
        s.next_serial += 1;
        let index = match s.free.pop() {
            Some(index) => index,
            None => {
                // Four billion connections at once is far past anything a game makes.
                let index = u32::try_from(s.connections.len()).unwrap_or(u32::MAX);
                s.connections.push(ConnectionSlot {
                    generation: 0,
                    connection: None,
                });
                index
            }
        };
        let generation = match s.connections.get_mut(index as usize) {
            Some(slot) => {
                slot.connection = Some(connection);
                slot.generation
            }
            None => 0,
        };
        s.by_source.entry(id).or_default().push(index);
        s.live += 1;
        if let Some(counter) = walk.and_then(|w| s.walkers.get_mut(w)) {
            *counter += 1;
        }
        if let Some(slot) = self.slots.get_mut(id) {
            slot.watched = true;
        }
        ConnectionId { index, generation }
    }

    /// Ends a connection (`Disconnect` in scripts): it receives nothing more, not even events already queued.
    /// Returns `false` when it was already disconnected, or dropped with its destroyed instance (which
    /// [`DataModel::drain_dropped_connections`] reports instead).
    pub fn disconnect(&mut self, connection: ConnectionId) -> bool {
        let s = &mut self.signals;
        let current = s
            .connections
            .get(connection.index as usize)
            .is_some_and(|slot| slot.generation == connection.generation && slot.connection.is_some());
        if !current {
            return false;
        }
        let Some(removed) = s.free_connection(connection.index) else {
            return false;
        };
        let source = removed.source;
        let emptied = match s.by_source.get_mut(&source) {
            Some(list) => {
                if let Some(at) = list.iter().position(|&i| i == connection.index) {
                    list.remove(at);
                }
                list.is_empty()
            }
            None => false,
        };
        if emptied {
            s.by_source.remove(&source);
            if let Some(slot) = self.slots.get_mut(source) {
                slot.watched = false;
            }
        }
        true
    }

    /// Whether any connection listens to the live instance `id`. One flag check.
    #[inline]
    pub fn is_watched(&self, id: Id) -> bool {
        self.slots.get(id).is_some_and(|s| s.watched)
    }

    /// How many connections exist in the whole data model, those of instances destroyed since the last drain
    /// included.
    pub fn connection_count(&self) -> usize {
        self.signals.live
    }

    /// Fires the instance's signal `signal` with `args`, for class code (`BindableEvent:Fire()`). It is queued only
    /// when a connection listens to that signal on that instance; otherwise `args` are dropped and nothing else
    /// happens.
    ///
    /// Refuses a stale id and a signal the class does not have.
    pub fn fire(&mut self, id: Id, signal: SignalId, args: Vec<ScriptValue>) -> Result<(), DmError> {
        let class = self.slot(id)?.class;
        if self
            .registry
            .class(class)
            .and_then(|c| c.signal_by_id(signal))
            .is_none()
        {
            return Err(DmError::UnknownSignal {
                class: self.class_label(class),
                signal: format!("#{}", signal.raw()),
            });
        }
        self.notify(Notice::Fired { id, signal, args });
        Ok(())
    }

    /// Fires the instance's signal named `name`. For tests and edges; hot paths look the id up once.
    pub fn fire_by_name(&mut self, id: Id, name: &str, args: Vec<ScriptValue>) -> Result<(), DmError> {
        let signal = self.signal_id(id, name)?;
        self.fire(id, signal, args)
    }

    /// How many events are queued for the next drain.
    pub fn pending_events(&self) -> usize {
        self.signals.events.len()
    }

    /// The class name and name of a destroyed instance whose events are being delivered (see the module docs on
    /// graves). `None` for a live instance, and for one destroyed while nothing was connected anywhere.
    pub fn grave(&self, id: Id) -> Option<(&'static str, &str)> {
        if self.slots.contains(id) {
            return None;
        }
        self.signals.graves.get(&id).map(|g| (g.class, &*g.name))
    }

    /// Appends one [`Delivery`] per queued event and connection to `out`, events in fire order and each event's
    /// connections in connection order, and empties the queue.
    ///
    /// The caller runs the listeners after this returns; whatever they change is delivered by the next drain (see
    /// the module docs). Then drops the connections of instances destroyed since the last drain, which
    /// [`DataModel::drain_dropped_connections`] reports. Allocates nothing beyond growing `out`, copying each
    /// delivery's arguments and growing the dropped-connection list.
    pub fn drain_deliveries(&mut self, out: &mut Vec<Delivery>) {
        let s = &mut self.signals;
        // Graves handed out by the previous drain have been seen by its listeners.
        let epoch = s.epoch;
        if !s.graves.is_empty() {
            s.graves.retain(|_, g| g.epoch >= epoch);
        }
        s.epoch += 1;

        out.reserve(s.events.len());
        for event in s.events.drain(..) {
            let Some(list) = s.by_source.get(&event.source) else {
                continue;
            };
            let args = s.args.get(event.start..event.end).unwrap_or_default();
            for &index in list {
                let Some(slot) = s.connections.get(index as usize) else {
                    continue;
                };
                let Some(connection) = &slot.connection else {
                    continue;
                };
                if connection.key == event.key && connection.serial < event.serial {
                    out.push(Delivery {
                        connection: ConnectionId {
                            index,
                            generation: slot.generation,
                        },
                        listener: connection.listener,
                        source: event.source,
                        args: args.to_vec(),
                    });
                }
            }
        }
        s.args.clear();

        // Destroyed instances can queue nothing more, and this drain delivered what they had: drop their
        // connections. One a hook saved from destruction keeps its connections.
        let mut dead = std::mem::take(&mut s.dead);
        for id in dead.drain(..) {
            if self.slots.contains(id) {
                continue;
            }
            if let Some(list) = s.by_source.remove(&id) {
                for index in list {
                    // The id as `connect` returned it: freeing moves the generation on.
                    let generation = s.connections.get(index as usize).map_or(0, |slot| slot.generation);
                    if let Some(connection) = s.free_connection(index) {
                        s.dropped
                            .push((ConnectionId { index, generation }, connection.listener));
                    }
                }
            }
        }
        s.dead = dead;
    }

    /// Appends to `out` every connection the data model dropped on its own since the last call, with the listener
    /// it was made with, and forgets them. Today that is the connections of destroyed instances, dropped by the
    /// [`DataModel::drain_deliveries`] that delivered their last events (see the module docs on dropped
    /// connections).
    ///
    /// A connection ended by [`DataModel::disconnect`] is not reported: the caller already knows. Allocates nothing
    /// beyond growing `out`.
    pub fn drain_dropped_connections(&mut self, out: &mut Vec<(ConnectionId, ListenerKey)>) {
        out.append(&mut self.signals.dropped);
    }

    /// The id of the built-in signal `which` on the live instance's class, if the class declares it.
    #[inline]
    fn builtin(&self, id: Id, which: BuiltinSignal) -> Option<SignalId> {
        let slot = self.slots.get(id)?;
        let cache = self.classes.get(slot.class.index())?;
        cache.builtins.get(which as usize).copied().flatten()
    }

    /// The built-in signal `which` of `id`, when a connection listens to it.
    fn listening(&self, id: Id, which: BuiltinSignal) -> Option<SignalId> {
        if !self.is_watched(id) {
            return None;
        }
        let signal = self.builtin(id, which)?;
        self.signals.listens(id, Key::Signal(signal)).then_some(signal)
    }

    /// Turns one notice into the events it causes. Called by `notify` for every notice; an unwatched instance costs
    /// one flag check (plus, for a move, the whole-model counters).
    ///
    /// Always inlined, so each caller keeps only the arm of the notice it builds, as a flag test in its own body,
    /// and the notice itself is never built when nothing listens. The work behind each test stays out of line.
    /// Called out of line, this function's prologue and dispatch cost an unwatched `set` about 50 instructions,
    /// 10% of the write (issue #49).
    #[inline(always)]
    pub(super) fn queue_events(&mut self, notice: Notice) {
        match notice {
            Notice::PropertyChanged { id, prop } => {
                if self.is_watched(id) {
                    self.queue_changed(id, None, Some(prop));
                }
            }
            Notice::NameChanged { id } => {
                if self.is_watched(id) {
                    self.queue_renamed(id);
                }
            }
            Notice::AttributeChanged { id, name } => {
                if self.is_watched(id) {
                    self.queue_attribute_changed(id, name);
                }
            }
            Notice::Destroying { id } => {
                if self.signals.live > 0 {
                    self.queue_destroying(id);
                }
            }
            Notice::ParentChanged { id, old, new } => {
                if self.signals.live > 0 {
                    self.queue_moved(id, old, new);
                }
            }
            Notice::Fired { id, signal, args } => {
                if self.is_watched(id) {
                    self.queue_fired(id, signal, args);
                }
            }
        }
    }

    /// `Changed("Name")` and the `Name` property's signal, on the watched instance `id`.
    #[inline(never)]
    fn queue_renamed(&mut self, id: Id) {
        let prop = self
            .slots
            .get(id)
            .and_then(|s| self.classes.get(s.class.index())?.name_prop);
        self.queue_changed(id, Some("Name"), prop);
    }

    /// `AttributeChanged(name)` on the watched instance `id`.
    #[inline(never)]
    fn queue_attribute_changed(&mut self, id: Id, name: Arc<str>) {
        if let Some(signal) = self.listening(id, BuiltinSignal::AttributeChanged) {
            self.signals.push(id, Key::Signal(signal), [ScriptValue::String(name)]);
        }
    }

    /// A signal class code fired on the watched instance `id`, when a connection listens to it.
    #[inline(never)]
    fn queue_fired(&mut self, id: Id, signal: SignalId, args: Vec<ScriptValue>) {
        if self.signals.listens(id, Key::Signal(signal)) {
            self.signals.push(id, Key::Signal(signal), args);
        }
    }

    /// `Changed(name)` and the property's own changed signal, on the watched instance `id`, then the same for each
    /// computed property derived from it, in declaration order (see the module docs on derived properties).
    /// `name` defaults to the property's declared name.
    ///
    /// On a class whose `Changed` passes a value (see the module docs on value objects), `Changed` carries the new
    /// value instead, and fires only for that property.
    #[inline(never)]
    fn queue_changed(&mut self, id: Id, name: Option<&'static str>, prop: Option<PropId>) {
        let class = self.slots.get(id).map(|s| s.class);
        // Looked up once for the property and its dependents: nothing connects or disconnects meanwhile.
        let mut changed = self.listening(id, BuiltinSignal::Changed);
        let value_prop = class.and_then(|c| self.classes.get(c.index())?.changed_value);
        if let Some(value_prop) = value_prop {
            if let Some(signal) = changed
                && prop == Some(value_prop)
                && let Ok(value) = self.get(id, value_prop)
            {
                self.signals.push(id, Key::Signal(signal), [ScriptValue::from(value)]);
            }
            // Nothing else on this instance fires `Changed`: not its other properties, nor their dependents.
            changed = None;
        }
        let name = name.or_else(|| Some(self.registry.prop(class?, prop?)?.meta.name));
        self.queue_one_changed(id, changed, name, prop);
        if let (Some(class), Some(prop)) = (class, prop)
            && !self.registry.dependents(class, prop).is_empty()
        {
            self.queue_dependents_changed(id, class, changed, prop);
        }
    }

    /// `Changed` (when `changed` is connected) and the property signal of each computed property of `class`
    /// derived from `prop`.
    #[inline(never)]
    fn queue_dependents_changed(&mut self, id: Id, class: ClassId, changed: Option<SignalId>, prop: PropId) {
        // By index: queueing needs `self` mutably while the list borrows the registry.
        let mut i = 0;
        while let Some(&dependent) = self.registry.dependents(class, prop).get(i) {
            let name = self.registry.prop(class, dependent).map(|p| p.meta.name);
            self.queue_one_changed(id, changed, name, Some(dependent));
            i += 1;
        }
    }

    /// `Changed(name)` on the `changed` signal, if connected, and the property's own changed signal.
    #[inline(always)]
    fn queue_one_changed(
        &mut self,
        id: Id,
        changed: Option<SignalId>,
        name: Option<&'static str>,
        prop: Option<PropId>,
    ) {
        if let Some(signal) = changed
            && let Some(name) = name
        {
            let name = self.signals.intern(name);
            self.signals.push(id, Key::Signal(signal), [ScriptValue::String(name)]);
        }
        if let Some(prop) = prop
            && self.signals.listens(id, Key::Property(prop))
        {
            self.signals.push(id, Key::Property(prop), []);
        }
    }

    /// Records the grave of `id` (something is connected somewhere, so a handler may ask for it), queues its
    /// `Destroying`, and marks its connections to be dropped by the next drain.
    #[inline(never)]
    fn queue_destroying(&mut self, id: Id) {
        let Some(class) = self.slots.get(id).map(|s| s.class) else {
            return;
        };
        if let Ok(name) = self.shared_name(id) {
            let grave = Grave {
                class: self.class_label(class),
                name,
                epoch: self.signals.epoch,
            };
            self.signals.graves.insert(id, grave);
        }
        if self.is_watched(id) {
            if let Some(signal) = self.listening(id, BuiltinSignal::Destroying) {
                self.signals.push(id, Key::Signal(signal), []);
            }
            self.signals.dead.push(id);
        }
    }

    /// The events of `id` moving from `old` to `new` (`None` when it left the tree, or is being destroyed), in this
    /// order: `DescendantRemoving` on the old ancestors, `ChildRemoved`, `ChildAdded`, `DescendantAdded` on the new
    /// ancestors, `AncestryChanged` on the moved subtree, then `Changed("Parent")` and the `Parent` property's
    /// signal.
    #[inline(never)]
    fn queue_moved(&mut self, id: Id, old: Option<Id>, new: Option<Id>) {
        let child = ScriptValue::Instance(id);
        if let Some(old) = old {
            if self.signals.walkers[1] > 0 {
                self.queue_on_ancestors(old, id, BuiltinSignal::DescendantRemoving);
            }
            if let Some(signal) = self.listening(old, BuiltinSignal::ChildRemoved) {
                self.signals.push(old, Key::Signal(signal), [child.clone()]);
            }
        }
        if let Some(new) = new {
            if let Some(signal) = self.listening(new, BuiltinSignal::ChildAdded) {
                self.signals.push(new, Key::Signal(signal), [child.clone()]);
            }
            if self.signals.walkers[0] > 0 {
                self.queue_on_ancestors(new, id, BuiltinSignal::DescendantAdded);
            }
        }
        if self.signals.walkers[2] > 0 {
            let parent = new.map_or(ScriptValue::Nil, ScriptValue::Instance);
            let mut current = Some(id);
            while let Some(x) = current {
                if let Some(signal) = self.listening(x, BuiltinSignal::AncestryChanged) {
                    self.signals
                        .push(x, Key::Signal(signal), [child.clone(), parent.clone()]);
                }
                current = self.next_in_subtree(id, x);
            }
        }
        if self.is_watched(id) {
            let prop = self
                .slots
                .get(id)
                .and_then(|s| self.classes.get(s.class.index())?.parent_prop);
            self.queue_changed(id, Some("Parent"), prop);
        }
    }

    /// `which` (`DescendantAdded` or `DescendantRemoving`) on `start` and each of its ancestors that listens to it,
    /// once for `subtree` and once for each of its descendants, parent first.
    fn queue_on_ancestors(&mut self, start: Id, subtree: Id, which: BuiltinSignal) {
        let mut ancestor = Some(start);
        while let Some(a) = ancestor {
            if let Some(signal) = self.listening(a, which) {
                let mut current = Some(subtree);
                while let Some(x) = current {
                    self.signals.push(a, Key::Signal(signal), [ScriptValue::Instance(x)]);
                    current = self.next_in_subtree(subtree, x);
                }
            }
            ancestor = self.slots.parent(a);
        }
    }
}
