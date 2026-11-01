//! ImageButton: a button with an image.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ImageButton", Some("GuiButton"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .insert_order(440)
    .describe("A button with an image.")
    .props(&[
        PropDesc::new("Image", ValueType::String, || Value::String(String::new()))
            .editor(Editor::new().asset())
            .describe("The image asset the button shows."),
        PropDesc::new("ImageColor3", ValueType::Color3, || Value::Color3(WHITE))
            .describe("A colour the image is tinted with; white leaves it as it is."),
        PropDesc::new("ImageTransparency", ValueType::Number, || {
            Value::Number(0.0)
        })
        .editor(Editor::new().drag(NumberUnit::Fraction))
        .describe("How see-through the image is: 0 is solid, 1 is invisible."),
    ]);
