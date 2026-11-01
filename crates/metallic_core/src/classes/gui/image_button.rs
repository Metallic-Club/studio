//! ImageButton: a button with an image.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ImageButton", Some("GuiButton"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .describe("A button with an image.")
    .props(&[
        PropDesc::new("Image", ValueType::String, || Value::String(String::new())),
        PropDesc::new("ImageColor3", ValueType::Color3, || Value::Color3(WHITE)),
        PropDesc::new("ImageTransparency", ValueType::Number, || {
            Value::Number(0.0)
        }),
    ]);
