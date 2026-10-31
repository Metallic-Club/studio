//! Lighting from the world: reads the `Lighting` service and its effect children
//! (`metallic_core::lighting`) and turns them into the sun, sky, ambient, fog, exposure, finish
//! and bloom settings of a frame.
//!
//! The sun follows `ClockTime` and `GeographicLatitude` on an equinox day (sunrise 06:00).
//! Sky colours, sun colour and a metered exposure come from a per-elevation table generated
//! by the physical-sky reference (`docs/agents/37-lighting/m1_sky_table.md`); the GPU sky LUTs
//! replace it later. All light is pre-exposed on the CPU, so the HDR target holds display-scale
//! values even at night. Property meanings: `docs/research/2026-10-30-lighting.md` section 5.
use crate::sky::Sky;
use crate::tonemap::{FinishSettings, ToneCurve};
use glam::Vec3;
use metallic_core::classes::{default_value, find_property};
use metallic_core::lighting::lighting_child;
use metallic_core::{InstanceId, Value, World};

/// The lighting properties of a place, as plain numbers. Colours are 0..1 sRGB-encoded (the
/// Color3 value divided by 255). [`LightingInputs::default`] is a place with a `Lighting`
/// service at its defaults and no effect children.
#[derive(Debug, Clone, PartialEq)]
pub struct LightingInputs {
    /// `Lighting.ClockTime`, hours.
    pub clock_time: f32,
    /// `Lighting.GeographicLatitude`, degrees.
    pub latitude: f32,
    /// `Lighting.Brightness`: sun strength.
    pub brightness: f32,
    /// `Lighting.ExposureCompensation`, EV.
    pub exposure_compensation: f32,
    /// `Lighting.Ambient`.
    pub ambient: [f32; 3],
    /// `Lighting.OutdoorAmbient`.
    pub outdoor_ambient: [f32; 3],
    /// `Lighting.ColorShift_Top`: sun tint.
    pub color_shift_top: [f32; 3],
    /// `Lighting.ColorShift_Bottom`: ground bounce tint.
    pub color_shift_bottom: [f32; 3],
    /// `Lighting.EnvironmentDiffuseScale`.
    pub environment_diffuse: f32,
    /// `Lighting.EnvironmentSpecularScale`.
    pub environment_specular: f32,
    /// `Lighting.GlobalShadows`.
    pub global_shadows: bool,
    /// `Lighting.ShadowSoftness`, 0..1.
    pub shadow_softness: f32,
    /// `Lighting.LightingStyle` is `Soft`.
    pub soft_style: bool,
    /// `Lighting.FogColor`.
    pub fog_color: [f32; 3],
    /// `Lighting.FogStart`, studs.
    pub fog_start: f32,
    /// `Lighting.FogEnd`, studs; 100000 or more turns the distance fog off.
    pub fog_end: f32,
    /// `Lighting.NightAdaptation`, 0..1.
    pub night_adaptation: f32,
    /// `Lighting.MoonBrightness`.
    pub moon_brightness: f32,
    /// The `Atmosphere` child. A place without one reads its defaults, so a fresh place and
    /// the Clear Day preset look the same; `None` (set from code) turns the atmosphere off.
    pub atmosphere: Option<AtmosphereInputs>,
    /// The `Sky` child (defaults when absent).
    pub sky: SkyInputs,
    /// `Clouds.Cover` (0 when absent or disabled).
    pub cloud_cover: f32,
    /// `Clouds.Color`.
    pub cloud_color: [f32; 3],
    /// `BloomEffect` (`None` when disabled; the Realistic Look's 0.04 / 24 / 0 when absent).
    pub bloom: Option<BloomInputs>,
    /// `ColorCorrectionEffect` (`None` when absent or disabled).
    pub color_correction: Option<ColorCorrectionInputs>,
    /// `ColorGradingEffect.TonemapperPreset` (`Default` when absent or disabled).
    pub tonemapper: String,
}

/// `Atmosphere` properties.
#[derive(Debug, Clone, PartialEq)]
pub struct AtmosphereInputs {
    /// Fog density at `FogHeight`, 0..1.
    pub density: f32,
    /// 0..1; lowers the fog's maximum opacity.
    pub offset: f32,
    /// Fog tint toward the sun.
    pub color: [f32; 3],
    /// Fog tint away from the sun.
    pub decay: [f32; 3],
    /// Sun glow in the fog, 0..10.
    pub glare: f32,
    /// Haziness, 0..10.
    pub haze: f32,
    /// Per stud.
    pub height_falloff: f32,
    /// Studs.
    pub fog_height: f32,
    /// Extra extinction per channel (water).
    pub absorption: [f32; 3],
    /// Sky tint relative to Earth air.
    pub air_color: [f32; 3],
}

/// `Sky` properties.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyInputs {
    /// Degrees.
    pub sun_angular_size: f32,
    /// Degrees.
    pub moon_angular_size: f32,
    /// Stars on the whole sky.
    pub star_count: f32,
    /// Sun disc, moon and stars.
    pub celestial_bodies_shown: bool,
}

/// `BloomEffect` properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomInputs {
    /// Blend toward the blurred image, 0..1.
    pub intensity: f32,
    /// Blur radius in pixels at 1080p (picks the mip count).
    pub size: f32,
    /// Display-referred luminance below which nothing blooms.
    pub threshold: f32,
}

/// `ColorCorrectionEffect` properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionInputs {
    /// Added after the curve.
    pub brightness: f32,
    /// Around scene grey.
    pub contrast: f32,
    /// -1 grey .. 1 double.
    pub saturation: f32,
    /// Multiplier, sRGB-encoded 0..1.
    pub tint: [f32; 3],
}

const DEFAULT_BLOOM: BloomInputs = BloomInputs {
    intensity: 0.04,
    size: 24.0,
    threshold: 0.0,
};

impl Default for LightingInputs {
    fn default() -> Self {
        LightingInputs::read(&World::new_with_services())
    }
}

struct Reader<'a> {
    world: &'a World,
    id: Option<InstanceId>,
    class: &'static str,
}

impl Reader<'_> {
    fn value(&self, prop: &str) -> Option<Value> {
        match self.id {
            Some(id) => self.world.get_ref(id, prop).ok().cloned(),
            None => metallic_core::classes::default_value(self.class, prop),
        }
    }
    fn num(&self, prop: &str) -> f32 {
        match self.value(prop) {
            Some(Value::Number(n)) => n as f32,
            _ => 0.0,
        }
    }
    fn color(&self, prop: &str) -> [f32; 3] {
        match self.value(prop) {
            Some(Value::Color3(c)) => [c.r as f32, c.g as f32, c.b as f32].map(|v| v / 255.0),
            _ => [0.0; 3],
        }
    }
    fn flag(&self, prop: &str) -> bool {
        matches!(self.value(prop), Some(Value::Bool(true)))
    }
    fn text(&self, prop: &str) -> String {
        match self.value(prop) {
            Some(Value::String(s)) => s,
            _ => String::new(),
        }
    }
}

/// `x` inside the registry range of `class.prop` (wrapped for `ClockTime`); the default when
/// not finite.
fn sane(class: &str, prop: &str, x: f32) -> f32 {
    let default = match default_value(class, prop) {
        Some(Value::Number(n)) => n as f32,
        _ => 0.0,
    };
    if !x.is_finite() {
        return default;
    }
    match find_property(class, prop).and_then(|d| d.range) {
        Some(r) => r.apply_number(f64::from(x)) as f32,
        None => x,
    }
}

/// A 0..1 colour of `class.prop`: components clamped to 0..1, the default where not finite.
fn sane_colour(class: &str, prop: &str, c: [f32; 3]) -> [f32; 3] {
    let default = match default_value(class, prop) {
        Some(Value::Color3(d)) => [d.r, d.g, d.b].map(|v| v as f32 / 255.0),
        _ => [0.0; 3],
    };
    std::array::from_fn(|i| {
        if c[i].is_finite() {
            c[i].clamp(0.0, 1.0)
        } else {
            default[i]
        }
    })
}

impl LightingInputs {
    /// The inputs with every number inside its property's range and finite. [`evaluate`] uses
    /// this, so values set from code (not through `World::set`) cannot put NaN or infinity into
    /// the shaders.
    pub fn sanitized(&self) -> LightingInputs {
        let l = |p: &str, x: f32| sane("Lighting", p, x);
        let lc = |p: &str, c: [f32; 3]| sane_colour("Lighting", p, c);
        LightingInputs {
            clock_time: l("ClockTime", self.clock_time),
            latitude: l("GeographicLatitude", self.latitude),
            brightness: l("Brightness", self.brightness),
            exposure_compensation: l("ExposureCompensation", self.exposure_compensation),
            ambient: lc("Ambient", self.ambient),
            outdoor_ambient: lc("OutdoorAmbient", self.outdoor_ambient),
            color_shift_top: lc("ColorShift_Top", self.color_shift_top),
            color_shift_bottom: lc("ColorShift_Bottom", self.color_shift_bottom),
            environment_diffuse: l("EnvironmentDiffuseScale", self.environment_diffuse),
            environment_specular: l("EnvironmentSpecularScale", self.environment_specular),
            global_shadows: self.global_shadows,
            shadow_softness: l("ShadowSoftness", self.shadow_softness),
            soft_style: self.soft_style,
            fog_color: lc("FogColor", self.fog_color),
            fog_start: l("FogStart", self.fog_start),
            fog_end: l("FogEnd", self.fog_end),
            night_adaptation: l("NightAdaptation", self.night_adaptation),
            moon_brightness: l("MoonBrightness", self.moon_brightness),
            atmosphere: self.atmosphere.as_ref().map(|a| {
                let n = |p: &str, x: f32| sane("Atmosphere", p, x);
                let c = |p: &str, x: [f32; 3]| sane_colour("Atmosphere", p, x);
                AtmosphereInputs {
                    density: n("Density", a.density),
                    offset: n("Offset", a.offset),
                    color: c("Color", a.color),
                    decay: c("Decay", a.decay),
                    glare: n("Glare", a.glare),
                    haze: n("Haze", a.haze),
                    height_falloff: n("HeightFalloff", a.height_falloff),
                    fog_height: n("FogHeight", a.fog_height),
                    absorption: c("Absorption", a.absorption),
                    air_color: c("AirColor", a.air_color),
                }
            }),
            sky: SkyInputs {
                sun_angular_size: sane("Sky", "SunAngularSize", self.sky.sun_angular_size),
                moon_angular_size: sane("Sky", "MoonAngularSize", self.sky.moon_angular_size),
                star_count: sane("Sky", "StarCount", self.sky.star_count),
                celestial_bodies_shown: self.sky.celestial_bodies_shown,
            },
            cloud_cover: sane("Clouds", "Cover", self.cloud_cover),
            cloud_color: sane_colour("Clouds", "Color", self.cloud_color),
            bloom: self.bloom.map(|b| BloomInputs {
                intensity: sane("BloomEffect", "Intensity", b.intensity),
                size: sane("BloomEffect", "Size", b.size),
                threshold: sane("BloomEffect", "Threshold", b.threshold),
            }),
            color_correction: self.color_correction.map(|c| {
                let n = |p: &str, x: f32| sane("ColorCorrectionEffect", p, x);
                ColorCorrectionInputs {
                    brightness: n("Brightness", c.brightness),
                    contrast: n("Contrast", c.contrast),
                    saturation: n("Saturation", c.saturation),
                    tint: sane_colour("ColorCorrectionEffect", "TintColor", c.tint),
                }
            }),
            tonemapper: self.tonemapper.clone(),
        }
    }

    /// Reads the place's lighting. A world without a `Lighting` service reads as defaults.
    pub fn read(world: &World) -> LightingInputs {
        let child = |class: &'static str| Reader {
            world,
            id: lighting_child(world, class),
            class,
        };
        let l = Reader {
            world,
            id: world.root("Lighting"),
            class: "Lighting",
        };
        let atmo = child("Atmosphere");
        let sky = child("Sky");
        let clouds = child("Clouds");
        let bloom = child("BloomEffect");
        let cc = child("ColorCorrectionEffect");
        let grading = child("ColorGradingEffect");
        let enabled = |r: &Reader<'_>| r.id.is_some() && r.flag("Enabled");
        LightingInputs {
            clock_time: l.num("ClockTime"),
            latitude: l.num("GeographicLatitude"),
            brightness: l.num("Brightness"),
            exposure_compensation: l.num("ExposureCompensation"),
            ambient: l.color("Ambient"),
            outdoor_ambient: l.color("OutdoorAmbient"),
            color_shift_top: l.color("ColorShift_Top"),
            color_shift_bottom: l.color("ColorShift_Bottom"),
            environment_diffuse: l.num("EnvironmentDiffuseScale"),
            environment_specular: l.num("EnvironmentSpecularScale"),
            global_shadows: l.flag("GlobalShadows"),
            shadow_softness: l.num("ShadowSoftness"),
            soft_style: l.text("LightingStyle") == "Soft",
            fog_color: l.color("FogColor"),
            fog_start: l.num("FogStart"),
            fog_end: l.num("FogEnd"),
            night_adaptation: l.num("NightAdaptation"),
            moon_brightness: l.num("MoonBrightness"),
            atmosphere: Some(AtmosphereInputs {
                density: atmo.num("Density"),
                offset: atmo.num("Offset"),
                color: atmo.color("Color"),
                decay: atmo.color("Decay"),
                glare: atmo.num("Glare"),
                haze: atmo.num("Haze"),
                height_falloff: atmo.num("HeightFalloff"),
                fog_height: atmo.num("FogHeight"),
                absorption: atmo.color("Absorption"),
                air_color: atmo.color("AirColor"),
            }),
            sky: SkyInputs {
                sun_angular_size: sky.num("SunAngularSize"),
                moon_angular_size: sky.num("MoonAngularSize"),
                star_count: sky.num("StarCount"),
                celestial_bodies_shown: sky.flag("CelestialBodiesShown"),
            },
            cloud_cover: if enabled(&clouds) {
                clouds.num("Cover")
            } else {
                0.0
            },
            cloud_color: clouds.color("Color"),
            bloom: match bloom.id {
                None => Some(DEFAULT_BLOOM),
                Some(_) if !bloom.flag("Enabled") => None,
                Some(_) => Some(BloomInputs {
                    intensity: bloom.num("Intensity"),
                    size: bloom.num("Size"),
                    threshold: bloom.num("Threshold"),
                }),
            },
            color_correction: enabled(&cc).then(|| ColorCorrectionInputs {
                brightness: cc.num("Brightness"),
                contrast: cc.num("Contrast"),
                saturation: cc.num("Saturation"),
                tint: cc.color("TintColor"),
            }),
            tonemapper: if enabled(&grading) {
                grading.text("TonemapperPreset")
            } else {
                "Default".to_owned()
            },
        }
    }
}

/// One row of the physical-sky table: values per sun elevation, in renderer units before
/// exposure (45 degree sun = the old fixed `sun_color`). Below the horizon `sun` is the full
/// moon's light (MoonBrightness 1).
struct SkyRow {
    elevation: f32,
    sun: [f32; 3],
    zenith: [f32; 3],
    horizon: [f32; 3],
    ground: [f32; 3],
    exposure: f32,
}

const fn row(
    elevation: f32,
    sun: [f32; 3],
    zenith: [f32; 3],
    horizon: [f32; 3],
    ground: [f32; 3],
    exposure: f32,
) -> SkyRow {
    SkyRow {
        elevation,
        sun,
        zenith,
        horizon,
        ground,
        exposure,
    }
}

/// `docs/agents/37-lighting/m1_sky_table.md`, ascending elevation.
const SKY_TABLE: [SkyRow; 17] = [
    row(
        -18.0,
        [6.82e-6, 5.28e-6, 3.48e-6],
        [3.44e-8, 6.81e-8, 1.52e-7],
        [5.08e-7, 6.03e-7, 5.98e-7],
        [1.52e-7, 1.32e-7, 1.21e-7],
        1.7e5,
    ),
    row(
        -12.0,
        [6.2e-6, 4.26e-6, 2.31e-6],
        [8.17e-8, 1.1e-7, 3.49e-7],
        [6.52e-6, 2.63e-6, 1.52e-6],
        [1.27e-7, 1.02e-7, 1.42e-7],
        3.5e4,
    ),
    row(
        -9.0,
        [5.65e-6, 3.48e-6, 1.55e-6],
        [8.79e-7, 9.91e-7, 4.3e-6],
        [9.94e-5, 3.47e-5, 2.08e-5],
        [7.34e-7, 5.45e-7, 1.5e-6],
        3.16e3,
    ),
    row(
        -6.0,
        [4.77e-6, 2.41e-6, 7.45e-7],
        [3.44e-5, 2.43e-5, 7.16e-5],
        [1.07e-3, 3.6e-4, 2.06e-4],
        [1.82e-5, 1.12e-5, 2.5e-5],
        575.0,
    ),
    row(
        -4.0,
        [3.84e-6, 1.53e-6, 2.91e-7],
        [4.43e-4, 3.41e-4, 8.76e-4],
        [4.82e-3, 2.03e-3, 1.62e-3],
        [1.85e-4, 1.27e-4, 2.71e-4],
        161.0,
    ),
    row(
        -2.0,
        [2.45e-6, 6.2e-7, 4.28e-8],
        [1.98e-3, 1.5e-3, 2.95e-3],
        [1.64e-2, 6.46e-3, 3.86e-3],
        [8.48e-4, 5.7e-4, 9.2e-4],
        54.0,
    ),
    row(
        0.0,
        [0.265, 0.0226, 0.000108],
        [5.09e-3, 4.96e-3, 8.77e-3],
        [7.81e-2, 2.79e-2, 1.26e-2],
        [2.4e-3, 1.95e-3, 2.77e-3],
        12.3,
    ),
    row(
        1.0,
        [0.736, 0.124, 0.0032],
        [6.57e-3, 7.22e-3, 1.24e-2],
        [0.121, 0.0504, 0.0191],
        [4.04e-3, 3.06e-3, 3.98e-3],
        7.34,
    ),
    row(
        3.0,
        [1.54, 0.51, 0.0649],
        [8.79e-3, 1.19e-2, 2.1e-2],
        [0.186, 0.108, 0.0449],
        [9.63e-3, 6.71e-3, 7.17e-3],
        3.84,
    ),
    row(
        6.0,
        [2.29, 1.16, 0.357],
        [0.011, 0.018, 0.0347],
        [0.237, 0.184, 0.107],
        [0.021, 0.0154, 0.0142],
        2.36,
    ),
    row(
        10.0,
        [2.81, 1.81, 0.87],
        [0.0131, 0.024, 0.0501],
        [0.254, 0.244, 0.186],
        [0.038, 0.0304, 0.027],
        1.78,
    ),
    row(
        15.0,
        [3.15, 2.32, 1.42],
        [0.0152, 0.0296, 0.0651],
        [0.247, 0.278, 0.256],
        [0.0598, 0.0509, 0.0459],
        1.52,
    ),
    row(
        20.0,
        [3.34, 2.65, 1.82],
        [0.0172, 0.0345, 0.0776],
        [0.237, 0.295, 0.304],
        [0.0813, 0.0719, 0.0661],
        1.37,
    ),
    row(
        30.0,
        [3.54, 3.02, 2.33],
        [0.0212, 0.0435, 0.0994],
        [0.229, 0.316, 0.366],
        [0.122, 0.113, 0.107],
        1.17,
    ),
    row(
        45.0,
        [3.67, 3.28, 2.73],
        [0.0284, 0.0572, 0.129],
        [0.23, 0.336, 0.418],
        [0.176, 0.167, 0.161],
        1.0,
    ),
    row(
        60.0,
        [3.74, 3.41, 2.93],
        [0.0396, 0.0736, 0.158],
        [0.232, 0.346, 0.446],
        [0.218, 0.209, 0.204],
        0.904,
    ),
    row(
        90.0,
        [3.77, 3.47, 3.04],
        [0.118, 0.151, 0.239],
        [0.233, 0.35, 0.459],
        [0.252, 0.244, 0.239],
        0.889,
    ),
];

/// The table at `elevation` degrees. Colours are interpolated in log space (they span ten
/// orders of magnitude through twilight); so is the exposure.
fn sky_at(elevation: f32) -> SkyRow {
    let e = elevation.clamp(SKY_TABLE[0].elevation, SKY_TABLE[16].elevation);
    let i = SKY_TABLE
        .windows(2)
        .position(|w| e <= w[1].elevation)
        .unwrap_or(15);
    let (a, b) = (&SKY_TABLE[i], &SKY_TABLE[i + 1]);
    let t = (e - a.elevation) / (b.elevation - a.elevation);
    let lerp_log = |x: f32, y: f32| (x.max(1e-12).ln() * (1.0 - t) + y.max(1e-12).ln() * t).exp();
    let c = |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|k| lerp_log(x[k], y[k]));
    SkyRow {
        elevation: e,
        sun: c(a.sun, b.sun),
        zenith: c(a.zenith, b.zenith),
        horizon: c(a.horizon, b.horizon),
        ground: c(a.ground, b.ground),
        exposure: lerp_log(a.exposure, b.exposure),
    }
}

/// Unit vector toward the sun at `clock_time` hours and `latitude` degrees on an equinox day.
/// +X is east, +Y up, +Z south (so the midday sun of the northern hemisphere is at +Z).
pub fn sun_direction(clock_time: f32, latitude: f32) -> Vec3 {
    let h = ((clock_time - 12.0) * 15.0).to_radians();
    let phi = latitude.clamp(-90.0, 90.0).to_radians();
    Vec3::new(-h.sin(), h.cos() * phi.cos(), h.cos() * phi.sin()).normalize_or(Vec3::Y)
}

fn srgb_to_linear(c: f32) -> f32 {
    crate::scene::srgb_to_linear(c.clamp(0.0, 1.0))
}

fn lin3(c: [f32; 3]) -> [f32; 3] {
    c.map(srgb_to_linear)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn scale(c: [f32; 3], s: f32) -> [f32; 3] {
    c.map(|v| v * s)
}

fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// `c` divided by `reference` per channel, with the result's luminance normalised to 1, so a
/// tint changes hue, not brightness.
fn relative_tint(c: [f32; 3], reference: [f32; 3]) -> [f32; 3] {
    let r: [f32; 3] = std::array::from_fn(|i| lin3(c)[i] / lin3(reference)[i].max(1e-4));
    let l = luma(r).max(1e-4);
    r.map(|v| v / l)
}

/// The scene value that the finish pass shows as display colour `c` (sRGB 0..1), per channel.
fn inverse_curve(curve: ToneCurve, c: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|ch| {
        let target = srgb_to_linear(c[ch]).min(0.98);
        let (mut lo, mut hi) = (0.0f32, 64.0f32);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if curve.map([mid; 3])[1] < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    })
}

/// Scale from the table's metered exposure to the image: -0.6 EV, chosen on the reference
/// scene so a sunlit mid-grey baseplate sits near sRGB 0.65 instead of clipping toward white
/// under AgX Punchy.
const EXPOSURE_KEY: f32 = 0.66;

/// Everything a frame needs from the lighting, evaluated on the CPU.
#[derive(Debug, Clone, PartialEq)]
pub struct LightingFrame {
    /// Sun or moon (whichever lights the scene), sky gradient and colours, pre-exposed.
    pub sky: Sky,
    /// Finish pass settings: curve and grade, including the night tint.
    pub finish: FinishSettings,
    /// Bloom, or `None` for off.
    pub bloom: Option<BloomInputs>,
    pub(crate) uniform: LightingUniform,
}

/// Lighting block of the frame uniforms; mirrors the `light_*`, `fog_*` and `celestial`
/// fields at the end of `Globals` in `common.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightingUniform {
    /// rgb: `Ambient` (pre-exposed radiance on white), w: `EnvironmentDiffuseScale`.
    pub light_ambient: [f32; 4],
    /// rgb: `OutdoorAmbient`, w: `EnvironmentSpecularScale`.
    pub light_outdoor: [f32; 4],
    /// rgb: ground bounce multiplier (1 + `ColorShift_Bottom`), w: shadow filter radius in texels.
    pub light_bounce: [f32; 4],
    /// x: 1 when shadows are on, y: diffuse wrap (0 Realistic, 0.3 Soft), z: star density,
    /// w: star brightness.
    pub light_flags: [f32; 4],
    /// x: fog density per stud at fog height, y: falloff per stud, z: fog height, w: max opacity.
    pub fog_params: [f32; 4],
    /// rgb: fog colour toward the sun (pre-exposed), w: in-scatter anisotropy g.
    pub fog_toward: [f32; 4],
    /// rgb: fog colour away from the sun, w: sun glare strength.
    pub fog_away: [f32; 4],
    /// rgb: extinction multiplier per channel, w: 1 when the height fog is on.
    pub fog_extinction: [f32; 4],
    /// rgb: `FogColor` as scene light, w: 1 when the distance fog is on.
    pub legacy_fog: [f32; 4],
    /// x: `FogStart`, y: `FogEnd`.
    pub legacy_range: [f32; 4],
    /// xyz: toward the sun (the disc; the light may be the moon), w: cos of the disc radius.
    pub sun_disc: [f32; 4],
    /// xyz: toward the moon, w: cos of the disc radius.
    pub moon_disc: [f32; 4],
    /// x: sun disc radiance scale, y: moon disc radiance, z: 1 when bodies are shown,
    /// w: twilight factor (0 day .. 1 night).
    pub celestial: [f32; 4],
    /// x: the toon light reference (`toon_light` of the default lighting), so toon shading is
    /// 1 under a fresh place and follows the scene's displayed brightness otherwise.
    pub toon: [f32; 4],
}

/// The light toon shading follows: the sun plus some sky, pre-exposed (as `toon_light` in
/// `skinned.wgsl` reads it from the frame uniforms).
fn toon_energy(sky: &Sky) -> f32 {
    let s: [f32; 3] = std::array::from_fn(|i| {
        let fill = sky.horizon[i] + (sky.zenith[i] - sky.horizon[i]) * 0.45;
        sky.sun_color[i] * 0.27 + fill * 0.3
    });
    luma(s).max(1e-6)
}

/// `toon_energy` of the default lighting, computed once.
fn toon_reference() -> f32 {
    static REF: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *REF.get_or_init(|| toon_energy(&evaluate_base(&LightingInputs::default()).sky))
}

/// Turns lighting properties into a frame's settings.
pub fn evaluate(inp: &LightingInputs) -> LightingFrame {
    let mut frame = evaluate_base(inp);
    frame.uniform.toon = [toon_reference(), 0.0, 0.0, 0.0];
    frame
}

fn evaluate_base(inp: &LightingInputs) -> LightingFrame {
    let inp = &inp.sanitized();
    let to_sun = sun_direction(inp.clock_time, inp.latitude);
    let elevation = to_sun.y.clamp(-1.0, 1.0).asin().to_degrees();
    let t = sky_at(elevation);
    // 0 by day, 1 once the sun is 10 degrees below the horizon.
    let night = ((-elevation - 0.0) / 10.0).clamp(0.0, 1.0);
    let night = night * night * (3.0 - 2.0 * night);
    let to_moon = -to_sun;
    // The sun lights the scene until it is 1 degree below the horizon, then the moon.
    let light_is_sun = elevation > -1.0;
    let cover = inp.cloud_cover.clamp(0.0, 1.0);

    let mut sun = if light_is_sun {
        let top = inp.color_shift_top;
        let s = scale(t.sun, inp.brightness.max(0.0));
        std::array::from_fn(|i| s[i] * (1.0 + top[i]))
    } else {
        scale(t.sun, inp.moon_brightness.max(0.0))
    };
    sun = scale(sun, (1.0 - cover).powf(1.5));

    let mut zenith = t.zenith;
    let mut horizon = t.horizon;
    let mut ground = t.ground;
    if let Some(a) = &inp.atmosphere {
        let air = relative_tint(a.air_color, [116.0 / 255.0, 172.0 / 255.0, 1.0]);
        zenith = mul(zenith, air);
        // The horizon is mostly multiply-scattered light: half the tint.
        horizon = mul(horizon, mix3([1.0; 3], air, 0.5));
    }
    if cover > 0.0 {
        // Overcast: an even, slightly brighter-at-zenith grey of the clear sky's energy.
        let energy = 0.35 * (luma(zenith) + 2.0 * luma(horizon)) + 0.25 * luma(t.sun) * 0.05;
        let tint = lin3(inp.cloud_color);
        let w = cover.powf(0.7);
        zenith = mix3(zenith, scale(tint, energy * 1.1), w);
        horizon = mix3(horizon, scale(tint, energy), w);
        ground = mix3(ground, scale(tint, energy * 0.45), w);
    }

    // Exposure: the table's metered value times the scene key, compensation, and a night bias
    // so night stays dark.
    let night_bias = night * inp.night_adaptation.clamp(0.0, 1.0);
    let exposure =
        t.exposure * EXPOSURE_KEY * 2f32.powf(inp.exposure_compensation - 0.9 * night_bias);
    let pre = |c: [f32; 3]| scale(c, exposure);

    let curve = match (inp.tonemapper.as_str(), inp.soft_style) {
        ("Retro", _) => ToneCurve::Clamp,
        ("AgX", _) => ToneCurve::Agx,
        ("Neutral", _) => ToneCurve::Neutral,
        ("ACES", _) => ToneCurve::Aces,
        ("Filmic", _) => ToneCurve::Filmic,
        (_, true) => ToneCurve::Neutral,
        _ => ToneCurve::AgxPunchy,
    };

    let mut finish = FinishSettings {
        curve,
        ..FinishSettings::default()
    };
    if let Some(cc) = inp.color_correction {
        finish.brightness = cc.brightness;
        finish.contrast = cc.contrast;
        finish.saturation = cc.saturation;
        finish.tint = lin3(cc.tint);
    }
    // Scotopic night look: blue-grey and less saturated.
    let night_tint = mix3([1.0; 3], [0.74, 0.86, 1.16], night_bias);
    finish.tint = mul(finish.tint, night_tint);
    finish.saturation -= 0.2 * night_bias;

    // Ambient terms are exposure-relative: colour 255 adds 0.5 of pre-exposed radiance.
    let ambient = scale(lin3(inp.ambient), 0.5);
    let outdoor = scale(lin3(inp.outdoor_ambient), 0.5);
    let bounce: [f32; 3] = inp.color_shift_bottom.map(|c| 1.0 + c);

    let softness = inp.shadow_softness.clamp(0.0, 1.0).max(cover * 0.9);
    let softness = if inp.soft_style {
        softness.max(0.35)
    } else {
        softness
    };
    // Filter radius in shadow-map texels: 1 = the old 3x3 PCF at softness 0.2.
    let shadow_radius = 0.6 + 2.0 * softness + 10.0 * softness * softness;

    let horizon_pre = pre(horizon);
    let (fog_params, fog_toward, fog_away, fog_extinction) = match &inp.atmosphere {
        Some(a) if a.density > 0.0 => {
            let density = 0.012 * a.density * a.density * (1.0 + 0.8 * a.haze) / 1.8;
            let max_opacity = 1.0 - 0.75 * a.offset.clamp(0.0, 1.0);
            let toward = relative_tint(a.color, [199.0 / 255.0; 3]);
            let away = relative_tint(a.decay, [106.0 / 255.0, 112.0 / 255.0, 125.0 / 255.0]);
            // Fog is lit by the sky near the horizon and by the flat ambient (city glow). With a
            // low sun the horizon's own orange would drown the Atmosphere colours, so half its
            // hue is dropped then.
            let hl = luma(horizon_pre);
            let low_sun = 0.5 * (1.0 - (elevation / 20.0).clamp(0.0, 1.0));
            let base: [f32; 3] = std::array::from_fn(|i| {
                let h = horizon_pre[i] + (hl - horizon_pre[i]) * low_sun;
                h * 0.9 + ambient[i] * 0.8 + outdoor[i] * 0.4
            });
            let toward_col = mul(base, scale(toward, luma(lin3(a.color)) / 0.55));
            let away_col = mul(base, scale(away, luma(lin3(a.decay)) / 0.15));
            let g = (0.76 + 0.019 * a.glare).min(0.95);
            let glare = if a.glare <= 0.0 {
                0.0
            } else {
                0.02 + 0.012 * a.glare
            };
            let ext = a.absorption.map(|v| 1.0 + 3.0 * v);
            (
                [
                    density,
                    a.height_falloff.max(0.0),
                    a.fog_height,
                    max_opacity,
                ],
                [toward_col[0], toward_col[1], toward_col[2], g],
                [away_col[0], away_col[1], away_col[2], glare],
                [ext[0], ext[1], ext[2], 1.0],
            )
        }
        _ => ([0.0; 4], [0.0; 4], [0.0; 4], [1.0, 1.0, 1.0, 0.0]),
    };

    let legacy_on = inp.fog_end < 100_000.0 && inp.fog_end > inp.fog_start;
    let legacy = inverse_curve(curve, inp.fog_color);

    let disc_cos = |deg: f32| (0.5 * deg.clamp(0.05, 60.0)).to_radians().cos();
    let shown = inp.sky.celestial_bodies_shown;
    // Stars fade in from civil twilight; their count scales the density of the hash field.
    let stars = (inp.sky.star_count.max(0.0) / 3000.0).min(4.0);
    let moon_radiance = 0.06 * inp.moon_brightness.max(0.0) * night.max(0.15);

    let sky = Sky {
        zenith: pre(zenith),
        horizon: horizon_pre,
        ground: pre(ground),
        to_sun: if light_is_sun { to_sun } else { to_moon },
        sun_color: pre(sun),
        fog_density: 0.0,
        exposure: 1.0,
    };
    let uniform = LightingUniform {
        light_ambient: [
            ambient[0],
            ambient[1],
            ambient[2],
            inp.environment_diffuse.max(0.0),
        ],
        light_outdoor: [
            outdoor[0],
            outdoor[1],
            outdoor[2],
            inp.environment_specular.max(0.0),
        ],
        light_bounce: [bounce[0], bounce[1], bounce[2], shadow_radius],
        light_flags: [
            if inp.global_shadows { 1.0 } else { 0.0 },
            if inp.soft_style { 0.3 } else { 0.0 },
            stars,
            if shown { night * 1.2 } else { 0.0 },
        ],
        fog_params,
        fog_toward,
        fog_away,
        fog_extinction,
        legacy_fog: [
            legacy[0],
            legacy[1],
            legacy[2],
            if legacy_on { 1.0 } else { 0.0 },
        ],
        legacy_range: [inp.fog_start, inp.fog_end, 0.0, 0.0],
        sun_disc: [
            to_sun.x,
            to_sun.y,
            to_sun.z,
            disc_cos(inp.sky.sun_angular_size),
        ],
        moon_disc: [
            to_moon.x,
            to_moon.y,
            to_moon.z,
            disc_cos(inp.sky.moon_angular_size),
        ],
        celestial: [
            if shown {
                12.0 * (1.0 - cover).powf(3.0)
            } else {
                0.0
            },
            if shown { moon_radiance } else { 0.0 },
            if shown { 1.0 } else { 0.0 },
            night,
        ],
        toon: [1.0, 0.0, 0.0, 0.0],
    };
    LightingFrame {
        sky,
        finish,
        bloom: inp.bloom,
        uniform,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_rises_in_the_east_and_peaks_south_at_noon() {
        let dawn = sun_direction(6.0, 41.73);
        assert!(dawn.y.abs() < 1e-5 && dawn.x > 0.99);
        let noon = sun_direction(12.0, 41.73);
        assert!((noon.y - 41.73f32.to_radians().cos()).abs() < 1e-5);
        assert!(noon.z > 0.6 && noon.x.abs() < 1e-5);
        assert!(sun_direction(18.0, 0.0).x < -0.99);
        assert!(sun_direction(0.0, 0.0).y < -0.99);
    }

    #[test]
    fn table_interpolation_hits_rows_and_stays_between_them() {
        let r = sky_at(45.0);
        assert!((r.exposure - 1.0).abs() < 1e-5);
        let mid = sky_at(37.5);
        assert!(mid.sun[0] > 3.54 && mid.sun[0] < 3.67);
        assert!(sky_at(-40.0).exposure > 1e5);
        assert!(sky_at(95.0).sun[0] > 3.7);
    }

    #[test]
    fn day_light_is_pre_exposed_near_the_old_sun() {
        let f = evaluate(&LightingInputs::default());
        assert!(
            f.sky.sun_color[0] > 2.5 && f.sky.sun_color[0] < 5.0,
            "{f:?}"
        );
        assert!(f.sky.to_sun.y > 0.5);
        assert_eq!(f.finish.curve, ToneCurve::AgxPunchy);
    }

    #[test]
    fn night_is_dark_but_not_black_and_lit_by_the_moon() {
        let inp = LightingInputs {
            clock_time: 0.0,
            ..LightingInputs::default()
        };
        let f = evaluate(&inp);
        assert!(f.sky.to_sun.y > 0.5, "the moon lights the night");
        let l = luma(f.sky.sun_color);
        assert!(l > 0.1 && l < 1.5, "moonlight {l}");
        assert!(f.finish.saturation < 0.0);
        assert!(f.uniform.light_flags[3] > 0.5, "stars visible");
    }

    #[test]
    fn inverse_curve_round_trips_greys() {
        for curve in ToneCurve::ALL {
            for c in [0.1f32, 0.5, 0.75] {
                let v = inverse_curve(curve, [c; 3]);
                let enc = crate::tonemap::linear_to_srgb(curve.map(v)[1]);
                assert!((enc - c).abs() < 0.02, "{curve:?}: {enc} vs {c}");
            }
        }
    }

    /// Inputs with every number set to `x` (colours too), every optional block present.
    fn extreme(x: f32) -> LightingInputs {
        LightingInputs {
            clock_time: x,
            latitude: x,
            brightness: x,
            exposure_compensation: x,
            ambient: [x; 3],
            outdoor_ambient: [x; 3],
            color_shift_top: [x; 3],
            color_shift_bottom: [x; 3],
            environment_diffuse: x,
            environment_specular: x,
            global_shadows: true,
            shadow_softness: x,
            soft_style: false,
            fog_color: [x; 3],
            fog_start: x,
            fog_end: x,
            night_adaptation: x,
            moon_brightness: x,
            atmosphere: Some(AtmosphereInputs {
                density: x,
                offset: x,
                color: [x; 3],
                decay: [x; 3],
                glare: x,
                haze: x,
                height_falloff: x,
                fog_height: x,
                absorption: [x; 3],
                air_color: [x; 3],
            }),
            sky: SkyInputs {
                sun_angular_size: x,
                moon_angular_size: x,
                star_count: x,
                celestial_bodies_shown: true,
            },
            cloud_cover: x,
            cloud_color: [x; 3],
            bloom: Some(BloomInputs {
                intensity: x,
                size: x,
                threshold: x,
            }),
            color_correction: Some(ColorCorrectionInputs {
                brightness: x,
                contrast: x,
                saturation: x,
                tint: [x; 3],
            }),
            tonemapper: "Default".into(),
        }
    }

    /// Huge, infinite and NaN inputs (from code, or 1e39 read as f32) never reach the shaders.
    #[test]
    fn extreme_inputs_give_a_finite_frame() {
        for x in [
            1e39_f64 as f32,
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            0.0,
        ] {
            let f = evaluate(&extreme(x));
            let floats: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&f.uniform));
            for (i, v) in floats.iter().enumerate() {
                assert!(v.is_finite(), "x = {x}: uniform float {i} is {v}");
            }
            let colours = [
                f.sky.zenith,
                f.sky.horizon,
                f.sky.ground,
                f.sky.sun_color,
                f.sky.to_sun.to_array(),
                f.finish.tint,
            ];
            for v in colours.iter().flatten() {
                assert!(v.is_finite(), "x = {x}: sky / tint {colours:?}");
            }
            let scalars = [
                f.sky.fog_density,
                f.sky.exposure,
                f.finish.exposure,
                f.finish.saturation,
                f.finish.contrast,
                f.finish.brightness,
                f.finish.vignette,
            ];
            assert!(
                scalars.iter().all(|v| v.is_finite()),
                "x = {x}: {scalars:?}"
            );
            if let Some(b) = f.bloom {
                assert!(b.intensity.is_finite() && b.size.is_finite() && b.threshold.is_finite());
            }
        }
    }
}
