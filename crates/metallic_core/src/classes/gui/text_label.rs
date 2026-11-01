//! TextLabel: a rectangle with text. Under `Frame`, unlike Roblox (design section 3.4).
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("TextLabel", Some("Frame"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .describe("A rectangle with text.")
    .props(&[
        PropDesc::new("Text", ValueType::String, || Value::String(String::new())),
        PropDesc::new("TextColor3", ValueType::Color3, || {
            Value::Color3(Color3::new(0.0, 0.0, 0.0))
        }),
        PropDesc::new("TextSize", ValueType::Number, || Value::Number(24.0)),
    ]);
