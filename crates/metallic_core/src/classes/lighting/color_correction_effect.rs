//! ColorCorrectionEffect: brightness, contrast, saturation and tint.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ColorCorrectionEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("Adjusts brightness, contrast, saturation and tint of the final image.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        num("Brightness", || Value::Number(0.0), -1.0, 1.0),
        num("Contrast", || Value::Number(0.0), -1.0, 1.0),
        num("Saturation", || Value::Number(0.0), -1.0, 1.0),
        color("TintColor", || Value::Color3(WHITE)),
    ]);
