//! Atmosphere: air and haze, a child of Lighting.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Atmosphere", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .describe("Air and haze; takes effect as a child of Lighting.")
    .props(&[
        num("Density", || Value::Number(0.3), 0.0, 1.0),
        num("Offset", || Value::Number(0.25), 0.0, 1.0),
        color("Color", || Value::Color3(Color3::new(199.0, 199.0, 199.0))),
        color("Decay", || Value::Color3(Color3::new(106.0, 112.0, 125.0))),
        num("Glare", || Value::Number(2.0), 0.0, 10.0),
        num("Haze", || Value::Number(1.0), 0.0, 10.0),
        num("HeightFalloff", || Value::Number(0.02), 0.0, 1.0),
        num("FogHeight", || Value::Number(0.0), -1e5, 1e5),
        color("Absorption", || Value::Color3(BLACK)),
        color("AirColor", || {
            Value::Color3(Color3::new(116.0, 172.0, 255.0))
        }),
    ]);
