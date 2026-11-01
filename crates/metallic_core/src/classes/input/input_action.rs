//! InputAction: something a player can do, independent of the control that does it.
use crate::classes::prelude::*;

/// The value an action holds.
const INPUT_ACTION_TYPE: EnumDesc = EnumDesc::new(
    "InputActionType",
    &[
        "Bool",
        "Direction1D",
        "Direction2D",
        "Direction3D",
        "ViewportPosition",
    ],
);

/// When an action fires `Triggered`.
const INPUT_TRIGGER: EnumDesc = EnumDesc::new(
    "InputTrigger",
    &["Press", "Release", "Hold", "Tap", "DoubleTap", "Repeat"],
);

/// What an action is for.
const INPUT_ROLE: EnumDesc = EnumDesc::new(
    "InputRole",
    &[
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
    ],
);

/// Which touch control a Direction2D action uses.
const TOUCH_SLOT: EnumDesc = EnumDesc::new("TouchSlot", &["Auto", "Stick", "Look"]);

/// How an action's touch control is placed.
const TOUCH_LAYOUT: EnumDesc = EnumDesc::new("TouchLayout", &["Auto", "Custom"]);

pub static CLASS: ClassDesc = ClassDesc::new("InputAction", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .describe("Something a player can do, independent of the control that does it.")
    .enums(&[
        INPUT_ACTION_TYPE,
        INPUT_TRIGGER,
        INPUT_ROLE,
        TOUCH_SLOT,
        TOUCH_LAYOUT,
    ])
    .props(&[
        enumerated("Type", || Value::String("Bool".into()), &INPUT_ACTION_TYPE),
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("DisplayName", ValueType::String, || {
            Value::String("".into())
        }),
        enumerated("Trigger", || Value::String("Press".into()), &INPUT_TRIGGER),
        PropDesc::new("HoldTime", ValueType::Number, || Value::Number(500.0)),
        PropDesc::new("TapTime", ValueType::Number, || Value::Number(200.0)),
        PropDesc::new("DoubleTapWindow", ValueType::Number, || {
            Value::Number(300.0)
        }),
        PropDesc::new("RepeatDelay", ValueType::Number, || Value::Number(400.0)),
        PropDesc::new("RepeatRate", ValueType::Number, || Value::Number(100.0)),
        enumerated("Role", || Value::String("None".into()), &INPUT_ROLE),
        PropDesc::new("Rebindable", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("Required", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("CanToggle", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("ShowOnTouch", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("TouchOrder", ValueType::Number, || Value::Number(0.0)),
        enumerated("TouchSlot", || Value::String("Auto".into()), &TOUCH_SLOT),
        PropDesc::new("TouchLabel", ValueType::String, || Value::String("".into())),
        PropDesc::new("TouchImage", ValueType::String, || Value::String("".into())),
        enumerated(
            "TouchLayout",
            || Value::String("Auto".into()),
            &TOUCH_LAYOUT,
        ),
        PropDesc::new("TouchPosition", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
        }),
        PropDesc::new("TouchSize", ValueType::Number, || Value::Number(0.0)),
    ]);
