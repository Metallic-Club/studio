//! The class registry: every class, its superclass and its properties with defaults.
//!
//! To add a class, add one [`ClassDef`] entry to `CLASSES` below and its index to [`class_def`].
use crate::cframe::CFrame;
use crate::value::{Color3, UDim2, Value, ValueType, Vec3};
use std::collections::HashMap;
use std::sync::LazyLock;

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

/// `Lighting.LightingStyle` values.
pub const LIGHTING_STYLES: &[&str] = &["Realistic", "Soft"];

/// `ColorGradingEffect.TonemapperPreset` values. `Default` and `Retro` are Roblox's names; the
/// rest are Metallic additions (`docs/research/2026-10-30-lighting.md` 5.4). `Default` follows
/// the Look's curve (AgX Punchy under Realistic); `AgXPunchy` names that curve explicitly
/// (`docs/design/look-presets.md` conflict 14).
pub const TONEMAPPER_PRESETS: &[&str] = &[
    "Default",
    "Retro",
    "AgX",
    "Neutral",
    "ACES",
    "Filmic",
    "AgXPunchy",
];

/// Definition of one property of a class.
pub struct PropDef {
    /// Property name.
    pub name: &'static str,
    /// Type that values must have.
    pub ty: ValueType,
    /// Builds an owned default; the borrowed-default registry calls this once during initialization.
    pub default: fn() -> Value,
    /// When set, a String value must be one of these.
    pub allowed: Option<&'static [&'static str]>,
    /// Engine bookkeeping that Luau and the Properties panel do not show (for example the scale
    /// behind `Model:GetScale`). Saved and cloned like any other property.
    pub hidden: bool,
    /// When set, Number values and every Color3 component are brought into this range on
    /// write (and so on load and from scripts).
    pub range: Option<NumRange>,
}

/// A numeric property's sane range: values outside are clamped (or wrapped, for a time of day).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumRange {
    /// Smallest value.
    pub min: f64,
    /// Largest value (exclusive when `wrap`).
    pub max: f64,
    /// Wrap around instead of clamping (`ClockTime` 25 is 1).
    pub wrap: bool,
}

impl NumRange {
    /// `x` brought into the range. `x` must be finite.
    pub fn apply_number(self, x: f64) -> f64 {
        if self.wrap {
            let span = self.max - self.min;
            let w = self.min + (x - self.min).rem_euclid(span);
            // rem_euclid can round up to `span` for tiny negative inputs.
            if w >= self.max { self.min } else { w }
        } else {
            x.clamp(self.min, self.max)
        }
    }

    /// `value` brought into the range: Numbers and Color3 components; other types unchanged.
    pub fn apply(self, value: Value) -> Value {
        match value {
            Value::Number(n) => Value::Number(self.apply_number(n)),
            Value::Color3(c) => Value::Color3(Color3::new(
                self.apply_number(c.r),
                self.apply_number(c.g),
                self.apply_number(c.b),
            )),
            other => other,
        }
    }
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
        range: None,
        hidden: false,
    }
}

/// A property that only the engine reads and writes (see [`PropDef::hidden`]).
const fn hidden(name: &'static str, ty: ValueType, default: fn() -> Value) -> PropDef {
    PropDef {
        name,
        ty,
        default,
        allowed: None,
        hidden: true,
        range: None,
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
const BLACK: Color3 = Color3::new(0.0, 0.0, 0.0);
const WHITE: Color3 = Color3::new(255.0, 255.0, 255.0);

/// A Number clamped to `min..=max`.
const fn num(name: &'static str, default: fn() -> Value, min: f64, max: f64) -> PropDef {
    PropDef {
        range: Some(NumRange {
            min,
            max,
            wrap: false,
        }),
        ..p(name, ValueType::Number, default)
    }
}

/// A Color3 with components clamped to 0..=255.
const fn color(name: &'static str, default: fn() -> Value) -> PropDef {
    PropDef {
        range: Some(NumRange {
            min: 0.0,
            max: 255.0,
            wrap: false,
        }),
        ..p(name, ValueType::Color3, default)
    }
}

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
    // Lighting service and its effect children: Roblox names, Metallic defaults and meanings
    // (docs/research/2026-10-30-lighting.md section 5). Presets: `crate::lighting`.
    class(
        "Lighting",
        "Instance",
        &[
            PropDef {
                range: Some(NumRange {
                    min: 0.0,
                    max: 24.0,
                    wrap: true,
                }),
                ..p("ClockTime", ValueType::Number, || Value::Number(14.5))
            },
            num("GeographicLatitude", || Value::Number(41.73), -90.0, 90.0),
            num("Brightness", || Value::Number(1.0), 0.0, 10.0),
            num("ExposureCompensation", || Value::Number(0.0), -5.0, 5.0),
            color("Ambient", || Value::Color3(BLACK)),
            color("OutdoorAmbient", || Value::Color3(BLACK)),
            color("ColorShift_Top", || Value::Color3(BLACK)),
            color("ColorShift_Bottom", || Value::Color3(BLACK)),
            num("EnvironmentDiffuseScale", || Value::Number(1.0), 0.0, 1.0),
            num("EnvironmentSpecularScale", || Value::Number(1.0), 0.0, 1.0),
            p("GlobalShadows", ValueType::Bool, || Value::Bool(true)),
            num("ShadowSoftness", || Value::Number(0.2), 0.0, 1.0),
            PropDef {
                name: "LightingStyle",
                ty: ValueType::String,
                default: || Value::String("Realistic".into()),
                allowed: Some(LIGHTING_STYLES),
                range: None,
                hidden: false,
            },
            color("FogColor", || {
                Value::Color3(Color3::new(192.0, 192.0, 192.0))
            }),
            num("FogStart", || Value::Number(0.0), 0.0, 1e7),
            num("FogEnd", || Value::Number(100000.0), 0.0, 1e7),
            num("NightAdaptation", || Value::Number(1.0), 0.0, 1.0),
            num("MoonBrightness", || Value::Number(1.0), 0.0, 10.0),
            // Histogram metering (#151; lighting doc 3.9 and 5.5). Off: the fixed exposure from
            // the sun elevation. EV100 limits of the meter.
            p("AutoExposure", ValueType::Bool, || Value::Bool(true)),
            num("ExposureMin", || Value::Number(-4.0), -10.0, 20.0),
            num("ExposureMax", || Value::Number(16.0), -10.0, 20.0),
        ],
    ),
    class("RunService", "Instance", &[]),
    class("Folder", "Instance", &[]),
    // Pivots and scale (docs/design/building-tools.md section 4.4, `metallic_core::transform`).
    class(
        "Model",
        "Instance",
        &[
            p("PrimaryPart", ValueType::Ref, || Value::Ref(None)),
            p("WorldPivot", ValueType::CFrame, || {
                Value::CFrame(Box::new(CFrame::IDENTITY))
            }),
            // False until WorldPivot is first set: the pivot is then the bounding-box centre.
            hidden("WorldPivotSet", ValueType::Bool, || Value::Bool(false)),
            // What `GetScale` returns; `ScaleTo` changes it.
            hidden("ScaleFactor", ValueType::Number, || Value::Number(1.0)),
        ],
    ),
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
                range: None,
                hidden: false,
            },
            p("Anchored", ValueType::Bool, || Value::Bool(false)),
            p("CanCollide", ValueType::Bool, || Value::Bool(true)),
            p("Transparency", ValueType::Number, || Value::Number(0.0)),
            PropDef {
                name: "Shape",
                ty: ValueType::String,
                default: || Value::String("Block".into()),
                allowed: Some(SHAPES),
                range: None,
                hidden: false,
            },
            // Last, so lookups of the older properties scan no further than before.
            p("PivotOffset", ValueType::CFrame, || {
                Value::CFrame(Box::new(CFrame::IDENTITY))
            }),
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
                range: None,
                hidden: false,
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
                range: None,
                hidden: false,
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
                range: None,
                hidden: false,
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
                range: None,
                hidden: false,
            },
            p("TouchLabel", ValueType::String, || Value::String("".into())),
            p("TouchImage", ValueType::String, || Value::String("".into())),
            PropDef {
                name: "TouchLayout",
                ty: ValueType::String,
                default: || Value::String("Auto".into()),
                allowed: Some(TOUCH_LAYOUTS),
                range: None,
                hidden: false,
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
    class(
        "Atmosphere",
        "Instance",
        &[
            num("Density", || Value::Number(0.3), 0.0, 1.0),
            num("Offset", || Value::Number(0.25), 0.0, 1.0),
            color("Color", || Value::Color3(Color3::new(199.0, 199.0, 199.0))),
            color("Decay", || Value::Color3(Color3::new(106.0, 112.0, 125.0))),
            num("Glare", || Value::Number(2.0), 0.0, 10.0),
            num("Haze", || Value::Number(1.0), 0.0, 10.0),
            num("HeightFalloff", || Value::Number(0.02), 0.0, 1.0),
            num("FogHeight", || Value::Number(0.0), -1e5, 1e5),
            color("Absorption", || Value::Color3(BLACK)),
            color("AirColor", || {
                Value::Color3(Color3::new(116.0, 172.0, 255.0))
            }),
        ],
    ),
    class(
        "Sky",
        "Instance",
        &[
            num("SunAngularSize", || Value::Number(1.5), 0.0, 60.0),
            num("MoonAngularSize", || Value::Number(1.5), 0.0, 60.0),
            num("StarCount", || Value::Number(3000.0), 0.0, 10000.0),
            p("CelestialBodiesShown", ValueType::Bool, || {
                Value::Bool(true)
            }),
        ],
    ),
    class(
        "Clouds",
        "Instance",
        &[
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            num("Cover", || Value::Number(0.0), 0.0, 1.0),
            color("Color", || Value::Color3(WHITE)),
        ],
    ),
    class(
        "BloomEffect",
        "Instance",
        &[
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            num("Intensity", || Value::Number(0.04), 0.0, 1.0),
            num("Size", || Value::Number(24.0), 0.0, 64.0),
            num("Threshold", || Value::Number(0.0), 0.0, 10.0),
        ],
    ),
    class(
        "ColorCorrectionEffect",
        "Instance",
        &[
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            num("Brightness", || Value::Number(0.0), -1.0, 1.0),
            num("Contrast", || Value::Number(0.0), -1.0, 1.0),
            num("Saturation", || Value::Number(0.0), -1.0, 1.0),
            color("TintColor", || Value::Color3(WHITE)),
        ],
    ),
    class(
        "ColorGradingEffect",
        "Instance",
        &[
            p("Enabled", ValueType::Bool, || Value::Bool(true)),
            PropDef {
                name: "TonemapperPreset",
                ty: ValueType::String,
                default: || Value::String("Default".into()),
                allowed: Some(TONEMAPPER_PRESETS),
                range: None,
                hidden: false,
            },
        ],
    ),
];

/// Looks up a class by name.
///
/// A `match` on the name instead of a scan of `CLASSES`: this runs on every property read and
/// write (through [`find_property`]), and the scan got measurably slower whenever a class was
/// added (CI instructions gate, PR #278). Keep the indices in `CLASSES` order; the test
/// `class_def_matches_the_registry` fails if they drift.
pub fn class_def(name: &str) -> Option<&'static ClassDef> {
    let index = match name {
        "Instance" => 0,
        "Workspace" => 1,
        "Players" => 2,
        "PlayerGui" => 3,
        "ReplicatedStorage" => 4,
        "ServerScriptService" => 5,
        "StarterPlayer" => 6,
        "StarterPlayerScripts" => 7,
        "StarterGui" => 8,
        "Lighting" => 9,
        "RunService" => 10,
        "Folder" => 11,
        "Model" => 12,
        "BasePart" => 13,
        "Part" => 14,
        "SpawnLocation" => 15,
        "Checkpoint" => 16,
        "Script" => 17,
        "LocalScript" => 18,
        "RemoteEvent" => 19,
        "Humanoid" => 20,
        "ScreenGui" => 21,
        "Frame" => 22,
        "TextLabel" => 23,
        "Player" => 24,
        "InputContext" => 25,
        "InputAction" => 26,
        "InputBinding" => 27,
        // Appended after the hot classes so their lookups do not change.
        "Atmosphere" => 28,
        "Sky" => 29,
        "Clouds" => 30,
        "BloomEffect" => 31,
        "ColorCorrectionEffect" => 32,
        "ColorGradingEffect" => 33,
        _ => return None,
    };
    CLASSES.get(index)
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

static DEFAULT_VALUES: LazyLock<HashMap<&'static str, HashMap<&'static str, Value>>> =
    LazyLock::new(|| {
        let mut values = HashMap::new();
        for class in CLASSES {
            let mut properties = HashMap::new();
            for property in all_properties(class.name) {
                let value = if property.name == "Name" {
                    Value::String(class.name.to_owned())
                } else {
                    (property.default)()
                };
                properties.insert(property.name, value);
            }
            values.insert(class.name, properties);
        }
        values
    });

/// Borrows the shared default for a property; `Name` defaults to the concrete class name.
///
/// The immutable registry initializes once on the first call. Subsequent successful reads do
/// not allocate or clone, and the returned values remain valid for the life of the process.
/// Unknown classes or properties return `None`.
pub fn default_value_ref(class: &str, property: &str) -> Option<&'static Value> {
    DEFAULT_VALUES.get(class)?.get(property)
}

/// An owned default value; `Name` defaults to the concrete class name.
///
/// Use [`default_value_ref`] to inspect defaults without cloning string values.
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
    fn class_def_matches_the_registry() {
        for (i, c) in CLASSES.iter().enumerate() {
            let found =
                class_def(c.name).unwrap_or_else(|| panic!("{} missing from class_def", c.name));
            assert!(
                std::ptr::eq(found, &CLASSES[i]),
                "{} maps to {}",
                c.name,
                found.name
            );
        }
        assert!(class_def("NotAClass").is_none());
        assert!(class_def("part").is_none());
    }

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
