//! Frame: a plain rectangle.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Frame", Some("GuiObject"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .no_insert()
    .describe("A plain rectangle that holds other GUI.");
