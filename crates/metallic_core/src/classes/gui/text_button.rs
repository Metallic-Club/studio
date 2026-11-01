//! TextButton: a button with text.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("TextButton", Some("GuiButton"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .describe("A button with text.")
    .props(&[
        PropDesc::new("Text", ValueType::String, || Value::String("Button".into())),
        PropDesc::new("TextColor3", ValueType::Color3, || {
            Value::Color3(Color3::new(27.0, 42.0, 53.0))
        }),
        PropDesc::new("TextSize", ValueType::Number, || Value::Number(14.0)),
    ]);
