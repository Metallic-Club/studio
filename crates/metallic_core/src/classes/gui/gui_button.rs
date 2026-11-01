//! GuiButton: clickable GUI (#79). Roblox's GuiButton derives from GuiObject; here it stays under
//! `Frame`, so a button is also a Frame (design section 3.4).
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("GuiButton", Some("Frame"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .describe("A rectangle that takes clicks.")
    .props(&[PropDesc::new("AutoButtonColor", ValueType::Bool, || {
        Value::Bool(true)
    })]);
