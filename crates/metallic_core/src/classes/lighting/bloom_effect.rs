//! BloomEffect: light bleeding around bright areas.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("BloomEffect", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("Light bleeding around bright areas.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        num("Intensity", || Value::Number(0.04), 0.0, 1.0),
        num("Size", || Value::Number(24.0), 0.0, 64.0),
        num("Threshold", || Value::Number(0.0), 0.0, 10.0),
    ]);
