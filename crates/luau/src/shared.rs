//! The backend's state that C functions and mlua callbacks reach during a call (through `ffi::with_shared`): the
//! per-class member cache, the enum index, the thread and listener tables, and the registry references of the
//! tables C functions push from.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_int;
use std::sync::Arc;

use metallic_core::{
    ClassId, EnumDesc, EnumItem, ListenerKey, MethodDesc, PropFlags, PropId, PropType, Registry, SignalId,
};
use metallic_script::ThreadId;
use mlua::{Function, MultiValue, Thread};

use crate::ffi::EnumRef;

/// Registry references (`lua_ref`) of tables that C functions push values from.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Refs {
    /// Weak-valued table: instance slot index + 1 → its userdata.
    pub(crate) instances: c_int,
    /// Every enum item's userdata, flat (see [`EnumIndex::flat_index`]).
    pub(crate) enum_items: c_int,
    /// `Vector3` methods by name, as function values.
    pub(crate) vector3_methods: c_int,
    /// `Vector2` methods by name.
    pub(crate) vector2_methods: c_int,
    /// `CFrame` methods by name.
    pub(crate) cframe_methods: c_int,
    /// `Color3` methods by name.
    pub(crate) color3_methods: c_int,
}

/// What a member name of a class resolves to, cached per (class, atom).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Member {
    /// Not looked up yet.
    Unresolved,
    /// A property.
    Prop(PropMember),
    /// A method from the registry.
    Method(&'static MethodDesc),
    /// A signal.
    Signal(SignalId),
    /// `GetPropertyChangedSignal`, which the backend provides on every instance.
    GetPropertyChangedSignal,
    /// No member of that name (indexing then looks for a child).
    Missing,
}

/// A property as the bindings need it, so a read or write never goes back to the registry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PropMember {
    /// Its id.
    pub(crate) id: PropId,
    /// Its declared type, for converting writes.
    pub(crate) ty: PropType,
    /// Scripts may not write it.
    pub(crate) read_only: bool,
    /// Computed (`Position`...): read through `DataModel::get`, not `get_ref`.
    pub(crate) computed: bool,
}

/// The name of the backend-provided method every instance has.
pub(crate) const GET_PROPERTY_CHANGED_SIGNAL: &str = "GetPropertyChangedSignal";

/// Every enum of the registry, with positions for [`EnumRef`] and a fast way back from an [`EnumItem`].
pub(crate) struct EnumIndex {
    descs: Vec<&'static EnumDesc>,
    /// `items[e][i]`: the registry's shared copy of item `i` of enum `e`.
    items: Vec<Vec<EnumItem>>,
    /// The 1-based flat index of each enum's first item.
    base: Vec<c_int>,
    /// Shared copies by the address of their name, so a property's item is found without hashing text.
    by_address: HashMap<usize, EnumRef>,
    by_type: HashMap<&'static str, u16>,
}

impl EnumIndex {
    /// Indexes every enum of `registry`.
    pub(crate) fn new(registry: &Registry) -> Self {
        let mut index = Self {
            descs: Vec::new(),
            items: Vec::new(),
            base: Vec::new(),
            by_address: HashMap::new(),
            by_type: HashMap::new(),
        };
        let mut next: c_int = 1;
        for desc in registry.enums() {
            let Ok(enum_index) = u16::try_from(index.descs.len()) else {
                break;
            };
            let mut items = Vec::with_capacity(desc.items.len());
            for (i, name) in desc.items.iter().enumerate() {
                let (Some(item), Ok(item_index)) = (registry.enum_item(desc.name, name), u16::try_from(i)) else {
                    continue;
                };
                index
                    .by_address
                    .insert(item.name().as_ptr() as usize, EnumRef { enum_index, item_index });
                items.push(item);
            }
            index.base.push(next);
            next = next.saturating_add(c_int::try_from(items.len()).unwrap_or(c_int::MAX));
            index.by_type.insert(desc.name, enum_index);
            index.descs.push(desc);
            index.items.push(items);
        }
        index
    }

    /// Every enum, by [`EnumRef::enum_index`].
    pub(crate) fn descs(&self) -> &[&'static EnumDesc] {
        &self.descs
    }

    /// The items of enum `enum_index`, by [`EnumRef::item_index`].
    pub(crate) fn items(&self, enum_index: u16) -> &[EnumItem] {
        self.items.get(usize::from(enum_index)).map_or(&[], Vec::as_slice)
    }

    /// The item a reference names.
    pub(crate) fn item(&self, item: EnumRef) -> Option<&EnumItem> {
        self.items(item.enum_index).get(usize::from(item.item_index))
    }

    /// The enum a reference belongs to.
    pub(crate) fn desc(&self, item: EnumRef) -> Option<&'static EnumDesc> {
        self.descs.get(usize::from(item.enum_index)).copied()
    }

    /// Where the item's userdata sits in the flat item table.
    pub(crate) fn flat_index(&self, item: EnumRef) -> Option<c_int> {
        let base = *self.base.get(usize::from(item.enum_index))?;
        (usize::from(item.item_index) < self.items(item.enum_index).len()).then(|| base + c_int::from(item.item_index))
    }

    /// The reference of an item: by address for the registry's shared copies, by name otherwise.
    pub(crate) fn find(&self, item: &EnumItem) -> Option<EnumRef> {
        if let Some(found) = self.by_address.get(&(item.name().as_ptr() as usize)) {
            return Some(*found);
        }
        let enum_index = *self.by_type.get(item.enum_type())?;
        let item_index = self.items(enum_index).iter().position(|i| i.name() == item.name())?;
        Some(EnumRef {
            enum_index,
            item_index: u16::try_from(item_index).ok()?,
        })
    }
}

/// Every live thread of the backend.
#[derive(Default)]
pub(crate) struct Threads {
    next: u64,
    by_id: HashMap<ThreadId, Thread>,
    by_address: HashMap<usize, ThreadId>,
    /// Arguments kept in the VM for the first resume of a `task.defer` or `task.delay` thread, so tables and
    /// functions pass through unchanged (the host would only carry engine values).
    pending: HashMap<ThreadId, MultiValue>,
}

impl Threads {
    /// The id of `thread`, giving it one if it has none.
    pub(crate) fn add(&mut self, thread: Thread) -> ThreadId {
        let address = thread.to_pointer() as usize;
        if let Some(id) = self.by_address.get(&address) {
            return *id;
        }
        self.next += 1;
        let id = ThreadId(self.next);
        self.by_id.insert(id, thread);
        self.by_address.insert(address, id);
        id
    }

    /// The thread with this id.
    pub(crate) fn get(&self, id: ThreadId) -> Option<Thread> {
        self.by_id.get(&id).cloned()
    }

    /// The id of `thread`, if the backend tracks it.
    pub(crate) fn id_of(&self, thread: &Thread) -> Option<ThreadId> {
        self.by_address.get(&(thread.to_pointer() as usize)).copied()
    }

    /// Forgets a thread.
    pub(crate) fn remove(&mut self, id: ThreadId) {
        if let Some(thread) = self.by_id.remove(&id) {
            self.by_address.remove(&(thread.to_pointer() as usize));
        }
        self.pending.remove(&id);
    }

    /// Keeps the arguments of a thread's first resume.
    pub(crate) fn set_pending(&mut self, id: ThreadId, args: MultiValue) {
        self.pending.insert(id, args);
    }

    /// Takes the arguments kept for a thread's first resume.
    pub(crate) fn take_pending(&mut self, id: ThreadId) -> Option<MultiValue> {
        self.pending.remove(&id)
    }

    /// How many threads are live.
    pub(crate) fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Drops every thread (when the backend goes away).
    pub(crate) fn clear(&mut self) {
        self.by_id.clear();
        self.by_address.clear();
        self.pending.clear();
    }
}

/// The state C functions and callbacks share with the backend.
pub(crate) struct Shared {
    /// The registry the bindings were built from.
    pub(crate) registry: Arc<Registry>,
    /// Tables C functions push from.
    pub(crate) refs: Refs,
    /// Every enum.
    pub(crate) enums: EnumIndex,
    /// `members[class][atom]`.
    members: RefCell<Vec<Vec<Member>>>,
    /// Every live thread.
    pub(crate) threads: RefCell<Threads>,
    /// The function stored under each listener key.
    pub(crate) listeners: RefCell<HashMap<ListenerKey, Function>>,
    /// Set by the interrupt when it stops a thread: the line it stopped at, if known.
    pub(crate) timed_out: Cell<Option<Option<u32>>>,
    /// Whether the running call's context is lent out (`ffi::with_ctx`), so no second `&mut` is made.
    pub(crate) ctx_lent: Cell<bool>,
}

impl Shared {
    /// State for `registry`, with the table references filled in later by the backend.
    pub(crate) fn new(registry: Arc<Registry>) -> Self {
        let enums = EnumIndex::new(&registry);
        Self {
            registry,
            refs: Refs::default(),
            enums,
            members: RefCell::new(Vec::new()),
            threads: RefCell::new(Threads::default()),
            listeners: RefCell::new(HashMap::new()),
            timed_out: Cell::new(None),
            ctx_lent: Cell::new(false),
        }
    }

    /// What `name` (with its atom, or -1) means on instances of `class`. A known atom costs two array indexes
    /// after the first lookup.
    #[inline]
    pub(crate) fn member(&self, class: ClassId, atom: i32, name: &str) -> Member {
        let Ok(atom) = usize::try_from(atom) else {
            return self.resolve(class, name);
        };
        let Ok(mut cache) = self.members.try_borrow_mut() else {
            return self.resolve(class, name);
        };
        let class_index = class.index();
        if cache.len() <= class_index {
            cache.resize_with(class_index + 1, Vec::new);
        }
        let row = &mut cache[class_index];
        if row.len() <= atom {
            row.resize(atom + 1, Member::Unresolved);
        }
        match row[atom] {
            Member::Unresolved => {
                let member = self.resolve(class, name);
                row[atom] = member;
                member
            }
            member => member,
        }
    }

    /// Looks `name` up in the registry: a property, then a method, then a signal, then the backend's own.
    fn resolve(&self, class: ClassId, name: &str) -> Member {
        let Some(info) = self.registry.class(class) else {
            return Member::Missing;
        };
        if let Some(id) = info.prop_id(name)
            && let Some(prop) = info.prop(id)
        {
            return Member::Prop(PropMember {
                id,
                ty: prop.meta.ty,
                read_only: prop.meta.flags.contains(PropFlags::READ_ONLY)
                    || prop.meta.computed.is_some_and(|c| c.set.is_none()),
                computed: prop.meta.computed.is_some(),
            });
        }
        if let Some(method) = info.method(name) {
            return Member::Method(method);
        }
        if let Some(signal) = info.signal_id(name) {
            return Member::Signal(signal);
        }
        if name == GET_PROPERTY_CHANGED_SIGNAL {
            return Member::GetPropertyChangedSignal;
        }
        Member::Missing
    }
}
