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

/// Value types of an `InputAction` (`Enum.InputActionType`).
pub const INPUT_ACTION_TYPES: &[&str] = &[
    "Bool",
    "Direction1D",
    "Direction2D",
    "Direction3D",
    "ViewportPosition",
];

/// When an `InputAction` fires `Triggered` (`Enum.InputTrigger`).
pub const INPUT_TRIGGERS: &[&str] = &["Press", "Release", "Hold", "Tap", "DoubleTap", "Repeat"];

/// What an `InputAction` is for (`Enum.InputRole`).
pub const INPUT_ROLES: &[&str] = &[
    "None",
    "Move",
    "Look",
    "Jump",
    "Run",
    "Primary",
    "Secondary",
    "Interact",
    "Menu",
    "Zoom",
];

/// Which touch control an `InputAction` of type Direction2D uses (`Enum.TouchSlot`).
pub const TOUCH_SLOTS: &[&str] = &["Auto", "Stick", "Look"];

/// How an `InputAction`'s touch control is placed.
pub const TOUCH_LAYOUTS: &[&str] = &["Auto", "Custom"];

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
    // Appended last: `class_def` is a linear scan and Part/Model reads are on the hot path
    // (CI instructions gate).
    // Input actions (docs/design/input-actions.md section 3): a context holds actions, an action
    // holds bindings. Client-only; inert on the server.
    class(
        "InputContext",
        "Instance",
        &[
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            p("Priority", ValueType::Number, || Value::Number(0.0)),
            p("Sink", ValueType::Bool, || Value::Bool(false)),
            p("RebindGroup", ValueType::String, || {
                Value::String("Game".into())
            }),
        ],
    ),
    class(
        "InputAction",
        "Instance",
        &[
            PropDef {
                name: "Type",
                ty: ValueType::String,
                default: || Value::String("Bool".into()),
                allowed: Some(INPUT_ACTION_TYPES),
            },
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            p("DisplayName", ValueType::String, || {
                Value::String("".into())
            }),
            PropDef {
                name: "Trigger",
                ty: ValueType::String,
                default: || Value::String("Press".into()),
                allowed: Some(INPUT_TRIGGERS),
            },
            p("HoldTime", ValueType::Number, || Value::Number(500.0)),
            p("TapTime", ValueType::Number, || Value::Number(200.0)),
            p("DoubleTapWindow", ValueType::Number, || {
                Value::Number(300.0)
            }),
            p("RepeatDelay", ValueType::Number, || Value::Number(400.0)),
            p("RepeatRate", ValueType::Number, || Value::Number(100.0)),
            PropDef {
                name: "Role",
                ty: ValueType::String,
                default: || Value::String("None".into()),
                allowed: Some(INPUT_ROLES),
            },
            p("Rebindable", ValueType::Bool, || Value::Bool(true)),
            p("Required", ValueType::Bool, || Value::Bool(false)),
            p("CanToggle", ValueType::Bool, || Value::Bool(true)),
            p("ShowOnTouch", ValueType::Bool, || Value::Bool(false)),
            p("TouchOrder", ValueType::Number, || Value::Number(0.0)),
            PropDef {
                name: "TouchSlot",
                ty: ValueType::String,
                default: || Value::String("Auto".into()),
                allowed: Some(TOUCH_SLOTS),
            },
            p("TouchLabel", ValueType::String, || Value::String("".into())),
            p("TouchImage", ValueType::String, || Value::String("".into())),
            PropDef {
                name: "TouchLayout",
                ty: ValueType::String,
                default: || Value::String("Auto".into()),
                allowed: Some(TOUCH_LAYOUTS),
            },
            p("TouchPosition", ValueType::UDim2, || {
                Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
            }),
            p("TouchSize", ValueType::Number, || Value::Number(0.0)),
        ],
    ),
    // `KeyCode` and the composite/modifier fields name a control ("W", "ButtonA", "None");
    // the runtime checks the names. `Deadzone` -1 means "the default of the control".
    // `Vector2Scale` uses a Vector3 (z ignored) until the Vector2 value type exists.
    class(
        "InputBinding",
        "Instance",
        &[
            p("KeyCode", ValueType::String, || {
                Value::String("None".into())
            }),
            p("Up", ValueType::String, || Value::String("None".into())),
            p("Down", ValueType::String, || Value::String("None".into())),
            p("Left", ValueType::String, || Value::String("None".into())),
            p("Right", ValueType::String, || Value::String("None".into())),
            p("Forward", ValueType::String, || {
                Value::String("None".into())
            }),
            p("Backward", ValueType::String, || {
                Value::String("None".into())
            }),
            p("PrimaryModifier", ValueType::String, || {
                Value::String("None".into())
            }),
            p("SecondaryModifier", ValueType::String, || {
                Value::String("None".into())
            }),
            p("Scale", ValueType::Number, || Value::Number(1.0)),
            p("Vector2Scale", ValueType::Vector3, || {
                Value::Vector3(Vec3::new(1.0, 1.0, 0.0))
            }),
            p("Vector3Scale", ValueType::Vector3, || {
                Value::Vector3(Vec3::new(1.0, 1.0, 1.0))
            }),
            p("ResponseCurve", ValueType::Number, || Value::Number(1.0)),
            p("Deadzone", ValueType::Number, || Value::Number(-1.0)),
            p("ScaleByDeltaTime", ValueType::Bool, || Value::Bool(false)),
            p("ClampMagnitudeToOne", ValueType::Bool, || Value::Bool(true)),
            p("PressedThreshold", ValueType::Number, || Value::Number(0.5)),
            p("ReleasedThreshold", ValueType::Number, || {
                Value::Number(0.4)
            }),
            p("UIButton", ValueType::Ref, || Value::Ref(None)),
            p("PointerIndex", ValueType::Number, || Value::Number(0.0)),
            p("DisplayName", ValueType::String, || {
                Value::String("".into())
            }),
            p("DisplayImage", ValueType::String, || {
                Value::String("".into())
            }),
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
///
/// Runs on every property read and write, so it walks the class chain without allocating.
/// No class redeclares an inherited property (a test checks this), so the search order does
/// not change the result.
pub fn find_property(class: &str, property: &str) -> Option<&'static PropDef> {
    let mut cur = class_def(class);
    while let Some(c) = cur {
        if let Some(d) = c.props.iter().find(|d| d.name == property) {
            return Some(d);
        }
        cur = c.superclass.and_then(class_def);
    }
    None
}

/// The default value of a property; `Name` defaults to the class name.
pub fn default_value(class: &str, property: &str) -> Option<Value> {
    if property == "Name" {
        return class_def(class).map(|c| Value::String(c.name.to_string()));
    }
    find_property(class, property).map(|d| (d.default)())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_class_redeclares_an_inherited_property() {
        for c in CLASSES {
            let names: Vec<&str> = all_properties(c.name).iter().map(|d| d.name).collect();
            let mut unique = names.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                names.len(),
                unique.len(),
                "class {} repeats a property",
                c.name
            );
        }
    }
}
