//! Graphics settings (ADR 0008 §8): the presets, the player's overrides, the file they are saved in, and the Auto
//! preset's calibration.
//!
//! A [`GraphicsSettings`] is what a player chooses: a [`Preset`] (Low, Medium, High, Ultra or Auto) and any of the
//! per-setting overrides (render scale, dynamic resolution and its target, frame cap and vsync, view distance,
//! shadow quality and distance, local-light budget, LOD bias, anti-aliasing, effect quality). [`GraphicsSettings::quality`] resolves them into a
//! [`Quality`]: the settings of every system the presets drive ([`ResolutionSettings`], [`CullSettings`],
//! [`ShadowSettings`], [`LightSettings`], [`LodSettings`], [`AntiAliasing`], [`PostSettings`]), which the renderer takes as they are.
//!
//! **Presets scale cost, never a game's look.** Lower presets draw at a lower resolution, see less far, draw fewer
//! and coarser shadows, fewer local lights at once, coarser far levels of detail, cheaper anti-aliasing and bloom from
//! fewer texels; none turns the sun's shadows, the local lights or bloom off (that is the game's `Lighting.GlobalShadows`
//! and each light's `Enabled`), and no override can either.
//!
//! **Auto** picks a tier on the machine, then holds the frame time with dynamic resolution. The [`Calibrator`]
//! times a fixed workload (the engine's built-in calibration scene, drawn offscreen beside the game's own frames, #139)
//! at Ultra, then High, then Medium, and keeps the first tier whose frames fit its share of a 60 fps frame;
//! nothing waits for the GPU (the timings are the renderer's own, read back a few frames late). Because it never times
//! the game, the tier does not depend on which game ran first. The result ([`Calibration`]) is saved with the
//! settings, with the adapter, driver and workload it was measured on, and measured again when any of them changes.
//!
//! **The file** is a few `key = value` lines (a small subset of TOML, written and read here, so no dependency):
//! [`GraphicsSettings::to_text`] and [`GraphicsSettings::from_text`].

use std::fmt;
use std::str::FromStr;

use crate::cull::CullSettings;
use crate::lights::{LightSettings, MAX_GPU_LIGHTS};
use crate::lod::LodSettings;
use crate::post::{AntiAliasing, PostSettings};
use crate::resolution::{MAX_SCALE, MIN_SCALE, ResolutionSettings};
use crate::shadow::ShadowSettings;

/// A graphics preset: how much a frame may cost, never what the game looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Preset {
    /// Integrated GPUs and phones.
    Low,
    /// The mid-range desktop budget (60 fps on a GTX 1650 class GPU).
    Medium,
    /// Higher-end desktops.
    High,
    /// Everything at its most expensive.
    Ultra,
    /// Calibrated on the machine, with dynamic resolution holding the frame time.
    #[default]
    Auto,
}

impl Preset {
    /// Every preset, Low to Ultra, then Auto.
    pub const ALL: &[Preset] = &[Preset::Low, Preset::Medium, Preset::High, Preset::Ultra, Preset::Auto];

    /// The preset's name (`low`, ..., `auto`), as the command line and the settings file take it.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Low => "low",
            Preset::Medium => "medium",
            Preset::High => "high",
            Preset::Ultra => "ultra",
            Preset::Auto => "auto",
        }
    }

    /// The tier a fixed preset draws at; `None` for Auto, whose tier is calibrated.
    pub fn tier(self) -> Option<Tier> {
        match self {
            Preset::Low => Some(Tier::Low),
            Preset::Medium => Some(Tier::Medium),
            Preset::High => Some(Tier::High),
            Preset::Ultra => Some(Tier::Ultra),
            Preset::Auto => None,
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A name that is not a preset.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown preset `{0}` (presets: low, medium, high, ultra, auto)")]
pub struct UnknownPreset(pub String);

impl FromStr for Preset {
    type Err = UnknownPreset;

    /// Parses a preset's name, whatever its case.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Preset::ALL
            .iter()
            .copied()
            .find(|preset| preset.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownPreset(name.to_owned()))
    }
}

/// A quality tier: what a fixed preset draws at, and what Auto calibrates to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// [`Quality::LOW`].
    Low,
    /// [`Quality::MEDIUM`].
    Medium,
    /// [`Quality::HIGH`].
    High,
    /// [`Quality::ULTRA`].
    Ultra,
}

impl Tier {
    /// Every tier, cheapest first.
    pub const ALL: [Tier; 4] = [Tier::Low, Tier::Medium, Tier::High, Tier::Ultra];

    /// The tier's name (`low`, `medium`, `high`, `ultra`).
    pub fn name(self) -> &'static str {
        match self {
            Tier::Low => "low",
            Tier::Medium => "medium",
            Tier::High => "high",
            Tier::Ultra => "ultra",
        }
    }

    /// The tier as a quality level from 1 (Low) to 4 (Ultra), as scripts read it.
    pub fn level(self) -> u8 {
        self as u8 + 1
    }

    /// The fixed preset of this tier.
    pub fn preset(self) -> Preset {
        match self {
            Tier::Low => Preset::Low,
            Tier::Medium => Preset::Medium,
            Tier::High => Preset::High,
            Tier::Ultra => Preset::Ultra,
        }
    }

    /// The next cheaper tier; `None` for Low.
    pub fn cheaper(self) -> Option<Tier> {
        match self {
            Tier::Low => None,
            Tier::Medium => Some(Tier::Low),
            Tier::High => Some(Tier::Medium),
            Tier::Ultra => Some(Tier::High),
        }
    }
}

impl FromStr for Tier {
    type Err = UnknownPreset;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Tier::ALL
            .into_iter()
            .find(|tier| tier.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownPreset(name.to_owned()))
    }
}

/// The GPU frame time dynamic resolution holds unless told otherwise: a 60 fps frame with 10% to spare.
pub const DEFAULT_TARGET_MS: f32 = 15.0;
/// The lowest render scale dynamic resolution goes to unless told otherwise.
pub const DEFAULT_MIN_SCALE: f32 = 0.5;

/// The settings of every system a preset drives: what [`GraphicsSettings::quality`] resolves to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    /// The tier the settings start from.
    pub tier: Tier,
    /// The render scale and dynamic resolution.
    pub resolution: ResolutionSettings,
    /// The view distance and screen-size culling.
    pub cull: CullSettings,
    /// The sun's shadow cascades.
    pub shadows: ShadowSettings,
    /// The local lights' budget and shadow atlas.
    pub lights: LightSettings,
    /// The levels of detail's bias.
    pub lod: LodSettings,
    /// The anti-aliasing: MLAA, or MSAA where the device has the samples ([`AntiAliasing::supported`]).
    pub anti_aliasing: AntiAliasing,
    /// The post chain's effect quality: bloom's resolution and filter, MLAA's search ([`crate::post`]). Every tier
    /// draws bloom; the lower ones from fewer texels.
    pub post: PostSettings,
    /// Record the scene passes as one render pass ([`crate::PassId::Scene`]), keeping colour and depth in tile
    /// memory on tile-based GPUs (the engine merges them on any tier there). Every tier does (#97): the image is the
    /// same, each pass is still timed where the device can time inside a pass, and on desktop it saves the CPU of two
    /// render passes and never costs GPU time (docs/perf.md).
    pub merge_scene_passes: bool,
}

impl Quality {
    /// Low (Intel Iris Xe laptops, mid-range phones; ADR 0008's Low tier): the scene at 75% scale, 512 m of view,
    /// parts under 2 pixels culled, one 1024² sun cascade out to 128 m (as the ADR sets), 32 local lights a frame
    /// with 2 shadowed in a 1024² atlas, 1 extra sun, an LOD bias of 0.5, MLAA and bloom from a quarter of the output.
    pub const LOW: Self = Self {
        tier: Tier::Low,
        resolution: ResolutionSettings {
            scale: 0.75,
            max_scale: 0.75,
            ..ResolutionSettings::NATIVE
        },
        cull: CullSettings {
            enabled: true,
            max_distance: 512.0,
            min_screen_size: 2.0,
        },
        shadows: ShadowSettings {
            enabled: true,
            cascades: 1,
            resolution: 1024,
            distance: 128.0,
            dynamic_cascades: 1,
            // See-through parts cast no shadow at Low (#122).
            see_through: false,
            ..ShadowSettings::OFF
        },
        lights: LightSettings::LOW,
        lod: LodSettings::LOW,
        anti_aliasing: AntiAliasing::Mlaa,
        post: PostSettings::LOW,
        merge_scene_passes: true,
    };

    /// Medium (GTX 1650 / RX 6500 class at 60 fps, the Mid tier): full resolution, 1 km of view, parts under 1.5
    /// pixels culled, two 2048² sun cascades out to 192 m (the far one cached), 128 local lights with 8 shadowed in
    /// a 2048² atlas, 2 extra suns, an LOD bias of 1, MLAA and bloom from half the output.
    pub const MEDIUM: Self = Self {
        tier: Tier::Medium,
        resolution: ResolutionSettings::NATIVE,
        cull: CullSettings {
            enabled: true,
            max_distance: 1024.0,
            min_screen_size: 1.5,
        },
        shadows: ShadowSettings {
            enabled: true,
            cascades: 2,
            resolution: 2048,
            distance: 192.0,
            dynamic_cascades: 1,
            ..ShadowSettings::OFF
        },
        lights: LightSettings::MEDIUM,
        lod: LodSettings::MEDIUM,
        anti_aliasing: AntiAliasing::Mlaa,
        post: PostSettings::MEDIUM,
        merge_scene_passes: true,
    };

    /// High (RTX 3060 Ti class): full resolution, 2 km of view, parts under a pixel culled, three 2048² sun
    /// cascades out to 256 m (the far one cached; the renderer's defaults), 192 local lights with 8 shadowed in a
    /// 4096² atlas, 4 extra suns, an LOD bias of 2, 4x MSAA and bloom from half the output with 13-tap downsampling.
    pub const HIGH: Self = Self {
        tier: Tier::High,
        resolution: ResolutionSettings::NATIVE,
        cull: CullSettings {
            enabled: true,
            max_distance: 2048.0,
            min_screen_size: 1.0,
        },
        shadows: ShadowSettings {
            enabled: true,
            ..ShadowSettings::OFF
        },
        lights: LightSettings::HIGH,
        lod: LodSettings::HIGH,
        anti_aliasing: AntiAliasing::Msaa(4),
        post: PostSettings::HIGH,
        merge_scene_passes: true,
    };

    /// Ultra: full resolution, no view distance, parts under half a pixel culled, three 3072² sun cascades out to
    /// 256 m (High's, 1.5 times as sharp), 256 local lights with 10 shadowed, 4 extra suns, an LOD bias of 4, 8x
    /// MSAA and High's bloom.
    pub const ULTRA: Self = Self {
        tier: Tier::Ultra,
        resolution: ResolutionSettings::NATIVE,
        cull: CullSettings {
            enabled: true,
            max_distance: f32::INFINITY,
            min_screen_size: 0.5,
        },
        shadows: ShadowSettings {
            enabled: true,
            resolution: 3072,
            ..ShadowSettings::OFF
        },
        lights: LightSettings {
            budget: MAX_GPU_LIGHTS as u32,
            shadowed_lights: 10,
            ..LightSettings::HIGH
        },
        lod: LodSettings {
            enabled: true,
            bias: 4.0,
        },
        anti_aliasing: AntiAliasing::Msaa(8),
        post: PostSettings::HIGH,
        merge_scene_passes: true,
    };

    /// The settings of `tier`.
    pub fn for_tier(tier: Tier) -> Self {
        match tier {
            Tier::Low => Self::LOW,
            Tier::Medium => Self::MEDIUM,
            Tier::High => Self::HIGH,
            Tier::Ultra => Self::ULTRA,
        }
    }
}

/// What the Auto preset measured: the tier it chose, on which adapter and driver, with which workload.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// The tier chosen.
    pub tier: Tier,
    /// The median GPU frame time of the tier chosen, in milliseconds, as measured.
    pub gpu_ms: f32,
    /// The adapter it was measured on ([`crate::Capabilities::adapter_name`]).
    pub adapter: String,
    /// Its driver ([`crate::Capabilities::driver`]).
    pub driver: String,
    /// The version of the workload it timed (the engine's calibration scene and how it is measured): a calibration
    /// of another version is measured again. 0 is a calibration from before the workload existed (#84 timed the
    /// game being played), which a file without the key reads as.
    pub workload: u32,
}

/// What a player chooses: a preset and any per-setting overrides. `None` takes the preset's value (for the frame
/// cap and vsync, the app's). Saved per user as text ([`GraphicsSettings::to_text`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphicsSettings {
    /// The preset.
    pub preset: Preset,
    /// The render scale, 0.5 to 2 (with dynamic resolution, the scale it starts from).
    pub render_scale: Option<f32>,
    /// Dynamic resolution on or off (on for Auto, off for the fixed presets).
    pub dynamic_resolution: Option<bool>,
    /// The GPU frame time dynamic resolution holds, in milliseconds ([`DEFAULT_TARGET_MS`]).
    pub target_frame_ms: Option<f32>,
    /// The lowest scale dynamic resolution goes to ([`DEFAULT_MIN_SCALE`]).
    pub min_render_scale: Option<f32>,
    /// The highest scale dynamic resolution goes to (1, or the render scale when that is higher).
    pub max_render_scale: Option<f32>,
    /// Present with vsync.
    pub vsync: Option<bool>,
    /// The most frames per second; 0 for none.
    pub frame_cap: Option<u32>,
    /// How far the camera sees, in metres; 0 for no limit.
    pub view_distance: Option<f32>,
    /// The sun's shadow cascades (count and resolution) of this tier.
    pub shadow_quality: Option<Tier>,
    /// How far the sun's shadows reach, in metres (at least 16).
    pub shadow_distance: Option<f32>,
    /// The local lights a frame draws at full strength (1 to 256).
    pub light_budget: Option<u32>,
    /// The levels of detail's bias (0.25 to 8; higher is finer).
    pub lod_bias: Option<f32>,
    /// The anti-aliasing (off, MLAA, or MSAA 2x, 4x, 8x; MSAA falls back to what the device has).
    pub anti_aliasing: Option<AntiAliasing>,
    /// The post effects' quality of this tier: bloom's resolution and filter, MLAA's search. Bloom is never off.
    pub effect_quality: Option<Tier>,
    /// What Auto measured, kept with the settings.
    pub calibration: Option<Calibration>,
}

impl GraphicsSettings {
    /// `preset` with no overrides.
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            ..Self::default()
        }
    }

    /// The tier the settings draw at: the preset's, or for Auto the calibrated one (Medium until calibrated).
    pub fn tier(&self) -> Tier {
        self.preset
            .tier()
            .or(self.calibration.as_ref().map(|c| c.tier))
            .unwrap_or(Tier::Medium)
    }

    /// Whether Auto must calibrate on this adapter and driver with version `workload` of the calibration workload:
    /// Auto without a calibration, or with one measured on another adapter or driver, or with another workload.
    pub fn needs_calibration(&self, adapter: &str, driver: &str, workload: u32) -> bool {
        self.preset == Preset::Auto
            && !self
                .calibration
                .as_ref()
                .is_some_and(|c| c.adapter == adapter && c.driver == driver && c.workload == workload)
    }

    /// The settings of every system: the tier's ([`GraphicsSettings::tier`]), with the overrides applied.
    pub fn quality(&self) -> Quality {
        self.quality_at(self.tier())
    }

    /// The settings of every system at `tier`, with the overrides applied (Auto's calibration draws each tier it
    /// measures so).
    pub fn quality_at(&self, tier: Tier) -> Quality {
        let mut quality = Quality::for_tier(tier);
        let resolution = &mut quality.resolution;
        resolution.dynamic = self.dynamic_resolution.unwrap_or(self.preset == Preset::Auto);
        if let Some(scale) = self.render_scale.filter(|s| s.is_finite()) {
            resolution.scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        }
        resolution.max_scale = match self.max_render_scale.filter(|s| s.is_finite()) {
            Some(max) => max.clamp(MIN_SCALE, MAX_SCALE),
            None if resolution.dynamic => resolution.scale.max(1.0),
            None => resolution.scale,
        };
        resolution.min_scale = self
            .min_render_scale
            .filter(|s| s.is_finite())
            .unwrap_or(DEFAULT_MIN_SCALE)
            .clamp(MIN_SCALE, resolution.max_scale);
        if let Some(target) = self.target_frame_ms.filter(|t| t.is_finite() && *t > 0.0) {
            resolution.target_ms = target;
        } else {
            resolution.target_ms = DEFAULT_TARGET_MS;
        }
        if let Some(distance) = self.view_distance {
            quality.cull.max_distance = if distance > 0.0 && distance.is_finite() {
                distance
            } else {
                f32::INFINITY
            };
        }
        if let Some(shadow_tier) = self.shadow_quality {
            quality.shadows = Quality::for_tier(shadow_tier).shadows;
        }
        if let Some(distance) = self.shadow_distance.filter(|d| d.is_finite()) {
            quality.shadows.distance = distance.max(16.0);
        }
        if let Some(budget) = self.light_budget {
            // At least one: an override never turns a game's lights off.
            quality.lights.budget = budget.clamp(1, MAX_GPU_LIGHTS as u32);
        }
        if let Some(bias) = self.lod_bias.filter(|b| b.is_finite()) {
            quality.lod.bias = bias.clamp(0.25, 8.0);
        }
        if let Some(aa) = self.anti_aliasing {
            quality.anti_aliasing = aa;
        }
        if let Some(effects) = self.effect_quality {
            quality.post = Quality::for_tier(effects).post;
        }
        quality
    }

    /// The settings as the text of the settings file: the preset, each override set, the calibration, and, as
    /// comments, every override not set with the value the preset gives it.
    pub fn to_text(&self) -> String {
        use fmt::Write as _;
        let quality = self.quality();
        let mut out = String::with_capacity(1024);
        out.push_str(
            "# Metallic graphics settings. Presets: low, medium, high, ultra, auto.\n\
             # A line starting with `#` takes the preset's value; remove the `#` to set your own.\n",
        );
        let _ = writeln!(out, "preset = \"{}\"", self.preset.name());
        let mut line = |key: &str, value: Option<String>, preset: String| {
            let _ = match value {
                Some(value) => writeln!(out, "{key} = {value}"),
                None => writeln!(out, "# {key} = {preset}"),
            };
        };
        let number = |n: f32| format!("{n:?}");
        let resolution = &quality.resolution;
        line("render_scale", self.render_scale.map(number), number(resolution.scale));
        line(
            "dynamic_resolution",
            self.dynamic_resolution.map(|b| b.to_string()),
            resolution.dynamic.to_string(),
        );
        line(
            "target_frame_ms",
            self.target_frame_ms.map(number),
            number(resolution.target_ms),
        );
        line(
            "min_render_scale",
            self.min_render_scale.map(number),
            number(resolution.min_scale),
        );
        line(
            "max_render_scale",
            self.max_render_scale.map(number),
            number(resolution.max_scale),
        );
        line("vsync", self.vsync.map(|b| b.to_string()), "true".to_owned());
        line("frame_cap", self.frame_cap.map(|n| n.to_string()), "240".to_owned());
        let distance = quality.cull.max_distance;
        line(
            "view_distance",
            self.view_distance.map(number),
            number(if distance.is_finite() { distance } else { 0.0 }),
        );
        line(
            "shadow_quality",
            self.shadow_quality.map(|t| quote(t.name())),
            quote(quality.tier.name()),
        );
        line(
            "shadow_distance",
            self.shadow_distance.map(number),
            number(quality.shadows.distance),
        );
        line(
            "light_budget",
            self.light_budget.map(|n| n.to_string()),
            quality.lights.budget.to_string(),
        );
        line("lod_bias", self.lod_bias.map(number), number(quality.lod.bias));
        line(
            "anti_aliasing",
            self.anti_aliasing.map(|aa| quote(aa.name())),
            quote(quality.anti_aliasing.name()),
        );
        line(
            "effect_quality",
            self.effect_quality.map(|t| quote(t.name())),
            quote(quality.tier.name()),
        );
        if let Some(calibration) = &self.calibration {
            out.push_str(
                "\n# What Auto measured on this machine; remove the section to measure again.\n[calibration]\n",
            );
            let _ = writeln!(out, "tier = {}", quote(calibration.tier.name()));
            let _ = writeln!(out, "gpu_ms = {:?}", calibration.gpu_ms);
            let _ = writeln!(out, "adapter = {}", quote(&calibration.adapter));
            let _ = writeln!(out, "driver = {}", quote(&calibration.driver));
            let _ = writeln!(out, "workload = {}", calibration.workload);
        }
        out
    }

    /// Reads the text of a settings file ([`GraphicsSettings::to_text`]'s format): `key = value` lines, `#`
    /// comments, and a `[calibration]` section. Keys left out take the preset's values. Fails on the first line that
    /// is not understood (an unknown key, a value of the wrong kind), so a damaged file is noticed rather than half
    /// read.
    pub fn from_text(text: &str) -> Result<Self, SettingsError> {
        let mut settings = Self::default();
        let mut section = Section::Main;
        let (mut tier, mut gpu_ms, mut adapter, mut driver, mut workload) = (None, None, None, None, None);
        for (index, raw) in text.lines().enumerate() {
            let at = index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = match name.trim() {
                    "calibration" => Section::Calibration,
                    other => return Err(SettingsError::new(at, format!("unknown section `[{other}]`"))),
                };
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| SettingsError::new(at, "expected `key = value`"))?;
            let key = key.trim();
            let value = Value::parse(value.trim()).map_err(|reason| SettingsError::new(at, reason))?;
            let bad = |what: &str| SettingsError::new(at, format!("`{key}` needs {what}"));
            match (section, key) {
                (Section::Main, "preset") => {
                    settings.preset = value
                        .text()
                        .and_then(|name| name.parse().ok())
                        .ok_or_else(|| bad("a preset (low, medium, high, ultra or auto)"))?;
                }
                (Section::Main, "render_scale") => {
                    settings.render_scale = Some(value.number().ok_or_else(|| bad("a number"))?)
                }
                (Section::Main, "dynamic_resolution") => {
                    settings.dynamic_resolution = Some(value.boolean().ok_or_else(|| bad("true or false"))?);
                }
                (Section::Main, "target_frame_ms") => {
                    settings.target_frame_ms = Some(value.number().ok_or_else(|| bad("a number"))?);
                }
                (Section::Main, "min_render_scale") => {
                    settings.min_render_scale = Some(value.number().ok_or_else(|| bad("a number"))?);
                }
                (Section::Main, "max_render_scale") => {
                    settings.max_render_scale = Some(value.number().ok_or_else(|| bad("a number"))?);
                }
                (Section::Main, "vsync") => settings.vsync = Some(value.boolean().ok_or_else(|| bad("true or false"))?),
                (Section::Main, "frame_cap") => {
                    settings.frame_cap = Some(value.count().ok_or_else(|| bad("a whole number"))?);
                }
                (Section::Main, "view_distance") => {
                    settings.view_distance = Some(value.number().ok_or_else(|| bad("a number"))?);
                }
                (Section::Main, "shadow_quality") => {
                    settings.shadow_quality = Some(value.tier().ok_or_else(|| bad("low, medium, high or ultra"))?);
                }
                (Section::Main, "shadow_distance") => {
                    settings.shadow_distance = Some(value.number().ok_or_else(|| bad("a number"))?);
                }
                (Section::Main, "light_budget") => {
                    settings.light_budget = Some(value.count().ok_or_else(|| bad("a whole number"))?);
                }
                (Section::Main, "lod_bias") => settings.lod_bias = Some(value.number().ok_or_else(|| bad("a number"))?),
                (Section::Main, "anti_aliasing") => {
                    settings.anti_aliasing = Some(
                        value
                            .text()
                            .and_then(|name| name.parse().ok())
                            .ok_or_else(|| bad("off, mlaa, msaa2, msaa4 or msaa8"))?,
                    );
                }
                (Section::Main, "effect_quality") => {
                    settings.effect_quality = Some(value.tier().ok_or_else(|| bad("low, medium, high or ultra"))?);
                }
                (Section::Calibration, "tier") => tier = Some(value.tier().ok_or_else(|| bad("a tier"))?),
                (Section::Calibration, "gpu_ms") => gpu_ms = Some(value.number().ok_or_else(|| bad("a number"))?),
                (Section::Calibration, "adapter") => {
                    adapter = Some(value.text().ok_or_else(|| bad("a quoted string"))?.to_owned());
                }
                (Section::Calibration, "driver") => {
                    driver = Some(value.text().ok_or_else(|| bad("a quoted string"))?.to_owned());
                }
                (Section::Calibration, "workload") => {
                    workload = Some(value.count().ok_or_else(|| bad("a whole number"))?);
                }
                _ => return Err(SettingsError::new(at, format!("unknown setting `{key}`"))),
            }
        }
        if let Some(tier) = tier {
            settings.calibration = Some(Calibration {
                tier,
                gpu_ms: gpu_ms.unwrap_or(0.0),
                adapter: adapter.unwrap_or_default(),
                driver: driver.unwrap_or_default(),
                // A file from before #139: its calibration timed a game, and is measured again.
                workload: workload.unwrap_or(0),
            });
        }
        Ok(settings)
    }
}

/// Why a settings file could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct SettingsError {
    /// The line, from 1.
    pub line: usize,
    /// What is wrong with it.
    pub reason: String,
}

impl SettingsError {
    fn new(line: usize, reason: impl Into<String>) -> Self {
        Self {
            line,
            reason: reason.into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Main,
    Calibration,
}

/// One value of the settings file.
enum Value {
    Text(String),
    Number(f64),
    Boolean(bool),
}

impl Value {
    /// A quoted string (with `\"` and `\\` escapes), `true`, `false` or a number, then optionally a `#` comment.
    fn parse(text: &str) -> Result<Self, String> {
        if let Some(rest) = text.strip_prefix('"') {
            let mut value = String::new();
            let mut chars = rest.chars();
            loop {
                match chars.next() {
                    None => return Err("a string without its closing quote".to_owned()),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '\\')) => value.push(c),
                        _ => return Err("only \\\" and \\\\ are escapes".to_owned()),
                    },
                    Some(c) => value.push(c),
                }
            }
            let after = chars.as_str().trim();
            if !after.is_empty() && !after.starts_with('#') {
                return Err(format!("unexpected `{after}` after the string"));
            }
            return Ok(Value::Text(value));
        }
        let text = text.split('#').next().unwrap_or_default().trim();
        match text {
            "true" => Ok(Value::Boolean(true)),
            "false" => Ok(Value::Boolean(false)),
            _ => text
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(Value::Number)
                .ok_or_else(|| format!("`{text}` is not a number, true, false or a quoted string")),
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    fn number(&self) -> Option<f32> {
        match *self {
            Value::Number(n) => Some(n as f32),
            _ => None,
        }
    }

    fn count(&self) -> Option<u32> {
        match *self {
            Value::Number(n) if n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) => Some(n as u32),
            _ => None,
        }
    }

    fn boolean(&self) -> Option<bool> {
        match *self {
            Value::Boolean(b) => Some(b),
            _ => None,
        }
    }

    fn tier(&self) -> Option<Tier> {
        self.text().and_then(|name| name.parse().ok())
    }
}

/// `text` as a quoted string of the settings file.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        // A line break would end the line; the file has no escape for it.
        out.push(if c == '\n' || c == '\r' { ' ' } else { c });
    }
    out.push('"');
    out
}

/// The frame time calibration measures against: a 60 fps frame.
pub const CALIBRATION_BUDGET_MS: f32 = 1000.0 / 60.0;
/// Workload frames drawn at a tier, once its settings are drawn, before its frames are timed: the first ones draw
/// every shadow map and the cached cascade's static map again.
pub const CALIBRATION_SKIP: u64 = 4;
/// Timed workload frames per tier: their lower quartile is the tier's cost.
pub const CALIBRATION_SAMPLES: usize = 16;
/// The most workload frames drawn after one game frame. The first [`crate::TIMING_RING_SIZE`] of them are timed;
/// the rest keep the GPU busy, so its clocks are those of a loaded GPU.
pub const CALIBRATION_MAX_BATCH: u32 = 8;

/// The share of [`CALIBRATION_BUDGET_MS`] each tier's frames may take to be chosen: Ultra with twice the headroom
/// ADR 0008 asks of the High tier's machine, Medium with a fifth to spare. Low is what is left.
fn calibration_share(tier: Tier) -> f32 {
    match tier {
        Tier::Ultra => 0.5,
        Tier::High => 0.6,
        Tier::Medium => 0.8,
        Tier::Low => f32::INFINITY,
    }
}

/// What a [`Calibrator`] asks for after a workload frame's timings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalibrationStep {
    /// Keep drawing the workload at [`Calibrator::probe`].
    Measuring,
    /// The tier measured did not fit: draw the workload at this cheaper one from now on, and say when its settings
    /// are drawn ([`Calibrator::probe_drawn_from`]).
    Probe(Tier),
    /// Calibrated: the tier chosen and the GPU frame time it was measured at, the lower quartile of its frames (0
    /// when it was not measured: Low is chosen without measuring when Medium does not fit).
    Done(Tier, f32),
}

/// The Auto preset's calibration: times a fixed workload at Ultra, then cheaper tiers, and chooses the first whose
/// frames fit its share of a 60 fps frame (#139). It never sees the game's frames, so the tier does not depend on the
/// game being played; the engine draws the workload offscreen after the game's frames and feeds this each workload
/// frame's GPU time ([`Calibrator::observe`]). It never waits for the GPU and allocates nothing.
///
/// A tier counts from [`CALIBRATION_SKIP`] frames after its settings are drawn ([`Calibrator::probe_drawn_from`]; a
/// switch waits for its pipelines and textures, #86 and #152), then takes [`CALIBRATION_SAMPLES`] frames, fewer when
/// more than three quarters of them are already over its share. Its cost is the **lower quartile** of those frames,
/// not their median: the same frames of the same scene at the same settings only take longer when something else
/// slows them, a GPU whose clocks are still rising from idle (a light game at 60 fps leaves it idle most of the time)
/// or other work, so the faster frames are the truer measure, and a quartile rather than the fastest keeps one odd
/// frame from deciding. [`Calibrator::batch`] says how many workload frames to draw back to back after a game frame,
/// filling the GPU time the game leaves.
#[derive(Clone, Debug)]
pub struct Calibrator {
    probe: Tier,
    /// The first workload frame whose timings count for the probe; `None` until its settings are drawn.
    from_frame: Option<u64>,
    samples: [f32; CALIBRATION_SAMPLES],
    len: usize,
    /// Samples over the probe's share.
    over: usize,
    /// The newest workload frame taken in.
    last_frame: Option<u64>,
    /// Its GPU time, which sizes the batches.
    last_ms: Option<f32>,
}

impl Default for Calibrator {
    fn default() -> Self {
        Self::new()
    }
}

impl Calibrator {
    /// A calibration starting at Ultra, waiting for the workload to be drawn at it
    /// ([`Calibrator::probe_drawn_from`]).
    pub fn new() -> Self {
        Self {
            probe: Tier::Ultra,
            from_frame: None,
            samples: [0.0; CALIBRATION_SAMPLES],
            len: 0,
            over: 0,
            last_frame: None,
            last_ms: None,
        }
    }

    /// The tier to draw the workload at now.
    pub fn probe(&self) -> Tier {
        self.probe
    }

    /// Whether the workload is drawn at the probe's settings (its switch is done), so its frames are being timed.
    pub fn settled(&self) -> bool {
        self.from_frame.is_some()
    }

    /// Says the workload is drawn at the probe's settings from workload frame `frame` on: the frames from
    /// [`CALIBRATION_SKIP`] later count. Only the first call after each probe counts.
    pub fn probe_drawn_from(&mut self, frame: u64) {
        self.from_frame.get_or_insert(frame + CALIBRATION_SKIP);
    }

    /// How many workload frames to draw back to back after this game frame, given `room_ms`, the GPU time the game's
    /// frames leave between two of them: one while the probe's switch is being made (its textures are made one a
    /// frame, #152) or before any frame is timed, otherwise as many as fill the room at the newest frame's GPU time
    /// (to the nearest frame), from 1 to [`CALIBRATION_MAX_BATCH`]. Filling it keeps the GPU busy, so its clocks rise
    /// as they would for a game that needs them.
    pub fn batch(&self, room_ms: f32) -> u32 {
        match self.last_ms {
            Some(ms) if self.settled() && ms > 0.0 && room_ms.is_finite() => {
                ((room_ms / ms).round().max(0.0) as u32).clamp(1, CALIBRATION_MAX_BATCH)
            }
            _ => 1,
        }
    }

    /// Takes in workload frame `frame`'s GPU time in milliseconds, and says what to do next. Feed the frames in
    /// order: a frame not after the newest one taken in is ignored, as are frames drawn before the probe counts.
    pub fn observe(&mut self, frame: u64, gpu_ms: f32) -> CalibrationStep {
        if !gpu_ms.is_finite() || gpu_ms < 0.0 || self.last_frame.is_some_and(|last| last >= frame) {
            return CalibrationStep::Measuring;
        }
        self.last_frame = Some(frame);
        self.last_ms = Some(gpu_ms);
        if self.from_frame.is_none_or(|from| frame < from) {
            return CalibrationStep::Measuring;
        }
        let limit = calibration_share(self.probe) * CALIBRATION_BUDGET_MS;
        if let Some(slot) = self.samples.get_mut(self.len) {
            *slot = gpu_ms;
            self.len += 1;
            self.over += usize::from(gpu_ms > limit);
        }
        // The lower quartile is the sorted samples' quarter-way one: once more than three quarters are over, it is too.
        let failed = self.over >= CALIBRATION_SAMPLES - CALIBRATION_SAMPLES / 4;
        if self.len < CALIBRATION_SAMPLES && !failed {
            return CalibrationStep::Measuring;
        }
        if !failed {
            let samples = &mut self.samples[..self.len];
            samples.sort_unstable_by(f32::total_cmp);
            let quartile = samples[samples.len() / 4];
            if quartile <= limit {
                return CalibrationStep::Done(self.probe, quartile);
            }
        }
        match self.probe.cheaper() {
            Some(Tier::Low) | None => CalibrationStep::Done(Tier::Low, 0.0),
            Some(next) => {
                self.probe = next;
                self.from_frame = None;
                self.len = 0;
                self.over = 0;
                CalibrationStep::Probe(next)
            }
        }
    }

    /// The tier for a device whose frames cannot be timed (no timestamp queries), from its kind.
    pub fn untimed_tier(device_type: wgpu::DeviceType) -> Tier {
        match device_type {
            wgpu::DeviceType::DiscreteGpu => Tier::High,
            wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => Tier::Medium,
            wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::Cpu => Tier::Low,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse_by_name_whatever_the_case() {
        assert_eq!("HIGH".parse(), Ok(Preset::High));
        for preset in Preset::ALL {
            assert_eq!(preset.name().parse(), Ok(*preset));
            assert_eq!(preset.to_string(), preset.name());
        }
        let error = "epic".parse::<Preset>().unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown preset `epic` (presets: low, medium, high, ultra, auto)"
        );
    }

    #[test]
    fn presets_scale_cost_and_never_turn_shadows_or_lights_off() {
        let mut previous: Option<Quality> = None;
        for tier in Tier::ALL {
            let q = Quality::for_tier(tier);
            assert_eq!(q.tier, tier);
            assert!(q.shadows.enabled && q.shadows.cascade_count() >= 1, "{tier:?}");
            assert!(q.lights.enabled && q.lights.light_budget() >= 32, "{tier:?}");
            assert!(q.lights.shadows && q.lights.shadowed_lights >= 2, "{tier:?}");
            assert!(q.cull.enabled && q.lod.enabled, "{tier:?}");
            if let Some(p) = previous {
                // Each tier costs at least as much as the one below it, everywhere.
                assert!(q.resolution.scale >= p.resolution.scale, "{tier:?}");
                assert!(q.cull.max_distance >= p.cull.max_distance, "{tier:?}");
                assert!(q.cull.min_screen_size <= p.cull.min_screen_size, "{tier:?}");
                assert!(q.shadows.cascade_count() >= p.shadows.cascade_count(), "{tier:?}");
                assert!(q.shadows.resolution >= p.shadows.resolution, "{tier:?}");
                assert!(q.shadows.distance >= p.shadows.distance, "{tier:?}");
                assert!(q.lights.budget >= p.lights.budget, "{tier:?}");
                assert!(q.lights.atlas_resolution >= p.lights.atlas_resolution, "{tier:?}");
                assert!(q.lod.bias >= p.lod.bias, "{tier:?}");
                assert!(
                    q.anti_aliasing.msaa_samples() >= p.anti_aliasing.msaa_samples(),
                    "{tier:?}"
                );
                assert!(q.post.bloom.start.divisor() <= p.post.bloom.start.divisor(), "{tier:?}");
                assert!(q.post.mlaa.search >= p.post.mlaa.search, "{tier:?}");
            }
            // Bloom at every tier, as bright and as wide: only its resolution and filter change.
            assert_eq!(q.post.bloom.intensity, PostSettings::HIGH.bloom.intensity, "{tier:?}");
            assert_ne!(q.anti_aliasing, AntiAliasing::Off, "{tier:?}");
            previous = Some(q);
        }
        // The shadow atlas holds every shadowed light's tiles (a point light takes six).
        let ultra = Quality::ULTRA.lights;
        let tiles = (ultra.atlas_resolution / ultra.tile_resolution).pow(2);
        assert!(ultra.shadowed_lights * 6 <= tiles.min(crate::lights::MAX_SHADOW_TILES as u32));
    }

    #[test]
    fn overrides_apply_on_top_of_the_tier() {
        let settings = GraphicsSettings {
            preset: Preset::High,
            render_scale: Some(0.6),
            view_distance: Some(300.0),
            shadow_quality: Some(Tier::Low),
            shadow_distance: Some(90.0),
            light_budget: Some(0),
            lod_bias: Some(100.0),
            anti_aliasing: Some(AntiAliasing::Off),
            effect_quality: Some(Tier::Low),
            ..GraphicsSettings::default()
        };
        let q = settings.quality();
        assert_eq!(q.anti_aliasing, AntiAliasing::Off);
        assert_eq!(q.post, PostSettings::LOW);
        assert!(q.merge_scene_passes, "every tier merges the scene passes");
        assert_eq!(q.tier, Tier::High);
        assert_eq!(q.resolution.range(), (0.6, 0.6));
        assert!(!q.resolution.dynamic);
        assert_eq!(q.cull.max_distance, 300.0);
        assert_eq!(q.shadows.cascades, 1);
        assert_eq!(q.shadows.distance, 90.0);
        assert_eq!(q.lights.budget, 1, "never off");
        assert_eq!(q.lod.bias, 8.0);
        // No view distance.
        let far = GraphicsSettings {
            view_distance: Some(0.0),
            ..GraphicsSettings::new(Preset::Low)
        };
        assert_eq!(far.quality().cull.max_distance, f32::INFINITY);
    }

    #[test]
    fn auto_uses_its_calibration_with_dynamic_resolution() {
        let mut auto = GraphicsSettings::new(Preset::Auto);
        assert_eq!(auto.tier(), Tier::Medium, "until calibrated");
        assert!(auto.needs_calibration("GPU", "1.0", 1));
        auto.calibration = Some(Calibration {
            tier: Tier::Ultra,
            gpu_ms: 3.0,
            adapter: "GPU".into(),
            driver: "1.0".into(),
            workload: 1,
        });
        assert!(!auto.needs_calibration("GPU", "1.0", 1));
        assert!(auto.needs_calibration("GPU", "2.0", 1), "a new driver");
        assert!(auto.needs_calibration("Other GPU", "1.0", 1), "another adapter");
        assert!(auto.needs_calibration("GPU", "1.0", 2), "a new workload");
        let q = auto.quality();
        assert_eq!(q.tier, Tier::Ultra);
        assert!(q.resolution.dynamic);
        assert_eq!(q.resolution.range(), (DEFAULT_MIN_SCALE, 1.0));
        assert_eq!(q.resolution.target_ms, DEFAULT_TARGET_MS);
        // A fixed preset never calibrates.
        assert!(!GraphicsSettings::new(Preset::Low).needs_calibration("GPU", "1.0", 1));
    }

    #[test]
    fn a_calibration_from_before_the_workload_reads_and_is_measured_again() {
        // #84's file: no `workload` key. It still reads (the format is the same), as workload 0.
        let text = "preset = \"auto\"\n\n[calibration]\ntier = \"ultra\"\ngpu_ms = 3.93\n\
                    adapter = \"GPU\"\ndriver = \"1.0\"\n";
        let settings = GraphicsSettings::from_text(text).expect("an old file reads");
        let calibration = settings.calibration.as_ref().expect("its calibration");
        assert_eq!((calibration.tier, calibration.workload), (Tier::Ultra, 0));
        assert!(
            settings.needs_calibration("GPU", "1.0", 1),
            "measured again on the workload"
        );
        assert_eq!(settings.tier(), Tier::Ultra, "drawn at what it measured until then");
        assert!(GraphicsSettings::from_text("[calibration]\nworkload = -1").is_err());
    }

    #[test]
    fn the_text_reads_back_as_written() {
        let settings = GraphicsSettings {
            preset: Preset::Auto,
            render_scale: Some(0.85),
            dynamic_resolution: Some(true),
            target_frame_ms: Some(2.0),
            vsync: Some(false),
            frame_cap: Some(0),
            shadow_quality: Some(Tier::Medium),
            lod_bias: Some(1.5),
            anti_aliasing: Some(AntiAliasing::Msaa(2)),
            effect_quality: Some(Tier::Ultra),
            calibration: Some(Calibration {
                tier: Tier::High,
                gpu_ms: 4.25,
                adapter: "AMD Radeon \"RX\" 6650 XT".into(),
                driver: "32.0\\1".into(),
                workload: 3,
            }),
            ..GraphicsSettings::default()
        };
        let text = settings.to_text();
        assert!(text.contains("preset = \"auto\"\n"), "{text}");
        assert!(text.contains("render_scale = 0.85\n"), "{text}");
        assert!(text.contains("# view_distance = "), "{text}");
        assert!(text.contains("anti_aliasing = \"msaa2\"\n"), "{text}");
        assert!(text.contains("effect_quality = \"ultra\"\n"), "{text}");
        assert!(
            GraphicsSettings::new(Preset::Low)
                .to_text()
                .contains("# anti_aliasing = \"mlaa\"\n")
        );
        assert_eq!(GraphicsSettings::from_text(&text), Ok(settings));
        // Every preset with nothing set reads back as itself.
        for preset in Preset::ALL {
            let text = GraphicsSettings::new(*preset).to_text();
            assert_eq!(GraphicsSettings::from_text(&text), Ok(GraphicsSettings::new(*preset)));
        }
    }

    #[test]
    fn damaged_text_is_an_error_with_its_line() {
        for (text, line, reason) in [
            ("preset = \"epic\"", 1, "`preset` needs a preset"),
            ("# a comment\nrender_scale = big", 2, "`big` is not a number"),
            ("vsync = 1", 1, "`vsync` needs true or false"),
            ("colour = \"blue\"", 1, "unknown setting `colour`"),
            ("[video]", 1, "unknown section"),
            ("preset", 1, "expected `key = value`"),
            ("preset = \"high", 1, "closing quote"),
            ("frame_cap = 1.5", 1, "a whole number"),
            ("anti_aliasing = \"fxaa\"", 1, "off, mlaa"),
            ("effect_quality = 3", 1, "low, medium"),
            ("\u{0}\u{1}garbage", 1, "expected"),
        ] {
            let error = GraphicsSettings::from_text(text).unwrap_err();
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.reason.contains(reason), "{text:?}: {error}");
        }
        assert_eq!(GraphicsSettings::from_text(""), Ok(GraphicsSettings::default()));
        let commented = "preset = \"low\" # cheap\nlod_bias = 0.5 # coarse";
        let settings = GraphicsSettings::from_text(commented).unwrap();
        assert_eq!((settings.preset, settings.lod_bias), (Preset::Low, Some(0.5)));
    }

    /// How a simulated calibration went.
    #[derive(Debug)]
    struct Run {
        step: CalibrationStep,
        /// Game frames until it was done.
        game_frames: u64,
        /// The most workload GPU time drawn after one game frame.
        max_batch_ms: f32,
    }

    /// A probe's settings are drawn this many workload frames after it is asked for (its textures are made one a
    /// frame, #152).
    const SWITCH_FRAMES: u64 = 5;

    /// Drives a calibrator as the engine does: after each game frame [`Calibrator::batch`] workload frames are drawn
    /// back to back to fill `room_ms`, the `n`th costing `workload(tier drawn, n)`, and the timings of the first
    /// [`crate::TIMING_RING_SIZE`] of each batch (the renderer's readbacks) arrive two game frames later, all at once.
    /// The first probe is drawn at once (the workload's first frame waits for its settings); a later one
    /// [`SWITCH_FRAMES`] workload frames after it is asked for, the frames between drawn at the tier before.
    fn simulate(room_ms: f32, workload: impl Fn(Tier, u64) -> f32) -> Run {
        const LATENCY: u64 = 2;
        let mut calibrator = Calibrator::new();
        // Each workload frame: the game frame it was drawn after, what it cost, and whether it was timed.
        let mut drawn: Vec<(u64, f32, bool)> = Vec::new();
        let (mut fed, mut ready_at, mut tier_drawn) = (0, 0, Tier::Ultra);
        let mut max_batch_ms = 0.0f32;
        for game_frame in 0..1000u64 {
            while let Some(&(after, ms, timed)) = drawn.get(fed)
                && after + LATENCY <= game_frame
            {
                let step = if timed {
                    calibrator.observe(fed as u64, ms)
                } else {
                    CalibrationStep::Measuring
                };
                fed += 1;
                match step {
                    CalibrationStep::Done(..) => {
                        return Run {
                            step,
                            game_frames: game_frame,
                            max_batch_ms,
                        };
                    }
                    CalibrationStep::Probe(_) => ready_at = drawn.len() as u64 + SWITCH_FRAMES,
                    CalibrationStep::Measuring => {}
                }
            }
            let mut batch_ms = 0.0;
            for n in 0..calibrator.batch(room_ms) {
                let frame = drawn.len() as u64;
                if !calibrator.settled() && frame >= ready_at {
                    tier_drawn = calibrator.probe();
                    calibrator.probe_drawn_from(frame);
                }
                let ms = workload(tier_drawn, frame);
                drawn.push((game_frame, ms, (n as usize) < crate::TIMING_RING_SIZE));
                batch_ms += ms;
            }
            max_batch_ms = max_batch_ms.max(batch_ms);
        }
        panic!("never calibrated");
    }

    #[test]
    fn calibration_takes_the_first_tier_that_fits() {
        // A fast machine: Ultra, in batches of four frames (8 ms / 2 ms), within a sixth of a second at 60 Hz.
        let run = simulate(8.0, |_, _| 2.0);
        assert_eq!(run.step, CalibrationStep::Done(Tier::Ultra, 2.0));
        assert!(run.game_frames <= 10, "{run:?}");

        // 12 ms at Ultra, 9 at High (60% of 16.7 is 10).
        let run = simulate(8.0, |tier, _| match tier {
            Tier::Ultra => 12.0,
            _ => 9.0,
        });
        assert_eq!(run.step, CalibrationStep::Done(Tier::High, 9.0));

        // Too slow everywhere: Low, after measuring Ultra, High and Medium, each given up once more than three
        // quarters of its samples are over (its lower quartile cannot fit then).
        let run = simulate(8.0, |_, _| 40.0);
        assert_eq!(run.step, CalibrationStep::Done(Tier::Low, 0.0));
        let per_tier = CALIBRATION_SKIP + (CALIBRATION_SAMPLES - CALIBRATION_SAMPLES / 4) as u64 + SWITCH_FRAMES;
        assert!(run.game_frames <= 3 * per_tier, "{run:?}");
    }

    #[test]
    fn frames_before_a_probe_do_not_count_for_it() {
        // Ultra is slow, High fast: High's measurement must not include Ultra's frames still arriving late, nor the
        // frames drawn at Ultra while High's switch is made.
        let run = simulate(8.0, |tier, _| if tier == Tier::Ultra { 30.0 } else { 5.0 });
        assert_eq!(run.step, CalibrationStep::Done(Tier::High, 5.0));
    }

    #[test]
    fn frames_slowed_by_rising_clocks_do_not_decide() {
        // A GPU coming up from idle clocks: the first ten timed frames take four times as long. Their median would
        // be 10 ms (over Ultra's 8.3); the lower quartile is the loaded GPU's 2.5.
        let first_timed = CALIBRATION_SKIP;
        let run = simulate(1.0, |_, frame| if frame < first_timed + 10 { 10.0 } else { 2.5 });
        assert_eq!(run.step, CalibrationStep::Done(Tier::Ultra, 2.5));
    }

    #[test]
    fn batches_fill_the_room_the_game_leaves() {
        for room in [0.0, 4.0, 14.0] {
            for ms in [0.5, 1.0, 2.5, 3.0, 7.9, 20.0] {
                let run = simulate(room, |_, _| ms);
                let frames = (run.max_batch_ms / ms).round() as u32;
                // To the nearest frame, one at least, eight at most.
                let fill = ((room / ms).round() as u32).clamp(1, CALIBRATION_MAX_BATCH);
                assert_eq!(frames, fill, "{ms} ms in {room} ms: {run:?}");
                assert!(
                    run.max_batch_ms <= room + ms / 2.0 || frames == 1,
                    "{ms} ms in {room} ms: {run:?}"
                );
            }
        }
        let mut calibrator = Calibrator::new();
        assert_eq!(calibrator.batch(14.0), 1, "until a frame is timed");
        calibrator.probe_drawn_from(0);
        calibrator.observe(0, 1.0);
        assert_eq!(calibrator.batch(14.0), CALIBRATION_MAX_BATCH);
        calibrator.observe(1, 3.0);
        assert_eq!(calibrator.batch(14.0), 5);
        assert_eq!(calibrator.batch(0.0), 1, "one a frame however little room there is");
        assert_eq!(calibrator.batch(f32::NAN), 1);
        // Timings that are not a time, or not newer, change nothing.
        for (frame, ms) in [(2, f32::NAN), (3, -1.0), (1, 0.1)] {
            assert_eq!(calibrator.observe(frame, ms), CalibrationStep::Measuring);
        }
        assert_eq!(calibrator.batch(14.0), 5);
    }
}
