//! TextLabel: a rectangle with text. Under `Frame`, unlike Roblox (design section 3.4).
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("TextLabel", Some("Frame"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .insert_order(420)
    .describe("A rectangle with text.")
    .props(&[
        PropDesc::new("Text", ValueType::String, || Value::String(String::new()))
            .editor(Editor::section(Section::APPEARANCE))
            .describe("The words shown."),
        PropDesc::new("TextColor3", ValueType::Color3, || {
            Value::Color3(Color3::new(0.0, 0.0, 0.0))
        })
        .editor(Editor::section(Section::APPEARANCE))
        .describe("The colour of the words."),
        PropDesc::new("TextSize", ValueType::Number, || Value::Number(24.0))
            .editor(Editor::section(Section::APPEARANCE).drag(NumberUnit::Integer))
            .describe("The height of the words, in pixels."),
    ]);
