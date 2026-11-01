//! ColorGradingEffect: the tone curve.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ColorGradingEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("Chooses the tone curve of the final image.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        choice(
            "TonemapperPreset",
            || Value::String("Default".into()),
            TONEMAPPER_PRESETS,
        ),
    ]);
