//! GuiBase2d: anything drawn in 2D on the screen.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("GuiBase2d", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("GUI")
    .icon_like("ScreenGui")
    .describe("Anything drawn in 2D on the screen.");
