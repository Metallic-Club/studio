//! Value lists and limits shared with other crates. The lists become `EnumDesc`s in #469.

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

/// `Camera.CameraType` values. `Custom` is the game's own camera; `Scriptable` and `Fixed` leave
/// the camera to the script's `CFrame` and `FieldOfView`.
pub const CAMERA_TYPES: &[&str] = &["Custom", "Scriptable", "Fixed"];

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

/// Largest `Humanoid.Health` and `Humanoid.MaxHealth` (writes above are clamped).
pub const MAX_HUMANOID_HEALTH: f64 = 1e9;
/// Largest `Humanoid.WalkSpeed` in studs per second (writes above are clamped).
pub const MAX_WALK_SPEED: f64 = 1000.0;
/// Largest `Humanoid.JumpPower` in studs per second (writes above are clamped).
pub const MAX_JUMP_POWER: f64 = 1000.0;
/// Largest `Humanoid.MaxSlopeAngle` in degrees, as in Roblox (writes above are clamped).
pub const MAX_SLOPE_ANGLE: f64 = 89.0;
