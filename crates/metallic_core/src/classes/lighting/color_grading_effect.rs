//! ColorGradingEffect: the tone curve.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ColorGradingEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("Chooses the tone curve of the final image.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether the effect is on."),
        choice(
            "TonemapperPreset",
            || Value::String("Default".into()),
            TONEMAPPER_PRESETS,
        )
        .describe("The tone curve that turns the scene's light into screen colours."),
    ]);
