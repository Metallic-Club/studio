//! Scene sync (ADR 0011 §5): every peer's editor scene kept the same, the host deciding the order.
//!
//! - **What is synced** is what a save writes: every persisted service and, under it, every instance whose
//!   `Archivable` is on (with its ancestors'), its class, name, place among its siblings, saved properties and
//!   attributes. The current camera's properties are each user's own view and never travel.
//! - **Ids.** Each synced instance has a [`NetId`]: the scene's ids at the start, `(user << 40) | n` for what a user
//!   makes during the session.
//! - **A diff, not hooks.** [`SceneSync::collect`] drains the data model's changes and compares the changed instances
//!   with a shadow of what was last synced, so every way the editor edits (tools, Properties, undo, paste) is caught
//!   alike. [`SceneSync::apply`] applies another peer's operation and updates the shadow, so it never echoes back.
//! - **[`HostScene`] and [`ClientScene`]** add the session's rules on top: the host validates and orders; a
//!   participant applies its own edits at once and skips the host's operations on a key until the host has
//!   acknowledged its own (last writer in the host's order wins), takes corrections for what was refused, and asks for
//!   a resync when the host's digest says its scene drifted.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use metallic_core::{ClassId, ConsumerId, DataModel, DirtyBits, Id, PropFlags, PropId, Registry, Value};

use super::value::WireValue;
use crate::codec::{CodecError, Reader, Writer};
use crate::identity::UserId;

/// A synced instance's id in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetId(pub u64);

/// How far a user's number is shifted in the net ids they make.
pub const USER_SHIFT: u32 = 40;

impl NetId {
    /// The `n`th id `user` makes.
    pub const fn made_by(user: UserId, n: u64) -> Self {
        Self(((user.0 as u64) << USER_SHIFT) | (n & ((1 << USER_SHIFT) - 1)))
    }

    /// The user who made it, or 0 for an instance of the scene the session started from.
    pub const fn maker(self) -> u32 {
        (self.0 >> USER_SHIFT) as u32
    }
}

/// The most instances one operation may create.
pub const MAX_RECORDS: usize = 100_000;
/// The most operations one message may carry.
pub const MAX_OPS: usize = 100_000;
/// The longest instance or attribute name, in bytes.
pub const MAX_NAME: usize = 400;
/// The most properties or attributes one record may carry.
pub const MAX_FIELDS: usize = 512;

/// One synced instance, as a create or a resync carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Its id.
    pub id: NetId,
    /// Its parent (`None` for a service).
    pub parent: Option<NetId>,
    /// Its class's index in the registry.
    pub class: u16,
    /// Its name.
    pub name: String,
    /// Its saved properties that are not their default, by index.
    pub props: Vec<(u16, WireValue)>,
    /// Its attributes, in order.
    pub attrs: Vec<(String, WireValue)>,
}

/// A change to the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneOp {
    /// A subtree enters the tree under `parent`, before `before` (`None`: last): records parents first, children in
    /// order. An instance a peer already has (out of the tree) is put back and made to match.
    Create {
        /// The parent of the subtree's root.
        parent: NetId,
        /// The sibling it comes before.
        before: Option<NetId>,
        /// The subtree, its root first.
        records: Vec<Record>,
    },
    /// An instance moves under `parent`, before `before`.
    Move {
        /// The instance.
        id: NetId,
        /// Its new parent.
        parent: NetId,
        /// The sibling it comes before.
        before: Option<NetId>,
    },
    /// An instance leaves the tree (a delete; it is kept, as undo keeps it).
    Remove {
        /// The instance.
        id: NetId,
    },
    /// A property changes.
    Set {
        /// The instance.
        id: NetId,
        /// The property's index in its class.
        prop: u16,
        /// Its value.
        value: WireValue,
    },
    /// An instance is renamed.
    Name {
        /// The instance.
        id: NetId,
        /// Its name.
        name: String,
    },
    /// An attribute is set, or removed with `None`.
    Attr {
        /// The instance.
        id: NetId,
        /// The attribute.
        name: String,
        /// Its value.
        value: Option<WireValue>,
    },
}

/// What one operation is about: the unit of conflict.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// An instance's parent and place.
    Place(NetId),
    /// A property.
    Prop(NetId, u16),
    /// A name.
    Name(NetId),
    /// An attribute.
    Attr(NetId, String),
}

impl SceneOp {
    /// The keys it changes.
    pub fn keys(&self) -> Vec<Key> {
        match self {
            Self::Create { records, .. } => records.iter().map(|r| Key::Place(r.id)).collect(),
            Self::Move { id, .. } | Self::Remove { id } => vec![Key::Place(*id)],
            Self::Set { id, prop, .. } => vec![Key::Prop(*id, *prop)],
            Self::Name { id, .. } => vec![Key::Name(*id)],
            Self::Attr { id, name, .. } => vec![Key::Attr(*id, name.clone())],
        }
    }

    /// The instance it is about (a create's root).
    pub fn target(&self) -> Option<NetId> {
        match self {
            Self::Create { records, .. } => records.first().map(|r| r.id),
            Self::Move { id, .. }
            | Self::Remove { id }
            | Self::Set { id, .. }
            | Self::Name { id, .. }
            | Self::Attr { id, .. } => Some(*id),
        }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Self::Create {
                parent,
                before,
                records,
            } => {
                w.u8(0);
                w.u64(parent.0);
                w.option(*before, |w, b| w.u64(b.0));
                w.len_of(records.len());
                for record in records {
                    record.write(w);
                }
            }
            Self::Move { id, parent, before } => {
                w.u8(1);
                w.u64(id.0);
                w.u64(parent.0);
                w.option(*before, |w, b| w.u64(b.0));
            }
            Self::Remove { id } => {
                w.u8(2);
                w.u64(id.0);
            }
            Self::Set { id, prop, value } => {
                w.u8(3);
                w.u64(id.0);
                w.u32(u32::from(*prop));
                value.write(w);
            }
            Self::Name { id, name } => {
                w.u8(4);
                w.u64(id.0);
                w.str(name);
            }
            Self::Attr { id, name, value } => {
                w.u8(5);
                w.u64(id.0);
                w.str(name);
                w.option(value.as_ref(), |w, v| v.write(w));
            }
        }
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        let net = |r: &mut Reader<'_>| r.u64().map(NetId);
        Ok(match r.u8()? {
            0 => {
                let parent = net(r)?;
                let before = r.option(net)?;
                let count = r.count("records", MAX_RECORDS)?;
                if count == 0 {
                    return Err(CodecError::Invalid("a create of nothing"));
                }
                let records = (0..count).map(|_| Record::read(r)).collect::<Result<_, _>>()?;
                Self::Create {
                    parent,
                    before,
                    records,
                }
            }
            1 => Self::Move {
                id: net(r)?,
                parent: net(r)?,
                before: r.option(net)?,
            },
            2 => Self::Remove { id: net(r)? },
            3 => Self::Set {
                id: net(r)?,
                prop: r.u16()?,
                value: WireValue::read(r)?,
            },
            4 => Self::Name {
                id: net(r)?,
                name: r.str("a name", MAX_NAME)?.to_owned(),
            },
            5 => Self::Attr {
                id: net(r)?,
                name: r.str("an attribute name", MAX_NAME)?.to_owned(),
                value: r.option(WireValue::read)?,
            },
            tag => {
                return Err(CodecError::BadTag {
                    what: "a scene operation",
                    tag,
                });
            }
        })
    }
}

impl Record {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id.0);
        w.option(self.parent, |w, p| w.u64(p.0));
        w.u32(u32::from(self.class));
        w.str(&self.name);
        w.len_of(self.props.len());
        for (prop, value) in &self.props {
            w.u32(u32::from(*prop));
            value.write(w);
        }
        w.len_of(self.attrs.len());
        for (name, value) in &self.attrs {
            w.str(name);
            value.write(w);
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        let id = NetId(r.u64()?);
        let parent = r.option(|r| r.u64().map(NetId))?;
        let class = r.u16()?;
        let name = r.str("a name", MAX_NAME)?.to_owned();
        let count = r.count("properties", MAX_FIELDS)?;
        let props = (0..count)
            .map(|_| Ok((r.u16()?, WireValue::read(r)?)))
            .collect::<Result<_, CodecError>>()?;
        let count = r.count("attributes", MAX_FIELDS)?;
        let attrs = (0..count)
            .map(|_| Ok((r.str("an attribute name", MAX_NAME)?.to_owned(), WireValue::read(r)?)))
            .collect::<Result<_, CodecError>>()?;
        Ok(Self {
            id,
            parent,
            class,
            name,
            props,
            attrs,
        })
    }
}

/// Writes `ops` as one message's body.
pub fn encode_ops(ops: &[SceneOp], w: &mut Writer) {
    w.len_of(ops.len());
    for op in ops {
        op.write(w);
    }
}

/// Reads a message body written by [`encode_ops`].
pub fn decode_ops(r: &mut Reader<'_>) -> Result<Vec<SceneOp>, CodecError> {
    let count = r.count("operations", MAX_OPS)?;
    (0..count).map(|_| SceneOp::read(r)).collect()
}

/// Why an operation was not applied.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// It names an instance this peer does not have.
    #[error("it names an instance that is not in the scene")]
    Unknown,
    /// It breaks a rule: what it may not touch, a class that cannot be made, a value of the wrong type.
    #[error("{0}")]
    Invalid(String),
    /// The data model refused it (a cycle, a value it rejects).
    #[error("the scene refused it: {0}")]
    Model(String),
}

/// What is kept of a synced instance: what was last sent or received.
#[derive(Clone, Debug)]
struct Shadow {
    /// Whether it is in the synced tree.
    synced: bool,
    parent: Option<NetId>,
    before: Option<NetId>,
    name: Arc<str>,
    /// Saved properties that are not their default, by property.
    props: Vec<(PropId, Value)>,
    attrs: Vec<(Arc<str>, Value)>,
}

/// What a sync carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// What a save writes: Team Create's scene.
    #[default]
    Saved,
    /// What a game's clients get from its server: what a save writes but the server-only services' subtrees
    /// (`ServerStorage`, `ServerScriptService`).
    Replicated,
}

impl Scope {
    /// Whether the service `class` (persisted or not) is synced in this scope.
    fn takes(self, class: &str, persisted: bool) -> bool {
        persisted && (self == Scope::Saved || !matches!(class, "ServerStorage" | "ServerScriptService"))
    }
}

/// One peer's side of scene sync: the net ids, the shadow, and the data model's change consumer.
#[derive(Debug)]
pub struct SceneSync {
    user: UserId,
    scope: Scope,
    consumer: ConsumerId,
    to_net: HashMap<Id, NetId>,
    to_id: HashMap<NetId, Id>,
    next: u64,
    shadow: HashMap<Id, Shadow>,
    /// Each class's saved properties.
    plans: HashMap<ClassId, Arc<[PropId]>>,
    changed: Vec<(Id, DirtyBits)>,
    removed: Vec<Id>,
}

/// Whether a resolved property is one a save writes (and so one sync carries).
fn is_synced_prop(prop: &metallic_core::ResolvedProp) -> bool {
    !prop.hidden
        && prop.meta.computed.is_none()
        && !prop.meta.flags.intersects(PropFlags::NO_SAVE.union(PropFlags::RUNTIME))
}

/// The saved properties of `class`.
fn plan_of(registry: &Registry, class: ClassId) -> Arc<[PropId]> {
    registry
        .class(class)
        .map(|info| {
            info.props()
                .iter()
                .enumerate()
                .filter(|(_, prop)| is_synced_prop(prop))
                .filter_map(|(i, _)| Some(PropId::new(u16::try_from(i).ok()?)))
                .collect()
        })
        .unwrap_or_else(|| Arc::from([]))
}

/// The instance `Workspace.CurrentCamera` points at: each user's own view.
fn current_camera(dm: &DataModel) -> Option<Id> {
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let prop = dm.prop_id(workspace, "CurrentCamera").ok()?;
    match dm.get_ref(workspace, prop).ok()? {
        Value::Ref(Some(camera)) => Some(*camera),
        _ => None,
    }
}

/// Whether `id` is in the synced tree: a persisted service, or archivable under one.
fn in_scope(dm: &DataModel, id: Id, scope: Scope) -> bool {
    let registry = dm.registry();
    let mut at = id;
    loop {
        let Ok(class) = dm.class_of(at) else {
            return false;
        };
        let Some(info) = registry.class(class) else {
            return false;
        };
        if info.service_order().is_some() {
            return scope.takes(info.name(), info.persisted()) && dm.service(class) == Some(at);
        }
        if let Some(archivable) = info.prop_id("Archivable")
            && !matches!(dm.get_ref(at, archivable), Ok(Value::Bool(true)))
        {
            return false;
        }
        match dm.parent(at) {
            Ok(Some(parent)) => at = parent,
            _ => return false,
        }
    }
}

fn depth(dm: &DataModel, id: Id) -> usize {
    let mut depth = 0;
    let mut at = id;
    while let Ok(Some(parent)) = dm.parent(at) {
        depth += 1;
        at = parent;
    }
    depth
}

fn sibling_index(dm: &DataModel, id: Id) -> usize {
    let Ok(Some(parent)) = dm.parent(id) else {
        return 0;
    };
    dm.children(parent)
        .map(|mut kids| kids.position(|kid| kid == id).unwrap_or(0))
        .unwrap_or(0)
}

impl SceneSync {
    /// Sync for `dm`, whose synced instances have the net ids `ids` gives (the scene ids of the scene the session
    /// started from, for every peer); new instances are numbered for `user`.
    pub fn new(dm: &mut DataModel, ids: impl IntoIterator<Item = (Id, NetId)>, user: UserId) -> Result<Self, String> {
        Self::with_scope(dm, ids, user, Scope::Saved)
    }

    /// Sync for `dm` (see [`SceneSync::new`]) carrying `scope`.
    pub fn with_scope(
        dm: &mut DataModel,
        ids: impl IntoIterator<Item = (Id, NetId)>,
        user: UserId,
        scope: Scope,
    ) -> Result<Self, String> {
        let consumer = dm
            .register_consumer(DirtyBits::PROPERTY | DirtyBits::STRUCTURE | DirtyBits::CREATED)
            .map_err(|error| error.to_string())?;
        let mut sync = Self {
            user,
            scope,
            consumer,
            to_net: HashMap::new(),
            to_id: HashMap::new(),
            next: 1,
            shadow: HashMap::new(),
            plans: HashMap::new(),
            changed: Vec::new(),
            removed: Vec::new(),
        };
        for (id, net) in ids {
            sync.bind(id, net);
            if net.maker() == user.0 {
                sync.next = sync.next.max((net.0 & ((1 << USER_SHIFT) - 1)) + 1);
            }
        }
        sync.rebuild(dm);
        Ok(sync)
    }

    /// Stops draining `dm`'s changes.
    pub fn release(self, dm: &mut DataModel) {
        dm.unregister_consumer(self.consumer);
    }

    /// The net id of `id`, if it has one.
    pub fn net(&self, id: Id) -> Option<NetId> {
        self.to_net.get(&id).copied()
    }

    /// The instance with net id `net`, if this peer has it.
    pub fn local(&self, net: NetId) -> Option<Id> {
        self.to_id.get(&net).copied()
    }

    /// Every synced instance's local and net ids, in no order.
    pub fn pairs(&self) -> impl Iterator<Item = (Id, NetId)> + '_ {
        self.to_net.iter().map(|(&id, &net)| (id, net))
    }

    fn bind(&mut self, id: Id, net: NetId) {
        if let Some(old) = self.to_net.insert(id, net)
            && old != net
        {
            self.to_id.remove(&old);
        }
        self.to_id.insert(net, id);
    }

    fn assign(&mut self, id: Id) -> NetId {
        if let Some(net) = self.net(id) {
            return net;
        }
        let net = NetId::made_by(self.user, self.next);
        self.next += 1;
        self.bind(id, net);
        net
    }

    fn plan(&mut self, registry: &Registry, class: ClassId) -> Arc<[PropId]> {
        Arc::clone(self.plans.entry(class).or_insert_with(|| plan_of(registry, class)))
    }

    /// The synced sibling after `id`.
    fn synced_next(&self, dm: &DataModel, id: Id) -> Option<NetId> {
        let mut at = dm.next_sibling(id).ok().flatten();
        while let Some(sibling) = at {
            if self.shadow.get(&sibling).is_some_and(|s| s.synced)
                && let Some(net) = self.net(sibling)
            {
                return Some(net);
            }
            at = dm.next_sibling(sibling).ok().flatten();
        }
        None
    }

    /// `id`'s state now, as the shadow keeps it.
    fn read_shadow(&mut self, dm: &DataModel, id: Id, camera: Option<Id>) -> Shadow {
        let registry = Arc::clone(dm.registry());
        let class = dm.class_of(id).ok();
        let props = match class {
            Some(class) if Some(id) != camera => {
                let plan = self.plan(&registry, class);
                plan.iter()
                    .filter_map(|&prop| {
                        let value = dm.get_ref(id, prop).ok()?;
                        let default = &registry.prop(class, prop)?.default;
                        (value != default).then(|| (prop, value.clone()))
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        Shadow {
            synced: true,
            parent: if is_service(dm, id) {
                None
            } else {
                dm.parent(id).ok().flatten().and_then(|p| self.net(p))
            },
            before: self.synced_next(dm, id),
            name: dm.name(id).map(Arc::from).unwrap_or_else(|_| Arc::from("")),
            props,
            attrs: dm.attributes(id).map(<[_]>::to_vec).unwrap_or_default(),
        }
    }

    /// Rebuilds the shadow from `dm`: every synced instance as it is now, net ids for those without (drains the
    /// changes first, so they do not come back as edits).
    fn rebuild(&mut self, dm: &mut DataModel) {
        self.changed.clear();
        dm.drain_changes(self.consumer, &mut self.changed);
        self.changed.clear();
        dm.drain_removed(self.consumer, &mut self.removed);
        self.removed.clear();
        for shadow in self.shadow.values_mut() {
            shadow.synced = false;
        }
        let camera = current_camera(dm);
        let order = synced_order(dm, self.scope);
        for &id in &order {
            self.assign(id);
        }
        for &id in &order {
            let shadow = self.read_shadow(dm, id, camera);
            self.shadow.insert(id, shadow);
        }
    }

    /// The records of `id`'s synced subtree, parents first, children in order; ids given to those without.
    fn records(&mut self, dm: &DataModel, id: Id, camera: Option<Id>) -> Vec<Record> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            let net = self.assign(at);
            let registry = Arc::clone(dm.registry());
            let Ok(class) = dm.class_of(at) else {
                continue;
            };
            // A service has no parent in a record: the root is not synced.
            let parent = if is_service(dm, at) {
                None
            } else {
                dm.parent(at).ok().flatten().map(|p| self.assign(p))
            };
            let props = if Some(at) == camera {
                Vec::new()
            } else {
                let plan = self.plan(&registry, class);
                plan.iter()
                    .filter_map(|&prop| {
                        let value = dm.get_ref(at, prop).ok()?;
                        if registry.prop(class, prop).is_some_and(|r| r.default == *value) {
                            return None;
                        }
                        Some((prop.raw(), self.wire(value)))
                    })
                    .collect()
            };
            out.push(Record {
                id: net,
                parent,
                class: class.raw(),
                name: dm.name(at).unwrap_or_default().to_owned(),
                props,
                attrs: dm
                    .attributes(at)
                    .map(|attrs| {
                        attrs
                            .iter()
                            .map(|(name, value)| (name.to_string(), self.wire(value)))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
            if let Ok(kids) = dm.children(at) {
                let kids: Vec<Id> = kids.filter(|&kid| archivable(dm, kid)).collect();
                stack.extend(kids.into_iter().rev());
            }
        }
        out
    }

    fn wire(&self, value: &Value) -> WireValue {
        WireValue::of(value, |id| self.net(id))
    }

    /// What changed in `dm` since the last call (or since an applied operation), as operations, the shadow updated
    /// to match.
    pub fn collect(&mut self, dm: &mut DataModel) -> Vec<SceneOp> {
        self.changed.clear();
        dm.drain_changes(self.consumer, &mut self.changed);
        dm.drain_removed(self.consumer, &mut self.removed);
        let mut ops = Vec::new();
        for id in std::mem::take(&mut self.removed) {
            if let Some(shadow) = self.shadow.remove(&id)
                && let Some(net) = self.to_net.remove(&id)
            {
                self.to_id.remove(&net);
                if shadow.synced {
                    ops.push(SceneOp::Remove { id: net });
                }
            }
        }
        if self.changed.is_empty() {
            return ops;
        }
        let camera = current_camera(dm);
        let mut dirty: Vec<(usize, std::cmp::Reverse<usize>, Id, DirtyBits)> = self
            .changed
            .iter()
            .filter(|(id, _)| dm.contains(*id))
            .map(|&(id, bits)| (depth(dm, id), std::cmp::Reverse(sibling_index(dm, id)), id, bits))
            .collect();
        dirty.sort_by_key(|d| (d.0, d.1));
        let mut covered: HashSet<Id> = HashSet::new();
        let mut left: HashSet<Id> = HashSet::new();
        let mut touched_parents: HashSet<Id> = HashSet::new();
        for (_, _, id, bits) in dirty {
            if covered.contains(&id) {
                continue;
            }
            let now = in_scope(dm, id, self.scope);
            let was = self.shadow.get(&id).is_some_and(|s| s.synced);
            match (was, now) {
                (false, true) => {
                    let Some(parent) = dm.parent(id).ok().flatten() else {
                        continue;
                    };
                    let parent_net = self.assign(parent);
                    let before = self.synced_next(dm, id);
                    let records = self.records(dm, id, camera);
                    for record in &records {
                        if let Some(local) = self.local(record.id) {
                            covered.insert(local);
                            let shadow = self.read_shadow(dm, local, camera);
                            self.shadow.insert(local, shadow);
                        }
                    }
                    touched_parents.insert(parent);
                    ops.push(SceneOp::Create {
                        parent: parent_net,
                        before,
                        records,
                    });
                }
                (true, false) => {
                    let parent = dm.parent(id).ok().flatten();
                    if parent.is_some_and(|p| left.contains(&p)) {
                        // Its ancestor's removal took it along.
                        left.insert(id);
                        self.unsync_subtree(dm, id);
                        continue;
                    }
                    if let Some(shadow_parent) = self.shadow.get(&id).and_then(|s| s.parent)
                        && let Some(old) = self.local(shadow_parent)
                    {
                        touched_parents.insert(old);
                    }
                    left.insert(id);
                    self.unsync_subtree(dm, id);
                    if let Some(net) = self.net(id) {
                        ops.push(SceneOp::Remove { id: net });
                    }
                }
                (true, true) => {
                    let Some(net) = self.net(id) else {
                        continue;
                    };
                    let fresh = self.read_shadow(dm, id, camera);
                    let Some(old) = self.shadow.get(&id).cloned() else {
                        continue;
                    };
                    if bits.contains(DirtyBits::STRUCTURE)
                        && (fresh.parent != old.parent || fresh.before != old.before)
                        && let Some(parent) = fresh.parent
                    {
                        ops.push(SceneOp::Move {
                            id: net,
                            parent,
                            before: fresh.before,
                        });
                        if let Some(p) = dm.parent(id).ok().flatten() {
                            touched_parents.insert(p);
                        }
                        if let Some(p) = old.parent.and_then(|p| self.local(p)) {
                            touched_parents.insert(p);
                        }
                    }
                    if fresh.name != old.name {
                        ops.push(SceneOp::Name {
                            id: net,
                            name: fresh.name.to_string(),
                        });
                    }
                    if Some(id) != camera
                        && let Ok(class) = dm.class_of(id)
                    {
                        let registry = Arc::clone(dm.registry());
                        let plan = self.plan(&registry, class);
                        for &prop in plan.iter() {
                            let now = fresh.props.iter().find(|(p, _)| *p == prop).map(|(_, v)| v);
                            let then = old.props.iter().find(|(p, _)| *p == prop).map(|(_, v)| v);
                            if now != then {
                                let value = match now {
                                    Some(value) => value.clone(),
                                    None => match registry.prop(class, prop) {
                                        Some(resolved) => resolved.default.clone(),
                                        None => continue,
                                    },
                                };
                                ops.push(SceneOp::Set {
                                    id: net,
                                    prop: prop.raw(),
                                    value: self.wire(&value),
                                });
                            }
                        }
                    }
                    for (name, value) in &fresh.attrs {
                        if old.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value) {
                            ops.push(SceneOp::Attr {
                                id: net,
                                name: name.to_string(),
                                value: Some(self.wire(value)),
                            });
                        }
                    }
                    for (name, _) in &old.attrs {
                        if !fresh.attrs.iter().any(|(n, _)| n == name) {
                            ops.push(SceneOp::Attr {
                                id: net,
                                name: name.to_string(),
                                value: None,
                            });
                        }
                    }
                    self.shadow.insert(id, fresh);
                }
                (false, false) => {}
            }
        }
        for parent in touched_parents {
            self.refresh_children(dm, parent);
        }
        ops
    }

    fn unsync_subtree(&mut self, dm: &DataModel, id: Id) {
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            if let Some(shadow) = self.shadow.get_mut(&at) {
                shadow.synced = false;
            }
            if let Ok(kids) = dm.children(at) {
                stack.extend(kids);
            }
        }
    }

    /// Brings the shadow's places of `parent`'s children up to date after a move among them.
    fn refresh_children(&mut self, dm: &DataModel, parent: Id) {
        let Ok(kids) = dm.children(parent) else {
            return;
        };
        let kids: Vec<Id> = kids.collect();
        for kid in kids {
            if self.shadow.get(&kid).is_some_and(|s| s.synced) {
                let before = self.synced_next(dm, kid);
                if let Some(shadow) = self.shadow.get_mut(&kid) {
                    shadow.before = before;
                }
            }
        }
    }

    /// The class `class` (an index), checked to exist.
    fn class(registry: &Registry, class: u16) -> Result<ClassId, Refused> {
        let id = ClassId::new(class);
        registry
            .class(id)
            .map(|_| id)
            .ok_or_else(|| Refused::Invalid(format!("no class number {class}")))
    }

    /// Applies another peer's operation to `dm`. With `from` (the host checking a participant's operation), it is
    /// held to what a participant may do first. Returns the operation as it now stands in `dm` (a value clamped to
    /// its range, a sibling that is not one dropped), for the host to pass on.
    pub fn apply(&mut self, dm: &mut DataModel, op: &SceneOp, from: Option<UserId>) -> Result<SceneOp, Refused> {
        let camera = current_camera(dm);
        let registry = Arc::clone(dm.registry());
        let result = match op {
            SceneOp::Create {
                parent,
                before,
                records,
            } => self.apply_create(dm, *parent, *before, records, from),
            SceneOp::Move { id, parent, before } => {
                let local = self.local(*id).ok_or(Refused::Unknown)?;
                let parent_local = self.local(*parent).ok_or(Refused::Unknown)?;
                if is_service(dm, local) {
                    return Err(Refused::Invalid("a service cannot move".into()));
                }
                let old = dm.parent(local).ok().flatten();
                place(dm, local, parent_local, before.and_then(|b| self.local(b)))?;
                if let Some(old) = old {
                    self.refresh_children(dm, old);
                }
                self.resync_subtree(dm, local, camera);
                self.refresh_children(dm, parent_local);
                Ok(())
            }
            SceneOp::Remove { id } => {
                let local = self.local(*id).ok_or(Refused::Unknown)?;
                if is_service(dm, local) {
                    return Err(Refused::Invalid("a service cannot be removed".into()));
                }
                let old = dm.parent(local).ok().flatten();
                dm.set_parent(local, None).map_err(|e| Refused::Model(e.to_string()))?;
                self.unsync_subtree(dm, local);
                if let Some(old) = old {
                    self.refresh_children(dm, old);
                }
                Ok(())
            }
            SceneOp::Set { id, prop, value } => {
                let local = self.local(*id).ok_or(Refused::Unknown)?;
                if Some(local) == camera {
                    // Each user's own view: accepted, and nothing changes.
                    return Ok(op.clone());
                }
                let class = dm.class_of(local).map_err(|e| Refused::Model(e.to_string()))?;
                let prop_id = PropId::new(*prop);
                let plan = self.plan(&registry, class);
                if !plan.contains(&prop_id) {
                    return Err(Refused::Invalid(format!("property {prop} is not one that syncs")));
                }
                let resolved = registry
                    .prop(class, prop_id)
                    .ok_or_else(|| Refused::Invalid("no such property".into()))?;
                if !value.fits(&resolved.meta.ty) {
                    return Err(Refused::Invalid(format!(
                        "{} must be {}",
                        resolved.meta.name, resolved.meta.ty
                    )));
                }
                let value = value
                    .to_value(&registry, |net| self.local(net))
                    .ok_or_else(|| Refused::Invalid("a value that cannot be stored".into()))?;
                dm.set(local, prop_id, value)
                    .map_err(|e| Refused::Model(e.to_string()))?;
                let shadow = self.read_shadow(dm, local, camera);
                let stored = dm
                    .get_ref(local, prop_id)
                    .map(|v| self.wire(v))
                    .unwrap_or(WireValue::Ref(None));
                self.keep_shadow(local, shadow);
                return Ok(SceneOp::Set {
                    id: *id,
                    prop: *prop,
                    value: stored,
                });
            }
            SceneOp::Name { id, name } => {
                let local = self.local(*id).ok_or(Refused::Unknown)?;
                if is_service(dm, local) && from.is_some() {
                    return Err(Refused::Invalid("a service keeps its name".into()));
                }
                dm.set_name(local, name.as_str().into())
                    .map_err(|e| Refused::Model(e.to_string()))?;
                let shadow = self.read_shadow(dm, local, camera);
                self.keep_shadow(local, shadow);
                Ok(())
            }
            SceneOp::Attr { id, name, value } => {
                let local = self.local(*id).ok_or(Refused::Unknown)?;
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Err(Refused::Invalid("an attribute needs a name".into()));
                }
                let value = match value {
                    Some(WireValue::Ref(_)) => {
                        return Err(Refused::Invalid("an attribute cannot hold an instance".into()));
                    }
                    Some(value) => Some(
                        value
                            .to_value(&registry, |_| None)
                            .ok_or_else(|| Refused::Invalid("a value that cannot be stored".into()))?,
                    ),
                    None => None,
                };
                if value.is_some()
                    && dm.attribute(local, name).ok().flatten().is_none()
                    && dm.attributes(local).map(<[_]>::len).unwrap_or(0) >= MAX_FIELDS
                {
                    return Err(Refused::Invalid("too many attributes".into()));
                }
                dm.set_attribute(local, name, value)
                    .map_err(|e| Refused::Model(e.to_string()))?;
                let shadow = self.read_shadow(dm, local, camera);
                self.keep_shadow(local, shadow);
                Ok(())
            }
        };
        result.map(|()| op.clone())
    }

    /// Keeps `shadow` for `local`, keeping its synced flag.
    fn keep_shadow(&mut self, local: Id, mut shadow: Shadow) {
        if let Some(old) = self.shadow.get(&local) {
            shadow.synced = old.synced;
        }
        self.shadow.insert(local, shadow);
    }

    /// Re-reads the shadow of `id`'s subtree (after it moved), marking it synced if it is in scope.
    fn resync_subtree(&mut self, dm: &DataModel, id: Id, camera: Option<Id>) {
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            if self.net(at).is_some() {
                let mut shadow = self.read_shadow(dm, at, camera);
                shadow.synced = in_scope(dm, at, self.scope);
                self.shadow.insert(at, shadow);
            }
            if let Ok(kids) = dm.children(at) {
                stack.extend(kids);
            }
        }
    }

    fn apply_create(
        &mut self,
        dm: &mut DataModel,
        parent: NetId,
        before: Option<NetId>,
        records: &[Record],
        from: Option<UserId>,
    ) -> Result<(), Refused> {
        let registry = Arc::clone(dm.registry());
        let parent_local = self.local(parent).ok_or(Refused::Unknown)?;
        let root = records
            .first()
            .ok_or_else(|| Refused::Invalid("a create of nothing".into()))?;
        if root.parent != Some(parent) {
            return Err(Refused::Invalid("a create whose root is not under its parent".into()));
        }
        // Check everything before changing anything.
        let mut seen = HashSet::new();
        for (i, record) in records.iter().enumerate() {
            if !seen.insert(record.id) {
                return Err(Refused::Invalid("an instance given twice".into()));
            }
            if i > 0 && !record.parent.is_some_and(|p| seen.contains(&p)) {
                return Err(Refused::Invalid("a record whose parent comes later".into()));
            }
            let class = Self::class(&registry, record.class)?;
            let info = registry.class(class).ok_or(Refused::Unknown)?;
            if info.service_order().is_some() {
                return Err(Refused::Invalid(format!("{} is a service", info.name())));
            }
            if let Some(local) = self.local(record.id) {
                if dm.class_of(local).ok() != Some(class) {
                    return Err(Refused::Invalid("an instance cannot change class".into()));
                }
            } else if let Some(user) = from
                && record.id.maker() != user.0
            {
                return Err(Refused::Invalid("a new instance with someone else's id".into()));
            }
            if from.is_some() && !info.creatable() && self.local(record.id).is_none() {
                return Err(Refused::Invalid(format!("{} cannot be made", info.name())));
            }
            if record.props.len() > MAX_FIELDS || record.attrs.len() > MAX_FIELDS {
                return Err(Refused::Invalid("too many fields".into()));
            }
        }
        // Make or find each instance.
        let mut made: Vec<Id> = Vec::with_capacity(records.len());
        for record in records {
            let local = match self.local(record.id) {
                Some(local) if dm.contains(local) => local,
                _ => {
                    let class = ClassId::new(record.class);
                    let local = dm.create(class).map_err(|e| Refused::Model(e.to_string()))?;
                    self.bind(local, record.id);
                    local
                }
            };
            made.push(local);
        }
        // Names, attributes and properties (references once every instance exists).
        for (record, &local) in records.iter().zip(&made) {
            let class = ClassId::new(record.class);
            let _ = dm.set_name(local, record.name.as_str().into());
            let plan = self.plan(&registry, class);
            for &prop in plan.iter() {
                let value = match record.props.iter().find(|(p, _)| *p == prop.raw()) {
                    Some((_, wire)) => {
                        let Some(resolved) = registry.prop(class, prop) else {
                            continue;
                        };
                        if !wire.fits(&resolved.meta.ty) {
                            continue;
                        }
                        match wire.to_value(&registry, |net| self.local(net)) {
                            Some(value) => value,
                            None => continue,
                        }
                    }
                    None => match registry.prop(class, prop) {
                        Some(resolved) => resolved.default.clone(),
                        None => continue,
                    },
                };
                // A value the model refuses keeps what it had: the host's digest will say if that differs.
                let _ = dm.set(local, prop, value);
            }
            let existing: Vec<Arc<str>> = dm
                .attributes(local)
                .map(|a| a.iter().map(|(n, _)| Arc::clone(n)).collect())
                .unwrap_or_default();
            for name in existing {
                if !record.attrs.iter().any(|(n, _)| **n == *name) {
                    let _ = dm.set_attribute(local, &name, None);
                }
            }
            for (name, wire) in &record.attrs {
                if let Some(value) = wire.to_value(&registry, |_| None) {
                    let _ = dm.set_attribute(local, name, Some(value));
                }
            }
        }
        // Places: the root under its parent, then each child appended in order, so the subtree has the sender's
        // order; children it has that the sender did not send leave the tree.
        let old_parent = dm.parent(made[0]).ok().flatten();
        place(dm, made[0], parent_local, before.and_then(|b| self.local(b)))?;
        for (record, &local) in records.iter().zip(&made).skip(1) {
            let Some(parent) = record.parent.and_then(|p| self.local(p)) else {
                continue;
            };
            dm.set_parent_before(local, parent, None)
                .map_err(|e| Refused::Model(e.to_string()))?;
        }
        let kept: HashSet<Id> = made.iter().copied().collect();
        for &local in &made {
            let kids: Vec<Id> = dm.children(local).map(Iterator::collect).unwrap_or_default();
            for kid in kids {
                if !kept.contains(&kid) && archivable(dm, kid) {
                    let _ = dm.set_parent(kid, None);
                    self.unsync_subtree(dm, kid);
                }
            }
        }
        let camera = current_camera(dm);
        for &local in &made {
            let shadow = self.read_shadow(dm, local, camera);
            self.shadow.insert(local, shadow);
        }
        if let Some(old) = old_parent {
            self.refresh_children(dm, old);
        }
        self.refresh_children(dm, parent_local);
        Ok(())
    }

    /// The host's state of the instance `net` is about, as operations that put a peer back to it: what a refused
    /// operation is answered with.
    pub fn correction(&mut self, dm: &DataModel, op: &SceneOp) -> Vec<SceneOp> {
        let Some(net) = op.target() else {
            return Vec::new();
        };
        let Some(local) = self.local(net).filter(|&l| dm.contains(l)) else {
            return vec![SceneOp::Remove { id: net }];
        };
        let camera = current_camera(dm);
        if !in_scope(dm, local, self.scope) {
            return vec![SceneOp::Remove { id: net }];
        }
        match op {
            SceneOp::Set { prop, .. } => {
                let value = dm.get_ref(local, PropId::new(*prop)).map(|v| self.wire(v));
                match value {
                    Ok(value) => vec![SceneOp::Set {
                        id: net,
                        prop: *prop,
                        value,
                    }],
                    Err(_) => Vec::new(),
                }
            }
            SceneOp::Name { .. } => vec![SceneOp::Name {
                id: net,
                name: dm.name(local).unwrap_or_default().to_owned(),
            }],
            SceneOp::Attr { name, .. } => vec![SceneOp::Attr {
                id: net,
                name: name.clone(),
                value: dm.attribute(local, name).ok().flatten().map(|v| self.wire(v)),
            }],
            SceneOp::Create { .. } | SceneOp::Move { .. } | SceneOp::Remove { .. } => {
                let Some(parent) = dm.parent(local).ok().flatten() else {
                    return vec![SceneOp::Remove { id: net }];
                };
                let Some(parent_net) = self.net(parent) else {
                    return vec![SceneOp::Remove { id: net }];
                };
                let before = self.synced_next(dm, local);
                vec![SceneOp::Create {
                    parent: parent_net,
                    before,
                    records: self.records(dm, local, camera),
                }]
            }
        }
    }

    /// Every synced instance, services first, parents before children, in tree order: a resync's full state.
    pub fn state(&mut self, dm: &DataModel) -> Vec<Record> {
        let camera = current_camera(dm);
        let mut out = Vec::new();
        for service in persisted_services(dm, self.scope) {
            out.extend(self.records(dm, service, camera));
        }
        out
    }

    /// Makes `dm` match `records` ([`SceneSync::state`]'s), in place: instances made, moved, updated, and those not
    /// in it taken out of the tree; except what `keep` says is the peer's own edit still under way.
    pub fn reconcile(&mut self, dm: &mut DataModel, records: &[Record], keep: impl Fn(&Key) -> bool) {
        let registry = Arc::clone(dm.registry());
        let mut wanted: HashSet<Id> = HashSet::new();
        // Services are found by class; their properties are set with the rest.
        let mut made = Vec::new();
        for record in records.iter().filter(|r| r.parent.is_none()) {
            if let Some(service) = registry
                .class(ClassId::new(record.class))
                .and_then(|info| dm.service(info.id()))
            {
                self.bind(service, record.id);
                wanted.insert(service);
                made.push((record, service));
            }
        }
        for record in records.iter().filter(|r| r.parent.is_some()) {
            let class = ClassId::new(record.class);
            let local = match self.local(record.id).filter(|&l| dm.contains(l)) {
                Some(local) if dm.class_of(local).ok() == Some(class) => local,
                _ => match dm.create(class) {
                    Ok(local) => {
                        self.bind(local, record.id);
                        local
                    }
                    Err(_) => continue,
                },
            };
            wanted.insert(local);
            made.push((record, local));
        }
        for (record, local) in &made {
            if !keep(&Key::Name(record.id)) {
                let _ = dm.set_name(*local, record.name.as_str().into());
            }
            let class = ClassId::new(record.class);
            let plan = self.plan(&registry, class);
            for &prop in plan.iter() {
                if keep(&Key::Prop(record.id, prop.raw())) {
                    continue;
                }
                let value = match record.props.iter().find(|(p, _)| *p == prop.raw()) {
                    Some((_, wire)) => match wire.to_value(&registry, |net| self.local(net)) {
                        Some(value) => value,
                        None => continue,
                    },
                    None => match registry.prop(class, prop) {
                        Some(resolved) => resolved.default.clone(),
                        None => continue,
                    },
                };
                let _ = dm.set(*local, prop, value);
            }
            let existing: Vec<Arc<str>> = dm
                .attributes(*local)
                .map(|a| a.iter().map(|(n, _)| Arc::clone(n)).collect())
                .unwrap_or_default();
            for name in existing {
                if !record.attrs.iter().any(|(n, _)| **n == *name) && !keep(&Key::Attr(record.id, name.to_string())) {
                    let _ = dm.set_attribute(*local, &name, None);
                }
            }
            for (name, wire) in &record.attrs {
                if !keep(&Key::Attr(record.id, name.clone()))
                    && let Some(value) = wire.to_value(&registry, |_| None)
                {
                    let _ = dm.set_attribute(*local, name, Some(value));
                }
            }
        }
        // Places, in order: appending each in turn gives every parent the host's order.
        for (record, local) in &made {
            if keep(&Key::Place(record.id)) {
                continue;
            }
            if let Some(parent) = record.parent.and_then(|p| self.local(p)) {
                let _ = dm.set_parent_before(*local, parent, None);
            }
        }
        // What the host does not have leaves the tree.
        for id in synced_order(dm, self.scope) {
            if !wanted.contains(&id) && !is_service(dm, id) && !self.net(id).is_some_and(|net| keep(&Key::Place(net))) {
                let _ = dm.set_parent(id, None);
            }
        }
        self.rebuild(dm);
    }

    /// A number that is the same for two peers exactly when their synced scenes are: every synced instance's id,
    /// class, parent, place, name, properties (bit for bit) and attributes.
    pub fn digest(&self, dm: &DataModel) -> u64 {
        let camera = current_camera(dm);
        let mut hash = crate::Fnv::new();
        let mut w = Writer::new();
        for id in synced_order(dm, self.scope) {
            w.u64(self.net(id).map_or(0, |n| n.0));
            if let Ok(class) = dm.class_of(id) {
                w.u32(u32::from(class.raw()));
                w.option(dm.parent(id).ok().flatten().and_then(|p| self.net(p)), |w, p| {
                    w.u64(p.0)
                });
                w.str(dm.name(id).unwrap_or_default());
                if Some(id) != camera {
                    let plan = self
                        .plans
                        .get(&class)
                        .cloned()
                        .unwrap_or_else(|| plan_of(dm.registry(), class));
                    for &prop in plan.iter() {
                        if let Ok(value) = dm.get_ref(id, prop) {
                            self.wire(value).write(&mut w);
                        }
                    }
                }
                if let Ok(attrs) = dm.attributes(id) {
                    let mut attrs: Vec<_> = attrs.iter().collect();
                    attrs.sort_by(|a, b| a.0.cmp(&b.0));
                    for (name, value) in attrs {
                        w.str(name);
                        self.wire(value).write(&mut w);
                    }
                }
            }
            hash.bytes(w.as_bytes());
            w = Writer::new();
        }
        hash.0
    }
}

fn is_service(dm: &DataModel, id: Id) -> bool {
    dm.class_of(id)
        .ok()
        .and_then(|class| dm.registry().class(class))
        .is_some_and(|info| info.service_order().is_some())
}

fn archivable(dm: &DataModel, id: Id) -> bool {
    match dm.prop_id(id, "Archivable") {
        Ok(prop) => matches!(dm.get_ref(id, prop), Ok(Value::Bool(true))),
        Err(_) => true,
    }
}

/// The persisted services, in service order.
fn persisted_services(dm: &DataModel, scope: Scope) -> Vec<Id> {
    let registry = dm.registry();
    registry
        .services()
        .iter()
        .filter(|&&class| {
            registry
                .class(class)
                .is_some_and(|info| scope.takes(info.name(), info.persisted()))
        })
        .filter_map(|&class| dm.service(class))
        .collect()
}

/// Every synced instance, in tree order.
fn synced_order(dm: &DataModel, scope: Scope) -> Vec<Id> {
    let mut out = Vec::new();
    for service in persisted_services(dm, scope) {
        let mut stack = vec![service];
        while let Some(at) = stack.pop() {
            out.push(at);
            if let Ok(kids) = dm.children(at) {
                let kids: Vec<Id> = kids.filter(|&kid| archivable(dm, kid)).collect();
                stack.extend(kids.into_iter().rev());
            }
        }
    }
    out
}

/// Puts `id` under `parent` before `before` when that is one of `parent`'s children, else last.
fn place(dm: &mut DataModel, id: Id, parent: Id, before: Option<Id>) -> Result<(), Refused> {
    let before = before.filter(|&b| b != id && dm.contains(b) && dm.parent(b).ok().flatten() == Some(parent));
    dm.set_parent_before(id, parent, before)
        .map_err(|e| Refused::Model(e.to_string()))
}

/// The message kinds of scene sync, in [`super::protocol::ClientMsg::Op`] and [`super::protocol::HostMsg::Op`].
pub mod kind {
    /// A participant's edits: a sequence number and operations.
    pub const PROPOSE: u8 = 1;
    /// The host's order: who made them, and the operations.
    pub const APPLY: u8 = 2;
    /// To the proposer: the sequence number taken, and corrections for what was refused.
    pub const ACK: u8 = 3;
    /// The host's scene digest, for drift.
    pub const DIGEST: u8 = 4;
    /// A participant asks for the whole scene.
    pub const RESYNC_ASK: u8 = 5;
    /// The whole scene.
    pub const RESYNC: u8 = 6;
}

/// The host's side of scene sync.
#[derive(Debug)]
pub struct HostScene {
    /// The sync.
    pub sync: SceneSync,
}

/// Who a message of the host's goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum To {
    /// Every participant but `except`.
    All {
        /// The one left out (the proposer, who has the edits already).
        except: Option<UserId>,
    },
    /// One participant.
    One(UserId),
}

/// What the host has to send after a step of scene sync, in the order it is to be sent: per connection, a
/// participant must get its acknowledgement before the edits the host ordered after its own.
#[derive(Debug, Default, PartialEq)]
pub struct HostOut {
    /// Each message: who it goes to, its kind and bytes.
    pub messages: Vec<(To, u8, Vec<u8>)>,
}

impl HostOut {
    fn all(&mut self, kind: u8, bytes: Vec<u8>, except: Option<UserId>) {
        self.messages.push((To::All { except }, kind, bytes));
    }

    fn one(&mut self, to: UserId, kind: u8, bytes: Vec<u8>) {
        self.messages.push((To::One(to), kind, bytes));
    }
}

impl HostScene {
    /// The host's sync over `dm`, whose instances have the scene ids `ids`.
    pub fn new(dm: &mut DataModel, ids: impl IntoIterator<Item = (Id, NetId)>) -> Result<Self, String> {
        Ok(Self {
            sync: SceneSync::new(dm, ids, UserId::HOST)?,
        })
    }

    /// The host's side over a sync made elsewhere ([`SceneSync::with_scope`]).
    pub fn from_sync(sync: SceneSync) -> Self {
        Self { sync }
    }

    /// The host's own edits since the last call, for everyone.
    pub fn collect(&mut self, dm: &mut DataModel, out: &mut HostOut) {
        let ops = self.sync.collect(dm);
        if !ops.is_empty() {
            out.all(kind::APPLY, apply_body(UserId::HOST, &ops), None);
        }
    }

    /// A message of `kind` from participant `from`. A message that cannot be read is an error: drop the peer.
    pub fn receive(
        &mut self,
        dm: &mut DataModel,
        from: UserId,
        kind: u8,
        bytes: &[u8],
        out: &mut HostOut,
    ) -> Result<(), CodecError> {
        let mut r = Reader::new(bytes);
        match kind {
            kind::PROPOSE => {
                let seq = r.u64()?;
                let ops = decode_ops(&mut r)?;
                r.finish()?;
                // The host's own edits first, so they are ordered before these.
                self.collect(dm, out);
                let mut accepted = Vec::with_capacity(ops.len());
                let mut corrections = Vec::new();
                for op in &ops {
                    match self.sync.apply(dm, op, Some(from)) {
                        Ok(applied) => accepted.push(applied),
                        Err(why) => {
                            log::info!("team create: refused an edit from {from}: {why}");
                            corrections.extend(self.sync.correction(dm, op));
                        }
                    }
                }
                if !accepted.is_empty() {
                    out.all(kind::APPLY, apply_body(from, &accepted), Some(from));
                }
                let mut w = Writer::new();
                w.u64(seq);
                encode_ops(&corrections, &mut w);
                out.one(from, kind::ACK, w.finish());
            }
            kind::RESYNC_ASK => {
                r.finish()?;
                self.collect(dm, out);
                let records = self.sync.state(dm);
                out.one(from, kind::RESYNC, records_body(&records));
            }
            _ => {
                return Err(CodecError::BadTag {
                    what: "a scene message",
                    tag: kind,
                });
            }
        }
        Ok(())
    }

    /// The digest message, for everyone.
    pub fn digest(&mut self, dm: &mut DataModel, out: &mut HostOut) {
        self.collect(dm, out);
        let mut w = Writer::new();
        w.u64(self.sync.digest(dm));
        out.all(kind::DIGEST, w.finish(), None);
    }
}

fn apply_body(origin: UserId, ops: &[SceneOp]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(origin.0);
    encode_ops(ops, &mut w);
    w.finish()
}

/// Writes `records` (a whole world, as [`SceneSync::state`] gives it).
pub fn write_records(records: &[Record], w: &mut Writer) {
    w.len_of(records.len());
    for record in records {
        record.write(w);
    }
}

/// Reads records written by [`write_records`].
pub fn read_records(r: &mut Reader<'_>) -> Result<Vec<Record>, CodecError> {
    let count = r.count("records", MAX_RECORDS * 10)?;
    (0..count).map(|_| Record::read(r)).collect()
}

fn records_body(records: &[Record]) -> Vec<u8> {
    let mut w = Writer::new();
    w.len_of(records.len());
    for record in records {
        record.write(&mut w);
    }
    w.finish()
}

/// A participant's side of scene sync.
#[derive(Debug)]
pub struct ClientScene {
    /// The sync.
    pub sync: SceneSync,
    seq: u64,
    /// The keys of each proposal not acknowledged yet, oldest first.
    inflight: VecDeque<(u64, Vec<Key>)>,
    pending: HashMap<Key, u32>,
    /// The host's operations that could not apply yet (they wait on the participant's own in flight).
    deferred: Vec<SceneOp>,
    /// Whether a resync was asked for and not answered.
    asked: bool,
    /// Keys this peer owns: the host's operations on them are never applied (a game client's own character).
    owned: HashSet<Key>,
}

impl ClientScene {
    /// The participant `me`'s sync over `dm`, whose instances have the scene ids `ids`.
    pub fn new(dm: &mut DataModel, ids: impl IntoIterator<Item = (Id, NetId)>, me: UserId) -> Result<Self, String> {
        Ok(Self {
            sync: SceneSync::new(dm, ids, me)?,
            seq: 0,
            inflight: VecDeque::new(),
            pending: HashMap::new(),
            deferred: Vec::new(),
            asked: false,
            owned: HashSet::new(),
        })
    }

    /// A participant's side over a sync made elsewhere ([`SceneSync::with_scope`]).
    pub fn from_sync(sync: SceneSync) -> Self {
        Self {
            sync,
            seq: 0,
            inflight: VecDeque::new(),
            pending: HashMap::new(),
            deferred: Vec::new(),
            asked: false,
            owned: HashSet::new(),
        }
    }

    /// Makes `key` the participant's own: the host's operations on it are never applied from now on.
    pub fn own(&mut self, key: Key) {
        self.owned.insert(key);
    }

    /// Whether every edit of the participant's was acknowledged.
    pub fn is_settled(&self) -> bool {
        self.inflight.is_empty() && self.deferred.is_empty()
    }

    /// The participant's edits since the last call, as a proposal (kind, bytes) for the host.
    pub fn collect(&mut self, dm: &mut DataModel) -> Option<(u8, Vec<u8>)> {
        let ops = self.sync.collect(dm);
        if ops.is_empty() {
            return None;
        }
        self.seq += 1;
        let keys: Vec<Key> = ops.iter().flat_map(SceneOp::keys).collect();
        for key in &keys {
            *self.pending.entry(key.clone()).or_default() += 1;
        }
        self.inflight.push_back((self.seq, keys));
        let mut w = Writer::new();
        w.u64(self.seq);
        encode_ops(&ops, &mut w);
        Some((kind::PROPOSE, w.finish()))
    }

    fn is_pending(&self, key: &Key) -> bool {
        self.owned.contains(key) || self.pending.get(key).is_some_and(|&n| n > 0)
    }

    fn apply_all(&mut self, dm: &mut DataModel, ops: Vec<SceneOp>) {
        for op in ops {
            if op.keys().iter().any(|key| self.is_pending(key)) {
                // Its own edit of the same thing is later in the host's order.
                continue;
            }
            if self.sync.apply(dm, &op, None).is_err() {
                self.deferred.push(op);
            }
        }
    }

    /// A message of `kind` from the host. Returns a message for the host (a resync asked for), if one is due. A
    /// message that cannot be read is an error: the session is broken.
    pub fn receive(&mut self, dm: &mut DataModel, kind: u8, bytes: &[u8]) -> Result<Option<(u8, Vec<u8>)>, CodecError> {
        let mut r = Reader::new(bytes);
        match kind {
            kind::APPLY => {
                let _origin = UserId(r.u32()?);
                let ops = decode_ops(&mut r)?;
                r.finish()?;
                self.apply_all(dm, ops);
            }
            kind::ACK => {
                let seq = r.u64()?;
                let corrections = decode_ops(&mut r)?;
                r.finish()?;
                match self.inflight.pop_front() {
                    Some((sent, keys)) if sent == seq => {
                        for key in keys {
                            if let Some(n) = self.pending.get_mut(&key) {
                                *n = n.saturating_sub(1);
                                if *n == 0 {
                                    self.pending.remove(&key);
                                }
                            }
                        }
                    }
                    _ => return Err(CodecError::Invalid("an acknowledgement out of order")),
                }
                self.apply_all(dm, corrections);
            }
            kind::DIGEST => {
                let digest = r.u64()?;
                r.finish()?;
                if self.is_settled() && !self.asked && self.sync.digest(dm) != digest {
                    log::warn!("team create: the scene drifted from the host's; asking for it again");
                    self.asked = true;
                    return Ok(Some((kind::RESYNC_ASK, Vec::new())));
                }
                return Ok(None);
            }
            kind::RESYNC => {
                let count = r.count("records", MAX_RECORDS * 10)?;
                let records: Vec<Record> = (0..count).map(|_| Record::read(&mut r)).collect::<Result<_, _>>()?;
                r.finish()?;
                let pending = self.pending.clone();
                let owned = self.owned.clone();
                self.sync.reconcile(dm, &records, |key| {
                    pending.get(key).is_some_and(|&n| n > 0) || owned.contains(key)
                });
                self.deferred.clear();
                self.asked = false;
                return Ok(None);
            }
            _ => {
                return Err(CodecError::BadTag {
                    what: "a scene message",
                    tag: kind,
                });
            }
        }
        // What waited may apply now; what still cannot, once nothing is in flight, needs the whole scene.
        let deferred = std::mem::take(&mut self.deferred);
        for op in deferred {
            if op.keys().iter().any(|key| self.is_pending(key)) {
                continue;
            }
            if self.sync.apply(dm, &op, None).is_err() {
                self.deferred.push(op);
            }
        }
        if !self.deferred.is_empty() && self.inflight.is_empty() && !self.asked {
            log::warn!(
                "team create: {} edits could not apply; asking for the scene",
                self.deferred.len()
            );
            self.asked = true;
            self.deferred.clear();
            return Ok(Some((kind::RESYNC_ASK, Vec::new())));
        }
        Ok(None)
    }
}
