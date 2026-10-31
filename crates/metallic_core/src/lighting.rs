//! Lighting presets: twelve complete, named lighting states for the `Lighting` service and its
//! effect children (`Atmosphere`, `Sky`, `Clouds`, `BloomEffect`, `ColorCorrectionEffect`,
//! `ColorGradingEffect`).
//!
//! A preset is the class defaults plus a short list of overrides, so [`apply_preset`] gives the
//! same result whatever the place had before, and [`matching_preset`] can tell whether a place
//! still uses a preset unchanged (Studio offers to switch presets only then;
//! `docs/design/look-presets.md` 5.1). Presets set lighting only: no parts, lights or Look.
//! Each preset is described with its intent and values in `docs/lighting/presets.md`, with a
//! rendered preview in `docs/lighting/presets/`.
use crate::classes::all_properties;
use crate::value::{Color3, Value};
use crate::{CoreError, InstanceId, World};

/// Classes a preset manages under `Lighting`, one instance of each.
pub const LIGHTING_CHILD_CLASSES: [&str; 6] = [
    "Atmosphere",
    "Sky",
    "Clouds",
    "BloomEffect",
    "ColorCorrectionEffect",
    "ColorGradingEffect",
];

/// A property value in a preset table (`const`-friendly form of [`Value`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresetValue {
    /// A number.
    Number(f64),
    /// A Color3 in 0..255 per channel.
    Color(f64, f64, f64),
    /// A boolean.
    Bool(bool),
    /// One of the property's allowed strings.
    Text(&'static str),
}

impl PresetValue {
    /// The [`Value`] to store.
    pub fn to_value(self) -> Value {
        match self {
            PresetValue::Number(n) => Value::Number(n),
            PresetValue::Color(r, g, b) => Value::Color3(Color3::new(r, g, b)),
            PresetValue::Bool(b) => Value::Bool(b),
            PresetValue::Text(s) => Value::String(s.to_owned()),
        }
    }
}

/// One override: class (`Lighting` or one of [`LIGHTING_CHILD_CLASSES`]), property, value.
pub type Override = (&'static str, &'static str, PresetValue);

/// A named lighting state.
#[derive(Debug)]
pub struct LightingPreset {
    /// Stable identifier (`golden-hour`), for files and scripts.
    pub id: &'static str,
    /// Display name (`Golden Hour`).
    pub name: &'static str,
    /// One line on what the preset is for.
    pub summary: &'static str,
    /// Values that differ from the class defaults.
    pub overrides: &'static [Override],
}

use PresetValue::{Color, Number, Text};

/// The twelve built-in presets, in picker order.
pub static PRESETS: [LightingPreset; 12] = [
    LightingPreset {
        id: "clear-day",
        name: "Clear Day",
        summary: "Crisp mid-afternoon sun, blue sky, light haze. The default for new places.",
        overrides: &[],
    },
    LightingPreset {
        id: "golden-hour",
        name: "Golden Hour",
        summary: "Low warm sun an hour before sunset, long soft shadows with a violet fill.",
        overrides: &[
            ("Lighting", "ClockTime", Number(17.25)),
            ("Lighting", "Brightness", Number(1.15)),
            ("Lighting", "ExposureCompensation", Number(0.2)),
            ("Lighting", "OutdoorAmbient", Color(18.0, 14.0, 34.0)),
            ("Lighting", "ShadowSoftness", Number(0.3)),
            ("Atmosphere", "Density", Number(0.3)),
            ("Atmosphere", "Color", Color(235.0, 200.0, 160.0)),
            ("Atmosphere", "Decay", Color(110.0, 100.0, 130.0)),
            ("Atmosphere", "Glare", Number(4.0)),
            ("Atmosphere", "Haze", Number(1.5)),
            ("BloomEffect", "Intensity", Number(0.06)),
            ("ColorCorrectionEffect", "Saturation", Number(0.05)),
        ],
    },
    LightingPreset {
        id: "overcast",
        name: "Overcast",
        summary: "Even grey daylight under full cloud: no hard shadows, quiet colour.",
        overrides: &[
            ("Lighting", "ClockTime", Number(13.0)),
            ("Lighting", "ExposureCompensation", Number(0.2)),
            ("Lighting", "ShadowSoftness", Number(1.0)),
            ("Atmosphere", "Density", Number(0.32)),
            ("Atmosphere", "Color", Color(205.0, 208.0, 214.0)),
            ("Atmosphere", "Decay", Color(150.0, 156.0, 166.0)),
            ("Atmosphere", "Glare", Number(0.0)),
            ("Atmosphere", "Haze", Number(2.0)),
            ("Atmosphere", "HeightFalloff", Number(0.012)),
            ("Clouds", "Cover", Number(0.9)),
            ("ColorGradingEffect", "TonemapperPreset", Text("Neutral")),
            ("BloomEffect", "Intensity", Number(0.02)),
            ("ColorCorrectionEffect", "Contrast", Number(0.1)),
            ("ColorCorrectionEffect", "Saturation", Number(0.1)),
        ],
    },
    LightingPreset {
        id: "night-city",
        name: "Night City",
        summary: "Late evening under a violet city glow; fog lit from below, few stars.",
        overrides: &[
            ("Lighting", "ClockTime", Number(22.5)),
            ("Lighting", "Ambient", Color(62.0, 52.0, 82.0)),
            ("Lighting", "OutdoorAmbient", Color(12.0, 12.0, 22.0)),
            ("Lighting", "NightAdaptation", Number(0.5)),
            ("Lighting", "MoonBrightness", Number(0.5)),
            ("Atmosphere", "Density", Number(0.4)),
            ("Atmosphere", "Color", Color(118.0, 108.0, 132.0)),
            ("Atmosphere", "Decay", Color(58.0, 54.0, 78.0)),
            ("Atmosphere", "Glare", Number(1.0)),
            ("Atmosphere", "Haze", Number(3.0)),
            ("Atmosphere", "HeightFalloff", Number(0.01)),
            ("Sky", "StarCount", Number(300.0)),
            ("BloomEffect", "Intensity", Number(0.08)),
            ("ColorCorrectionEffect", "Saturation", Number(-0.1)),
        ],
    },
    LightingPreset {
        id: "moonlit",
        name: "Moonlit",
        summary: "Full moon on a clear night: cool silver light, crisp shadows, many stars.",
        overrides: &[
            ("Lighting", "ClockTime", Number(1.0)),
            ("Lighting", "ExposureCompensation", Number(0.6)),
            ("Lighting", "ShadowSoftness", Number(0.15)),
            ("Lighting", "MoonBrightness", Number(1.5)),
            ("Atmosphere", "Density", Number(0.15)),
            ("Atmosphere", "Glare", Number(1.0)),
            ("Atmosphere", "Haze", Number(0.5)),
            ("Sky", "StarCount", Number(5000.0)),
            ("BloomEffect", "Intensity", Number(0.05)),
        ],
    },
    LightingPreset {
        id: "horror-fog",
        name: "Horror Fog",
        summary: "Dusk swallowed by thick green-grey fog; drained colour, nothing far is safe.",
        overrides: &[
            ("Lighting", "ClockTime", Number(18.15)),
            ("Lighting", "Brightness", Number(0.4)),
            ("Lighting", "ExposureCompensation", Number(-0.3)),
            ("Lighting", "ShadowSoftness", Number(0.8)),
            ("Atmosphere", "Density", Number(0.9)),
            ("Atmosphere", "Offset", Number(0.0)),
            ("Atmosphere", "Color", Color(140.0, 172.0, 146.0)),
            ("Atmosphere", "Decay", Color(58.0, 84.0, 70.0)),
            ("Atmosphere", "Glare", Number(0.0)),
            ("Atmosphere", "Haze", Number(5.0)),
            ("Atmosphere", "HeightFalloff", Number(0.05)),
            ("Sky", "StarCount", Number(0.0)),
            ("ColorGradingEffect", "TonemapperPreset", Text("AgX")),
            ("BloomEffect", "Intensity", Number(0.03)),
            ("ColorCorrectionEffect", "Contrast", Number(0.15)),
            ("ColorCorrectionEffect", "Saturation", Number(-0.45)),
            (
                "ColorCorrectionEffect",
                "TintColor",
                Color(204.0, 236.0, 214.0),
            ),
        ],
    },
    LightingPreset {
        id: "sci-fi-interior",
        name: "Sci-fi Interior",
        summary: "Clean, cool halls: soft overhead light, steel-blue fill, no haze.",
        overrides: &[
            ("Lighting", "ClockTime", Number(12.0)),
            ("Lighting", "Brightness", Number(0.55)),
            ("Lighting", "ExposureCompensation", Number(0.2)),
            ("Lighting", "Ambient", Color(64.0, 78.0, 104.0)),
            ("Lighting", "EnvironmentDiffuseScale", Number(0.5)),
            ("Lighting", "ShadowSoftness", Number(0.35)),
            ("Atmosphere", "Density", Number(0.0)),
            ("ColorGradingEffect", "TonemapperPreset", Text("Neutral")),
            ("BloomEffect", "Intensity", Number(0.07)),
            ("ColorCorrectionEffect", "Contrast", Number(0.08)),
            ("ColorCorrectionEffect", "Saturation", Number(-0.12)),
            (
                "ColorCorrectionEffect",
                "TintColor",
                Color(222.0, 236.0, 255.0),
            ),
        ],
    },
    LightingPreset {
        id: "cartoon-bright",
        name: "Cartoon Bright",
        summary: "Toy-box noon: saturated colour, soft wrap light, lifted shadows, no grime.",
        overrides: &[
            ("Lighting", "ClockTime", Number(13.0)),
            ("Lighting", "Brightness", Number(1.2)),
            ("Lighting", "ExposureCompensation", Number(-0.3)),
            ("Lighting", "OutdoorAmbient", Color(40.0, 44.0, 64.0)),
            ("Lighting", "ColorShift_Top", Color(20.0, 12.0, 0.0)),
            ("Lighting", "EnvironmentDiffuseScale", Number(0.7)),
            ("Lighting", "ShadowSoftness", Number(0.05)),
            ("Lighting", "LightingStyle", Text("Soft")),
            ("Atmosphere", "Density", Number(0.12)),
            ("Atmosphere", "Offset", Number(0.5)),
            ("Atmosphere", "Glare", Number(1.0)),
            ("Atmosphere", "Haze", Number(0.0)),
            ("Atmosphere", "AirColor", Color(90.0, 170.0, 255.0)),
            ("ColorGradingEffect", "TonemapperPreset", Text("Neutral")),
            ("BloomEffect", "Intensity", Number(0.02)),
            ("ColorCorrectionEffect", "Saturation", Number(0.2)),
        ],
    },
    LightingPreset {
        id: "pixel-dusk",
        name: "Pixel Dusk",
        summary: "Peach light, violet shadows, hard edges and a big sun, for pixel styles.",
        overrides: &[
            ("Lighting", "ClockTime", Number(17.55)),
            ("Lighting", "OutdoorAmbient", Color(40.0, 22.0, 72.0)),
            ("Lighting", "ShadowSoftness", Number(0.0)),
            ("Atmosphere", "Color", Color(255.0, 176.0, 130.0)),
            ("Atmosphere", "Decay", Color(110.0, 80.0, 160.0)),
            ("Atmosphere", "Glare", Number(3.0)),
            ("Atmosphere", "Haze", Number(1.5)),
            ("Sky", "SunAngularSize", Number(4.0)),
            ("ColorGradingEffect", "TonemapperPreset", Text("Neutral")),
            ("BloomEffect", "Intensity", Number(0.0)),
            ("ColorCorrectionEffect", "Saturation", Number(0.1)),
        ],
    },
    LightingPreset {
        id: "underwater",
        name: "Underwater",
        summary: "Teal water that eats red light and swallows distance; soft sun from above.",
        overrides: &[
            ("Lighting", "ClockTime", Number(13.0)),
            ("Lighting", "Brightness", Number(0.6)),
            ("Lighting", "ExposureCompensation", Number(0.3)),
            ("Lighting", "ShadowSoftness", Number(0.6)),
            ("Atmosphere", "Density", Number(0.9)),
            ("Atmosphere", "Offset", Number(0.0)),
            ("Atmosphere", "Color", Color(50.0, 180.0, 175.0)),
            ("Atmosphere", "Decay", Color(12.0, 78.0, 92.0)),
            ("Atmosphere", "Glare", Number(0.0)),
            ("Atmosphere", "Haze", Number(0.0)),
            ("Atmosphere", "HeightFalloff", Number(0.0)),
            ("Atmosphere", "Absorption", Color(230.0, 90.0, 40.0)),
            ("Sky", "StarCount", Number(0.0)),
            ("ColorGradingEffect", "TonemapperPreset", Text("AgX")),
            ("BloomEffect", "Intensity", Number(0.06)),
            ("ColorCorrectionEffect", "Saturation", Number(0.05)),
        ],
    },
    LightingPreset {
        id: "desert-noon",
        name: "Desert Noon",
        summary: "Hard high sun at low latitude: short black shadows, bleached warm haze.",
        overrides: &[
            ("Lighting", "ClockTime", Number(12.3)),
            ("Lighting", "GeographicLatitude", Number(25.0)),
            ("Lighting", "Brightness", Number(1.5)),
            ("Lighting", "ExposureCompensation", Number(-0.4)),
            ("Lighting", "ShadowSoftness", Number(0.1)),
            ("Atmosphere", "Color", Color(240.0, 214.0, 176.0)),
            ("Atmosphere", "Decay", Color(170.0, 152.0, 132.0)),
            ("Atmosphere", "Glare", Number(3.0)),
            ("Atmosphere", "Haze", Number(2.5)),
            ("ColorCorrectionEffect", "Contrast", Number(0.08)),
            ("ColorCorrectionEffect", "Saturation", Number(0.05)),
            (
                "ColorCorrectionEffect",
                "TintColor",
                Color(255.0, 246.0, 232.0),
            ),
        ],
    },
    LightingPreset {
        id: "alien-dusk",
        name: "Alien Dusk",
        summary: "Sunset on another world: amber sky, teal shade, a cold blue glow toward the sun.",
        overrides: &[
            ("Lighting", "ClockTime", Number(17.0)),
            ("Lighting", "OutdoorAmbient", Color(40.0, 120.0, 130.0)),
            ("Lighting", "EnvironmentDiffuseScale", Number(0.35)),
            ("Lighting", "ColorShift_Top", Color(0.0, 120.0, 255.0)),
            ("Atmosphere", "Density", Number(0.22)),
            ("Atmosphere", "Color", Color(120.0, 190.0, 255.0)),
            ("Atmosphere", "Decay", Color(190.0, 120.0, 80.0)),
            ("Atmosphere", "Glare", Number(6.0)),
            ("Atmosphere", "Haze", Number(3.0)),
            ("Atmosphere", "AirColor", Color(255.0, 170.0, 110.0)),
            ("BloomEffect", "Intensity", Number(0.06)),
        ],
    },
];

/// Finds a preset by id (`golden-hour`) or display name (`Golden Hour`).
pub fn find_preset(id_or_name: &str) -> Option<&'static LightingPreset> {
    PRESETS
        .iter()
        .find(|p| p.id == id_or_name || p.name == id_or_name)
}

/// The first child of `Lighting` with class `class`, if any.
pub fn lighting_child(world: &World, class: &str) -> Option<InstanceId> {
    let lighting = world.root("Lighting")?;
    world
        .children(lighting)
        .iter()
        .copied()
        .find(|&c| world.class_name(c) == Some(class))
}

impl LightingPreset {
    /// The value this preset gives `class.property`: its override, else the class default.
    pub fn value(&self, class: &str, property: &str) -> Option<Value> {
        if let Some((_, _, v)) = self
            .overrides
            .iter()
            .find(|(c, p, _)| *c == class && *p == property)
        {
            return Some(v.to_value());
        }
        crate::classes::default_value(class, property)
    }
}

fn managed_properties(class: &str) -> impl Iterator<Item = &'static str> {
    all_properties(class)
        .into_iter()
        .map(|d| d.name)
        .filter(|n| *n != "Name")
}

/// Sets every `Lighting` property and every property of the effect children to `preset`'s
/// values, creating missing children (named after their class). Other children of `Lighting`
/// are left alone.
pub fn apply_preset(world: &mut World, preset: &LightingPreset) -> Result<(), CoreError> {
    let lighting = world.root("Lighting").ok_or(CoreError::InvalidInstance)?;
    for prop in managed_properties("Lighting") {
        let v = preset
            .value("Lighting", prop)
            .ok_or(CoreError::InvalidInstance)?;
        world.set(lighting, prop, v)?;
    }
    for class in LIGHTING_CHILD_CLASSES {
        let id = match lighting_child(world, class) {
            Some(id) => id,
            None => world.create(class, Some(lighting))?,
        };
        for prop in managed_properties(class) {
            let v = preset
                .value(class, prop)
                .ok_or(CoreError::InvalidInstance)?;
            world.set(id, prop, v)?;
        }
    }
    Ok(())
}

/// The preset the place's lighting equals exactly, if any. A missing effect child counts as
/// one at its defaults.
pub fn matching_preset(world: &World) -> Option<&'static LightingPreset> {
    let lighting = world.root("Lighting")?;
    PRESETS.iter().find(|preset| {
        let same = |id: Option<InstanceId>, class: &str| {
            managed_properties(class).all(|prop| {
                let current = match id {
                    Some(id) => world.get(id, prop).ok(),
                    None => crate::classes::default_value(class, prop),
                };
                current == preset.value(class, prop)
            })
        };
        same(Some(lighting), "Lighting")
            && LIGHTING_CHILD_CLASSES
                .iter()
                .all(|class| same(lighting_child(world, class), class))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classes::find_property;

    #[test]
    fn every_override_names_a_real_property_with_the_right_type() {
        for preset in &PRESETS {
            for (class, prop, v) in preset.overrides {
                assert!(
                    *class == "Lighting" || LIGHTING_CHILD_CLASSES.contains(class),
                    "{}: {class} is not managed",
                    preset.id
                );
                let def = find_property(class, prop)
                    .unwrap_or_else(|| panic!("{}: {class}.{prop} missing", preset.id));
                let value = v.to_value();
                assert_eq!(value.value_type(), def.ty, "{}: {class}.{prop}", preset.id);
                if let (Some(allowed), Value::String(s)) = (def.allowed, &value) {
                    assert!(allowed.contains(&s.as_str()), "{}: {s}", preset.id);
                }
                if let Some(range) = def.range {
                    assert_eq!(
                        range.apply(value.clone()),
                        value,
                        "{}: {class}.{prop} out of range",
                        preset.id
                    );
                }
            }
        }
    }

    #[test]
    fn ids_and_names_are_unique() {
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.name, b.name);
            }
            assert_eq!(find_preset(a.id).map(|p| p.id), Some(a.id));
            assert_eq!(find_preset(a.name).map(|p| p.id), Some(a.id));
        }
    }

    #[test]
    fn presets_differ_from_each_other() {
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                let mut w = World::new_with_services();
                apply_preset(&mut w, a).expect("apply");
                assert_ne!(matching_preset(&w).map(|p| p.id), Some(b.id));
            }
        }
    }
}
