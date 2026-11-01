//! InputAction: something a player can do, independent of the control that does it.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputAction", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .describe("Something a player can do, independent of the control that does it.")
    .props(&[
        choice("Type", || Value::String("Bool".into()), INPUT_ACTION_TYPES),
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("DisplayName", ValueType::String, || {
            Value::String("".into())
        }),
        choice("Trigger", || Value::String("Press".into()), INPUT_TRIGGERS),
        PropDesc::new("HoldTime", ValueType::Number, || Value::Number(500.0)),
        PropDesc::new("TapTime", ValueType::Number, || Value::Number(200.0)),
        PropDesc::new("DoubleTapWindow", ValueType::Number, || {
            Value::Number(300.0)
        }),
        PropDesc::new("RepeatDelay", ValueType::Number, || Value::Number(400.0)),
        PropDesc::new("RepeatRate", ValueType::Number, || Value::Number(100.0)),
        choice("Role", || Value::String("None".into()), INPUT_ROLES),
        PropDesc::new("Rebindable", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("Required", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("CanToggle", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("ShowOnTouch", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("TouchOrder", ValueType::Number, || Value::Number(0.0)),
        choice("TouchSlot", || Value::String("Auto".into()), TOUCH_SLOTS),
        PropDesc::new("TouchLabel", ValueType::String, || Value::String("".into())),
        PropDesc::new("TouchImage", ValueType::String, || Value::String("".into())),
        choice(
            "TouchLayout",
            || Value::String("Auto".into()),
            TOUCH_LAYOUTS,
        ),
        PropDesc::new("TouchPosition", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
        }),
        PropDesc::new("TouchSize", ValueType::Number, || Value::Number(0.0)),
    ]);
