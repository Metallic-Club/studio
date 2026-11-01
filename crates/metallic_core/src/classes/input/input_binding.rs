//! InputBinding: a control that drives an action.
//!
//! `KeyCode` and the composite/modifier fields name a control ("W", "ButtonA", "None"); the runtime
//! checks the names. `Deadzone` -1 means "the default of the control". `Vector2Scale` uses a
//! Vector3 (z ignored) until the Vector2 value type exists.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputBinding", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .describe("A control that drives its parent action.")
    .props(&[
        PropDesc::new("KeyCode", ValueType::String, || {
            Value::String("None".into())
        }),
        PropDesc::new("Up", ValueType::String, || Value::String("None".into())),
        PropDesc::new("Down", ValueType::String, || Value::String("None".into())),
        PropDesc::new("Left", ValueType::String, || Value::String("None".into())),
        PropDesc::new("Right", ValueType::String, || Value::String("None".into())),
        PropDesc::new("Forward", ValueType::String, || {
            Value::String("None".into())
        }),
        PropDesc::new("Backward", ValueType::String, || {
            Value::String("None".into())
        }),
        PropDesc::new("PrimaryModifier", ValueType::String, || {
            Value::String("None".into())
        }),
        PropDesc::new("SecondaryModifier", ValueType::String, || {
            Value::String("None".into())
        }),
        PropDesc::new("Scale", ValueType::Number, || Value::Number(1.0)),
        PropDesc::new("Vector2Scale", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(1.0, 1.0, 0.0))
        }),
        PropDesc::new("Vector3Scale", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(1.0, 1.0, 1.0))
        }),
        PropDesc::new("ResponseCurve", ValueType::Number, || Value::Number(1.0)),
        PropDesc::new("Deadzone", ValueType::Number, || Value::Number(-1.0)),
        PropDesc::new("ScaleByDeltaTime", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("ClampMagnitudeToOne", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("PressedThreshold", ValueType::Number, || Value::Number(0.5)),
        PropDesc::new("ReleasedThreshold", ValueType::Number, || {
            Value::Number(0.4)
        }),
        PropDesc::new("UIButton", ValueType::Ref, || Value::Ref(None)),
        PropDesc::new("PointerIndex", ValueType::Number, || Value::Number(0.0)),
        PropDesc::new("DisplayName", ValueType::String, || {
            Value::String("".into())
        }),
        PropDesc::new("DisplayImage", ValueType::String, || {
            Value::String("".into())
        }),
    ]);
