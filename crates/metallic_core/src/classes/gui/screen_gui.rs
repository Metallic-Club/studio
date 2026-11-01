//! ScreenGui: one layer of GUI on the screen.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ScreenGui", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("GUI")
    .describe("One layer of GUI on the screen.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        // Higher draws (and takes clicks) above lower; equal orders keep tree order.
        PropDesc::new("DisplayOrder", ValueType::Number, || Value::Number(0.0)),
    ]);
