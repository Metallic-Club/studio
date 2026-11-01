//! Lighting service: Roblox names, Metallic defaults and meanings
//! (docs/research/2026-10-30-lighting.md section 5). Presets: `crate::lighting`.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Lighting", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("The sun, ambient light, exposure and fog of the world; holds the effect classes.")
    .props(&[
        PropDesc::new("ClockTime", ValueType::Number, || Value::Number(14.5)).wrapped(0.0, 24.0),
        num("GeographicLatitude", || Value::Number(41.73), -90.0, 90.0),
        num("Brightness", || Value::Number(1.0), 0.0, 10.0),
        num("ExposureCompensation", || Value::Number(0.0), -5.0, 5.0),
        color("Ambient", || Value::Color3(BLACK)),
        color("OutdoorAmbient", || Value::Color3(BLACK)),
        color("ColorShift_Top", || Value::Color3(BLACK)),
        color("ColorShift_Bottom", || Value::Color3(BLACK)),
        num("EnvironmentDiffuseScale", || Value::Number(1.0), 0.0, 1.0),
        num("EnvironmentSpecularScale", || Value::Number(1.0), 0.0, 1.0),
        PropDesc::new("GlobalShadows", ValueType::Bool, || Value::Bool(true)),
        num("ShadowSoftness", || Value::Number(0.2), 0.0, 1.0),
        choice(
            "LightingStyle",
            || Value::String("Realistic".into()),
            LIGHTING_STYLES,
        ),
        color("FogColor", || {
            Value::Color3(Color3::new(192.0, 192.0, 192.0))
        }),
        num("FogStart", || Value::Number(0.0), 0.0, 1e7),
        num("FogEnd", || Value::Number(100000.0), 0.0, 1e7),
        num("NightAdaptation", || Value::Number(1.0), 0.0, 1.0),
        num("MoonBrightness", || Value::Number(1.0), 0.0, 10.0),
        // Histogram metering (#151; lighting doc 3.9 and 5.5). Off: the fixed exposure from
        // the sun elevation. EV100 limits of the meter.
        PropDesc::new("AutoExposure", ValueType::Bool, || Value::Bool(true)),
        num("ExposureMin", || Value::Number(-4.0), -10.0, 20.0),
        num("ExposureMax", || Value::Number(16.0), -10.0, 20.0),
        // EV added only while metering, so a look can match with auto exposure on and off.
        num("AutoExposureBias", || Value::Number(0.0), -5.0, 5.0),
    ]);
