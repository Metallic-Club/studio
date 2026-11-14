//! Lighting: the sun (or the moon), the hemisphere ambient and the sky, from the `Lighting` service.
//!
//! [`SceneLighting`] is what the renderer draws with ([`crate::Renderer::set_lighting`]): `Lighting`'s properties as
//! plain values, so tests and tools can light a scene without a data model. [`LightingReader`] reads it from a
//! data model's `Lighting` service; the engine re-reads it only when the scene's change consumer says `Lighting`
//! changed ([`crate::InstanceScene::watch`]), never per frame.
//!
//! # From properties to light
//!
//! Everything is linear light in one set of units, for surfaces and the sky alike, so the HDR post chain (#85) can
//! add exposure and tone mapping without retuning it. Colour properties are sRGB-encoded (like every `Color3`) and
//! decoded first. `e` below is the sun's elevation: the Y of `SunDirection`, which `Lighting` computes from
//! `ClockTime` and `GeographicLatitude`.
//!
//! - **One directional light.** By day it is the sun: [`SUN_RADIANCE`] × `Brightness`, white at height and orange
//!   towards the horizon, fading out as the sun reaches it. By night it is the **moon**, opposite the sun:
//!   [`MOON_RADIANCE`] × `Brightness`, pale blue. Both are zero at the horizon, so swapping one for the other at
//!   sunset never shows.
//! - **The hemisphere ambient** (light from the sky above and the ground below): `OutdoorAmbient` ×
//!   [`OUTDOOR_AMBIENT_SCALE`], tinted towards `SkyColor` by day. At night it keeps [`NIGHT_AMBIENT`] of its day
//!   level and turns blue. The ground side is the sky side times [`GROUND_BOUNCE`] (light bounced off the ground).
//! - **`Ambient`** is added evenly to both sides at every hour: the floor a game sets for dark places and nights.
//! - **The sky**: `SkyColor` overhead and `HorizonColor` at the horizon by day, night colours ([`NIGHT_ZENITH`],
//!   [`NIGHT_HORIZON`]) by night, a darker band below the horizon, an orange glow towards the sun at twilight, and
//!   the sun and moon discs.
//!
//! **Night stays readable**: with the defaults a white surface facing the moon is lit at about a sixth of its noon
//! level (moonlight plus a third of the day's sky light), dark blue but legible; colours still tell apart. `Ambient`
//! raises that floor exactly (it is never dimmed), and `Brightness` scales the moon with the sun.

use metallic_core::glam::Vec3;
use metallic_core::{Color3, DataModel, Id, PropId, Value};

/// The sun's radiance per unit of `Brightness`, linear. The default `Brightness` of 2 gives about 2.3: a white
/// surface facing the noon sun reflects about 0.73 of it, plus the sky's ambient.
pub const SUN_RADIANCE: f32 = 1.15;
/// `FogEnd` from which there is no fog: Roblox's default, 100,000 studs.
pub const NO_FOG: f32 = 100_000.0;
/// How far the sky is, for the fog (#600): with `FogEnd` well short of it, the sky is the fog's colour.
pub const SKY_FOG_DISTANCE: f32 = 10_000.0;
/// The moon's radiance per unit of `Brightness`: about 1/16 of the sun.
pub const MOON_RADIANCE: f32 = 0.07;
/// The hemisphere ambient's sky side per unit of `OutdoorAmbient` (decoded to linear).
pub const OUTDOOR_AMBIENT_SCALE: f32 = 1.9;
/// How much of the day's hemisphere ambient is left at night.
pub const NIGHT_AMBIENT: f32 = 0.35;
/// The ground side of the hemisphere ambient, as a share of the sky side (a warm, darker bounce).
pub const GROUND_BOUNCE: Vec3 = Vec3::new(0.47, 0.36, 0.26);
/// The sky overhead at night, linear.
pub const NIGHT_ZENITH: Vec3 = Vec3::new(0.004, 0.006, 0.016);
/// The sky at the horizon at night, linear.
pub const NIGHT_HORIZON: Vec3 = Vec3::new(0.014, 0.02, 0.04);

/// The moonlight's colour.
const MOON_TINT: Vec3 = Vec3::new(0.62, 0.72, 1.0);
/// The ambient's tint at night.
const NIGHT_TINT: Vec3 = Vec3::new(0.45, 0.6, 1.0);
/// The sun's colour at the horizon and high in the sky.
const SUN_LOW: Vec3 = Vec3::new(1.0, 0.5, 0.22);
const SUN_HIGH: Vec3 = Vec3::new(1.0, 0.96, 0.9);
/// The twilight glow at the horizon around the sun.
const TWILIGHT_GLOW: Vec3 = Vec3::new(0.9, 0.4, 0.14);
/// How much of `SkyColor`'s hue tints the daytime ambient.
const SKY_TINT: f32 = 0.35;
/// The sky below the horizon, as a share of the horizon colour.
const BELOW_HORIZON: f32 = 0.35;
/// The sun disc's radiance as a multiple of the sunlight, and its halo's.
const SUN_DISC: f32 = 12.0;
const SUN_HALO: f32 = 0.25;
/// The moon disc's radiance per unit of `Brightness`.
const MOON_DISC: f32 = 0.25;
/// Angular radii of the discs, in degrees (larger than the real 0.27°, so they read on screen).
const SUN_RADIUS_DEG: f32 = 1.4;
const MOON_RADIUS_DEG: f32 = 1.1;

/// What the renderer lights a frame with: `Lighting`'s properties as values (see the [module
/// documentation](self) for what each does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLighting {
    /// `SunDirection`: towards the sun, below the horizon at night (normalised when used).
    pub sun_direction: Vec3,
    /// `Brightness`: the strength of the sun and the moon.
    pub brightness: f32,
    /// `Ambient`: light added evenly everywhere.
    pub ambient: Color3,
    /// `OutdoorAmbient`: the sky's light from above.
    pub outdoor_ambient: Color3,
    /// `SkyColor`: the sky overhead by day.
    pub sky_color: Color3,
    /// `HorizonColor`: the sky at the horizon by day.
    pub horizon_color: Color3,
    /// `GlobalShadows`: the sun (or moon) casts shadows ([`crate::shadow`]).
    pub global_shadows: bool,
    /// `ColorShift_Top`: added to the sunlight (and moonlight) on surfaces facing it (#600).
    pub color_shift_top: Color3,
    /// `ColorShift_Bottom`: light of this colour on surfaces facing away from the sun (or moon) (#600).
    pub color_shift_bottom: Color3,
    /// `EnvironmentDiffuseScale`: how much of the sky's diffuse light (the hemisphere ambient) surfaces take, 0 to 1.
    pub environment_diffuse_scale: f32,
    /// `EnvironmentSpecularScale`: how much of the sky surfaces reflect, 0 to 1.
    pub environment_specular_scale: f32,
    /// `ShadowSoftness`: how soft the sun's shadow edges are, 0 to 1 ([`crate::shadow`]).
    pub shadow_softness: f32,
    /// `ExposureCompensation`: the frame's exposure in stops, -3 to 3.
    pub exposure_compensation: f32,
    /// `FogColor`: the colour things fade to with distance.
    pub fog_color: Color3,
    /// `FogStart` and `FogEnd`: the distances the fog starts at and is whole at, in studs. No fog from
    /// [`NO_FOG`] on, Roblox's default.
    pub fog_start: f32,
    /// See [`SceneLighting::fog_start`].
    pub fog_end: f32,
}

impl Default for SceneLighting {
    /// `Lighting`'s defaults: 14:00 at 41.733° north (the sun high in the south-west), `Brightness` 2, no colour shifts,
    /// the whole environment, a 0.2 shadow softness, no exposure change, no fog, no
    /// `Ambient`, a mid-grey `OutdoorAmbient`, a light blue sky and shadows on.
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(-0.5, 0.646_276, 0.576_479),
            brightness: 2.0,
            ambient: Color3::new(0.0, 0.0, 0.0),
            outdoor_ambient: Color3::new(0.5, 0.5, 0.5),
            sky_color: Color3::new(0.53, 0.72, 0.95),
            horizon_color: Color3::new(0.8, 0.88, 0.96),
            global_shadows: true,
            color_shift_top: Color3::new(0.0, 0.0, 0.0),
            color_shift_bottom: Color3::new(0.0, 0.0, 0.0),
            environment_diffuse_scale: 1.0,
            environment_specular_scale: 1.0,
            shadow_softness: 0.2,
            exposure_compensation: 0.0,
            fog_color: Color3::new(0.75, 0.75, 0.75),
            fog_start: 0.0,
            fog_end: NO_FOG,
        }
    }
}

/// [`SceneLighting`] worked out into what the shaders read (the lighting half of the frame uniform).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ResolvedLighting {
    pub light_direction: Vec3,
    pub light_color: Vec3,
    pub sky_ambient: Vec3,
    pub ground_ambient: Vec3,
    pub sky_zenith: Vec3,
    pub sky_horizon: Vec3,
    pub sky_ground: Vec3,
    pub sky_glow: Vec3,
    pub sun_direction: Vec3,
    pub sun_cos_radius: f32,
    pub sun_disc: Vec3,
    pub sun_halo: Vec3,
    pub moon_disc: Vec3,
    pub moon_cos_radius: f32,
    /// Light on surfaces facing away from the sun or moon (`ColorShift_Bottom`), at the light's strength.
    pub color_shift_bottom: Vec3,
    /// The fog's colour, and whether there is fog.
    pub fog_color: Vec3,
    pub fog: bool,
    /// Where the fog starts and is whole, in studs.
    pub fog_start: f32,
    pub fog_end: f32,
    /// The environment's diffuse and specular scales, the shadow filter's spread (1 at the default softness) and
    /// the exposure as a factor.
    pub environment: [f32; 4],
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

impl SceneLighting {
    /// The unit direction towards the frame's one directional light: the sun by day, the moon (opposite it) by
    /// night. Shadows are cast along it ([`crate::shadow`]).
    pub fn light_direction(&self) -> Vec3 {
        self.resolve().light_direction
    }

    /// Works the lighting out for the shaders (see the module documentation). Runs when the lighting changes, not
    /// per frame.
    pub(crate) fn resolve(&self) -> ResolvedLighting {
        let sun = self
            .sun_direction
            .try_normalize()
            .filter(|v| v.is_finite())
            .unwrap_or(Vec3::Y);
        let brightness = finite_or(self.brightness, 0.0).max(0.0);
        let e = sun.y;
        let day = smoothstep(-0.1, 0.25, e);

        let sun_tint = SUN_LOW.lerp(SUN_HIGH, smoothstep(0.0, 0.45, e));
        let sun_radiance = sun_tint * brightness * SUN_RADIANCE;
        // The light's strength without its colour, for the colour shifts (#600).
        let strength = if e >= 0.0 {
            brightness * SUN_RADIANCE * smoothstep(0.0, 0.15, e)
        } else {
            brightness * MOON_RADIANCE * smoothstep(0.0, 0.15, -e)
        };
        let shift_top = self.color_shift_top.to_linear().max(Vec3::ZERO) * strength;
        let shift_bottom = self.color_shift_bottom.to_linear().max(Vec3::ZERO) * strength;
        let (light_direction, light_color) = if e >= 0.0 {
            (sun, sun_radiance * smoothstep(0.0, 0.15, e) + shift_top)
        } else {
            (
                -sun,
                MOON_TINT * brightness * MOON_RADIANCE * smoothstep(0.0, 0.15, -e) + shift_top,
            )
        };

        let sky = self.sky_color.to_linear().max(Vec3::ZERO);
        let sky_hue = sky / sky.max_element().max(1e-4);
        let tint = NIGHT_TINT.lerp(Vec3::ONE.lerp(sky_hue, SKY_TINT), day);
        let level = NIGHT_AMBIENT + (1.0 - NIGHT_AMBIENT) * day;
        let outdoor = self.outdoor_ambient.to_linear().max(Vec3::ZERO) * OUTDOOR_AMBIENT_SCALE * tint * level;
        let flat = self.ambient.to_linear().max(Vec3::ZERO);
        let fog_start = finite_or(self.fog_start, 0.0).max(0.0);
        let fog_end = finite_or(self.fog_end, NO_FOG).max(fog_start);

        let horizon = NIGHT_HORIZON.lerp(self.horizon_color.to_linear().max(Vec3::ZERO), day);
        let twilight = smoothstep(-0.2, 0.0, e) * (1.0 - smoothstep(0.0, 0.3, e));
        let sun_visible = smoothstep(-0.03, 0.02, e);
        let moon_visible = smoothstep(-0.03, 0.02, -e);
        ResolvedLighting {
            light_direction,
            light_color,
            sky_ambient: outdoor + flat,
            ground_ambient: outdoor * GROUND_BOUNCE + flat,
            sky_zenith: NIGHT_ZENITH.lerp(sky, day),
            sky_horizon: horizon,
            sky_ground: horizon * BELOW_HORIZON,
            sky_glow: TWILIGHT_GLOW * twilight * brightness.min(4.0) * 0.5,
            sun_direction: sun,
            sun_cos_radius: SUN_RADIUS_DEG.to_radians().cos(),
            sun_disc: sun_radiance * SUN_DISC * sun_visible,
            sun_halo: sun_radiance * SUN_HALO * sun_visible,
            moon_disc: MOON_TINT * brightness * MOON_DISC * moon_visible,
            moon_cos_radius: MOON_RADIUS_DEG.to_radians().cos(),
            color_shift_bottom: shift_bottom,
            fog_color: self.fog_color.to_linear().max(Vec3::ZERO),
            fog: fog_end < NO_FOG,
            fog_start,
            fog_end,
            environment: [
                finite_or(self.environment_diffuse_scale, 1.0).clamp(0.0, 1.0),
                finite_or(self.environment_specular_scale, 1.0).clamp(0.0, 1.0),
                // 0.2, the default, is the filter as it was; 0 half as wide and 1 three times as wide.
                0.5 + 2.5 * finite_or(self.shadow_softness, 0.2).clamp(0.0, 1.0),
                2f32.powf(finite_or(self.exposure_compensation, 0.0).clamp(-3.0, 3.0)),
            ],
        }
    }
}

/// The `Lighting` property ids of one instance in one data model.
#[derive(Clone, Copy, Debug)]
struct LightingProps {
    /// The data model ([`DataModel::uid`]) and instance they belong to.
    owner: (u64, Id),
    sun_direction: Option<PropId>,
    brightness: Option<PropId>,
    ambient: Option<PropId>,
    outdoor_ambient: Option<PropId>,
    sky_color: Option<PropId>,
    horizon_color: Option<PropId>,
    global_shadows: Option<PropId>,
    color_shift_top: Option<PropId>,
    color_shift_bottom: Option<PropId>,
    environment_diffuse_scale: Option<PropId>,
    environment_specular_scale: Option<PropId>,
    shadow_softness: Option<PropId>,
    exposure_compensation: Option<PropId>,
    fog_color: Option<PropId>,
    fog_start: Option<PropId>,
    fog_end: Option<PropId>,
}

/// Reads [`SceneLighting`] from a `Lighting` instance, looking its property ids up by name only when the instance
/// or the data model changes. Read it when `Lighting` changed, not every frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct LightingReader {
    props: Option<LightingProps>,
}

impl LightingReader {
    /// A reader that has resolved nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The lighting `lighting` (the `Lighting` service) describes in `dm`. A property it does not have, or cannot
    /// be read, keeps [`SceneLighting::default`]'s value.
    pub fn read(&mut self, dm: &DataModel, lighting: Id) -> SceneLighting {
        let props = match self.props {
            Some(props) if props.owner == (dm.uid(), lighting) => props,
            _ => {
                let id = |name| dm.prop_id(lighting, name).ok();
                let props = LightingProps {
                    owner: (dm.uid(), lighting),
                    sun_direction: id("SunDirection"),
                    brightness: id("Brightness"),
                    ambient: id("Ambient"),
                    outdoor_ambient: id("OutdoorAmbient"),
                    sky_color: id("SkyColor"),
                    horizon_color: id("HorizonColor"),
                    global_shadows: id("GlobalShadows"),
                    color_shift_top: id("ColorShift_Top"),
                    color_shift_bottom: id("ColorShift_Bottom"),
                    environment_diffuse_scale: id("EnvironmentDiffuseScale"),
                    environment_specular_scale: id("EnvironmentSpecularScale"),
                    shadow_softness: id("ShadowSoftness"),
                    exposure_compensation: id("ExposureCompensation"),
                    fog_color: id("FogColor"),
                    fog_start: id("FogStart"),
                    fog_end: id("FogEnd"),
                };
                self.props = Some(props);
                props
            }
        };
        let defaults = SceneLighting::default();
        let color = |prop: Option<PropId>, default| prop.and_then(|p| dm.color3(lighting, p).ok()).unwrap_or(default);
        let number = |prop: Option<PropId>, default: f32| {
            prop.and_then(|p| dm.number(lighting, p).ok())
                .map_or(default, |n| n as f32)
        };
        SceneLighting {
            sun_direction: props
                .sun_direction
                .and_then(|p| match dm.get(lighting, p) {
                    Ok(Value::Vector3(v)) => Some(v),
                    _ => None,
                })
                .unwrap_or(defaults.sun_direction),
            brightness: props
                .brightness
                .and_then(|p| dm.number(lighting, p).ok())
                .map_or(defaults.brightness, |b| b as f32),
            ambient: color(props.ambient, defaults.ambient),
            outdoor_ambient: color(props.outdoor_ambient, defaults.outdoor_ambient),
            sky_color: color(props.sky_color, defaults.sky_color),
            horizon_color: color(props.horizon_color, defaults.horizon_color),
            global_shadows: props
                .global_shadows
                .and_then(|p| dm.bool(lighting, p).ok())
                .unwrap_or(defaults.global_shadows),
            color_shift_top: color(props.color_shift_top, defaults.color_shift_top),
            color_shift_bottom: color(props.color_shift_bottom, defaults.color_shift_bottom),
            environment_diffuse_scale: number(props.environment_diffuse_scale, defaults.environment_diffuse_scale),
            environment_specular_scale: number(props.environment_specular_scale, defaults.environment_specular_scale),
            shadow_softness: number(props.shadow_softness, defaults.shadow_softness),
            exposure_compensation: number(props.exposure_compensation, defaults.exposure_compensation),
            fog_color: color(props.fog_color, defaults.fog_color),
            fog_start: number(props.fog_start, defaults.fog_start),
            fog_end: number(props.fog_end, defaults.fog_end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lighting(dm: &DataModel) -> Id {
        let class = dm.registry().class_id("Lighting").unwrap();
        dm.service(class).unwrap()
    }

    fn at(clock_time: f64) -> SceneLighting {
        let mut dm = metallic_classes::new_game().unwrap();
        let id = lighting(&dm);
        dm.set_by_name(id, "ClockTime", Value::Number(clock_time)).unwrap();
        LightingReader::new().read(&dm, id)
    }

    #[test]
    fn the_default_is_a_new_games_lighting() {
        let dm = metallic_classes::new_game().unwrap();
        let read = LightingReader::new().read(&dm, lighting(&dm));
        let default = SceneLighting::default();
        assert!(read.sun_direction.abs_diff_eq(default.sun_direction, 1e-5), "{read:?}");
        assert_eq!(
            SceneLighting {
                sun_direction: default.sun_direction,
                ..read
            },
            default
        );
    }

    #[test]
    fn reads_every_property_and_follows_changes() {
        let mut dm = metallic_classes::new_game().unwrap();
        let id = lighting(&dm);
        let mut reader = LightingReader::new();
        let set = |dm: &mut DataModel, name: &str, value: Value| dm.set_by_name(id, name, value).unwrap();
        set(&mut dm, "ClockTime", Value::Number(6.0));
        set(&mut dm, "Brightness", Value::Number(3.0));
        set(&mut dm, "Ambient", Value::Color3(Color3::new(0.1, 0.2, 0.3)));
        set(&mut dm, "OutdoorAmbient", Value::Color3(Color3::new(0.4, 0.4, 0.4)));
        set(&mut dm, "SkyColor", Value::Color3(Color3::new(0.2, 0.3, 0.9)));
        set(&mut dm, "HorizonColor", Value::Color3(Color3::new(0.9, 0.8, 0.7)));
        assert!(reader.read(&dm, id).global_shadows);
        set(&mut dm, "GlobalShadows", Value::Bool(false));
        let read = reader.read(&dm, id);
        assert!(!read.global_shadows);
        assert!(read.sun_direction.abs_diff_eq(Vec3::X, 1e-5), "{read:?}");
        assert_eq!(read.brightness, 3.0);
        assert_eq!(read.ambient, Color3::new(0.1, 0.2, 0.3));
        assert_eq!(read.outdoor_ambient, Color3::new(0.4, 0.4, 0.4));
        assert_eq!(read.sky_color, Color3::new(0.2, 0.3, 0.9));
        assert_eq!(read.horizon_color, Color3::new(0.9, 0.8, 0.7));
        // Roblox's (#600).
        set(&mut dm, "ColorShift_Top", Value::Color3(Color3::new(0.3, 0.2, 0.1)));
        set(&mut dm, "ColorShift_Bottom", Value::Color3(Color3::new(0.1, 0.2, 0.3)));
        set(&mut dm, "EnvironmentDiffuseScale", Value::Number(0.25));
        set(&mut dm, "EnvironmentSpecularScale", Value::Number(0.5));
        set(&mut dm, "ShadowSoftness", Value::Number(1.0));
        set(&mut dm, "ExposureCompensation", Value::Number(-1.0));
        set(&mut dm, "FogColor", Value::Color3(Color3::new(0.5, 0.6, 0.7)));
        set(&mut dm, "FogStart", Value::Number(10.0));
        set(&mut dm, "FogEnd", Value::Number(200.0));
        let read = reader.read(&dm, id);
        assert_eq!(read.color_shift_top, Color3::new(0.3, 0.2, 0.1));
        assert_eq!(read.color_shift_bottom, Color3::new(0.1, 0.2, 0.3));
        assert_eq!(
            (read.environment_diffuse_scale, read.environment_specular_scale),
            (0.25, 0.5)
        );
        assert_eq!((read.shadow_softness, read.exposure_compensation), (1.0, -1.0));
        assert_eq!(read.fog_color, Color3::new(0.5, 0.6, 0.7));
        assert_eq!((read.fog_start, read.fog_end), (10.0, 200.0));
        set(&mut dm, "ClockTime", Value::Number(18.0));
        assert!(reader.read(&dm, id).sun_direction.abs_diff_eq(Vec3::NEG_X, 1e-5));
        // Another instance without the properties reads as the defaults.
        let folder = dm.create(dm.registry().class_id("Folder").unwrap()).unwrap();
        assert_eq!(reader.read(&dm, folder), SceneLighting::default());
    }

    /// #600: the colour shifts, the environment's scales, the shadows' softness, the exposure and the fog, as the
    /// shaders get them; the defaults change nothing.
    #[test]
    fn roblox_s_lighting_properties_resolve_for_the_shaders() {
        let plain = SceneLighting::default().resolve();
        assert_eq!(
            plain.environment,
            [1.0, 1.0, 1.0, 1.0],
            "the defaults are the look as it was"
        );
        assert!(!plain.fog && plain.color_shift_bottom == Vec3::ZERO);
        let shifted = SceneLighting {
            color_shift_top: Color3::new(1.0, 0.0, 0.0),
            color_shift_bottom: Color3::new(0.0, 0.0, 1.0),
            environment_diffuse_scale: 0.0,
            environment_specular_scale: 2.0,
            shadow_softness: 1.0,
            exposure_compensation: -2.0,
            fog_end: 300.0,
            fog_start: 400.0,
            ..SceneLighting::default()
        }
        .resolve();
        assert!(shifted.light_color.x > plain.light_color.x + 1.0, "{shifted:?}");
        assert_eq!(shifted.light_color.y, plain.light_color.y);
        assert!(shifted.color_shift_bottom.z > 1.0 && shifted.color_shift_bottom.x == 0.0);
        assert_eq!(shifted.environment, [0.0, 1.0, 3.0, 0.25], "clamped to their ranges");
        assert!(shifted.fog);
        assert_eq!(
            (shifted.fog_start, shifted.fog_end),
            (400.0, 400.0),
            "the end is never before the start"
        );
        let fog_off = SceneLighting {
            fog_end: NO_FOG,
            ..SceneLighting::default()
        };
        assert!(!fog_off.resolve().fog);
    }

    #[test]
    fn the_sun_lights_the_day_and_the_moon_the_night() {
        let noon = at(12.0).resolve();
        assert!(noon.light_direction.y > 0.5 && noon.light_color.x > 2.0, "{noon:?}");
        // The default ambient is close to the temporary light #77 drew with.
        let default = SceneLighting::default().resolve();
        assert!(
            default.sky_ambient.abs_diff_eq(Vec3::new(0.3, 0.34, 0.4), 0.03),
            "{default:?}"
        );

        let midnight = at(0.0).resolve();
        // The moon is up, opposite the sun, and much dimmer.
        assert!(midnight.light_direction.y > 0.5, "{midnight:?}");
        assert!(midnight.light_color.max_element() < noon.light_color.max_element() / 10.0);
        assert!(midnight.light_color.max_element() > 0.0);
        // Readable: a white surface facing the moon still gets about a sixth of its noon light, bluer.
        let lit = |r: &ResolvedLighting| r.light_color / std::f32::consts::PI + r.sky_ambient;
        let ratio = lit(&midnight).max_element() / lit(&noon).max_element();
        assert!((0.05..0.25).contains(&ratio), "night at {ratio} of noon");
        assert!(lit(&midnight).z > lit(&midnight).x);
        // The sky is dark at night, the sky colour at noon.
        assert!(midnight.sky_zenith.max_element() < 0.05);
        assert!(
            noon.sky_zenith
                .abs_diff_eq(Color3::new(0.53, 0.72, 0.95).to_linear(), 1e-4)
        );
    }

    #[test]
    fn the_sun_has_no_upper_brightness() {
        // Brightness 50 is kept by the data model (#481) and lights 25 times as strongly as the default 2.
        let mut dm = metallic_classes::new_game().unwrap();
        let id = lighting(&dm);
        dm.set_by_name(id, "ClockTime", Value::Number(12.0)).unwrap();
        let normal = LightingReader::new().read(&dm, id).resolve();
        dm.set_by_name(id, "Brightness", Value::Number(50.0)).unwrap();
        let read = LightingReader::new().read(&dm, id);
        assert_eq!(read.brightness, 50.0);
        let bright = read.resolve();
        assert!(
            bright.light_color.abs_diff_eq(normal.light_color * 25.0, 1e-3),
            "{bright:?}"
        );
    }

    #[test]
    fn sunset_hands_over_to_the_moon_without_a_jump() {
        // Around 18:00 the light changes direction where both the sun and the moon are dark.
        let mut last: Option<ResolvedLighting> = None;
        for step in 0..=120 {
            let lighting = at(17.0 + f64::from(step) / 60.0).resolve();
            if let Some(last) = last {
                let jump = (lighting.light_color - last.light_color).abs().max_element();
                assert!(jump < 0.08, "the light jumped by {jump} at step {step}");
            }
            last = Some(lighting);
        }
        // A twilight glow at sunset, none at noon.
        assert!(at(18.0).resolve().sky_glow.x > 0.1);
        assert_eq!(at(12.0).resolve().sky_glow, Vec3::ZERO);
    }

    #[test]
    fn ambient_raises_the_floor_and_brightness_zero_turns_the_light_off() {
        let base = at(0.0);
        let raised = SceneLighting {
            ambient: Color3::new(0.5, 0.5, 0.5),
            ..base
        }
        .resolve();
        let floor = Color3::new(0.5, 0.5, 0.5).to_linear();
        assert!(raised.sky_ambient.abs_diff_eq(base.resolve().sky_ambient + floor, 1e-5));
        assert!(
            raised
                .ground_ambient
                .abs_diff_eq(base.resolve().ground_ambient + floor, 1e-5)
        );
        let dark = SceneLighting {
            brightness: 0.0,
            ..SceneLighting::default()
        }
        .resolve();
        assert_eq!((dark.light_color, dark.sun_disc), (Vec3::ZERO, Vec3::ZERO));
        // Nonsense in, something sane out.
        let nan = SceneLighting {
            sun_direction: Vec3::NAN,
            brightness: f32::NAN,
            ..SceneLighting::default()
        }
        .resolve();
        assert_eq!(nan.light_direction, Vec3::Y);
        assert_eq!(nan.light_color, Vec3::ZERO);
    }
}
