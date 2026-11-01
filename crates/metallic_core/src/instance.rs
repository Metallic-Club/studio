//! The instance tree: [`World`] and [`InstanceId`].
use crate::CoreError;
use crate::classes::{self, class_def, default_value, default_value_ref, find_property};
use crate::signal::Signal;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

mod extras;
pub use extras::{
    AttributeChange, AttributeChanges, AttributeCursor, MAX_ATTRIBUTE_NAME, MAX_ATTRIBUTE_STRING,
    MAX_ATTRIBUTES, MAX_EXTRAS_BYTES, MAX_TAG, MAX_TAGS, TagChange, TagChanges, TagCursor,
    check_attribute_name, check_attribute_value, check_tag,
};
use extras::{AttributeLog, Extras, TagLog};

/// Most journal entries kept before the oldest half is dropped.
const JOURNAL_LIMIT: usize = 16384;
static NEXT_WORLD_UID: AtomicU64 = AtomicU64::new(1);
static NULL_REFERENCE: Value = Value::Ref(None);

/// A position in a world's change journal, shared by [`World::changes_since`] and
/// [`World::changes_since_detailed`].
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

/// The mutation that invalidated an instance, without copying property names or values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// An instance was created, including each instance in a cloned subtree.
    Created,
    /// An instance was destroyed, including each instance in a destroyed subtree.
    Destroyed,
    /// This instance changed parent. Read its current sibling position from the world.
    /// Descendants retain their direct parents and are not additional reparent operations.
    Reparented,
    /// This instance changed position among siblings without changing its parent.
    /// The parent receives only a coarse invalidation; descendants do not move.
    Reordered,
    /// A property changed. The token is its canonical static registry name, including
    /// inherited properties; it is not a wire index and owns no string allocation.
    Property(&'static str),
}

/// One ordered mutation notice returned by [`World::changes_since_detailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The affected instance, including ids that have since been destroyed.
    pub id: InstanceId,
    /// What changed on the instance.
    pub kind: ChangeKind,
}

/// Ordered mutation notices since a [`ChangeCursor`], or a request for a full reread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailedChanges {
    /// Notices in mutation order, without deduplication or coalescing. Values and parent
    /// snapshots are not retained: read the current world state or keep a separate operation log.
    Changes(Vec<Change>),
    /// The shared bounded history no longer reaches the cursor, or it belongs to another world.
    /// The caller must reread its complete relevant state, as with [`Changes::Overflow`].
    Overflow,
}

/// The history retains one thin registry pointer; the public fat string token is formed on read.
#[derive(Clone, Copy)]
enum JournalKind {
    Created,
    Destroyed,
    Reparented,
    Reordered,
    Property(&'static classes::PropDef),
}

impl JournalKind {
    fn public_kind(self) -> ChangeKind {
        match self {
            Self::Created => ChangeKind::Created,
            Self::Destroyed => ChangeKind::Destroyed,
            Self::Reparented => ChangeKind::Reparented,
            Self::Reordered => ChangeKind::Reordered,
            Self::Property(property) => ChangeKind::Property(property.name),
        }
    }
}

#[derive(Clone, Copy)]
struct JournalEntry {
    revision: u64,
    id: InstanceId,
    /// A moved ancestor invalidates descendants for coarse readers without changing their Parent.
    kind: Option<JournalKind>,
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
    /// Attributes (and later tags); `None` for the common instance that has none.
    extras: Option<Box<Extras>>,
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
    "ServerStorage",
];

/// A whole game tree: owns all instances, their properties and events.
pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    roots: Vec<InstanceId>,
    /// Fires `(parent, child)` whenever an instance gets a parent (create or reparent).
    pub child_added: Signal<(InstanceId, InstanceId)>,
    prop_signals: HashMap<(InstanceId, Cow<'static, str>), Signal<Value>>,
    uid: u64,
    revision: u64,
    journal: Vec<JournalEntry>,
    /// Journal entries with a revision up to this value were dropped.
    journal_floor: u64,
    attribute_log: AttributeLog,
    /// Tag -> instances carrying it (detached ones too); `tagged` filters by game tree.
    tag_index: HashMap<String, BTreeSet<InstanceId>>,
    tag_log: TagLog,
    /// Bytes of attribute data held, against `MAX_EXTRAS_BYTES`.
    extras_bytes: usize,
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
            attribute_log: AttributeLog::default(),
            tag_index: HashMap::new(),
            tag_log: TagLog::default(),
            extras_bytes: 0,
        }
    }

    fn record(&mut self, id: InstanceId, kind: Option<JournalKind>) {
        self.revision += 1;
        self.journal.push(JournalEntry {
            revision: self.revision,
            id,
            kind,
        });
        if self.journal.len() > JOURNAL_LIMIT {
            let cut = self.journal.len() / 2;
            self.journal_floor = self.journal[cut - 1].revision;
            self.journal.drain(..cut);
        }
    }

    fn record_subtree(&mut self, id: InstanceId) {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            self.record(cur, (cur == id).then_some(JournalKind::Reparented));
            stack.extend_from_slice(self.children(cur));
        }
    }

    /// The current end of the shared change journal. Pass it to [`World::changes_since`] or
    /// [`World::changes_since_detailed`] later.
    pub fn change_cursor(&self) -> ChangeCursor {
        ChangeCursor {
            world: self.uid,
            revision: self.revision,
        }
    }

    /// Instances changed since `cursor` plus a new cursor to continue from. Every mutation
    /// (`create`, `destroy`, `set_parent`, `set_parent_at`, `set` with a new value, `clone_subtree`) is recorded; this
    /// needs only `&World`. The journal is bounded: when it no longer reaches back to `cursor`,
    /// or `cursor` came from a different world, the result is [`Changes::Overflow`].
    pub fn changes_since(&self, cursor: ChangeCursor) -> (Changes, ChangeCursor) {
        let now = self.change_cursor();
        let Some(start) = self.journal_start(cursor) else {
            return (Changes::Overflow, now);
        };
        let mut seen = HashSet::new();
        let ids = self.journal[start..]
            .iter()
            .filter(|entry| seen.insert(entry.id))
            .map(|entry| entry.id)
            .collect();
        (Changes::Ids(ids), now)
    }

    /// Ordered typed notices since `cursor`, plus the same continuation cursor used by
    /// [`Self::changes_since`]. Readers are independent and require only `&World`.
    ///
    /// Accepted property changes are not deduplicated; equal-value writes and rejected mutations
    /// add no notices. Creation and destruction include every affected instance. Subtree clones
    /// record parent-before-child creations in allocation order. A reparent notice identifies only
    /// its operation root; descendant invalidations still appear in the coarse reader. Same-parent
    /// `set_parent` is a no-op. An explicit same-parent `set_parent_at` reorder produces one
    /// [`ChangeKind::Reordered`] notice, with the parent invalidated only in the coarse reader.
    ///
    /// This is a bounded invalidation feed, not a replay log: it retains no historic values or
    /// parents. Reparent descendant invalidations share its retention budget, so either reader
    /// reports overflow at the same cursor boundary. Construction and [`Self::deep_clone`] start
    /// fresh histories. Foreign or expired cursors return [`DetailedChanges::Overflow`].
    pub fn changes_since_detailed(&self, cursor: ChangeCursor) -> (DetailedChanges, ChangeCursor) {
        let now = self.change_cursor();
        let Some(start) = self.journal_start(cursor) else {
            return (DetailedChanges::Overflow, now);
        };
        let changes = self.journal[start..]
            .iter()
            .filter_map(|entry| {
                entry.kind.map(|kind| Change {
                    id: entry.id,
                    kind: kind.public_kind(),
                })
            })
            .collect();
        (DetailedChanges::Changes(changes), now)
    }

    fn journal_start(&self, cursor: ChangeCursor) -> Option<usize> {
        if cursor.world != self.uid || cursor.revision < self.journal_floor {
            return None;
        }
        Some(
            self.journal
                .partition_point(|entry| entry.revision <= cursor.revision),
        )
    }

    /// A world with the standard services (`Workspace`, `Players`, `ReplicatedStorage`,
    /// `ServerScriptService`, `StarterPlayer` with `StarterPlayerScripts`, `StarterGui`,
    /// `Lighting`, `RunService`, `ServerStorage`) as parentless built-in roots.
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
            extras: None,
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
        self.record(id, Some(JournalKind::Created));
        Ok(id)
    }

    /// Destroys an instance and all its descendants; their ids become invalid. Unknown ids are ignored.
    pub fn destroy(&mut self, id: InstanceId) {
        if !self.is_alive(id) {
            return;
        }
        if !self.tag_index.is_empty() {
            if self.is_in_tree(id) {
                self.log_subtree_tags(id, false);
            }
            self.forget_subtree_tags(id);
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
                if let Some(extras) = &node.extras {
                    self.extras_bytes -= extras::extras_cost(extras);
                }
                stack.extend(node.children);
                slot.generation += 1;
                self.free.push(cur.index);
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
            self.record(g, Some(JournalKind::Destroyed));
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
        let was_in_tree = !self.tag_index.is_empty() && self.is_in_tree(id);
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
        // Tagged instances count as active while they are in the game tree.
        if !self.tag_index.is_empty() && was_in_tree != self.is_in_tree(id) {
            self.log_subtree_tags(id, !was_in_tree);
        }
        Ok(())
    }

    /// Like [`World::set_parent`], then puts `id` at position `index` among its siblings (the
    /// roots for `None`); an `index` past the end means last. This is how an editor restores
    /// an order (undo) or reorders within one parent, which `set_parent` never does.
    ///
    /// `child_added` fires only when the parent changes. A move to the place it already has is
    /// a no-op; a reorder within the same parent fires nothing and journals `id` and the parent.
    /// Detailed readers receive one [`ChangeKind::Reordered`] for a same-parent reorder,
    /// or one [`ChangeKind::Reparented`] when the parent changes, at its final position.
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
            self.record(id, Some(JournalKind::Reordered));
            if let Some(p) = parent {
                self.record(p, None);
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
            self.record(id, Some(JournalKind::Property(def)));
            // Avoid hashing when no property signal has been registered. The validated registry
            // name is static, so even an unrelated signal needs no temporary owned lookup name.
            if !self.prop_signals.is_empty()
                && let Some(sig) = self.prop_signals.get_mut(&(id, Cow::Borrowed(def.name)))
            {
                sig.fire(&value);
            }
        }
        Ok(())
    }

    /// The signal fired with the new value when `prop` of `id` changes (created on first use).
    ///
    /// `id` and `prop` are not validated. While `id` does not resolve in this world (never allocated
    /// here, destroyed or stale), the signal cannot fire; unresolved registrations are discarded by
    /// the next successful [`World::destroy`]. An id that becomes live before cleanup addresses that
    /// instance normally. Ids are world-local ([`InstanceId`]), so an id taken from another world
    /// that equals a live id here connects to this world's instance, and the signal fires for it.
    pub fn property_changed(&mut self, id: InstanceId, prop: &str) -> &mut Signal<Value> {
        let name = match self
            .node(id)
            .and_then(|node| find_property(node.class, prop))
        {
            Some(def) => Cow::Borrowed(def.name),
            None => Cow::Owned(prop.to_string()),
        };
        self.prop_signals.entry((id, name)).or_default()
    }

    /// True when the instance's class is `class` or inherits from it.
    pub fn is_a(&self, id: InstanceId, class: &str) -> bool {
        self.node(id)
            .is_some_and(|n| classes::is_subclass(n.class, class))
    }

    /// Copies an instance and its descendants into a new parentless subtree and returns its root.
    /// Refs that point inside the copied subtree point to the copies. The copy is never built-in.
    /// Journal notices follow parent-before-child allocation order, preserving sibling order.
    ///
    /// Errors: [`CoreError::InvalidInstance`] if `id` is destroyed or unknown.
    pub fn clone_subtree(&mut self, id: InstanceId) -> Result<InstanceId, CoreError> {
        if !self.is_alive(id) {
            return Err(CoreError::InvalidInstance);
        }
        // The copy holds its own attributes: refuse a clone the world has no budget for.
        let extra = self.subtree_extras_cost(id);
        if self.extras_bytes + extra > MAX_EXTRAS_BYTES {
            return Err(CoreError::InvalidValue {
                property: "Clone".into(),
                reason: format!(
                    "the attribute data budget of a world is {} MiB",
                    MAX_EXTRAS_BYTES >> 20
                ),
            });
        }
        self.extras_bytes += extra;
        let mut map = HashMap::new();
        let root = self.clone_rec(id, None, &mut map);
        self.roots.push(root);
        // `clone_rec` already journaled each copy in parent-before-child order.
        let copies: Vec<InstanceId> = map.values().copied().collect();
        for copy in copies {
            self.index_tags_of(copy);
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
        let (class, props, kids, extras) = match self.node(id) {
            Some(n) => (
                n.class,
                n.props.clone(),
                n.children.clone(),
                n.extras.clone(),
            ),
            None => return id,
        };
        let copy = self.alloc(class, parent, props, false);
        if let Some(node) = self.node_mut(copy) {
            node.extras = extras;
        }
        map.insert(id, copy);
        self.record(copy, Some(JournalKind::Created));
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
            attribute_log: AttributeLog::default(),
            tag_index: self.tag_index.clone(),
            tag_log: TagLog::default(),
            extras_bytes: self.extras_bytes,
        }
    }
}

#[cfg(test)]
mod layout_probe;
