//! Atmosphere: air and haze, a child of Lighting.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Atmosphere", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("Air and haze; takes effect as a child of Lighting.")
    .props(&[
        num("Density", || Value::Number(0.3), 0.0, 1.0)
            .describe("How thick the fog is near the ground."),
        num("Offset", || Value::Number(0.25), 0.0, 1.0)
            .describe("Higher lets more show through the fog, however far away it is."),
        color("Color", || Value::Color3(Color3::new(199.0, 199.0, 199.0)))
            .editor(Editor::section(Section::APPEARANCE))
            .describe("The colour of the fog."),
        color("Decay", || Value::Color3(Color3::new(106.0, 112.0, 125.0)))
            .describe("The tint of the fog away from the sun."),
        num("Glare", || Value::Number(2.0), 0.0, 10.0)
            .describe("How tightly the fog brightens around the sun."),
        num("Haze", || Value::Number(1.0), 0.0, 10.0)
            .describe("How much the air scatters sunlight into a pale haze."),
        num("HeightFalloff", || Value::Number(0.02), 0.0, 1.0)
            .describe("How quickly the fog thins with height."),
        num("FogHeight", || Value::Number(0.0), -1e5, 1e5)
            .describe("The height of the bottom of the fog, in studs."),
        color("Absorption", || Value::Color3(BLACK))
            .describe("How much light the air swallows, per colour: for water or smoke."),
        color("AirColor", || {
            Value::Color3(Color3::new(116.0, 172.0, 255.0))
        })
        .describe("The tint of the sky's air, for skies unlike Earth's."),
    ]);
