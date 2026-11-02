//! InputAction: something a player can do, independent of the control that does it.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputAction", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .no_insert()
    .describe("Something a player can do, independent of the control that does it.")
    .props(&[
        choice("Type", || Value::String("Bool".into()), INPUT_ACTION_TYPES)
            .describe("What kind of value the action has: on or off, a direction or a position."),
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether the action responds to its controls."),
        PropDesc::new("DisplayName", ValueType::String, || {
            Value::String("".into())
        })
        .describe("The name a player sees for this action."),
        choice("Trigger", || Value::String("Press".into()), INPUT_TRIGGERS)
            .describe("What a control has to do to fire the action."),
        PropDesc::new("HoldTime", ValueType::Number, || Value::Number(500.0))
            .describe("How long a Hold trigger must be held, in milliseconds."),
        PropDesc::new("TapTime", ValueType::Number, || Value::Number(200.0))
            .describe("The longest press that still counts as a tap, in milliseconds."),
        PropDesc::new("DoubleTapWindow", ValueType::Number, || {
            Value::Number(300.0)
        })
        .describe("The longest gap between the two taps of a double tap, in milliseconds."),
        PropDesc::new("RepeatDelay", ValueType::Number, || Value::Number(400.0))
            .describe("The wait before a Repeat trigger first repeats, in milliseconds."),
        PropDesc::new("RepeatRate", ValueType::Number, || Value::Number(100.0))
            .describe("The gap between repeats of a Repeat trigger, in milliseconds."),
        choice("Role", || Value::String("None".into()), INPUT_ROLES)
            .describe("What the action is for, such as Move or Jump; presets and prompts use it."),
        PropDesc::new("Rebindable", ValueType::Bool, || Value::Bool(true))
            .describe("Whether it shows on the player's Controls page."),
        PropDesc::new("Required", ValueType::Bool, || Value::Bool(false))
            .describe("The player cannot leave it without a binding on a device that has any."),
        PropDesc::new("CanToggle", ValueType::Bool, || Value::Bool(true))
            .describe("Whether the player may turn a held action into a toggle."),
        PropDesc::new("ShowOnTouch", ValueType::Bool, || Value::Bool(false))
            .describe("Shows a control on touch screens."),
        PropDesc::new("TouchOrder", ValueType::Number, || Value::Number(0.0))
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("Lower comes first: 0 is the main button, then the ring around it."),
        choice("TouchSlot", || Value::String("Auto".into()), TOUCH_SLOTS)
            .describe("For a two-way direction: use the left stick or the look pad."),
        PropDesc::new("TouchLabel", ValueType::String, || Value::String("".into()))
            .describe("The text on the touch button; empty uses DisplayName."),
        PropDesc::new("TouchImage", ValueType::String, || Value::String("".into()))
            .editor(Editor::new().asset())
            .describe("The image on the touch button; empty uses the action's icon."),
        choice(
            "TouchLayout",
            || Value::String("Auto".into()),
            TOUCH_LAYOUTS,
        )
        .describe("Custom places the touch control by TouchPosition and TouchSize."),
        PropDesc::new("TouchPosition", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
        })
        .describe("Where a Custom touch control sits on the screen."),
        PropDesc::new("TouchSize", ValueType::Number, || Value::Number(0.0))
            .describe("How big a Custom touch control is."),
    ]);
