//! Clouds: a cloud layer.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Clouds", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("A cloud layer.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        num("Cover", || Value::Number(0.0), 0.0, 1.0),
        color("Color", || Value::Color3(WHITE)),
    ]);
