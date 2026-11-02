//! ColorCorrectionEffect: brightness, contrast, saturation and tint.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ColorCorrectionEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("Adjusts brightness, contrast, saturation and tint of the final image.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether the effect is on."),
        num("Brightness", || Value::Number(0.0), -1.0, 1.0)
            .describe("Makes the image darker (below 0) or brighter (above 0)."),
        num("Contrast", || Value::Number(0.0), -1.0, 1.0)
            .describe("Lowers or raises the difference between light and dark."),
        num("Saturation", || Value::Number(0.0), -1.0, 1.0)
            .describe("Takes colour out of the image (below 0) or adds it (above 0)."),
        color("TintColor", || Value::Color3(WHITE))
            .describe("A colour the image is tinted with; white leaves it as it is."),
    ]);
