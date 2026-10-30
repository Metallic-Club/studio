//! The class registry: every class, its superclass and its properties with defaults.
//!
//! To add a class, add one [`ClassDef`] entry to `CLASSES` below.
use crate::value::{Color3, UDim2, Value, ValueType, Vec3};

/// Materials a part may use.
pub const MATERIALS: &[&str] = &[
    "Plastic",
    "SmoothPlastic",
    "Metal",
    "Wood",
    "Concrete",
    "Glass",
    "Grass",
    "Brick",
    "Ice",
    "Sand",
];

/// Shapes a part may have.
pub const SHAPES: &[&str] = &["Block", "Ball", "Cylinder", "Wedge"];

/// Definition of one property of a class.
pub struct PropDef {
    /// Property name.
    pub name: &'static str,
    /// Type that values must have.
    pub ty: ValueType,
    /// Builds the default value (a function because strings allocate).
    pub default: fn() -> Value,
    /// When set, a String value must be one of these.
    pub allowed: Option<&'static [&'static str]>,
}

/// Definition of one class.
pub struct ClassDef {
    /// Class name.
    pub name: &'static str,
    /// Name of the superclass, `None` only for `Instance`.
    pub superclass: Option<&'static str>,
    /// Properties declared by this class (not inherited ones).
    pub props: &'static [PropDef],
}

const fn p(name: &'static str, ty: ValueType, default: fn() -> Value) -> PropDef {
    PropDef {
        name,
        ty,
        default,
        allowed: None,
    }
}

const fn class(
    name: &'static str,
    superclass: &'static str,
    props: &'static [PropDef],
) -> ClassDef {
    ClassDef {
        name,
        superclass: Some(superclass),
        props,
    }
}

const GREY: Color3 = Color3::new(163.0, 162.0, 165.0);

static CLASSES: &[ClassDef] = &[
    ClassDef {
        name: "Instance",
        superclass: None,
        // The default Name is the class name; `default_value` special-cases it.
        props: &[p(
            "Name",
            ValueType::String,
            || Value::String(String::new()),
        )],
    },
    class(
        "Workspace",
        "Instance",
        &[p("FallenPartsDestroyHeight", ValueType::Number, || {
            Value::Number(-500.0)
        })],
    ),
    class("Players", "Instance", &[]),
    // Child of each Player; the player's GUI and LocalScripts live here.
    class("PlayerGui", "Instance", &[]),
    class("ReplicatedStorage", "Instance", &[]),
    class("ServerScriptService", "Instance", &[]),
    class("StarterPlayer", "Instance", &[]),
    class("StarterPlayerScripts", "Instance", &[]),
    class("StarterGui", "Instance", &[]),
    class("Lighting", "Instance", &[]),
    class("RunService", "Instance", &[]),
    class("Folder", "Instance", &[]),
    class("Model", "Instance", &[]),
    class(
        "BasePart",
        "Instance",
        &[
            p("Position", ValueType::Vector3, || {
                Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
            }),
            p("Orientation", ValueType::Vector3, || {
                Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
            }),
            p("Size", ValueType::Vector3, || {
                Value::Vector3(Vec3::new(4.0, 1.0, 2.0))
            }),
            p("Color", ValueType::Color3, || Value::Color3(GREY)),
            PropDef {
                name: "Material",
                ty: ValueType::String,
                default: || Value::String("Plastic".into()),
                allowed: Some(MATERIALS),
            },
            p("Anchored", ValueType::Bool, || Value::Bool(false)),
            p("CanCollide", ValueType::Bool, || Value::Bool(true)),
            p("Transparency", ValueType::Number, || Value::Number(0.0)),
            PropDef {
                name: "Shape",
                ty: ValueType::String,
                default: || Value::String("Block".into()),
                allowed: Some(SHAPES),
            },
        ],
    ),
    class("Part", "BasePart", &[]),
    class("SpawnLocation", "Part", &[]),
    class(
        "Checkpoint",
        "Part",
        &[p("Order", ValueType::Number, || Value::Number(0.0))],
    ),
    class(
        "Script",
        "Instance",
        &[
            p("Source", ValueType::String, || Value::String(String::new())),
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
        ],
    ),
    class("LocalScript", "Script", &[]),
    class("RemoteEvent", "Instance", &[]),
    class(
        "Humanoid",
        "Instance",
        &[
            p("Health", ValueType::Number, || Value::Number(100.0)),
            p("MaxHealth", ValueType::Number, || Value::Number(100.0)),
            p("WalkSpeed", ValueType::Number, || Value::Number(16.0)),
            p("JumpPower", ValueType::Number, || Value::Number(50.0)),
            p("MaxSlopeAngle", ValueType::Number, || Value::Number(89.0)),
        ],
    ),
    class(
        "ScreenGui",
        "Instance",
        &[p("Enabled", ValueType::Bool, || Value::Bool(true))],
    ),
    class(
        "Frame",
        "Instance",
        &[
            p("Position", ValueType::UDim2, || {
                Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
            }),
            p("Size", ValueType::UDim2, || {
                Value::UDim2(UDim2::new(0.0, 100.0, 0.0, 100.0))
            }),
            p("BackgroundColor3", ValueType::Color3, || {
                Value::Color3(GREY)
            }),
            p("BackgroundTransparency", ValueType::Number, || {
                Value::Number(0.0)
            }),
            p("Visible", ValueType::Bool, || Value::Bool(true)),
        ],
    ),
    class(
        "TextLabel",
        "Frame",
        &[
            p("Text", ValueType::String, || Value::String(String::new())),
            p("TextColor3", ValueType::Color3, || {
                Value::Color3(Color3::new(0.0, 0.0, 0.0))
            }),
            p("TextSize", ValueType::Number, || Value::Number(24.0)),
        ],
    ),
    class(
        "Player",
        "Instance",
        &[
            p("UserId", ValueType::Number, || Value::Number(0.0)),
            p("RespawnLocation", ValueType::Ref, || Value::Ref(None)),
            p("Character", ValueType::Ref, || Value::Ref(None)),
        ],
    ),
];

/// Looks up a class by name.
pub fn class_def(name: &str) -> Option<&'static ClassDef> {
    CLASSES.iter().find(|c| c.name == name)
}

/// True when `class` is `of` or inherits from it.
pub fn is_subclass(class: &str, of: &str) -> bool {
    let mut cur = class_def(class);
    while let Some(c) = cur {
        if c.name == of {
            return true;
        }
        cur = c.superclass.and_then(class_def);
    }
    false
}

/// All properties of a class including inherited ones, base class first.
pub fn all_properties(class: &str) -> Vec<&'static PropDef> {
    let mut chain = Vec::new();
    let mut cur = class_def(class);
    while let Some(c) = cur {
        chain.push(c);
        cur = c.superclass.and_then(class_def);
    }
    chain.iter().rev().flat_map(|c| c.props.iter()).collect()
}

/// Finds a property (own or inherited) of a class.
pub fn find_property(class: &str, property: &str) -> Option<&'static PropDef> {
    all_properties(class)
        .into_iter()
        .find(|d| d.name == property)
}

/// The default value of a property; `Name` defaults to the class name.
pub fn default_value(class: &str, property: &str) -> Option<Value> {
    if property == "Name" {
        return class_def(class).map(|c| Value::String(c.name.to_string()));
    }
    find_property(class, property).map(|d| (d.default)())
}
