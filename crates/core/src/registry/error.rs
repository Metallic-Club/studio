//! What can be wrong with the class descriptors, as reported by `Registry::build`.

use std::fmt;

use thiserror::Error;

use crate::PropType;

/// Where a property is declared: in a class or in a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The class with this name.
    Class(&'static str),
    /// The component with this name.
    Component(&'static str),
}

impl Owner {
    /// The class or component name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Class(name) | Self::Component(name) => name,
        }
    }
}

/// Writes `class `SpotLight`` or `component `Transform``.
impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Class(name) => write!(f, "class `{name}`"),
            Self::Component(name) => write!(f, "component `{name}`"),
        }
    }
}

/// The kind of a class member, for messages about members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    /// A property (or one of its aliases).
    Property,
    /// A method.
    Method,
    /// A signal.
    Signal,
}

impl fmt::Display for MemberKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Property => "property",
            Self::Method => "method",
            Self::Signal => "signal",
        })
    }
}

/// One problem in the class, component or enum descriptors. Each message names where the problem is, what is
/// wrong and how to fix it.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum RegistryError {
    /// Two classes have the same name.
    #[error("class `{class}` is declared twice; rename one of them")]
    DuplicateClass {
        /// The repeated name.
        class: &'static str,
    },
    /// A class names a parent that does not exist.
    #[error("class `{class}` has parent `{parent}`, which is not a class; fix the name or add the class")]
    UnknownParent {
        /// The class.
        class: &'static str,
        /// The missing parent.
        parent: &'static str,
    },
    /// Classes inherit from each other in a loop.
    #[error("classes inherit in a cycle ({}); one of them must have a different parent", .classes.join(" -> "))]
    InheritanceCycle {
        /// The classes in the cycle, ending with the first one again.
        classes: Vec<&'static str>,
    },
    /// More than one class has no parent.
    #[error(
        "classes {} have no parent, but only the root class may have none; give the others a parent",
        list(.roots)
    )]
    MultipleRoots {
        /// Every class without a parent.
        roots: Vec<&'static str>,
    },
    /// There are more classes than a `ClassId` can number.
    #[error("there are {count} classes, more than the {} a ClassId can number", u16::MAX)]
    TooManyClasses {
        /// How many classes there are.
        count: usize,
    },
    /// A class ends up with more properties than a `PropId` can number.
    #[error(
        "class `{class}` has {count} properties (inherited included), more than the {} a PropId can number",
        u16::MAX
    )]
    TooManyProps {
        /// The class.
        class: &'static str,
        /// How many properties it has.
        count: usize,
    },
    /// The same class or component declares a property twice.
    #[error("{owner} declares property `{prop}` twice; remove or rename one")]
    DuplicateProp {
        /// Where it is declared.
        owner: Owner,
        /// The property.
        prop: &'static str,
    },
    /// A class or component declares a property its class already inherits, without saying it overrides it.
    #[error(
        "{owner} declares property `{prop}`, which class `{class}` already gets from `{inherited_from}`; rename it, \
         or mark it with PropFlags::OVERRIDE to replace the inherited one"
    )]
    RedeclaredProp {
        /// Where the new declaration is.
        owner: Owner,
        /// The class being resolved.
        class: &'static str,
        /// The property.
        prop: &'static str,
        /// The class that first declared it.
        inherited_from: &'static str,
    },
    /// A property is marked `OVERRIDE` but there is nothing to override.
    #[error(
        "{owner} marks property `{prop}` as an override, but class `{class}` inherits no `{prop}`; remove PropFlags::OVERRIDE"
    )]
    OverridesNothing {
        /// Where the property is declared.
        owner: Owner,
        /// The class being resolved.
        class: &'static str,
        /// The property.
        prop: &'static str,
    },
    /// An override changes the type of the property it replaces.
    #[error("{owner} overrides property `{prop}` as {found}, but it is inherited as {expected}; keep the type")]
    OverrideChangesType {
        /// Where the override is declared.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The inherited type.
        expected: PropType,
        /// The override's type.
        found: PropType,
    },
    /// A class declares the same method or signal twice.
    #[error("class `{class}` declares {kind} `{name}` twice; remove or rename one")]
    DuplicateMember {
        /// The class.
        class: &'static str,
        /// Method or signal.
        kind: MemberKind,
        /// The name.
        name: &'static str,
    },
    /// Two visible members of different kinds share a name, so `instance.Name` would be ambiguous in scripts.
    #[error("class `{class}` has both a {first} and a {second} called `{name}`; rename one, or hide the inherited one")]
    MemberNameClash {
        /// The class.
        class: &'static str,
        /// The shared name.
        name: &'static str,
        /// The kind found first.
        first: MemberKind,
        /// The kind that clashes with it.
        second: MemberKind,
    },
    /// A property alias is also the name or alias of another property.
    #[error(
        "in class `{class}`, alias `{alias}` of property `{prop}` is already the name or an alias of `{other}`; remove the alias"
    )]
    AliasCollision {
        /// The class.
        class: &'static str,
        /// The colliding alias.
        alias: &'static str,
        /// The property the alias belongs to.
        prop: &'static str,
        /// The property that already uses the name.
        other: &'static str,
    },
    /// A stored property declares sources with `derived_from(..)`; only a computed property is derived from others.
    #[error(
        "class `{class}` has stored property `{prop}` marked as derived from other properties; only a computed \
         property can be, so remove derived_from or make it computed"
    )]
    DerivedStoredProp {
        /// The class.
        class: &'static str,
        /// The stored property.
        prop: &'static str,
    },
    /// `derived_from(..)` names a property the class does not have.
    #[error(
        "in class `{class}`, property `{prop}` is derived from `{from}`, but the class has no property by that \
         name (its own or inherited); fix the name"
    )]
    UnknownDerivedSource {
        /// The class.
        class: &'static str,
        /// The computed property.
        prop: &'static str,
        /// The name in `derived_from`.
        from: &'static str,
    },
    /// `derived_from(..)` names a computed property, which never changes by itself.
    #[error(
        "in class `{class}`, property `{prop}` is derived from `{from}`, which is computed; name the stored \
         properties `{from}` is derived from instead"
    )]
    DerivedFromComputed {
        /// The class.
        class: &'static str,
        /// The computed property.
        prop: &'static str,
        /// The computed property named in `derived_from`.
        from: &'static str,
    },
    /// `changed_passes_value(..)` applies to a class without a visible `Changed` signal.
    #[error(
        "class `{class}` makes Changed pass the value of `{prop}`, but it has no Changed signal; declare one (or \
         inherit it) or remove changed_passes_value"
    )]
    ChangedValueWithoutSignal {
        /// The class.
        class: &'static str,
        /// The property named in `changed_passes_value`.
        prop: &'static str,
    },
    /// `changed_passes_value(..)` names a property a class that can have instances does not have.
    #[error(
        "class `{class}` makes Changed pass the value of `{prop}`, but it has no property by that name; declare \
         `{prop}` or fix the name"
    )]
    UnknownChangedValue {
        /// The class (the one declaring the option, or a subclass that inherits it).
        class: &'static str,
        /// The property named in `changed_passes_value`.
        prop: &'static str,
    },
    /// `changed_passes_value(..)` names a computed property, which never reports a change of its own.
    #[error("class `{class}` makes Changed pass the value of `{prop}`, which is computed; name a stored property")]
    ChangedValueComputed {
        /// The class.
        class: &'static str,
        /// The computed property.
        prop: &'static str,
    },
    /// `hide(..)` names a member the class does not have.
    #[error("class `{class}` hides `{member}`, but it has no property, method or signal by that name; fix the name")]
    UnknownHidden {
        /// The class.
        class: &'static str,
        /// The name in `hide`.
        member: &'static str,
    },
    /// `defaults(..)` names a property the class does not have.
    #[error("class `{class}` sets the default of `{prop}`, but it has no property by that name; fix the name")]
    UnknownDefault {
        /// The class.
        class: &'static str,
        /// The name in `defaults`.
        prop: &'static str,
    },
    /// A default value does not fit the property's type.
    #[error(
        "{owner} gives property `{prop}` a {found} default, but the property is {expected}; use a matching DefaultValue"
    )]
    DefaultTypeMismatch {
        /// Where the default is set.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The property's type.
        expected: PropType,
        /// What kind of default was given.
        found: &'static str,
    },
    /// An enum property names an enum that does not exist.
    #[error(
        "{owner} declares property `{prop}` of type Enum.{enum_type}, which is not an enum; fix the name or add the enum"
    )]
    UnknownEnum {
        /// Where the property is declared.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The missing enum.
        enum_type: &'static str,
    },
    /// An enum default names an item its enum does not have.
    #[error(
        "{owner} gives property `{prop}` the default `{item}`, but Enum.{enum_type} has no such item; fix the name"
    )]
    UnknownEnumItem {
        /// Where the default is set.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The enum.
        enum_type: &'static str,
        /// The missing item.
        item: &'static str,
    },
    /// A reference property names a class that does not exist.
    #[error("{owner} declares property `{prop}` as a reference to `{class}`, which is not a class; fix the name")]
    UnknownRefClass {
        /// Where the property is declared.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The missing class.
        class: &'static str,
    },
    /// A range has `min > max` (or a NaN bound).
    #[error("{owner} gives property `{prop}` the range ({min}, {max}); min must not be greater than max")]
    InvalidRange {
        /// Where the property is declared.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The lower bound.
        min: f64,
        /// The upper bound.
        max: f64,
    },
    /// A number default lies outside the property's range.
    #[error(
        "{owner} gives property `{prop}` the default {default}, outside its range ({min}, {max}); move one of them"
    )]
    DefaultOutOfRange {
        /// Where the default is set.
        owner: Owner,
        /// The property.
        prop: &'static str,
        /// The default.
        default: f64,
        /// The lower bound.
        min: f64,
        /// The upper bound.
        max: f64,
    },
    /// Two components have the same name.
    #[error("component `{component}` is declared twice; rename one of them")]
    DuplicateComponent {
        /// The repeated name.
        component: &'static str,
    },
    /// A class lists a component that does not exist.
    #[error("class `{class}` uses component `{component}`, which is not declared; fix the name or add the component")]
    UnknownComponent {
        /// The class.
        class: &'static str,
        /// The missing component.
        component: &'static str,
    },
    /// A component requires a component that does not exist.
    #[error("component `{component}` requires `{requires}`, which is not declared; fix the name or add the component")]
    UnknownRequiredComponent {
        /// The component.
        component: &'static str,
        /// The missing component.
        requires: &'static str,
    },
    /// A class has a component without one the component requires.
    #[error("class `{class}` uses component `{component}`, which requires `{requires}`; add `{requires}` to the class")]
    MissingRequiredComponent {
        /// The class.
        class: &'static str,
        /// The component with the requirement.
        component: &'static str,
        /// The component the class lacks.
        requires: &'static str,
    },
    /// Two enums have the same name.
    #[error("enum `{name}` is declared twice; rename one of them")]
    DuplicateEnum {
        /// The repeated name.
        name: &'static str,
    },
    /// An enum lists the same item twice.
    #[error("enum `{name}` lists item `{item}` twice; remove one")]
    DuplicateEnumItem {
        /// The enum.
        name: &'static str,
        /// The repeated item.
        item: &'static str,
    },
    /// An enum's explicit numbers do not match its items one to one.
    #[error("enum `{name}` has {items} items but {numbers} numbers; give one number per item, or none")]
    EnumNumbersMismatch {
        /// The enum.
        name: &'static str,
        /// How many items it has.
        items: usize,
        /// How many numbers it has.
        numbers: usize,
    },
}

/// Joins names as `` `A`, `B` ``.
fn list(names: &[&str]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

/// Every problem `Registry::build` found, in the order it found them.
#[derive(Clone, Debug, PartialEq, Error)]
#[error("{} problem(s) in the class descriptors:\n{}", .0.len(), lines(.0))]
pub struct RegistryErrors(pub Vec<RegistryError>);

/// One error per line, each starting with `- `.
fn lines(errors: &[RegistryError]) -> String {
    errors.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_place_and_the_fix() {
        let err = RegistryError::RedeclaredProp {
            owner: Owner::Component("Transform"),
            class: "Part",
            prop: "CFrame",
            inherited_from: "BasePart",
        };
        assert_eq!(
            err.to_string(),
            "component `Transform` declares property `CFrame`, which class `Part` already gets from `BasePart`; rename \
             it, or mark it with PropFlags::OVERRIDE to replace the inherited one"
        );
        let cycle = RegistryError::InheritanceCycle {
            classes: vec!["A", "B", "A"],
        };
        assert!(cycle.to_string().contains("(A -> B -> A)"));
        let roots = RegistryError::MultipleRoots { roots: vec!["A", "B"] };
        assert!(roots.to_string().starts_with("classes `A`, `B` have no parent"));
    }

    #[test]
    fn errors_list_one_per_line() {
        let errors = RegistryErrors(vec![
            RegistryError::DuplicateClass { class: "Part" },
            RegistryError::DuplicateEnum { name: "Material" },
        ]);
        assert_eq!(
            errors.to_string(),
            "2 problem(s) in the class descriptors:\n- class `Part` is declared twice; rename one of them\n- enum \
             `Material` is declared twice; rename one of them"
        );
    }
}
