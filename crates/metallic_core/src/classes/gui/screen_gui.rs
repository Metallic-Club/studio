//! ScreenGui: one layer of GUI on the screen.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ScreenGui", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .insert_order(410)
    .describe("One layer of GUI on the screen.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether this layer is shown."),
        // Higher draws (and takes clicks) above lower; equal orders keep tree order.
        PropDesc::new("DisplayOrder", ValueType::Number, || Value::Number(0.0))
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("A layer with a higher number draws and takes clicks above a lower one."),
    ]);
