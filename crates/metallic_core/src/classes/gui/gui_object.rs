//! GuiObject: a rectangle on the screen; the shared GUI properties live here.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("GuiObject", Some("GuiBase2d"))
    .flags(ClassFlags::ABSTRACT)
    .category("GUI")
    .icon_like("Frame")
    .describe("A rectangle on the screen, with position, size and background.")
    .props(&[
        PropDesc::new("Position", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
        })
        .editor(Editor::section(Section::TRANSFORM))
        .describe("Where it sits in its parent: a fraction of the parent's size plus pixels."),
        PropDesc::new("Size", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 100.0, 0.0, 100.0))
        })
        .editor(Editor::section(Section::TRANSFORM))
        .describe("How big it is: a fraction of the parent's size plus pixels."),
        PropDesc::new("BackgroundColor3", ValueType::Color3, || {
            Value::Color3(GREY)
        })
        .editor(Editor::section(Section::APPEARANCE))
        .describe("The colour of the background."),
        PropDesc::new("BackgroundTransparency", ValueType::Number, || {
            Value::Number(0.0)
        })
        .editor(Editor::section(Section::APPEARANCE).drag(NumberUnit::Fraction))
        .describe("How see-through the background is: 0 is solid, 1 is invisible."),
        PropDesc::new("Visible", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::APPEARANCE))
            .describe("Whether it and everything inside it are drawn."),
        // Siblings draw in ZIndex order (ties in tree order), above their parent.
        PropDesc::new("ZIndex", ValueType::Number, || Value::Number(1.0))
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("Among siblings, a higher number draws on top."),
        // An Active element takes the pointer: clicks on it do not reach what is below
        // or the game. Buttons always do.
        PropDesc::new("Active", ValueType::Bool, || Value::Bool(false))
            .describe("Takes the pointer: clicks on it do not reach what is below or the game."),
        // Descendants are drawn and clicked only inside this element's rectangle.
        PropDesc::new("ClipsDescendants", ValueType::Bool, || Value::Bool(false))
            .describe("Draws and clicks what is inside it only within its own rectangle."),
    ]);
