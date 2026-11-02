//! Sky: the sun, moon and stars, a child of Lighting.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Sky", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Lighting")
    .no_insert()
    .describe("The sun, moon and stars; takes effect as a child of Lighting.")
    .props(&[
        num("SunAngularSize", || Value::Number(1.5), 0.0, 60.0)
            .editor(Editor::new().drag(NumberUnit::Degrees))
            .describe("How big the sun looks, in degrees."),
        num("MoonAngularSize", || Value::Number(1.5), 0.0, 60.0)
            .editor(Editor::new().drag(NumberUnit::Degrees))
            .describe("How big the moon looks, in degrees."),
        num("StarCount", || Value::Number(3000.0), 0.0, 10000.0)
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("How many stars are drawn."),
        PropDesc::new("CelestialBodiesShown", ValueType::Bool, || {
            Value::Bool(true)
        })
        .describe("Whether the sun, moon and stars are drawn."),
    ]);
