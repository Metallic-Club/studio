//! BloomEffect: light bleeding around bright areas.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("BloomEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("Light bleeding around bright areas.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether the effect is on."),
        num("Intensity", || Value::Number(0.04), 0.0, 1.0).describe("How strong the bloom is."),
        num("Size", || Value::Number(24.0), 0.0, 64.0)
            .editor(Editor::section(Section::TRANSFORM))
            .describe("How far the light bleeds."),
        num("Threshold", || Value::Number(0.0), 0.0, 10.0)
            .describe("How bright something must be before it blooms."),
    ]);
