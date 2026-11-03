//! The descriptors a class file declares: `ClassDesc`, `ComponentDesc`, `EnumDesc`, `MethodDesc`, `SignalDesc`, and
//! the optional `ClassBehavior` hooks.
//!
//! Everything is built with `const fn` so that a class file is a handful of `static`s. Names of parents,
//! components, enums and referenced classes are plain strings: class files never depend on each other in Rust, and
//! the registry checks every name when it is built.

use std::fmt;

use super::{PropId, PropMeta};
use crate::{Args, DataModel, Id, ScriptError, ScriptValue};

/// The arguments of one method call: the instance it was called on, the script's arguments and the data model.
#[derive(Debug)]
pub struct MethodCall<'a> {
    /// The instance the method was called on (`self` in Luau).
    pub this: Id,
    /// The arguments after the receiver.
    pub args: Args,
    /// The data model the instance lives in.
    pub dm: &'a mut DataModel,
}

/// The Rust function behind a method. It is language-neutral: every scripting backend calls it the same way.
pub type MethodFn = fn(&mut MethodCall<'_>) -> Result<ScriptValue, ScriptError>;

/// A method scripts can call on instances of a class, such as `Destroy` or `GetChildren`.
#[derive(Clone, Copy, Debug)]
pub struct MethodDesc {
    /// The name scripts call.
    pub name: &'static str,
    /// The function that runs.
    pub func: MethodFn,
    /// What the method does, for the API docs.
    pub description: &'static str,
}

impl MethodDesc {
    /// A method `name` that runs `func`.
    pub const fn new(name: &'static str, func: MethodFn) -> Self {
        Self {
            name,
            func,
            description: "",
        }
    }

    /// Sets the description.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }
}

/// A signal scripts can connect to, such as `ChildAdded`. It is fired from Rust; the parameters here are for docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalDesc {
    /// The name scripts connect to.
    pub name: &'static str,
    /// When the signal fires, for the API docs.
    pub description: &'static str,
    /// `(name, type)` of each value passed to listeners, such as `("child", "Instance")`.
    pub params: &'static [(&'static str, &'static str)],
}

impl SignalDesc {
    /// A signal `name` with no parameters.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            description: "",
            params: &[],
        }
    }

    /// Sets the description.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Sets the parameters listeners receive.
    pub const fn params(mut self, params: &'static [(&'static str, &'static str)]) -> Self {
        self.params = params;
        self
    }
}

/// An engine enum scripts see as `Enum.<name>.<item>`, such as `Enum.PartShape.Ball`.
///
/// A file declares its enums as `pub static ENUMS: &[EnumDesc]`, next to the class that owns them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumDesc {
    /// The enum's name, without `Enum.`.
    pub name: &'static str,
    /// The item names, in order.
    pub items: &'static [&'static str],
    /// The number of each item (`items[i]` has `numbers[i]`), or empty for 0, 1, 2, ... in order.
    pub numbers: &'static [i32],
    /// What the enum is for, for the API docs.
    pub description: &'static str,
}

impl EnumDesc {
    /// An enum whose items are numbered 0, 1, 2, ... in order.
    pub const fn new(name: &'static str, items: &'static [&'static str]) -> Self {
        Self {
            name,
            items,
            numbers: &[],
            description: "",
        }
    }

    /// Gives each item an explicit number. The registry checks there is one per item.
    pub const fn numbers(mut self, numbers: &'static [i32]) -> Self {
        self.numbers = numbers;
        self
    }

    /// Sets the description.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// The number of item `index`, or `None` when there is no such item.
    pub fn number(&self, index: usize) -> Option<i32> {
        if index >= self.items.len() {
            return None;
        }
        match self.numbers.get(index) {
            Some(n) => Some(*n),
            None => i32::try_from(index).ok(),
        }
    }
}

/// A named bundle of behaviour a class can list, such as `Transform` or `SpotLightEmitter`.
///
/// The descriptor only names the component and the properties it brings. The subsystem that implements it (the
/// renderer for `SpotLightEmitter`, physics for `Rigidbody`) registers itself under the same name, so the object
/// model compiles and runs without any of them.
#[derive(Clone, Copy, Debug)]
pub struct ComponentDesc {
    /// The name classes list it by.
    pub name: &'static str,
    /// What the component does, for the API docs.
    pub description: &'static str,
    /// Properties every class with this component gets.
    pub props: &'static [PropMeta],
    /// Other components a class must also have to use this one.
    pub requires: &'static [&'static str],
}

impl ComponentDesc {
    /// A component `name` with no properties.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            description: "",
            props: &[],
            requires: &[],
        }
    }

    /// Sets the description.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Sets the properties it brings.
    pub const fn props(mut self, props: &'static [PropMeta]) -> Self {
        self.props = props;
        self
    }

    /// Sets the components a class must also have.
    pub const fn requires(mut self, requires: &'static [&'static str]) -> Self {
        self.requires = requires;
        self
    }
}

/// Optional hooks a class runs at points in an instance's life. Every hook does nothing unless the class overrides
/// it; the data model (#14) calls them.
///
/// Hooks only touch the data model. Rendering and physics react to dirty bits and components instead.
pub trait ClassBehavior: Send + Sync {
    /// After a new instance is created and its defaults are set, before it has a parent.
    fn on_created(&self, dm: &mut DataModel, id: Id) {
        let _ = (dm, id);
    }

    /// After an instance is loaded from a scene file, with its saved properties set.
    fn on_loaded(&self, dm: &mut DataModel, id: Id) {
        let _ = (dm, id);
    }

    /// After property `prop` of the instance changed value.
    fn on_prop_changed(&self, dm: &mut DataModel, id: Id, prop: PropId) {
        let _ = (dm, id, prop);
    }

    /// After the instance moved from parent `old` to parent `new` (`None` is no parent).
    fn on_parent_changed(&self, dm: &mut DataModel, id: Id, old: Option<Id>, new: Option<Id>) {
        let _ = (dm, id, old, new);
    }

    /// Before the instance is destroyed, while it is still intact.
    fn on_destroying(&self, dm: &mut DataModel, id: Id) {
        let _ = (dm, id);
    }

    /// When Play starts, for every instance that exists then.
    fn on_started(&self, dm: &mut DataModel, id: Id) {
        let _ = (dm, id);
    }
}

/// Everything a class declares about itself. A class file exports one as `pub static CLASS: ClassDesc`.
///
/// ```
/// use metallic_core::{ClassDesc, PropMeta};
///
/// static PROPS: &[PropMeta] = &[PropMeta::number("Angle", 90.0).range(1.0, 179.0).visual()];
///
/// pub static CLASS: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
///     .describe("A cone of light.")
///     .category("Lights")
///     .creatable()
///     .components(&["SpotLightEmitter"])
///     .props(PROPS);
/// ```
#[derive(Clone, Copy)]
pub struct ClassDesc {
    /// The class name scripts and scene files use.
    pub name: &'static str,
    /// The parent class's name, or `None` for the root of the hierarchy (`Object`).
    pub parent: Option<&'static str>,
    /// What the class is for, for the API docs and Studio.
    pub description: &'static str,
    /// The group it is listed under in Studio's Insert menu, or empty for none.
    pub category: &'static str,
    /// Scripts (`Instance.new`) and Studio may create it.
    pub creatable: bool,
    /// Set for a service: there is exactly one, created at start-up in this order (lower first).
    pub service_order: Option<i16>,
    /// The service's contents are saved in scene files.
    pub persisted: bool,
    /// Scripts under the service run.
    pub runs_scripts: bool,
    /// Studio's Explorer does not list it.
    pub hidden_in_explorer: bool,
    /// Components it has, by name, on top of the inherited ones.
    pub components: &'static [&'static str],
    /// Properties it declares.
    pub props: &'static [PropMeta],
    /// Methods it declares. One with the name of an inherited method replaces it.
    pub methods: &'static [MethodDesc],
    /// Signals it declares.
    pub signals: &'static [SignalDesc],
    /// Inherited members (properties, methods, signals) that scripts and Studio no longer see. Properties keep their
    /// [`PropId`] slot, so code written for the parent still works.
    pub hide: &'static [&'static str],
    /// New defaults for inherited properties.
    pub defaults: &'static [(&'static str, super::DefaultValue)],
    /// Per-class hooks, or `None` when the class has none.
    pub behavior: Option<&'static dyn ClassBehavior>,
    /// Set when the class's `Changed` signal passes the new value of this property instead of a property name (see
    /// [`ClassDesc::changed_passes_value`]). Subclasses inherit it.
    pub changed_passes_value: Option<&'static str>,
}

impl ClassDesc {
    /// A class `name` under `parent`, with no members or flags.
    pub const fn new(name: &'static str, parent: Option<&'static str>) -> Self {
        Self {
            name,
            parent,
            description: "",
            category: "",
            creatable: false,
            service_order: None,
            persisted: false,
            runs_scripts: false,
            hidden_in_explorer: false,
            components: &[],
            props: &[],
            methods: &[],
            signals: &[],
            hide: &[],
            defaults: &[],
            behavior: None,
            changed_passes_value: None,
        }
    }

    /// Sets the description.
    pub const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Sets the Insert menu group.
    pub const fn category(mut self, category: &'static str) -> Self {
        self.category = category;
        self
    }

    /// Lets scripts and Studio create it.
    pub const fn creatable(mut self) -> Self {
        self.creatable = true;
        self
    }

    /// Makes it a service, created at start-up in `order` (lower first).
    pub const fn service(mut self, order: i16) -> Self {
        self.service_order = Some(order);
        self
    }

    /// Saves the service's contents in scene files.
    pub const fn persisted(mut self) -> Self {
        self.persisted = true;
        self
    }

    /// Runs scripts placed under the service.
    pub const fn runs_scripts(mut self) -> Self {
        self.runs_scripts = true;
        self
    }

    /// Leaves it out of Studio's Explorer.
    pub const fn hidden_in_explorer(mut self) -> Self {
        self.hidden_in_explorer = true;
        self
    }

    /// Sets the components it adds.
    pub const fn components(mut self, components: &'static [&'static str]) -> Self {
        self.components = components;
        self
    }

    /// Sets the properties it declares.
    pub const fn props(mut self, props: &'static [PropMeta]) -> Self {
        self.props = props;
        self
    }

    /// Sets the methods it declares.
    pub const fn methods(mut self, methods: &'static [MethodDesc]) -> Self {
        self.methods = methods;
        self
    }

    /// Sets the signals it declares.
    pub const fn signals(mut self, signals: &'static [SignalDesc]) -> Self {
        self.signals = signals;
        self
    }

    /// Hides inherited members from scripts and Studio (properties keep their slot).
    pub const fn hide(mut self, names: &'static [&'static str]) -> Self {
        self.hide = names;
        self
    }

    /// Changes the defaults of inherited properties.
    pub const fn defaults(mut self, defaults: &'static [(&'static str, super::DefaultValue)]) -> Self {
        self.defaults = defaults;
        self
    }

    /// Makes the class's `Changed` signal pass the new value of the property `prop` instead of a property name,
    /// as value objects do in Roblox (`IntValue.Changed(newValue)`). `Changed` then fires only when `prop` changes;
    /// every other property, `Name` and `Parent` included, fires only its own property-changed signal.
    ///
    /// Subclasses inherit it, so a base class can declare it for a property each subclass declares with its own
    /// type. The registry checks that the class has a `Changed` signal and that `prop` is a stored (not computed)
    /// property of every class that can have instances (creatable classes and services); an abstract class may
    /// lack it.
    pub const fn changed_passes_value(mut self, prop: &'static str) -> Self {
        self.changed_passes_value = Some(prop);
        self
    }

    /// Sets the class's hooks.
    pub const fn behavior(mut self, behavior: &'static dyn ClassBehavior) -> Self {
        self.behavior = Some(behavior);
        self
    }
}

/// Lists the declared data; the behaviour hooks are shown only as present or not.
impl fmt::Debug for ClassDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClassDesc")
            .field("name", &self.name)
            .field("parent", &self.parent)
            .field("category", &self.category)
            .field("creatable", &self.creatable)
            .field("service_order", &self.service_order)
            .field("components", &self.components)
            .field("props", &self.props.len())
            .field("methods", &self.methods.len())
            .field("signals", &self.signals.len())
            .field("hide", &self.hide)
            .field("behavior", &self.behavior.is_some())
            .field("changed_passes_value", &self.changed_passes_value)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultValue;

    struct Counts;
    impl ClassBehavior for Counts {}
    static COUNTS: Counts = Counts;

    fn nothing(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
        Ok(ScriptValue::Nil)
    }

    static CLASS: ClassDesc = ClassDesc::new("Workspace", Some("Instance"))
        .describe("The 3D world.")
        .category("Services")
        .creatable()
        .service(1)
        .persisted()
        .runs_scripts()
        .hidden_in_explorer()
        .components(&["Transform"])
        .props(&[PropMeta::number("Gravity", 9.81)])
        .methods(&[MethodDesc::new("Raycast", nothing).describe("Casts a ray.")])
        .signals(&[SignalDesc::new("Stepped")
            .params(&[("dt", "number")])
            .describe("Each step.")])
        .hide(&["Archivable"])
        .defaults(&[("Name", DefaultValue::Str("Workspace"))])
        .behavior(&COUNTS)
        .changed_passes_value("Gravity");

    #[test]
    fn class_builders_set_every_field() {
        assert_eq!(CLASS.name, "Workspace");
        assert_eq!(CLASS.parent, Some("Instance"));
        assert_eq!(CLASS.description, "The 3D world.");
        assert_eq!(CLASS.category, "Services");
        assert!(CLASS.creatable && CLASS.persisted && CLASS.runs_scripts && CLASS.hidden_in_explorer);
        assert_eq!(CLASS.service_order, Some(1));
        assert_eq!(CLASS.components, &["Transform"]);
        assert_eq!(CLASS.props[0].name, "Gravity");
        assert_eq!(CLASS.methods[0].description, "Casts a ray.");
        assert_eq!(CLASS.signals[0].params, &[("dt", "number")]);
        assert_eq!(CLASS.hide, &["Archivable"]);
        assert_eq!(CLASS.defaults[0].0, "Name");
        assert!(CLASS.behavior.is_some());
        assert_eq!(CLASS.changed_passes_value, Some("Gravity"));
        assert_eq!(ClassDesc::new("Part", None).changed_passes_value, None);
        assert!(format!("{CLASS:?}").contains("Workspace"));
    }

    #[test]
    fn enum_numbers_default_to_positions() {
        let shape = EnumDesc::new("PartShape", &["Block", "Ball"]).describe("Part shapes.");
        assert_eq!(shape.number(1), Some(1));
        assert_eq!(shape.number(2), None);
        let numbered = EnumDesc::new("KeyCode", &["A", "B"]).numbers(&[97, 98]);
        assert_eq!(numbered.number(0), Some(97));
    }

    #[test]
    fn component_builders_set_every_field() {
        const C: ComponentDesc = ComponentDesc::new("SpotLightEmitter")
            .describe("Draws a cone of light.")
            .props(&[PropMeta::boolean("Shadows", false)])
            .requires(&["Transform"]);
        assert_eq!(C.requires, &["Transform"]);
        assert_eq!(C.props.len(), 1);
        assert_eq!(C.description, "Draws a cone of light.");
    }
}
