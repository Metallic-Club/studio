//! TextButton: a button with text.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("TextButton", Some("GuiButton"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .insert_order(430)
    .describe("A button with text.")
    .props(&[
        PropDesc::new("Text", ValueType::String, || Value::String("Button".into()))
            .editor(Editor::section(Section::APPEARANCE))
            .describe("The words on the button."),
        PropDesc::new("TextColor3", ValueType::Color3, || {
            Value::Color3(Color3::new(27.0, 42.0, 53.0))
        })
        .editor(Editor::section(Section::APPEARANCE))
        .describe("The colour of the words."),
        PropDesc::new("TextSize", ValueType::Number, || Value::Number(14.0))
            .editor(Editor::section(Section::APPEARANCE).drag(NumberUnit::Integer))
            .describe("The height of the words, in pixels."),
    ]);
