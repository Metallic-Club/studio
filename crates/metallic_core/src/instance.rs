//! The instance tree: [`World`] and [`InstanceId`].
use crate::CoreError;
use crate::classes::{self, class_def, default_value, default_value_ref, find_property};
use crate::signal::Signal;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// Most journal entries kept before the oldest half is dropped.
const JOURNAL_LIMIT: usize = 16384;
/// Most destroyed instances [`World::take_destroyed`] keeps between two takes; later ones are
/// not kept, so a world nobody drains holds at most this many.
pub const DESTROYED_LIMIT: usize = 16384;
static NEXT_WORLD_UID: AtomicU64 = AtomicU64::new(1);
static NULL_REFERENCE: Value = Value::Ref(None);

/// A position in a world's change journal; see [`World::changes_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeCursor {
    world: u64,
    revision: u64,
}

/// What happened to a world since a [`ChangeCursor`] was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    /// These instances were created, destroyed, reparented (with their descendants) or had a
    /// property change. Deduplicated, in order of first change. Destroyed ids are included.
    Ids(Vec<InstanceId>),
    /// The journal no longer reaches back to the cursor (or the cursor is from another world):
    /// the caller must re-read everything.
    Overflow,
}

/// Stable handle to an instance. Generational: a destroyed instance's id never becomes valid again.
///
/// An id is local to the [`World`] that made it: an index and a generation, with no record of
/// which world it came from. Two independent worlds hand out the same ids in the same order, so an
/// id from one world can equal a live id in another and then refers to that world's instance.
/// [`World::deep_clone`] keeps ids on purpose. An id that does not resolve in a world (never
/// allocated there, destroyed or stale) is inert there: reads fail with
/// [`CoreError::InvalidInstance`](crate::CoreError::InvalidInstance) and its signals never fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InstanceId {
    index: u32,
    generation: u32,
}

impl InstanceId {
    /// The id packed into one number (generation in the high half). Unique per live instance; use it
    /// as a map key where the id itself cannot be one (for example a Lua table).
    pub fn to_bits(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }
}

#[derive(Clone)]
struct Node {
    class: &'static str,
    parent: Option<InstanceId>,
    children: Vec<InstanceId>,
    /// Only values that were set; everything else uses the class default.
    props: BTreeMap<String, Value>,
    builtin: bool,
}

#[derive(Clone)]
struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// Services created by [`World::new_with_services`], in order.
const SERVICES: &[&str] = &[
    "Workspace",
    "Players",
    "ReplicatedStorage",
    "ServerScriptService",
    "StarterPlayer",
    "StarterGui",
    "Lighting",
    "RunService",
];

/// What a destroyed instance was: its id, class and last property values (see
/// [`World::keep_destroyed`]).
#[derive(Debug, Clone)]
pub struct DestroyedInstance {
    id: InstanceId,
    class: &'static str,
    props: BTreeMap<String, Value>,
}

impl DestroyedInstance {
    /// The id the instance had.
    pub fn id(&self) -> InstanceId {
        self.id
    }

    /// Its class name.
    pub fn class_name(&self) -> &'static str {
        self.class
    }

    /// The last value of `prop` (the class default if it was never set), `None` if the class
    /// has no such property. References are returned as stored, live or not.
    pub fn get(&self, prop: &str) -> Option<Value> {
        match self.props.get(prop) {
            Some(v) => Some(v.clone()),
            None => default_value(self.class, prop),
        }
    }
}

/// A whole game tree: owns all instances, their properties and events.
pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    roots: Vec<InstanceId>,
    /// Fires `(parent, child)` whenever an instance gets a parent (create or reparent).
    pub child_added: Signal<(InstanceId, InstanceId)>,
    prop_signals: HashMap<(InstanceId, String), Signal<Value>>,
    uid: u64,
    revision: u64,
    journal: Vec<(u64, InstanceId)>,
    /// Journal entries with a revision up to this value were dropped.
    journal_floor: u64,
    /// Destroyed instances kept for [`World::take_destroyed`]; `None` keeps nothing.
    destroyed: Option<Vec<DestroyedInstance>>,
}

impl World {
    /// An empty world with no instances at all.
    pub fn new_empty() -> World {
        World {
            slots: Vec::new(),
            free: Vec::new(),
            roots: Vec::new(),
            child_added: Signal::default(),
            prop_signals: HashMap::new(),
            uid: NEXT_WORLD_UID.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            journal: Vec::new(),
            journal_floor: 0,
            destroyed: None,
        }
    }

    /// Whether [`World::destroy`] keeps what it destroys (id, class, property values) for
    /// [`World::take_destroyed`]. Off by default; turning it off drops what was kept. The script
    /// host turns it on so scripts can still read a destroyed instance, like Roblox.
    pub fn keep_destroyed(&mut self, keep: bool) {
        match (keep, self.destroyed.is_some()) {
            (true, false) => self.destroyed = Some(Vec::new()),
            (false, true) => self.destroyed = None,
            _ => {}
        }
    }

    /// Moves the instances destroyed since the last take into `out` (appended), at most
    /// [`DESTROYED_LIMIT`] of them. Nothing while [`World::keep_destroyed`] is off.
    pub fn take_destroyed(&mut self, out: &mut Vec<DestroyedInstance>) {
        if let Some(kept) = &mut self.destroyed {
            out.append(kept);
        }
    }

    fn record(&mut self, id: InstanceId) {
        self.revision += 1;
        self.journal.push((self.revision, id));
        if self.journal.len() > JOURNAL_LIMIT {
            let cut = self.journal.len() / 2;
            self.journal_floor = self.journal[cut - 1].0;
            self.journal.drain(..cut);
        }
    }

    fn record_subtree(&mut self, id: InstanceId) {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            self.record(cur);
            stack.extend_from_slice(self.children(cur));
        }
    }

    /// The current end of the change journal. Pass it to [`World::changes_since`] later.
    pub fn change_cursor(&self) -> ChangeCursor {
        ChangeCursor {
            world: self.uid,
            revision: self.revision,
        }
    }

    /// Instances changed since `cursor` plus a new cursor to continue from. Every mutation
    /// (`create`, `destroy`, `set_parent`, `set` with a new value, `clone_subtree`) is recorded; this
    /// needs only `&World`. The journal is bounded: when it no longer reaches back to `cursor`,
    /// or `cursor` came from a different world, the result is [`Changes::Overflow`].
    pub fn changes_since(&self, cursor: ChangeCursor) -> (Changes, ChangeCursor) {
        let now = self.change_cursor();
        if cursor.world != self.uid || cursor.revision < self.journal_floor {
            return (Changes::Overflow, now);
        }
        let start = self.journal.partition_point(|&(r, _)| r <= cursor.revision);
        let mut seen = HashSet::new();
        let ids = self.journal[start..]
            .iter()
            .filter(|(_, id)| seen.insert(*id))
            .map(|&(_, id)| id)
            .collect();
        (Changes::Ids(ids), now)
    }

    /// A world with the standard services (`Workspace`, `Players`, `ReplicatedStorage`,
    /// `ServerScriptService`, `StarterPlayer` with `StarterPlayerScripts`, `StarterGui`,
    /// `Lighting`, `RunService`) as parentless built-in roots.
    pub fn new_with_services() -> World {
        let mut w = World::new_empty();
        for s in SERVICES {
            let id = w.alloc(s, None, BTreeMap::new(), true);
            w.roots.push(id);
            if *s == "StarterPlayer" {
                let c = w.alloc("StarterPlayerScripts", Some(id), BTreeMap::new(), true);
                w.link_child(id, c);
            }
        }
        w
    }

    fn alloc(
        &mut self,
        class: &'static str,
        parent: Option<InstanceId>,
        props: BTreeMap<String, Value>,
        builtin: bool,
    ) -> InstanceId {
        let node = Node {
            class,
            parent,
            children: Vec::new(),
            props,
            builtin,
        };
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.node = Some(node);
            InstanceId {
                index,
                generation: slot.generation,
            }
        } else {
            self.slots.push(Slot {
                generation: 0,
                node: Some(node),
            });
            InstanceId {
                index: (self.slots.len() - 1) as u32,
                generation: 0,
            }
        }
    }

    fn link_child(&mut self, parent: InstanceId, child: InstanceId) {
        if let Some(n) = self.node_mut(parent) {
            n.children.push(child);
        }
    }

    fn node(&self, id: InstanceId) -> Option<&Node> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation == id.generation {
            slot.node.as_ref()
        } else {
            None
        }
    }

    fn node_mut(&mut self, id: InstanceId) -> Option<&mut Node> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation == id.generation {
            slot.node.as_mut()
        } else {
            None
        }
    }

    /// True while the instance exists.
    pub fn is_alive(&self, id: InstanceId) -> bool {
        self.node(id).is_some()
    }

    /// Parentless instances in order (the services come first in a fresh world).
    pub fn roots(&self) -> &[InstanceId] {
        &self.roots
    }

    /// The parentless built-in service with this class name, e.g. `root("Workspace")`.
    pub fn root(&self, class: &str) -> Option<InstanceId> {
        self.roots
            .iter()
            .copied()
            .find(|&r| self.node(r).is_some_and(|n| n.builtin && n.class == class))
    }

    /// The class name of an instance, or `None` if it does not exist.
    pub fn class_name(&self, id: InstanceId) -> Option<&'static str> {
        self.node(id).map(|n| n.class)
    }

    /// True for instances created by [`World::new_with_services`]; scene files do not recreate them.
    pub fn is_builtin(&self, id: InstanceId) -> bool {
        self.node(id).is_some_and(|n| n.builtin)
    }

    /// The parent of an instance, `None` for roots and for unknown ids.
    pub fn parent(&self, id: InstanceId) -> Option<InstanceId> {
        self.node(id).and_then(|n| n.parent)
    }

    /// Creates an instance of `class` (appended as the last child of `parent`, if any).
    ///
    /// Errors: [`CoreError::UnknownClass`], or [`CoreError::InvalidInstance`] if `parent` does not exist.
    /// Fires `child_added` when a parent is given.
    pub fn create(
        &mut self,
        class: &str,
        parent: Option<InstanceId>,
    ) -> Result<InstanceId, CoreError> {
        let def = class_def(class).ok_or_else(|| CoreError::UnknownClass(class.to_string()))?;
        if let Some(p) = parent
            && !self.is_alive(p)
        {
            return Err(CoreError::InvalidInstance);
        }
        let id = self.alloc(def.name, parent, BTreeMap::new(), false);
        match parent {
            Some(p) => {
                self.link_child(p, id);
                self.child_added.fire(&(p, id));
            }
            None => self.roots.push(id),
        }
        self.record(id);
        Ok(id)
    }

    /// Destroys an instance and all its descendants; their ids become invalid. Unknown ids are ignored.
    pub fn destroy(&mut self, id: InstanceId) {
        if !self.is_alive(id) {
            return;
        }
        self.detach(id);
        let mut gone = Vec::new();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            gone.push(cur);
            let Some(slot) = self.slots.get_mut(cur.index as usize) else {
                continue;
            };
            if let Some(node) = slot.node.take() {
                stack.extend(node.children);
                slot.generation += 1;
                self.free.push(cur.index);
                if let Some(kept) = &mut self.destroyed
                    && kept.len() < DESTROYED_LIMIT
                {
                    kept.push(DestroyedInstance {
                        id: cur,
                        class: node.class,
                        props: node.props,
                    });
                }
            }
        }
        // Keep only signals of live instances. Ids that never belonged to this world (out of range,
        // e.g. deserialized) or are stale are dropped here instead of indexing out of bounds.
        self.prop_signals.retain(|(k, _), _| {
            self.slots
                .get(k.index as usize)
                .is_some_and(|s| s.generation == k.generation && s.node.is_some())
        });
        for g in gone {
            self.record(g);
        }
    }

    fn detach(&mut self, id: InstanceId) {
        let parent = self.node(id).and_then(|n| n.parent);
        match parent {
            Some(p) => {
                if let Some(n) = self.node_mut(p) {
                    n.children.retain(|&c| c != id);
                }
            }
            None => self.roots.retain(|&c| c != id),
        }
    }

    /// Moves an instance under a new parent (or makes it a root with `None`), as the last child.
    ///
    /// Errors: [`CoreError::InvalidInstance`] if either id is destroyed, [`CoreError::Cycle`] if
    /// `parent` is the instance itself or one of its descendants. Fires `child_added`. Moving an
    /// instance to the parent it already has (or a root to `None`) is a no-op, as in Roblox: no
    /// signal, no change of order, nothing journaled (#97).
    pub fn set_parent(
        &mut self,
        id: InstanceId,
        parent: Option<InstanceId>,
    ) -> Result<(), CoreError> {
        if !self.is_alive(id) {
            return Err(CoreError::InvalidInstance);
        }
        if let Some(p) = parent {
            if !self.is_alive(p) {
                return Err(CoreError::InvalidInstance);
            }
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == id {
                    return Err(CoreError::Cycle);
                }
                cur = self.parent(c);
            }
        }
        if self.parent(id) == parent {
            return Ok(());
        }
        self.detach(id);
        if let Some(n) = self.node_mut(id) {
            n.parent = parent;
        }
        match parent {
            Some(p) => {
                self.link_child(p, id);
                self.child_added.fire(&(p, id));
            }
            None => self.roots.push(id),
        }
        self.record_subtree(id);
        Ok(())
    }

    /// Like [`World::set_parent`], then puts `id` at position `index` among its siblings (the
    /// roots for `None`); an `index` past the end means last. This is how an editor restores
    /// an order (undo) or reorders within one parent, which `set_parent` never does.
    ///
    /// `child_added` fires only when the parent changes. A move to the place it already has is
    /// a no-op; a reorder within the same parent fires nothing and journals `id` and the parent.
    pub fn set_parent_at(
        &mut self,
        id: InstanceId,
        parent: Option<InstanceId>,
        index: usize,
    ) -> Result<(), CoreError> {
        let moved = self.parent(id) != parent;
        self.set_parent(id, parent)?;
        let siblings = match parent {
            Some(p) => match self.node_mut(p) {
                Some(n) => &mut n.children,
                None => return Ok(()),
            },
            None => &mut self.roots,
        };
        let Some(from) = siblings.iter().position(|&c| c == id) else {
            return Ok(());
        };
        let to = index.min(siblings.len() - 1);
        if from == to {
            return Ok(());
        }
        let child = siblings.remove(from);
        siblings.insert(to, child);
        if !moved {
            self.record(id);
            if let Some(p) = parent {
                self.record(p);
            }
        }
        Ok(())
    }

    /// Children in order; empty for unknown ids.
    pub fn children(&self, id: InstanceId) -> &[InstanceId] {
        self.node(id).map_or(&[], |n| n.children.as_slice())
    }

    /// The first child whose `Name` equals `name`.
    pub fn find_first_child(&self, id: InstanceId, name: &str) -> Option<InstanceId> {
        self.children(id)
            .iter()
            .copied()
            .find(|&c| matches!(self.get_ref(c, "Name"), Ok(Value::String(s)) if s == name))
    }

    /// Dotted path of names from the root, e.g. `Workspace.Lava.Script`; empty for unknown ids.
    pub fn full_name(&self, id: InstanceId) -> String {
        let mut names = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.get_ref(c, "Name") {
                Ok(Value::String(s)) => names.push(s.as_str()),
                _ => break,
            }
            cur = self.parent(c);
        }
        names.reverse();
        names.join(".")
    }

    /// Reads an owned property value (the class default if never set).
    ///
    /// Errors: [`CoreError::InvalidInstance`], [`CoreError::UnknownProperty`].
    /// A `Ref` to a destroyed instance reads as `Ref(None)`. This clones string values;
    /// use [`World::get_ref`] when an owned copy is not needed.
    pub fn get(&self, id: InstanceId, prop: &str) -> Result<Value, CoreError> {
        let node = self.node(id).ok_or(CoreError::InvalidInstance)?;
        let value = match node.props.get(prop) {
            Some(v) => v.clone(),
            None => default_value(node.class, prop).ok_or_else(|| CoreError::UnknownProperty {
                class: node.class.to_string(),
                property: prop.to_string(),
            })?,
        };
        Ok(match value {
            Value::Ref(Some(r)) if !self.is_alive(r) => Value::Ref(None),
            v => v,
        })
    }

    /// Borrows a property value without cloning it, including inherited class defaults.
    ///
    /// Explicit values are borrowed from this world. Defaults come from the shared immutable
    /// registry, which initializes on the first default lookup; successful reads after that
    /// initialization do not allocate. The borrow prevents mutating the world while it is in use.
    /// A reference to a destroyed target reads as a shared `Ref(None)` value, as with [`World::get`].
    ///
    /// Errors: [`CoreError::InvalidInstance`], [`CoreError::UnknownProperty`].
    pub fn get_ref(&self, id: InstanceId, prop: &str) -> Result<&Value, CoreError> {
        let node = self.node(id).ok_or(CoreError::InvalidInstance)?;
        let value = match node.props.get(prop) {
            Some(v) => v,
            None => {
                default_value_ref(node.class, prop).ok_or_else(|| CoreError::UnknownProperty {
                    class: node.class.to_string(),
                    property: prop.to_string(),
                })?
            }
        };
        Ok(match value {
            Value::Ref(Some(r)) if !self.is_alive(*r) => &NULL_REFERENCE,
            v => v,
        })
    }

    /// Writes a property after checking it against the class registry.
    ///
    /// Errors: [`CoreError::InvalidInstance`], [`CoreError::UnknownProperty`],
    /// [`CoreError::TypeMismatch`] (wrong value type), [`CoreError::InvalidValue`] (e.g. unknown
    /// material or a non-finite Number/Vector3/Color3/UDim2 component). Rejected writes change
    /// neither properties, the change journal, nor signals. Properties with a registry range
    /// ([`crate::classes::NumRange`]) are clamped (or wrapped) into it. Fires `property_changed` when an accepted value changes.
    pub fn set(&mut self, id: InstanceId, prop: &str, value: Value) -> Result<(), CoreError> {
        let class = self.node(id).ok_or(CoreError::InvalidInstance)?.class;
        let def = find_property(class, prop).ok_or_else(|| CoreError::UnknownProperty {
            class: class.to_string(),
            property: prop.to_string(),
        })?;
        if value.value_type() != def.ty {
            return Err(CoreError::TypeMismatch {
                property: prop.to_string(),
                expected: def.ty.name().to_string(),
                found: value.value_type().name().to_string(),
            });
        }
        if !value.has_finite_components() {
            return Err(CoreError::InvalidValue {
                property: prop.to_string(),
                reason: "all numeric components must be finite".into(),
            });
        }
        // Lighting and effect numbers are brought into their sane range (1e39 would overflow
        // the renderer's f32 math).
        let mut value = match def.range {
            Some(range) => range.apply(value),
            None => value,
        };
        if let (Some(allowed), Value::String(s)) = (def.allowed, &value)
            && !allowed.contains(&s.as_str())
        {
            // A display name (`"Retro / PS1"`) is stored as its id.
            match def.normalize.and_then(|f| f(s)) {
                Some(stored) => value = Value::String(stored.to_string()),
                None => {
                    return Err(CoreError::InvalidValue {
                        property: prop.to_string(),
                        reason: format!("`{s}` is not one of {allowed:?}"),
                    });
                }
            }
        }
        let old = self.get(id, prop)?;
        let is_default = default_value(class, prop).as_ref() == Some(&value);
        if let Some(n) = self.node_mut(id) {
            if is_default {
                n.props.remove(prop);
            } else {
                n.props.insert(prop.to_string(), value.clone());
            }
        }
        if old != value {
            self.record(id);
            // The is_empty check skips allocating the lookup key when nothing listens at all.
            if !self.prop_signals.is_empty()
                && let Some(sig) = self.prop_signals.get_mut(&(id, prop.to_string()))
            {
                sig.fire(&value);
            }
        }
        Ok(())
    }

    /// The signal fired with the new value when `prop` of `id` changes (created on first use).
    ///
    /// `id` and `prop` are not validated. If `id` does not resolve in this world (never allocated
    /// here, destroyed or stale), the signal never fires and is discarded by the next
    /// [`World::destroy`]. Ids are world-local ([`InstanceId`]), so an id taken from another world
    /// that equals a live id here connects to this world's instance, and the signal fires for it.
    pub fn property_changed(&mut self, id: InstanceId, prop: &str) -> &mut Signal<Value> {
        self.prop_signals.entry((id, prop.to_string())).or_default()
    }

    /// True when the instance's class is `class` or inherits from it.
    pub fn is_a(&self, id: InstanceId, class: &str) -> bool {
        self.node(id)
            .is_some_and(|n| classes::is_subclass(n.class, class))
    }

    /// Copies an instance and its descendants into a new parentless subtree and returns its root.
    /// Refs that point inside the copied subtree point to the copies. The copy is never built-in.
    ///
    /// Errors: [`CoreError::InvalidInstance`] if `id` is destroyed or unknown.
    pub fn clone_subtree(&mut self, id: InstanceId) -> Result<InstanceId, CoreError> {
        if !self.is_alive(id) {
            return Err(CoreError::InvalidInstance);
        }
        let mut map = HashMap::new();
        let root = self.clone_rec(id, None, &mut map);
        self.roots.push(root);
        let copies: Vec<InstanceId> = map.values().copied().collect();
        for copy in copies {
            self.record(copy);
        }
        for &copy in map.values() {
            if let Some(n) = self.node_mut(copy) {
                for v in n.props.values_mut() {
                    if let Value::Ref(Some(r)) = v
                        && let Some(&mapped) = map.get(r)
                    {
                        *r = mapped;
                    }
                }
            }
        }
        Ok(root)
    }

    fn clone_rec(
        &mut self,
        id: InstanceId,
        parent: Option<InstanceId>,
        map: &mut HashMap<InstanceId, InstanceId>,
    ) -> InstanceId {
        let (class, props, kids) = match self.node(id) {
            Some(n) => (n.class, n.props.clone(), n.children.clone()),
            None => return id,
        };
        let copy = self.alloc(class, parent, props, false);
        map.insert(id, copy);
        for k in kids {
            let kc = self.clone_rec(k, Some(copy), map);
            self.link_child(copy, kc);
        }
        copy
    }

    /// An independent copy of the whole world with the same ids. Signal connections are not copied.
    pub fn deep_clone(&self) -> World {
        World {
            slots: self.slots.clone(),
            free: self.free.clone(),
            roots: self.roots.clone(),
            child_added: Signal::default(),
            prop_signals: HashMap::new(),
            uid: NEXT_WORLD_UID.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            journal: Vec::new(),
            journal_floor: 0,
            destroyed: None,
        }
    }
}
