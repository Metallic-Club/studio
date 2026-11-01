//! InputBinding: a control that drives an action.
//!
//! `KeyCode` and the composite/modifier fields name a control ("W", "ButtonA", "None"); the runtime
//! checks the names. `Deadzone` -1 means "the default of the control". `Vector2Scale` uses a
//! Vector3 (z ignored) until the Vector2 value type exists.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputBinding", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .no_insert()
    .describe("A control that drives its parent action.")
    .props(&[
        PropDesc::new("KeyCode", ValueType::String, || {
            Value::String("None".into())
        })
        .describe("The key or button that drives the action, such as W or ButtonA."),
        PropDesc::new("Up", ValueType::String, || Value::String("None".into()))
            .describe("The key for up, when several keys make one direction."),
        PropDesc::new("Down", ValueType::String, || Value::String("None".into()))
            .describe("The key for down, when several keys make one direction."),
        PropDesc::new("Left", ValueType::String, || Value::String("None".into()))
            .describe("The key for left, when several keys make one direction."),
        PropDesc::new("Right", ValueType::String, || Value::String("None".into()))
            .describe("The key for right, when several keys make one direction."),
        PropDesc::new("Forward", ValueType::String, || {
            Value::String("None".into())
        })
        .describe("The key for forward, when several keys make one direction."),
        PropDesc::new("Backward", ValueType::String, || {
            Value::String("None".into())
        })
        .describe("The key for backward, when several keys make one direction."),
        PropDesc::new("PrimaryModifier", ValueType::String, || {
            Value::String("None".into())
        })
        .describe("A key that must be down as well, such as Shift."),
        PropDesc::new("SecondaryModifier", ValueType::String, || {
            Value::String("None".into())
        })
        .describe("A second key that must be down as well."),
        PropDesc::new("Scale", ValueType::Number, || Value::Number(1.0))
            .describe("Multiplies the value; a negative number inverts it."),
        PropDesc::new("Vector2Scale", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(1.0, 1.0, 0.0))
        })
        .describe("Multiplies a two-way direction, per axis; Z is ignored."),
        PropDesc::new("Vector3Scale", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(1.0, 1.0, 1.0))
        })
        .describe("Multiplies a three-way direction, per axis."),
        PropDesc::new("ResponseCurve", ValueType::Number, || Value::Number(1.0))
            .describe("1 is linear; 2 gives finer control near the centre."),
        PropDesc::new("Deadzone", ValueType::Number, || Value::Number(-1.0))
            .describe("Movement smaller than this counts as none, from 0 to 1; -1 uses the control's default."),
        PropDesc::new("ScaleByDeltaTime", ValueType::Bool, || Value::Bool(false))
            .describe("Multiplies by the frame time, for controls that set a rate."),
        PropDesc::new("ClampMagnitudeToOne", ValueType::Bool, || Value::Bool(true))
            .describe("Keeps a direction's length at 1 or less."),
        PropDesc::new("PressedThreshold", ValueType::Number, || Value::Number(0.5))
            .describe("An analog control counts as pressed above this."),
        PropDesc::new("ReleasedThreshold", ValueType::Number, || {
            Value::Number(0.4)
        })
        .describe("An analog control counts as released below this."),
        PropDesc::new("UIButton", ValueType::Ref, || Value::Ref(None))
            .describe("A GUI button that drives this binding."),
        PropDesc::new("PointerIndex", ValueType::Number, || Value::Number(0.0))
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("Which finger counts: 0 is the first."),
        PropDesc::new("DisplayName", ValueType::String, || {
            Value::String("".into())
        })
        .describe("Replaces the text shown for this control."),
        PropDesc::new("DisplayImage", ValueType::String, || {
            Value::String("".into())
        })
        .editor(Editor::new().asset())
        .describe("Replaces the icon shown for this control."),
    ]);
