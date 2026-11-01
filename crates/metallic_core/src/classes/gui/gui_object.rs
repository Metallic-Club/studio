//! GuiObject: a rectangle on the screen; the shared GUI properties live here.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("GuiObject", Some("GuiBase2d"))
    .flags(ClassFlags::ABSTRACT)
    .category("GUI")
    .describe("A rectangle on the screen, with position, size and background.")
    .props(&[
        PropDesc::new("Position", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
        }),
        PropDesc::new("Size", ValueType::UDim2, || {
            Value::UDim2(UDim2::new(0.0, 100.0, 0.0, 100.0))
        }),
        PropDesc::new("BackgroundColor3", ValueType::Color3, || {
            Value::Color3(GREY)
        }),
        PropDesc::new("BackgroundTransparency", ValueType::Number, || {
            Value::Number(0.0)
        }),
        PropDesc::new("Visible", ValueType::Bool, || Value::Bool(true)),
        // Siblings draw in ZIndex order (ties in tree order), above their parent.
        PropDesc::new("ZIndex", ValueType::Number, || Value::Number(1.0)),
        // An Active element takes the pointer: clicks on it do not reach what is below
        // or the game. Buttons always do.
        PropDesc::new("Active", ValueType::Bool, || Value::Bool(false)),
        // Descendants are drawn and clicked only inside this element's rectangle.
        PropDesc::new("ClipsDescendants", ValueType::Bool, || Value::Bool(false)),
    ]);
