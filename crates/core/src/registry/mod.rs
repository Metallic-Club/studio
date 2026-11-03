//! The class registry: every class, component and enum, resolved once at start-up into dense, id-indexed tables.
//!
//! Class files declare themselves with the descriptors in [`desc`] and [`prop`]. [`Registry::build`] checks them
//! all and resolves each inheritance chain (see `resolve.rs`). Hot paths (the data model, renderer, physics,
//! script bindings) then work with [`ClassId`] and [`PropId`]; names are looked up once, at the edges.
//!
//! # Property ids are stable down the hierarchy
//!
//! A class's resolved property list is its parent's list first, unchanged, then the properties of the components
//! it adds, then its own new properties. So a property declared on a class has the same [`PropId`] in every class
//! that descends from it: `CFrame` on `BasePart` has one id for every kind of part. Hiding a property keeps its
//! slot, and an override (`PropFlags::OVERRIDE` or [`ClassDesc::defaults`]) replaces the entry in place.

mod desc;
mod error;
pub(crate) mod hash;
mod prop;
mod resolve;

use std::fmt;

pub use desc::{ClassBehavior, ClassDesc, ComponentDesc, EnumDesc, MethodCall, MethodDesc, MethodFn, SignalDesc};
pub use error::{MemberKind, Owner, RegistryError, RegistryErrors};
pub use prop::{Computed, DefaultValue, Getter, PropFlags, PropMeta, Setter, Widget};

use crate::{EnumItem, Value};
use hash::FxMap;

/// A class's index in the [`Registry`]. Ids are dense (`0..classes().len()`) and a parent's id is lower than its
/// children's.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClassId(u16);

impl ClassId {
    /// The id with this raw value. Ids come from the registry; building one by hand is for iterating
    /// `0..classes().len()`.
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// The raw value.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// The raw value as an index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A property's index in its class's resolved property list (see the module docs: a property keeps its id in
/// every descendant of the class that declares it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PropId(u16);

impl PropId {
    /// The id with this raw value. Ids come from the registry; building one by hand is for iterating
    /// `0..props(class).len()`.
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// The raw value.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// The raw value as an index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Everything the registry is built from. `metallic_classes::input()` returns the discovered set.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegistryInput<'a> {
    /// Every class.
    pub classes: &'a [&'static ClassDesc],
    /// Every component.
    pub components: &'a [&'static ComponentDesc],
    /// Every file's `ENUMS` list.
    pub enums: &'a [&'static [EnumDesc]],
}

/// One property of a resolved class.
#[derive(Clone, Debug)]
pub struct ResolvedProp {
    /// The declaration (the override's, when a class replaced it).
    pub meta: &'static PropMeta,
    /// The value a new instance of this class starts with, after any `defaults(..)` change. Cloning it never
    /// allocates.
    pub default: Value,
    /// The class that declared it, overrode it, or added the component that brings it.
    pub owner: ClassId,
    /// Hidden by `hide(..)`: scripts and Studio do not see it, but it keeps its slot.
    pub hidden: bool,
}

/// One method or signal of a resolved class.
#[derive(Debug)]
pub struct ResolvedMember<T: 'static> {
    /// The declaration (the nearest class's, when a child replaced a method).
    pub desc: &'static T,
    /// The class that declared it.
    pub owner: ClassId,
    /// Hidden by `hide(..)`: name lookups do not find it.
    pub hidden: bool,
}

// Derived `Clone` / `Copy` would require `T: Clone`; the member only holds a reference.
impl<T> Clone for ResolvedMember<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ResolvedMember<T> {}

/// A resolved method.
pub type ResolvedMethod = ResolvedMember<MethodDesc>;

/// A resolved signal.
pub type ResolvedSignal = ResolvedMember<SignalDesc>;

/// A class with its inheritance resolved: every property, component, method and signal it has, inherited ones
/// first.
#[derive(Debug)]
pub struct ClassInfo {
    id: ClassId,
    desc: &'static ClassDesc,
    parent: Option<ClassId>,
    /// Root first, this class last, so `ancestors[depth]` is the ancestor at that depth.
    ancestors: Box<[ClassId]>,
    props: Box<[ResolvedProp]>,
    /// Names and aliases, hidden properties included (lookups filter them).
    prop_ids: FxMap<&'static str, PropId>,
    components: Box<[&'static ComponentDesc]>,
    methods: Box<[ResolvedMethod]>,
    /// Visible methods only.
    method_ids: FxMap<&'static str, usize>,
    signals: Box<[ResolvedSignal]>,
    /// Visible signals only.
    signal_ids: FxMap<&'static str, usize>,
}

impl ClassInfo {
    /// The class's id.
    pub fn id(&self) -> ClassId {
        self.id
    }

    /// The class's name.
    pub fn name(&self) -> &'static str {
        self.desc.name
    }

    /// The descriptor it was built from.
    pub fn desc(&self) -> &'static ClassDesc {
        self.desc
    }

    /// The parent class, or `None` for the root.
    pub fn parent(&self) -> Option<ClassId> {
        self.parent
    }

    /// How many classes are above it: 0 for the root.
    pub fn depth(&self) -> usize {
        self.ancestors.len() - 1
    }

    /// Its ancestors from the root down, ending with the class itself.
    pub fn ancestors(&self) -> &[ClassId] {
        &self.ancestors
    }

    /// Whether scripts and Studio may create it.
    pub fn creatable(&self) -> bool {
        self.desc.creatable
    }

    /// The service creation order, or `None` when it is not a service.
    pub fn service_order(&self) -> Option<i16> {
        self.desc.service_order
    }

    /// Whether a service's contents are saved.
    pub fn persisted(&self) -> bool {
        self.desc.persisted
    }

    /// Whether scripts under the service run.
    pub fn runs_scripts(&self) -> bool {
        self.desc.runs_scripts
    }

    /// Whether Studio's Explorer leaves it out.
    pub fn hidden_in_explorer(&self) -> bool {
        self.desc.hidden_in_explorer
    }

    /// Every property, indexed by [`PropId`]; hidden ones included (check [`ResolvedProp::hidden`]).
    pub fn props(&self) -> &[ResolvedProp] {
        &self.props
    }

    /// The property with this id.
    pub fn prop(&self, id: PropId) -> Option<&ResolvedProp> {
        self.props.get(id.index())
    }

    /// The id of the visible property with this name or alias.
    pub fn prop_id(&self, name: &str) -> Option<PropId> {
        let id = *self.prop_ids.get(name)?;
        let hidden = self.props.get(id.index())?.hidden;
        (!hidden).then_some(id)
    }

    /// Every component, inherited ones first, without repeats.
    pub fn components(&self) -> &[&'static ComponentDesc] {
        &self.components
    }

    /// Whether it has the component with this name.
    pub fn has_component(&self, name: &str) -> bool {
        self.components.iter().any(|c| c.name == name)
    }

    /// Every method, inherited ones first; hidden ones included (check [`ResolvedMember::hidden`]).
    pub fn methods(&self) -> &[ResolvedMethod] {
        &self.methods
    }

    /// The visible method with this name.
    pub fn method(&self, name: &str) -> Option<&'static MethodDesc> {
        self.method_ids
            .get(name)
            .and_then(|&i| self.methods.get(i))
            .map(|m| m.desc)
    }

    /// Every signal, inherited ones first; hidden ones included (check [`ResolvedMember::hidden`]).
    pub fn signals(&self) -> &[ResolvedSignal] {
        &self.signals
    }

    /// The visible signal with this name.
    pub fn signal(&self, name: &str) -> Option<&'static SignalDesc> {
        self.signal_ids
            .get(name)
            .and_then(|&i| self.signals.get(i))
            .map(|s| s.desc)
    }
}

/// An enum with its items built once, so looking one up only clones a pointer.
#[derive(Debug)]
struct ResolvedEnum {
    desc: &'static EnumDesc,
    /// `items[i]` is `desc.items[i]`.
    items: Box<[EnumItem]>,
    item_ids: FxMap<&'static str, usize>,
}

/// Every class, component and enum, resolved and checked. Built once at start-up with [`Registry::build`], then
/// only read.
pub struct Registry {
    classes: Vec<ClassInfo>,
    class_ids: FxMap<&'static str, ClassId>,
    components: FxMap<&'static str, &'static ComponentDesc>,
    enums: Vec<ResolvedEnum>,
    enum_ids: FxMap<&'static str, usize>,
    services: Vec<ClassId>,
}

impl Registry {
    /// Checks every descriptor and resolves every class.
    ///
    /// On failure the error lists every problem found, not only the first, each naming the class, component or
    /// enum it is about. A class whose parent failed is not checked further, so one mistake is reported once.
    pub fn build(input: RegistryInput<'_>) -> Result<Self, RegistryErrors> {
        resolve::build(input)
    }

    /// The id of the class with this name.
    pub fn class_id(&self, name: &str) -> Option<ClassId> {
        self.class_ids.get(name).copied()
    }

    /// The class with this id.
    pub fn class(&self, id: ClassId) -> Option<&ClassInfo> {
        self.classes.get(id.index())
    }

    /// Every class, indexed by [`ClassId`].
    pub fn classes(&self) -> &[ClassInfo] {
        &self.classes
    }

    /// Whether `class` is `ancestor` or inherits from it. Constant time: one look into `class`'s ancestor table.
    pub fn is_a(&self, class: ClassId, ancestor: ClassId) -> bool {
        match (self.classes.get(class.index()), self.classes.get(ancestor.index())) {
            (Some(class), Some(ancestor_info)) => class.ancestors.get(ancestor_info.depth()) == Some(&ancestor),
            _ => false,
        }
    }

    /// The id of `class`'s visible property with this name or alias.
    pub fn prop_id(&self, class: ClassId, name: &str) -> Option<PropId> {
        self.class(class)?.prop_id(name)
    }

    /// `class`'s property with this id.
    pub fn prop(&self, class: ClassId, prop: PropId) -> Option<&ResolvedProp> {
        self.class(class)?.prop(prop)
    }

    /// Every property of `class`, indexed by [`PropId`] (empty for an unknown class).
    pub fn props(&self, class: ClassId) -> &[ResolvedProp] {
        self.class(class).map(ClassInfo::props).unwrap_or_default()
    }

    /// `class`'s visible method with this name.
    pub fn method(&self, class: ClassId, name: &str) -> Option<&'static MethodDesc> {
        self.class(class)?.method(name)
    }

    /// `class`'s visible signal with this name.
    pub fn signal(&self, class: ClassId, name: &str) -> Option<&'static SignalDesc> {
        self.class(class)?.signal(name)
    }

    /// The component with this name.
    pub fn component(&self, name: &str) -> Option<&'static ComponentDesc> {
        self.components.get(name).copied()
    }

    /// The enum with this name (without `Enum.`).
    pub fn enum_desc(&self, name: &str) -> Option<&'static EnumDesc> {
        self.enum_ids.get(name).and_then(|&i| self.enums.get(i)).map(|e| e.desc)
    }

    /// Every enum, in declaration order.
    pub fn enums(&self) -> impl Iterator<Item = &'static EnumDesc> + '_ {
        self.enums.iter().map(|e| e.desc)
    }

    /// The item `name` of the enum `enum_type`. It shares its storage with every other copy of that item, so this
    /// does not allocate.
    pub fn enum_item(&self, enum_type: &str, name: &str) -> Option<EnumItem> {
        let e = self.enums.get(*self.enum_ids.get(enum_type)?)?;
        e.items.get(*e.item_ids.get(name)?).cloned()
    }

    /// Every service, in creation order (by service order, then name).
    pub fn services(&self) -> &[ClassId] {
        &self.services
    }
}

/// Summarises the registry; the full tables are too large to print usefully.
impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("classes", &self.classes.len())
            .field("components", &self.components.len())
            .field("enums", &self.enums.len())
            .field("services", &self.services.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
