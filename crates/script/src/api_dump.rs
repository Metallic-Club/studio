//! The API dump: the whole class registry as plain, serializable data (ADR 0003, "API dump").
//!
//! [`ApiDump::from_registry`] reads every class, enum and component once and copies what scripts and tools need to
//! know about them: names, inheritance, flags, member signatures, defaults and descriptions. It is the single
//! source for the committed `api/api-dump.json` (written by `cargo xtask api-dump`), the generated Luau type
//! definitions, the online API docs and the bindings of future languages. Nothing here is Luau-specific: types are
//! [`ApiType`], a language-neutral copy of [`Ty`].
//!
//! # Shape
//!
//! - **Order is deterministic**, so diffs of the JSON are readable: classes in registry order (parents before
//!   children, then by name), members in declaration order, components by name, enums in registry order.
//! - **Each class lists only its own members**: the properties, methods, signals and callbacks its descriptor
//!   declares (re-declared ones included, marked by the `OVERRIDE` flag for a property or simply by the name for a
//!   method or signal). Inherited members are found by walking `parent`. Properties a class gets from the
//!   components it adds are listed on the component, not on the class.
//! - **Hidden members are marked on the class that hides them**: [`ApiClass::hides`] names the inherited members
//!   scripts and Studio no longer see on that class and its descendants. They stay listed on the class that declares
//!   them, so a consumer resolving inheritance drops them when it reaches the hiding class.
//! - [`ApiClass::defaults`] lists the inherited properties whose default the class changes.
//!
//! The data only grows with new fields; [`FORMAT`] is bumped when a field is removed or changes meaning.

use metallic_core::{
    CallbackDesc, ComponentDesc, DefaultValue, EnumDesc, MethodDesc, Param, PropFlags, PropMeta, PropType, Registry,
    SignalDesc, Ty,
};
use serde::{Deserialize, Serialize};

/// The version of the dump's layout, written as its `format` field.
pub const FORMAT: u32 = 1;

/// Every property flag with the name the dump writes for it, in bit order.
const FLAG_NAMES: [(PropFlags, &str); 8] = [
    (PropFlags::TRANSFORM, "TRANSFORM"),
    (PropFlags::VISUAL, "VISUAL"),
    (PropFlags::PHYSICS, "PHYSICS"),
    (PropFlags::RUNTIME, "RUNTIME"),
    (PropFlags::NO_SAVE, "NO_SAVE"),
    (PropFlags::READ_ONLY, "READ_ONLY"),
    (PropFlags::HIDDEN, "HIDDEN"),
    (PropFlags::OVERRIDE, "OVERRIDE"),
];

/// The whole scripting API: every class, enum and component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiDump {
    /// The layout version ([`FORMAT`]).
    pub format: u32,
    /// Every class, parents before children.
    pub classes: Vec<ApiClass>,
    /// Every enum.
    pub enums: Vec<ApiEnum>,
    /// Every component, by name.
    pub components: Vec<ApiComponent>,
}

/// One class and the members it declares itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiClass {
    /// The class name scripts and scene files use.
    pub name: String,
    /// The parent class, or `None` for the root of the hierarchy.
    pub parent: Option<String>,
    /// What the class is for.
    pub description: String,
    /// The Insert menu group, or empty for none.
    pub category: String,
    /// Scripts (`Instance.new`) and Studio may create it.
    pub creatable: bool,
    /// Set for a service: its creation order at start-up (lower first).
    pub service_order: Option<i16>,
    /// A service whose contents are saved in scene files.
    pub persisted: bool,
    /// Scripts under the service run.
    pub runs_scripts: bool,
    /// Studio's Explorer does not list it.
    pub hidden_in_explorer: bool,
    /// The components it adds, on top of the inherited ones.
    pub components: Vec<String>,
    /// Set when the class makes `Changed` pass the new value of this property instead of a property name; its
    /// descendants inherit it.
    pub changed_passes_value: Option<String>,
    /// Inherited members (properties, methods, signals, callbacks) that scripts and Studio no longer see on this class
    /// and its descendants.
    pub hides: Vec<String>,
    /// New defaults for inherited properties.
    pub defaults: Vec<ApiDefault>,
    /// The properties it declares, in declaration order.
    pub properties: Vec<ApiProperty>,
    /// The methods it declares; one named like an inherited method replaces it.
    pub methods: Vec<ApiMethod>,
    /// The signals it declares; one named like an inherited signal replaces it.
    pub signals: Vec<ApiSignal>,
    /// The callback members it declares.
    pub callbacks: Vec<ApiCallback>,
}

/// A new default for an inherited property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiDefault {
    /// The property's name.
    pub name: String,
    /// Its new default.
    pub value: ApiValue,
}

/// A property: its type, default, flags and docs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiProperty {
    /// The name scripts and scene files use.
    pub name: String,
    /// The type of its value. A reference to an instance is `Optional(Class(..))`, since it may be nil.
    #[serde(rename = "type")]
    pub ty: ApiType,
    /// The value a new instance starts with.
    pub default: ApiValue,
    /// The names of its flags (`TRANSFORM`, `VISUAL`, `PHYSICS`, `RUNTIME`, `NO_SAVE`, `READ_ONLY`, `HIDDEN`,
    /// `OVERRIDE`), in that order.
    pub flags: Vec<String>,
    /// Scripts cannot write it: it is `READ_ONLY`, or computed without a setter.
    pub read_only: bool,
    /// Its value is computed from other state instead of being stored.
    pub computed: bool,
    /// `[min, max]`: writes are clamped into this range.
    pub range: Option<[f64; 2]>,
    /// Studio's Properties panel group, or empty for the default group.
    pub group: String,
    /// What the property does.
    pub description: String,
    /// Older names that still work.
    pub aliases: Vec<String>,
    /// For a computed property: the stored properties its value is derived from.
    pub derived_from: Vec<String>,
}

/// A method scripts call on instances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiMethod {
    /// The name scripts call.
    pub name: String,
    /// The parameters after the receiver, or `None` when they are not declared.
    pub params: Option<Vec<ApiParam>>,
    /// What it returns ([`ApiType::Tuple`] for several values or none), or `None` when it is not declared.
    pub returns: Option<ApiType>,
    /// What the method does.
    pub description: String,
    /// The callback member of the same instance this method calls, if any (`BindableFunction:Invoke` calls
    /// `OnInvoke`).
    pub invokes_callback: Option<String>,
}

/// A signal scripts connect to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiSignal {
    /// The name scripts connect to.
    pub name: String,
    /// The values listeners receive, or `None` when they are not declared.
    pub params: Option<Vec<ApiParam>>,
    /// When it fires.
    pub description: String,
}

/// A callback member: a slot a script assigns one of its functions to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiCallback {
    /// The name scripts assign to.
    pub name: String,
    /// The values the function is called with, or `None` when they are not declared.
    pub params: Option<Vec<ApiParam>>,
    /// What the function should return, or `None` when it is not declared.
    pub returns: Option<ApiType>,
    /// What the callback is for and when it is called.
    pub description: String,
}

/// One parameter of a method, signal or callback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiParam {
    /// Its name, for docs and autocomplete.
    pub name: String,
    /// The type of the value ([`ApiType::Variadic`] for any number of values).
    #[serde(rename = "type")]
    pub ty: ApiType,
    /// The caller may leave it out.
    pub optional: bool,
}

/// An engine enum, `Enum.<name>` in scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiEnum {
    /// The enum's name, without `Enum.`.
    pub name: String,
    /// What the enum is for.
    pub description: String,
    /// Its items, in order.
    pub items: Vec<ApiEnumItem>,
}

/// One item of an enum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiEnumItem {
    /// The item's name.
    pub name: String,
    /// Its number (`Value` in scripts).
    pub value: i32,
}

/// A component: a named bundle of behaviour and properties that classes list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiComponent {
    /// The name classes list it by.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Other components a class must also have to use it.
    pub requires: Vec<String>,
    /// The properties every class with it gets.
    pub properties: Vec<ApiProperty>,
}

/// The type of a value, mirroring [`Ty`] without references so it can be stored and read back.
///
/// Serialized like the Rust variant: `"Bool"`, `{"Class": "BasePart"}`, `{"Optional": {"Class": "Instance"}}`,
/// `{"Map": ["String", "Any"]}`, `{"Tuple": []}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiType {
    /// A boolean.
    Bool,
    /// A whole number.
    Int,
    /// A number.
    Number,
    /// A string.
    String,
    /// A `Vector2`.
    Vector2,
    /// A `Vector3`.
    Vector3,
    /// A `CFrame`.
    CFrame,
    /// A `Color3`.
    Color3,
    /// An item of the named enum.
    Enum(String),
    /// An instance of the named class or a subclass, never nil.
    Class(String),
    /// Any value, nil included.
    Any,
    /// Only nil.
    Nil,
    /// The type or nil.
    Optional(Box<ApiType>),
    /// A list of values of one type.
    Array(Box<ApiType>),
    /// A table from keys of the first type to values of the second.
    Map(Box<ApiType>, Box<ApiType>),
    /// Any number of values of the type; only as the last parameter.
    Variadic(Box<ApiType>),
    /// Several values at once (none for `[]`); only as a return type.
    Tuple(Vec<ApiType>),
    /// A function.
    Function,
}

impl From<&Ty> for ApiType {
    fn from(ty: &Ty) -> Self {
        match *ty {
            Ty::Bool => Self::Bool,
            Ty::Int => Self::Int,
            Ty::Number => Self::Number,
            Ty::String => Self::String,
            Ty::Vector2 => Self::Vector2,
            Ty::Vector3 => Self::Vector3,
            Ty::CFrame => Self::CFrame,
            Ty::Color3 => Self::Color3,
            Ty::Enum(name) => Self::Enum(name.to_owned()),
            Ty::Class(name) => Self::Class(name.to_owned()),
            Ty::Any => Self::Any,
            Ty::Nil => Self::Nil,
            Ty::Optional(inner) => Self::Optional(Box::new(inner.into())),
            Ty::Array(item) => Self::Array(Box::new(item.into())),
            Ty::Map(key, value) => Self::Map(Box::new(key.into()), Box::new(value.into())),
            Ty::Variadic(item) => Self::Variadic(Box::new(item.into())),
            Ty::Tuple(items) => Self::Tuple(items.iter().map(Self::from).collect()),
            Ty::Function => Self::Function,
        }
    }
}

impl From<PropType> for ApiType {
    /// A property's type as a value type; a reference may be nil, so it is optional.
    fn from(ty: PropType) -> Self {
        match ty {
            PropType::Bool => Self::Bool,
            PropType::Int => Self::Int,
            PropType::Number => Self::Number,
            PropType::String => Self::String,
            PropType::Vector2 => Self::Vector2,
            PropType::Vector3 => Self::Vector3,
            PropType::CFrame => Self::CFrame,
            PropType::Color3 => Self::Color3,
            PropType::Enum(name) => Self::Enum(name.to_owned()),
            PropType::Ref(class) => Self::Optional(Box::new(Self::Class(class.to_owned()))),
        }
    }
}

/// A property's default value, as plain data. The property's type says how to read it.
///
/// Serialized as the bare JSON value: `true`, `3`, `2.5`, `"text"` (a string, or an enum item's name), `[x, y, z]`
/// (a vector, a colour's `r, g, b`, or a `CFrame`'s position: defaults have no rotation), `null` (a nil reference).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiValue {
    /// No value: a nil reference.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A number.
    Number(f64),
    /// A string, or the name of an enum item.
    Text(String),
    /// The components of a `Vector2`, `Vector3`, `Color3` or a `CFrame`'s position.
    Components(Vec<f32>),
}

impl From<DefaultValue> for ApiValue {
    fn from(value: DefaultValue) -> Self {
        match value {
            DefaultValue::Bool(b) => Self::Bool(b),
            DefaultValue::Int(n) => Self::Int(n),
            DefaultValue::Number(n) => Self::Number(n),
            DefaultValue::Str(s) | DefaultValue::Enum(s) => Self::Text(s.to_owned()),
            DefaultValue::Vector2(x, y) => Self::Components(vec![x, y]),
            DefaultValue::Vector3(x, y, z) | DefaultValue::CFrame(x, y, z) | DefaultValue::Color3(x, y, z) => {
                Self::Components(vec![x, y, z])
            }
            DefaultValue::NilRef => Self::Nil,
        }
    }
}

impl ApiDump {
    /// Every class, enum and component of `registry`, in the dump's deterministic order.
    pub fn from_registry(registry: &Registry) -> Self {
        Self {
            format: FORMAT,
            classes: registry.classes().iter().map(|info| class(info.desc())).collect(),
            enums: registry.enums().map(enumeration).collect(),
            components: registry.components().into_iter().map(component).collect(),
        }
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn class(desc: &metallic_core::ClassDesc) -> ApiClass {
    ApiClass {
        name: desc.name.to_owned(),
        parent: desc.parent.map(str::to_owned),
        description: desc.description.to_owned(),
        category: desc.category.to_owned(),
        creatable: desc.creatable,
        service_order: desc.service_order,
        persisted: desc.persisted,
        runs_scripts: desc.runs_scripts,
        hidden_in_explorer: desc.hidden_in_explorer,
        components: strings(desc.components),
        changed_passes_value: desc.changed_passes_value.map(str::to_owned),
        hides: strings(desc.hide),
        defaults: desc
            .defaults
            .iter()
            .map(|&(name, value)| ApiDefault {
                name: name.to_owned(),
                value: value.into(),
            })
            .collect(),
        properties: desc.props.iter().map(property).collect(),
        methods: desc.methods.iter().map(method).collect(),
        signals: desc.signals.iter().map(signal).collect(),
        callbacks: desc.callbacks.iter().map(callback).collect(),
    }
}

fn property(meta: &PropMeta) -> ApiProperty {
    let computed = meta.computed.is_some();
    ApiProperty {
        name: meta.name.to_owned(),
        ty: meta.ty.into(),
        default: meta.default.into(),
        flags: FLAG_NAMES
            .iter()
            .filter(|(flag, _)| meta.flags.contains(*flag))
            .map(|(_, name)| (*name).to_owned())
            .collect(),
        read_only: meta.flags.contains(PropFlags::READ_ONLY) || meta.computed.is_some_and(|c| c.set.is_none()),
        computed,
        range: meta.range.map(|(min, max)| [min, max]),
        group: meta.group.to_owned(),
        description: meta.description.to_owned(),
        aliases: strings(meta.aliases),
        derived_from: strings(meta.derived_from),
    }
}

fn params(params: Option<&[Param]>) -> Option<Vec<ApiParam>> {
    params.map(|params| {
        params
            .iter()
            .map(|p| ApiParam {
                name: p.name.to_owned(),
                ty: (&p.ty).into(),
                optional: p.optional,
            })
            .collect()
    })
}

fn method(desc: &MethodDesc) -> ApiMethod {
    ApiMethod {
        name: desc.name.to_owned(),
        params: params(desc.params),
        returns: desc.returns.as_ref().map(ApiType::from),
        description: desc.description.to_owned(),
        invokes_callback: desc.invokes.map(str::to_owned),
    }
}

fn signal(desc: &SignalDesc) -> ApiSignal {
    ApiSignal {
        name: desc.name.to_owned(),
        params: params(desc.params),
        description: desc.description.to_owned(),
    }
}

fn callback(desc: &CallbackDesc) -> ApiCallback {
    ApiCallback {
        name: desc.name.to_owned(),
        params: params(desc.params),
        returns: desc.returns.as_ref().map(ApiType::from),
        description: desc.description.to_owned(),
    }
}

fn enumeration(desc: &EnumDesc) -> ApiEnum {
    ApiEnum {
        name: desc.name.to_owned(),
        description: desc.description.to_owned(),
        items: desc
            .items
            .iter()
            .enumerate()
            .map(|(i, &name)| ApiEnumItem {
                name: name.to_owned(),
                value: desc.number(i).unwrap_or_default(),
            })
            .collect(),
    }
}

fn component(desc: &ComponentDesc) -> ApiComponent {
    ApiComponent {
        name: desc.name.to_owned(),
        description: desc.description.to_owned(),
        requires: strings(desc.requires),
        properties: desc.props.iter().map(property).collect(),
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::{ClassDesc, MethodCall, RegistryInput, ScriptError, ScriptValue};

    use super::*;

    fn nothing(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
        Ok(ScriptValue::Nil)
    }

    #[test]
    fn every_class_enum_and_component_is_dumped_in_registry_order() {
        let registry = metallic_classes::shared_registry().unwrap();
        let dump = ApiDump::from_registry(&registry);
        assert_eq!(dump.format, FORMAT);
        let names: Vec<&str> = dump.classes.iter().map(|c| c.name.as_str()).collect();
        let expected: Vec<&str> = registry.classes().iter().map(|c| c.name()).collect();
        assert_eq!(names, expected);
        assert_eq!(dump.enums.len(), registry.enums().count());
        assert_eq!(dump.components.len(), registry.components().len());
        assert!(dump.components.windows(2).all(|w| w[0].name < w[1].name));
        // Parents come before their children, so a consumer can resolve inheritance in one pass.
        for (i, class) in dump.classes.iter().enumerate() {
            if let Some(parent) = &class.parent {
                assert!(dump.classes[..i].iter().any(|c| &c.name == parent), "{}", class.name);
            }
        }
        assert_eq!(ApiDump::from_registry(&registry), dump);
    }

    #[test]
    fn members_signatures_and_flags_are_copied() {
        static PROPS: &[PropMeta] = &[
            PropMeta::number("Angle", 90.0)
                .range(1.0, 179.0)
                .visual()
                .desc("Cone angle."),
            PropMeta::reference("Target", "Base").read_only().aliases(&["target"]),
        ];
        static BASE: ClassDesc = ClassDesc::new("Base", None).signals(&[SignalDesc::new("Changed")
            .params(&[Param::new("property", Ty::String)])
            .describe("Changed.")]);
        static SPOT: ClassDesc = ClassDesc::new("Spot", Some("Base"))
            .creatable()
            .service(3)
            .props(PROPS)
            .methods(&[MethodDesc::new("Find", nothing)
                .params(&[
                    Param::new("name", Ty::String),
                    Param::new("recursive", Ty::Bool).optional(),
                ])
                .returns(Ty::Optional(&Ty::Class("Base")))
                .describe("Finds.")])
            .callbacks(&[CallbackDesc::new("OnCall")
                .params(&[Param::new("args", Ty::Variadic(&Ty::Any))])
                .returns(Ty::Tuple(&[Ty::Bool, Ty::Number]))])
            .hide(&["Changed"])
            .defaults(&[("Angle", DefaultValue::Number(45.0))]);
        static SHAPES: &[EnumDesc] = &[EnumDesc::new("Shape", &["Block", "Ball"]).numbers(&[1, 5])];
        static EMITTER: ComponentDesc = ComponentDesc::new("Emitter")
            .props(&[PropMeta::color3("Color", 1.0, 0.5, 0.0)])
            .requires(&[]);
        let registry = Registry::build(RegistryInput {
            classes: &[&SPOT, &BASE],
            components: &[&EMITTER],
            enums: &[SHAPES],
        })
        .unwrap();
        let dump = ApiDump::from_registry(&registry);

        let spot = &dump.classes[1];
        assert_eq!(spot.parent.as_deref(), Some("Base"));
        assert!(spot.creatable);
        assert_eq!(spot.service_order, Some(3));
        assert!(spot.signals.is_empty(), "only its own members");
        assert_eq!(spot.hides, ["Changed"]);
        assert_eq!(spot.defaults[0].value, ApiValue::Number(45.0));

        let angle = &spot.properties[0];
        assert_eq!(angle.ty, ApiType::Number);
        assert_eq!(angle.range, Some([1.0, 179.0]));
        assert_eq!(angle.flags, ["VISUAL"]);
        assert!(!angle.read_only && !angle.computed);
        let target = &spot.properties[1];
        assert_eq!(target.ty, ApiType::Optional(Box::new(ApiType::Class("Base".into()))));
        assert_eq!(target.default, ApiValue::Nil);
        assert!(target.read_only);
        assert_eq!(target.aliases, ["target"]);

        let find = &spot.methods[0];
        let params = find.params.as_ref().unwrap();
        assert_eq!(params[1].ty, ApiType::Bool);
        assert!(params[1].optional);
        assert_eq!(
            find.returns,
            Some(ApiType::Optional(Box::new(ApiType::Class("Base".into()))))
        );

        let on_call = &spot.callbacks[0];
        assert_eq!(
            on_call.params.as_ref().unwrap()[0].ty,
            ApiType::Variadic(Box::new(ApiType::Any))
        );
        assert_eq!(
            on_call.returns,
            Some(ApiType::Tuple(vec![ApiType::Bool, ApiType::Number]))
        );

        let items: Vec<(&str, i32)> = dump.enums[0].items.iter().map(|i| (i.name.as_str(), i.value)).collect();
        assert_eq!(items, [("Block", 1), ("Ball", 5)]);
        assert_eq!(
            dump.components[0].properties[0].default,
            ApiValue::Components(vec![1.0, 0.5, 0.0])
        );
    }
}
