//! The data model: the instance tree and every property value (ADR 0002 §3).
//!
//! - `storage.rs`: the generational slot map and the compact, sparse instance layout.
//! - `props.rs`: reads (borrowed and typed) and the single write path, `set`.
//! - `tree.rs`: parents, children, names, searches and `destroy`.
//! - `clone.rs`: `clone_tree`.
//! - `attributes.rs`: user-defined attributes.
//! - `changes.rs`: per-consumer dirty bits and the drains the renderer, physics and others read.
//! - `signals.rs`: signal connections, the built-in events and their deferred delivery.
//!
//! # Ids and destroyed instances
//!
//! An [`Id`] is a slot index plus the slot's generation. Destroying an instance moves its slot's generation on, so
//! every copy of the old id is stale, and every call refuses it with [`DmError::NoInstance`]. A reference property
//! that points at a destroyed instance is not rewritten; it reads as nil because the target fails the generation
//! check.

mod attributes;
mod changes;
mod clone;
mod error;
mod props;
mod signals;
mod storage;
mod tree;

#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::Arc;

pub use changes::{ConsumerId, DirtyBits};
pub use error::DmError;
pub use signals::{BuiltinSignal, ConnectionId, Delivery, ListenerKey};
pub use tree::{Children, Descendants};

use crate::{ClassBehavior, ClassId, ClassInfo, Id, PropId, Registry, ScriptValue, SignalId};
use changes::Changes;
use signals::Signals;
use storage::{Body, Slot, Slots};

/// How many change consumers can be registered (render, physics, replication and Studio).
pub const MAX_CONSUMERS: usize = 4;

/// What the data model caches per class, so hot paths do not go back to the registry for it.
#[derive(Debug)]
struct ClassCache {
    /// The class name, shared by every instance that still has its default name.
    name: Arc<str>,
    /// Whether the class or an ancestor has a [`ClassBehavior`], so classes without hooks skip the lookup.
    hooked: bool,
    /// Its `Archivable` property, if it has one.
    archivable: Option<PropId>,
    /// Whether it is a service.
    service: bool,
    /// The id of each [`BuiltinSignal`] the class declares, indexed by the enum.
    builtins: [Option<SignalId>; BuiltinSignal::COUNT],
    /// Its `Name` and `Parent` properties, whose changed signals a rename and a move fire.
    name_prop: Option<PropId>,
    parent_prop: Option<PropId>,
    /// The property whose new value `Changed` passes instead of a name (`ClassInfo::changed_value`).
    changed_value: Option<PropId>,
}

/// Something that happened in the tree, or a signal class code fired.
///
/// This is the one seam signals hang off: every change that a script-visible signal reports (`Changed`,
/// `ChildAdded`, `DescendantAdded`, `AncestryChanged`, `Destroying`, `AttributeChanged`, ...) and every
/// [`DataModel::fire`] passes through [`DataModel::notify`] with one of these, and `signals.rs` derives the events
/// from it. Tests also record them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    /// A stored property changed value.
    PropertyChanged { id: Id, prop: PropId },
    /// The instance was renamed.
    NameChanged { id: Id },
    /// The instance moved from `old` to `new`. A destroyed instance with a parent reports a move to `None` after
    /// its `Destroying`, while its subtree is still intact.
    ParentChanged { id: Id, old: Option<Id>, new: Option<Id> },
    /// The instance is about to be destroyed.
    Destroying { id: Id },
    /// An attribute was set, changed or removed.
    AttributeChanged { id: Id, name: Arc<str> },
    /// Class code fired one of the instance's signals.
    Fired {
        id: Id,
        signal: SignalId,
        args: Vec<ScriptValue>,
    },
}

/// The tree of instances that makes up a game (`game` in scripts), and the only source of truth for their
/// properties.
///
/// Every stored property changes through [`DataModel::set`]; the tree changes through
/// [`DataModel::set_parent`], [`DataModel::destroy`] and the create functions. Subsystems that mirror the tree
/// (renderer, physics) register a change consumer and drain what changed instead of re-reading every instance.
///
/// The data model is single-threaded (ADR 0002 §8): other threads work on data extracted from the drains.
pub struct DataModel {
    registry: Arc<Registry>,
    classes: Box<[ClassCache]>,
    slots: Slots,
    root: Option<Id>,
    /// Indexed by class id.
    services: Box<[Option<Id>]>,
    changes: Changes,
    signals: Signals,
    structure_revision: u64,
    /// Reused for subtree walks that must not hold a borrow while hooks run.
    scratch: Vec<Id>,
    /// Every notice, for tests to check the seam is called right.
    #[cfg(test)]
    notices: Vec<Notice>,
}

impl DataModel {
    /// An empty data model for the classes in `registry`. Create the root next.
    pub fn new(registry: Arc<Registry>) -> Self {
        let classes: Box<[ClassCache]> = registry
            .classes()
            .iter()
            .map(|info| ClassCache {
                name: Arc::from(info.name()),
                hooked: info
                    .ancestors()
                    .iter()
                    .any(|&a| registry.class(a).is_some_and(|c| c.desc().behavior.is_some())),
                archivable: info.prop_id("Archivable"),
                service: info.service_order().is_some(),
                builtins: BuiltinSignal::ALL.map(|b| info.signal_id(b.name())),
                name_prop: info.prop_id("Name"),
                parent_prop: info.prop_id("Parent"),
                changed_value: info.changed_value(),
            })
            .collect();
        let services = vec![None; classes.len()].into_boxed_slice();
        Self {
            registry,
            classes,
            slots: Slots::default(),
            root: None,
            services,
            changes: Changes::default(),
            signals: Signals::default(),
            structure_revision: 0,
            scratch: Vec::new(),
            #[cfg(test)]
            notices: Vec::new(),
        }
    }

    /// The registry the data model was built with.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Makes room for `additional` more instances, for loaders that know the scene size.
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// How many instances are alive, the root and services included.
    pub fn len(&self) -> usize {
        self.slots.live()
    }

    /// Whether there are no instances, not even a root.
    pub fn is_empty(&self) -> bool {
        self.slots.live() == 0
    }

    /// Whether `id` names a live instance.
    #[inline]
    pub fn contains(&self, id: Id) -> bool {
        self.slots.contains(id)
    }

    /// The instance's class.
    #[inline]
    pub fn class_of(&self, id: Id) -> Result<ClassId, DmError> {
        Ok(self.slot(id)?.class)
    }

    /// The name of the instance's class (`ClassName` in scripts).
    pub fn class_name(&self, id: Id) -> Result<&'static str, DmError> {
        let class = self.slot(id)?.class;
        Ok(self.class_info(class)?.name())
    }

    /// Whether the instance's class is `class` or inherits from it (`IsA` in scripts).
    pub fn is_a(&self, id: Id, class: ClassId) -> Result<bool, DmError> {
        Ok(self.registry.is_a(self.slot(id)?.class, class))
    }

    /// The root (`game`), once created.
    pub fn root(&self) -> Option<Id> {
        self.root
    }

    /// The service of class `class`, once created.
    pub fn service(&self, class: ClassId) -> Option<Id> {
        self.services.get(class.index()).copied().flatten()
    }

    /// Creates the root instance (`game`). It has no parent and cannot be moved, cloned or destroyed. Only one
    /// root can exist.
    pub fn create_root(&mut self, class: ClassId) -> Result<Id, DmError> {
        let info = self.class_info(class)?;
        if info.service_order().is_some() {
            return Err(DmError::IsService { class: info.name() });
        }
        if self.root.is_some() {
            return Err(DmError::RootExists);
        }
        let id = self.spawn(class)?;
        self.root = Some(id);
        self.run_hooks(id, class, |b, dm| b.on_created(dm, id));
        Ok(id)
    }

    /// Creates the service of class `class` under the root. Each service exists at most once and can never be
    /// moved or destroyed; [`DataModel::service`] finds it.
    pub fn create_service(&mut self, class: ClassId) -> Result<Id, DmError> {
        let info = self.class_info(class)?;
        let name = info.name();
        if info.service_order().is_none() {
            return Err(DmError::NotAService { class: name });
        }
        let root = self.root.ok_or(DmError::NoRoot)?;
        if self.service(class).is_some() {
            return Err(DmError::DuplicateService { class: name });
        }
        let id = self.spawn(class)?;
        if let Some(entry) = self.services.get_mut(class.index()) {
            *entry = Some(id);
        }
        self.run_hooks(id, class, |b, dm| b.on_created(dm, id));
        if self.slots.contains(id) {
            self.attach(id, root);
            self.mark(id, DirtyBits::STRUCTURE);
            self.run_hooks(id, class, |b, dm| b.on_parent_changed(dm, id, None, Some(root)));
            self.notify(Notice::ParentChanged {
                id,
                old: None,
                new: Some(root),
            });
        }
        Ok(id)
    }

    /// Creates an instance of `class` with every property at its default and no parent (`Instance.new`).
    /// Refuses classes that are not creatable, and services.
    pub fn create(&mut self, class: ClassId) -> Result<Id, DmError> {
        let info = self.class_info(class)?;
        if info.service_order().is_some() {
            return Err(DmError::IsService { class: info.name() });
        }
        if !info.creatable() {
            return Err(DmError::NotCreatable { class: info.name() });
        }
        let id = self.spawn(class)?;
        self.run_hooks(id, class, |b, dm| b.on_created(dm, id));
        Ok(id)
    }

    /// Adds an instance of `class` with its default name and marks it `CREATED`.
    fn spawn(&mut self, class: ClassId) -> Result<Id, DmError> {
        if self.classes.get(class.index()).is_none() {
            return Err(DmError::UnknownClass(class.raw()));
        }
        let id = self.slots.insert(class, Body::new());
        self.created(id);
        Ok(id)
    }

    /// Adds an instance of `class` with a copied body (for clones) and marks it `CREATED`.
    fn spawn_body(&mut self, class: ClassId, body: Body) -> Id {
        let id = self.slots.insert(class, body);
        self.created(id);
        id
    }

    fn created(&mut self, id: Id) {
        self.structure_revision += 1;
        self.mark(id, DirtyBits::CREATED);
    }

    /// The live slot of `id`.
    #[inline]
    fn slot(&self, id: Id) -> Result<&Slot, DmError> {
        self.slots.get(id).ok_or(DmError::NoInstance(id))
    }

    /// The live body of `id`.
    #[inline]
    fn body(&self, id: Id) -> Result<&Body, DmError> {
        self.slots.body(id).ok_or(DmError::NoInstance(id))
    }

    fn class_info(&self, class: ClassId) -> Result<&ClassInfo, DmError> {
        self.registry.class(class).ok_or(DmError::UnknownClass(class.raw()))
    }

    /// The class name for messages; empty for an unknown class.
    fn class_label(&self, class: ClassId) -> &'static str {
        self.registry.class(class).map_or("", ClassInfo::name)
    }

    fn is_service_class(&self, class: ClassId) -> bool {
        self.classes.get(class.index()).is_some_and(|c| c.service)
    }

    /// Records `bits` on the live instance `id` for every interested consumer.
    #[inline]
    fn mark(&mut self, id: Id, bits: DirtyBits) {
        if let Some(slot) = self.slots.get_mut(id) {
            self.changes.mark(slot, id, bits);
        }
    }

    /// Runs `hook` with the behaviour of every class in `class`'s chain that has one, root class first. Stops if a
    /// hook destroyed the instance. Classes without behaviours cost one flag check, inlined into the caller.
    #[inline(always)]
    fn run_hooks(&mut self, id: Id, class: ClassId, hook: impl Fn(&'static dyn ClassBehavior, &mut Self)) {
        if self.classes.get(class.index()).is_some_and(|c| c.hooked) {
            self.run_hooks_chain(id, class, hook);
        }
    }

    /// The part of [`DataModel::run_hooks`] for a class that has behaviours.
    #[inline(never)]
    fn run_hooks_chain(&mut self, id: Id, class: ClassId, hook: impl Fn(&'static dyn ClassBehavior, &mut Self)) {
        // A second handle, so the chain can be walked while hooks borrow the data model mutably.
        let registry = Arc::clone(&self.registry);
        let Some(info) = registry.class(class) else {
            return;
        };
        for &ancestor in info.ancestors() {
            if !self.slots.contains(id) {
                break;
            }
            if let Some(behavior) = registry.class(ancestor).and_then(|c| c.desc().behavior) {
                hook(behavior, self);
            }
        }
    }

    /// The seam for signals: every script-visible change reports here once, after it is complete, and every
    /// signal fired from class code passes through here. It queues the events the change causes; for an
    /// instance nothing listens to, that costs one flag check, inlined into the caller (see `queue_events`).
    #[inline(always)]
    fn notify(&mut self, notice: Notice) {
        #[cfg(test)]
        self.notices.push(notice.clone());
        self.queue_events(notice);
    }
}

/// Summarises the data model; the full tree is too large to print usefully.
impl fmt::Debug for DataModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataModel")
            .field("instances", &self.slots.live())
            .field("root", &self.root)
            .field("structure_revision", &self.structure_revision)
            .finish_non_exhaustive()
    }
}
