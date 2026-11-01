//! Sky: the sun, moon and stars, a child of Lighting.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Sky", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("The sun, moon and stars; takes effect as a child of Lighting.")
    .props(&[
        num("SunAngularSize", || Value::Number(1.5), 0.0, 60.0),
        num("MoonAngularSize", || Value::Number(1.5), 0.0, 60.0),
        num("StarCount", || Value::Number(3000.0), 0.0, 10000.0),
        PropDesc::new("CelestialBodiesShown", ValueType::Bool, || {
            Value::Bool(true)
        }),
    ]);
