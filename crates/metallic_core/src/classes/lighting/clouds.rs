//! Clouds: a cloud layer.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Clouds", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("A cloud layer.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether the clouds are on."),
        num("Cover", || Value::Number(0.0), 0.0, 1.0)
            .describe("How much of the sky is covered: 0 is clear, 1 is overcast."),
        color("Color", || Value::Color3(WHITE))
            .editor(Editor::section(Section::APPEARANCE))
            .describe("The colour of the clouds."),
    ]);
