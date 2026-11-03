//! Building the registry: checking every descriptor and resolving each class's inheritance chain.
//!
//! The order is: enums, then the class names, then components (their properties may name enums and classes), then
//! the inheritance graph (unknown parents, cycles, roots), then each class from the root down. A class is resolved
//! from its parent's finished lists, so it costs one copy of those lists plus its own additions.

use std::sync::Arc;

use super::hash::FxMap;
use super::{
    ClassDesc, ClassId, ClassInfo, ComponentDesc, DefaultValue, EnumDesc, MemberKind, Owner, PropFlags, PropId,
    PropMeta, Registry, RegistryError, RegistryErrors, RegistryInput, ResolvedEnum, ResolvedMember, ResolvedMethod,
    ResolvedProp, ResolvedSignal,
};
use crate::{CFrame, Color3, PropType, Value, Vector2, Vector3};

/// The most classes, or properties in one class, that a `u16` id can number.
const MAX_IDS: usize = u16::MAX as usize;

/// Builds the registry from `input`, or returns every problem found.
pub(super) fn build(input: RegistryInput<'_>) -> Result<Registry, RegistryErrors> {
    let mut errors = Vec::new();

    if input.classes.len() > MAX_IDS {
        errors.push(RegistryError::TooManyClasses {
            count: input.classes.len(),
        });
        return Err(RegistryErrors(errors));
    }

    let (enums, enum_ids) = build_enums(input.enums, &mut errors);

    let mut class_index: FxMap<&'static str, usize> = FxMap::default();
    let mut duplicate = vec![false; input.classes.len()];
    for (i, class) in input.classes.iter().enumerate() {
        if class_index.contains_key(class.name) {
            errors.push(RegistryError::DuplicateClass { class: class.name });
            duplicate[i] = true;
        } else {
            class_index.insert(class.name, i);
        }
    }

    let names = Names {
        classes: &class_index,
        enums: &enums,
        enum_ids: &enum_ids,
    };
    let components = build_components(input.components, &names, &mut errors);

    let depths = class_depths(input.classes, &class_index, &duplicate, &mut errors);

    let roots: Vec<&'static str> = input
        .classes
        .iter()
        .zip(&duplicate)
        .filter(|(class, dup)| class.parent.is_none() && !**dup)
        .map(|(class, _)| class.name)
        .collect();
    if roots.len() > 1 {
        errors.push(RegistryError::MultipleRoots { roots });
    }

    // Parents before children; by name within a depth, so ids do not depend on file order.
    let mut order: Vec<(usize, &'static ClassDesc)> = input
        .classes
        .iter()
        .zip(&depths)
        .filter_map(|(class, depth)| depth.map(|d| (d, *class)))
        .collect();
    order.sort_by_key(|(depth, class)| (*depth, class.name));

    let mut classes: Vec<ClassInfo> = Vec::with_capacity(order.len());
    let mut class_ids: FxMap<&'static str, ClassId> = FxMap::default();
    class_ids.reserve(order.len());
    for (_, desc) in order {
        let parent = match desc.parent {
            None => None,
            // A parent that failed to resolve has already been reported; its children are skipped.
            Some(name) => match class_ids.get(name) {
                Some(&id) => Some(id),
                None => continue,
            },
        };
        let Ok(raw) = u16::try_from(classes.len()) else {
            break; // unreachable: the class count was checked above
        };
        let id = ClassId(raw);
        let resolver = ClassResolver {
            desc,
            id,
            names: &names,
            components: &components,
            classes: &classes,
            errors: &mut errors,
        };
        if let Some(info) = resolver.resolve(parent) {
            class_ids.insert(desc.name, id);
            classes.push(info);
        }
    }

    if !errors.is_empty() {
        return Err(RegistryErrors(errors));
    }

    let mut services: Vec<ClassId> = classes
        .iter()
        .filter(|c| c.service_order().is_some())
        .map(|c| c.id)
        .collect();
    services.sort_by_key(|&id| {
        let class = &classes[id.index()];
        (class.service_order(), class.name())
    });

    Ok(Registry {
        classes,
        class_ids,
        components: components.iter().map(|(name, c)| (*name, c.desc)).collect(),
        enums,
        enum_ids,
        services,
    })
}

/// The names properties can refer to, for checking their types.
struct Names<'a> {
    /// Every class name (to its input index).
    classes: &'a FxMap<&'static str, usize>,
    enums: &'a [ResolvedEnum],
    enum_ids: &'a FxMap<&'static str, usize>,
}

impl Names<'_> {
    fn enum_by_name(&self, name: &str) -> Option<&ResolvedEnum> {
        self.enums.get(*self.enum_ids.get(name)?)
    }
}

/// Checks every enum and builds its items once.
fn build_enums(
    lists: &[&'static [EnumDesc]],
    errors: &mut Vec<RegistryError>,
) -> (Vec<ResolvedEnum>, FxMap<&'static str, usize>) {
    let mut enums = Vec::new();
    let mut enum_ids: FxMap<&'static str, usize> = FxMap::default();
    for desc in lists.iter().flat_map(|list| list.iter()) {
        if enum_ids.contains_key(desc.name) {
            errors.push(RegistryError::DuplicateEnum { name: desc.name });
            continue;
        }
        if !desc.numbers.is_empty() && desc.numbers.len() != desc.items.len() {
            errors.push(RegistryError::EnumNumbersMismatch {
                name: desc.name,
                items: desc.items.len(),
                numbers: desc.numbers.len(),
            });
        }
        let mut item_ids: FxMap<&'static str, usize> = FxMap::default();
        for (i, item) in desc.items.iter().enumerate() {
            if item_ids.insert(item, i).is_some() {
                errors.push(RegistryError::DuplicateEnumItem { name: desc.name, item });
            }
        }
        let items = desc
            .items
            .iter()
            .map(|item| crate::EnumItem::new(desc.name, item))
            .collect();
        enum_ids.insert(desc.name, enums.len());
        enums.push(ResolvedEnum { desc, items, item_ids });
    }
    (enums, enum_ids)
}

/// A component with its property defaults built once.
struct ComponentInfo {
    desc: &'static ComponentDesc,
    /// One per property; `None` for a property that failed its checks or repeats an earlier name.
    defaults: Vec<Option<Value>>,
    /// Whether every check passed. A class with a broken component is not built (the problem is reported once,
    /// for the component).
    ok: bool,
}

/// Checks every component and its properties.
fn build_components(
    components: &[&'static ComponentDesc],
    names: &Names<'_>,
    errors: &mut Vec<RegistryError>,
) -> FxMap<&'static str, ComponentInfo> {
    let mut out: FxMap<&'static str, ComponentInfo> = FxMap::default();
    for &desc in components {
        if out.contains_key(desc.name) {
            errors.push(RegistryError::DuplicateComponent { component: desc.name });
            continue;
        }
        let before = errors.len();
        let owner = Owner::Component(desc.name);
        let mut defaults = Vec::with_capacity(desc.props.len());
        for (i, meta) in desc.props.iter().enumerate() {
            if desc.props[..i].iter().any(|p| p.name == meta.name) {
                errors.push(RegistryError::DuplicateProp { owner, prop: meta.name });
                defaults.push(None);
            } else {
                defaults.push(check_prop(owner, meta, names, errors));
            }
        }
        for &requires in desc.requires {
            if !components.iter().any(|c| c.name == requires) {
                errors.push(RegistryError::UnknownRequiredComponent {
                    component: desc.name,
                    requires,
                });
            }
        }
        let ok = errors.len() == before;
        out.insert(desc.name, ComponentInfo { desc, defaults, ok });
    }
    out
}

/// Where a class stands while its inheritance graph is walked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Walk {
    New,
    Visiting,
    Depth(usize),
    Broken,
}

/// Each class's depth (0 for a root), or `None` when its chain is broken: an unknown parent, a cycle, a duplicate
/// class, or an ancestor with one of those. Reports each unknown parent and each cycle once.
fn class_depths(
    classes: &[&'static ClassDesc],
    class_index: &FxMap<&'static str, usize>,
    duplicate: &[bool],
    errors: &mut Vec<RegistryError>,
) -> Vec<Option<usize>> {
    let mut walk: Vec<Walk> = duplicate
        .iter()
        .map(|&dup| if dup { Walk::Broken } else { Walk::New })
        .collect();
    let mut path: Vec<usize> = Vec::new();
    for start in 0..classes.len() {
        if walk[start] != Walk::New {
            continue;
        }
        path.clear();
        let mut current = start;
        // The depth of `path.last()`, once the walk reaches a root or an already known class.
        let depth = loop {
            match walk[current] {
                Walk::Depth(d) => break Some(d + 1),
                Walk::Broken => break None,
                Walk::Visiting => {
                    let from = path.iter().position(|&i| i == current).unwrap_or(0);
                    let mut cycle: Vec<&'static str> = path[from..].iter().map(|&i| classes[i].name).collect();
                    cycle.push(classes[current].name);
                    errors.push(RegistryError::InheritanceCycle { classes: cycle });
                    break None;
                }
                Walk::New => {
                    walk[current] = Walk::Visiting;
                    path.push(current);
                    let class = classes[current];
                    match class.parent {
                        None => break Some(0),
                        Some(parent) => match class_index.get(parent) {
                            Some(&p) => current = p,
                            None => {
                                errors.push(RegistryError::UnknownParent {
                                    class: class.name,
                                    parent,
                                });
                                break None;
                            }
                        },
                    }
                }
            }
        };
        for (above, &i) in path.iter().rev().enumerate() {
            walk[i] = depth.map_or(Walk::Broken, |d| Walk::Depth(d + above));
        }
    }
    walk.into_iter()
        .map(|w| match w {
            Walk::Depth(d) => Some(d),
            _ => None,
        })
        .collect()
}

/// Checks one property declaration and builds its default.
fn check_prop(
    owner: Owner,
    meta: &'static PropMeta,
    names: &Names<'_>,
    errors: &mut Vec<RegistryError>,
) -> Option<Value> {
    let mut ok = true;
    if let PropType::Ref(class) = meta.ty
        && !names.classes.contains_key(class)
    {
        errors.push(RegistryError::UnknownRefClass {
            owner,
            prop: meta.name,
            class,
        });
        ok = false;
    }
    if let PropType::Enum(enum_type) = meta.ty
        && names.enum_by_name(enum_type).is_none()
    {
        errors.push(RegistryError::UnknownEnum {
            owner,
            prop: meta.name,
            enum_type,
        });
        return None;
    }
    if let Some((min, max)) = meta.range
        && !valid_range(min, max)
    {
        errors.push(RegistryError::InvalidRange {
            owner,
            prop: meta.name,
            min,
            max,
        });
        ok = false;
    }
    let value = default_value(owner, meta, meta.default, names, errors);
    if ok { value } else { None }
}

/// Whether `(min, max)` is a usable range: `min <= max`, which is false when either bound is NaN.
fn valid_range(min: f64, max: f64) -> bool {
    min <= max
}

/// Turns `default` into a value for the property `meta`, checking its type and range.
fn default_value(
    owner: Owner,
    meta: &PropMeta,
    default: DefaultValue,
    names: &Names<'_>,
    errors: &mut Vec<RegistryError>,
) -> Option<Value> {
    let value = match (default, meta.ty) {
        (DefaultValue::Bool(b), PropType::Bool) => Value::Bool(b),
        (DefaultValue::Int(i), PropType::Int) => Value::Int(i),
        (DefaultValue::Number(n), PropType::Number) => Value::Number(n),
        (DefaultValue::Str(s), PropType::String) => Value::String(Arc::from(s)),
        (DefaultValue::Vector2(x, y), PropType::Vector2) => Value::Vector2(Vector2::new(x, y)),
        (DefaultValue::Vector3(x, y, z), PropType::Vector3) => Value::Vector3(Vector3::new(x, y, z)),
        (DefaultValue::CFrame(x, y, z), PropType::CFrame) => Value::CFrame(CFrame::new(Vector3::new(x, y, z))),
        (DefaultValue::Color3(r, g, b), PropType::Color3) => Value::Color3(Color3::new(r, g, b)),
        (DefaultValue::Enum(item), PropType::Enum(enum_type)) => {
            // An unknown enum type is reported where the property is declared.
            let resolved = names.enum_by_name(enum_type)?;
            match resolved.item_ids.get(item).and_then(|&i| resolved.items.get(i)) {
                Some(found) => Value::Enum(found.clone()),
                None => {
                    errors.push(RegistryError::UnknownEnumItem {
                        owner,
                        prop: meta.name,
                        enum_type,
                        item,
                    });
                    return None;
                }
            }
        }
        (DefaultValue::NilRef, PropType::Ref(_)) => Value::Ref(None),
        (default, expected) => {
            errors.push(RegistryError::DefaultTypeMismatch {
                owner,
                prop: meta.name,
                expected,
                found: default.kind_name(),
            });
            return None;
        }
    };
    let number = match value {
        Value::Number(n) => Some(n),
        Value::Int(i) => Some(i as f64),
        _ => None,
    };
    if let (Some(n), Some((min, max))) = (number, meta.range)
        && valid_range(min, max)
        && (n < min || n > max)
    {
        errors.push(RegistryError::DefaultOutOfRange {
            owner,
            prop: meta.name,
            default: n,
            min,
            max,
        });
        return None;
    }
    Some(value)
}

/// A class's stored property → derived properties table: `ClassInfo::dependent_starts` and `dependents`.
type DependentTables = (Box<[u32]>, Box<[PropId]>);

/// Resolves one class from its parent's finished tables.
struct ClassResolver<'a> {
    desc: &'static ClassDesc,
    id: ClassId,
    names: &'a Names<'a>,
    components: &'a FxMap<&'static str, ComponentInfo>,
    /// The classes resolved so far: every ancestor is among them.
    classes: &'a [ClassInfo],
    errors: &'a mut Vec<RegistryError>,
}

/// The tables of the class being resolved, before they are frozen into a `ClassInfo`.
struct Tables {
    props: Vec<ResolvedProp>,
    /// Who declared each property, for messages and to tell a repeat from an override.
    owners: Vec<Owner>,
    /// Property names (not aliases) to their index.
    by_name: FxMap<&'static str, usize>,
    components: Vec<&'static ComponentDesc>,
    methods: Vec<ResolvedMethod>,
    signals: Vec<ResolvedSignal>,
}

impl ClassResolver<'_> {
    /// Resolves the class, or returns `None` after reporting its problems.
    fn resolve(mut self, parent: Option<ClassId>) -> Option<ClassInfo> {
        let (classes, components, names) = (self.classes, self.components, self.names);
        let parent = parent.and_then(|id| classes.get(id.index()));
        let before = self.errors.len();
        let mut ok = true;
        let mut t = self.inherit(parent);

        let first_new_component = t.components.len();
        for &name in self.desc.components {
            if t.components.iter().any(|c| c.name == name) {
                continue;
            }
            let Some(component) = components.get(name) else {
                self.errors.push(RegistryError::UnknownComponent {
                    class: self.desc.name,
                    component: name,
                });
                continue;
            };
            ok &= component.ok;
            t.components.push(component.desc);
            for (meta, default) in component.desc.props.iter().zip(&component.defaults) {
                if let Some(default) = default {
                    self.add_prop(&mut t, Owner::Component(name), meta, default.clone());
                }
            }
        }
        for component in &t.components[first_new_component..] {
            for &requires in component.requires {
                let declared = components.contains_key(requires);
                if declared && !t.components.iter().any(|c| c.name == requires) {
                    self.errors.push(RegistryError::MissingRequiredComponent {
                        class: self.desc.name,
                        component: component.name,
                        requires,
                    });
                }
            }
        }

        let owner = Owner::Class(self.desc.name);
        for meta in self.desc.props {
            match check_prop(owner, meta, names, self.errors) {
                Some(default) => self.add_prop(&mut t, owner, meta, default),
                None => ok = false,
            }
        }

        for &(name, default) in self.desc.defaults {
            let Some(&i) = t.by_name.get(name) else {
                self.errors.push(RegistryError::UnknownDefault {
                    class: self.desc.name,
                    prop: name,
                });
                continue;
            };
            let prop = &mut t.props[i];
            if let Some(value) = default_value(owner, prop.meta, default, names, self.errors) {
                prop.default = value;
            }
        }

        add_members(
            &mut t.methods,
            self.desc.methods,
            |m| m.name,
            self.id,
            MemberKind::Method,
            self.desc.name,
            self.errors,
        );
        add_members(
            &mut t.signals,
            self.desc.signals,
            |s| s.name,
            self.id,
            MemberKind::Signal,
            self.desc.name,
            self.errors,
        );

        for &name in self.desc.hide {
            let mut found = false;
            if let Some(&i) = t.by_name.get(name) {
                t.props[i].hidden = true;
                found = true;
            }
            for m in t.methods.iter_mut().filter(|m| m.desc.name == name) {
                m.hidden = true;
                found = true;
            }
            for s in t.signals.iter_mut().filter(|s| s.desc.name == name) {
                s.hidden = true;
                found = true;
            }
            if !found {
                self.errors.push(RegistryError::UnknownHidden {
                    class: self.desc.name,
                    member: name,
                });
            }
        }

        if t.props.len() > MAX_IDS {
            self.errors.push(RegistryError::TooManyProps {
                class: self.desc.name,
                count: t.props.len(),
            });
            return None;
        }

        let info = self.freeze(t, parent)?;
        (ok && self.errors.len() == before).then_some(info)
    }

    /// Starts from a copy of the parent's tables (empty for a root).
    fn inherit(&self, parent: Option<&ClassInfo>) -> Tables {
        let Some(parent) = parent else {
            return Tables {
                props: Vec::new(),
                owners: Vec::new(),
                by_name: FxMap::default(),
                components: Vec::new(),
                methods: Vec::new(),
                signals: Vec::new(),
            };
        };
        let extra = self.desc.props.len();
        let mut props = Vec::with_capacity(parent.props.len() + extra);
        props.extend_from_slice(&parent.props);
        let owners = props
            .iter()
            .map(|p| Owner::Class(self.classes.get(p.owner.index()).map_or("", ClassInfo::name)))
            .collect();
        let mut by_name: FxMap<&'static str, usize> = FxMap::default();
        by_name.reserve(props.len() + extra);
        by_name.extend(props.iter().enumerate().map(|(i, p)| (p.meta.name, i)));
        Tables {
            props,
            owners,
            by_name,
            components: parent.components.to_vec(),
            methods: parent.methods.to_vec(),
            signals: parent.signals.to_vec(),
        }
    }

    /// Adds a property, or replaces an earlier one when it is marked `OVERRIDE`.
    fn add_prop(&mut self, t: &mut Tables, owner: Owner, meta: &'static PropMeta, default: Value) {
        let class = self.desc.name;
        let fresh = ResolvedProp {
            meta,
            default,
            owner: self.id,
            hidden: false,
        };
        let Some(&i) = t.by_name.get(meta.name) else {
            if meta.flags.contains(PropFlags::OVERRIDE) {
                self.errors.push(RegistryError::OverridesNothing {
                    owner,
                    class,
                    prop: meta.name,
                });
            }
            t.by_name.insert(meta.name, t.props.len());
            t.props.push(fresh);
            t.owners.push(owner);
            return;
        };
        let existing = t.owners[i];
        if existing == owner {
            self.errors
                .push(RegistryError::DuplicateProp { owner, prop: meta.name });
        } else if !meta.flags.contains(PropFlags::OVERRIDE) {
            self.errors.push(RegistryError::RedeclaredProp {
                owner,
                class,
                prop: meta.name,
                inherited_from: existing.name(),
            });
        } else if t.props[i].meta.ty != meta.ty {
            self.errors.push(RegistryError::OverrideChangesType {
                owner,
                prop: meta.name,
                expected: t.props[i].meta.ty,
                found: meta.ty,
            });
        } else {
            t.props[i] = fresh;
            t.owners[i] = owner;
        }
    }

    /// Builds the lookup tables and the `ClassInfo`, checking aliases and member names. `None` on a problem.
    fn freeze(&mut self, t: Tables, parent: Option<&ClassInfo>) -> Option<ClassInfo> {
        let class = self.desc.name;
        let mut ok = true;

        let mut prop_ids: FxMap<&'static str, PropId> = FxMap::default();
        prop_ids.reserve(t.props.len());
        for (i, prop) in t.props.iter().enumerate() {
            let id = PropId(u16::try_from(i).ok()?);
            for &name in std::iter::once(&prop.meta.name).chain(prop.meta.aliases) {
                match prop_ids.get(name) {
                    Some(&other) if other != id => {
                        let other = t.props.get(other.index()).map_or("", |p| p.meta.name);
                        self.errors.push(RegistryError::AliasCollision {
                            class,
                            alias: name,
                            prop: prop.meta.name,
                            other,
                        });
                        ok = false;
                    }
                    Some(_) => {}
                    None => {
                        prop_ids.insert(name, id);
                    }
                }
            }
        }

        let dependents = self.dependents(&t.props, &prop_ids);
        ok &= dependents.is_some();
        let (dependent_starts, dependents) = dependents.unwrap_or_default();

        let method_ids = visible_ids(&t.methods, |m| m.name);
        let signal_ids = visible_ids(&t.signals, |s| s.name);
        let visible_prop = |name: &str| prop_ids.get(name).is_some_and(|id| !t.props[id.index()].hidden);
        let clashes = method_ids
            .keys()
            .filter(|name| visible_prop(name))
            .map(|name| (*name, MemberKind::Property, MemberKind::Method))
            .chain(signal_ids.keys().filter_map(|name| {
                if visible_prop(name) {
                    Some((*name, MemberKind::Property, MemberKind::Signal))
                } else if method_ids.contains_key(name) {
                    Some((*name, MemberKind::Method, MemberKind::Signal))
                } else {
                    None
                }
            }));
        let mut clashes: Vec<_> = clashes.collect();
        clashes.sort_by_key(|(name, ..)| *name);
        for (name, first, second) in clashes {
            self.errors.push(RegistryError::MemberNameClash {
                class,
                name,
                first,
                second,
            });
            ok = false;
        }

        let ancestors = match parent {
            Some(parent) => parent.ancestors.iter().copied().chain([self.id]).collect(),
            None => Box::from([self.id]),
        };
        ok.then(|| ClassInfo {
            id: self.id,
            desc: self.desc,
            parent: parent.map(|p| p.id),
            ancestors,
            props: t.props.into_boxed_slice(),
            prop_ids,
            dependent_starts,
            dependents,
            components: t.components.into_boxed_slice(),
            methods: t.methods.into_boxed_slice(),
            method_ids,
            signals: t.signals.into_boxed_slice(),
            signal_ids,
        })
    }

    /// Checks every `derived_from` list of the class and turns them into the stored property → computed
    /// properties table (see `ClassInfo::dependents`). Inherited properties are checked again because an override
    /// in this class can make a source computed. `None` after reporting a problem.
    fn dependents(
        &mut self,
        props: &[ResolvedProp],
        prop_ids: &FxMap<&'static str, PropId>,
    ) -> Option<DependentTables> {
        let class = self.desc.name;
        let before = self.errors.len();
        // (stored source, computed dependent), dependents in property order.
        let mut pairs: Vec<(PropId, PropId)> = Vec::new();
        for (i, prop) in props.iter().enumerate() {
            let meta = prop.meta;
            if meta.derived_from.is_empty() {
                continue;
            }
            if meta.computed.is_none() {
                self.errors
                    .push(RegistryError::DerivedStoredProp { class, prop: meta.name });
                continue;
            }
            let dependent = PropId(u16::try_from(i).ok()?);
            for &from in meta.derived_from {
                let Some(&source) = prop_ids.get(from) else {
                    self.errors.push(RegistryError::UnknownDerivedSource {
                        class,
                        prop: meta.name,
                        from,
                    });
                    continue;
                };
                if props.get(source.index()).is_none_or(|p| p.meta.computed.is_some()) {
                    self.errors.push(RegistryError::DerivedFromComputed {
                        class,
                        prop: meta.name,
                        from,
                    });
                    continue;
                }
                // A source named twice (or by a name and an alias) would fire the dependent twice per change.
                if !pairs.contains(&(source, dependent)) {
                    pairs.push((source, dependent));
                }
            }
        }
        if self.errors.len() != before {
            return None;
        }
        // Most classes derive nothing; their tables stay empty and cost no allocation.
        if pairs.is_empty() {
            return Some((Box::default(), Box::default()));
        }
        // Stable: each source's dependents keep property order.
        pairs.sort_by_key(|&(source, _)| source);
        let mut starts = Vec::with_capacity(props.len() + 1);
        let mut next = 0;
        for p in 0..props.len() {
            starts.push(u32::try_from(next).ok()?);
            while pairs.get(next).is_some_and(|&(source, _)| source.index() == p) {
                next += 1;
            }
        }
        starts.push(u32::try_from(next).ok()?);
        let dependents = pairs.into_iter().map(|(_, dependent)| dependent).collect();
        Some((starts.into_boxed_slice(), dependents))
    }
}

/// Adds a class's own methods or signals: a repeat within the class is an error, and a name the class inherits
/// replaces the inherited member in place.
fn add_members<T>(
    members: &mut Vec<ResolvedMember<T>>,
    own: &'static [T],
    name_of: impl Fn(&T) -> &'static str,
    id: ClassId,
    kind: MemberKind,
    class: &'static str,
    errors: &mut Vec<RegistryError>,
) {
    for (i, desc) in own.iter().enumerate() {
        let name = name_of(desc);
        if own[..i].iter().any(|d| name_of(d) == name) {
            errors.push(RegistryError::DuplicateMember { class, kind, name });
            continue;
        }
        let fresh = ResolvedMember {
            desc,
            owner: id,
            hidden: false,
        };
        match members.iter_mut().find(|m| name_of(m.desc) == name) {
            Some(inherited) => *inherited = fresh,
            None => members.push(fresh),
        }
    }
}

/// Name to index for the members that are not hidden.
fn visible_ids<T>(members: &[ResolvedMember<T>], name_of: impl Fn(&T) -> &'static str) -> FxMap<&'static str, usize> {
    members
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.hidden)
        .map(|(i, m)| (name_of(m.desc), i))
        .collect()
}
