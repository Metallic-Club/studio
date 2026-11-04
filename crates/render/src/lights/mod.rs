//! Point and spot lights: clustered forward lighting with a per-frame budget, and a shadow atlas (ADR 0008 §6).
//!
//! # From the scene to the frame
//!
//! The scene keeps every `PointLight` and `SpotLight` in the world as a record with its world placement
//! ([`crate::scene::SceneLight`], [`crate::InstanceScene::lights`]), from the same change consumer as the parts. Each
//! frame [`crate::Renderer::render_scene`] then, on the CPU:
//!
//! 1. **Selects** the lights to draw. A light that shines (`Enabled`, some `Brightness`, `Color` and `Range`) and
//!    whose sphere of influence (its range; for a spot, the sphere around its cone) reaches into the camera's view
//!    is a candidate. Candidates are ranked by **importance**: brightness × the colour's luminance × how large the
//!    sphere looks from the camera (its radius over its distance, 1 from inside it). The first
//!    [`LightSettings::budget`] are in the budget; lights already in it count [`BUDGET_HYSTERESIS`] times their
//!    importance, so two lights of about the same importance do not trade places every frame.
//! 2. **Fades**: a light entering the budget fades in and one leaving it fades out over [`LightSettings::fade_time`]
//!    seconds, so the budget never pops a light on or off. (What the game does is not faded: a light switched off,
//!    made or destroyed by a script changes at once, as it should.) A light seen for the first time, or again after
//!    being out of view, starts at its target.
//! 3. **Clusters** the drawn lights into a froxel grid: the view is cut into [`CLUSTERS_X`] × [`CLUSTERS_Y`] tiles
//!    and [`CLUSTERS_Z`] depth slices (exponential from [`FIRST_SLICE`] metres to the farthest light), and each
//!    cluster lists the lights whose sphere touches its box, most important first, at most
//!    [`LightSettings::per_cluster`] of them. A pixel loops over its cluster's list only.
//! 4. **Shadows**: the most important shadowed lights in the budget (`Shadows` on), at most
//!    [`LightSettings::shadowed_lights`], get tiles of the **shadow atlas**: one depth texture of
//!    [`LightSettings::atlas_resolution`]² cut into tiles of [`LightSettings::tile_resolution`]². A spot light
//!    takes one tile, a point light six (a cube's faces). Each tile culls its casters with the scene's culling grid
//!    and draws them into its square of the atlas (one draw per mesh kind and level of detail), in one pass for the
//!    whole atlas. A light's shadow fades in when it
//!    gets its tiles and out when it loses them, like the light itself.
//!
//! Selection and clustering run on the CPU every frame: a few hundred lights take a fraction of a millisecond
//! (`docs/perf.md`), and the same code serves every backend, GLES included, with no compute pass. Their time is
//! [`LightStats::cpu_time`].
//!
//! # Shading
//!
//! Each light adds Lambert diffuse and GGX specular, like the sun, with a **physically based falloff that reaches 0
//! at `Range`**: inverse-square, windowed by `(1 − (d / Range)⁴)²` (the falloff of Frostbite and Unreal), from a
//! source [`MIN_DISTANCE`] across so it stays finite at the light. `Brightness` 1 is [`LIGHT_INTENSITY`]: a white
//! surface 4 m from such a light, facing it, is lit about as brightly as one in the noon sun's shade, and the
//! light's pool is clearly visible at night out to half its range. A spot light's cone falls off smoothly over the
//! outer [`SPOT_SOFTNESS`] of its half-angle.
//!
//! # Shadows in detail
//!
//! Each tile is a perspective view from the light, slightly wider than its face (or cone) so that the filter's taps
//! near an edge stay inside the tile; the parts look the tile up by the major axis of the light-to-point direction
//! (point lights) and compare through a 3 × 3 tent of bilinear comparison taps. A tile holds its casters' faces
//! towards the light, pushed back by a slope-scaled rasterizer bias, so a part resting on another shadows it right up
//! to where they touch (#126); the receivers add a small normal offset and depth bias that grow with the distance
//! from the light, at most [`crate::shadow::MAX_RECEIVER_OFFSET`] (see [`LightSettings::depth_bias`]). The part a
//! light sits in casts no shadow for it (a bulb inside a lamp shines through its glass).
//!
//! **Static caching**: a tile is drawn again only when
//!
//! - it is new to its light, or its light moved, turned or changed range or angle;
//! - the camera's distance from the light plus the light's range crossed a power of two, which is what its round
//!   casters' levels of detail follow ([`crate::lod`]: a tile's shadow is never finer than what receives it);
//! - a static caster changed inside it ([`crate::InstanceScene::shadow_changes`]) or the scene was rebased or rebuilt
//!   ([`crate::InstanceScene::shadow_epoch`]);
//! - the scene changed and a moving caster (one in the dynamic region) is inside it now or was at its last drawing.
//!
//! Lights out of view or out of the budget keep their tiles (and their drawings) until another light needs the
//! room, so looking away and back redraws nothing.
//!
//! # On every backend
//!
//! The lights reach the fragment shader in a **uniform array** of [`MAX_GPU_LIGHTS`] (exactly the 16 KiB every
//! backend, WebGL 2 included, guarantees for a uniform block), and the cluster grid as an `R32Uint` **texture**: a
//! header per cluster (where its list starts and how long it is) and the lists packed four 8-bit light indices to a
//! texel. Neither needs storage buffers in fragment shaders, which GLES-class devices may not have, so both instance
//! paths ([`crate::scene::InstanceBinding`]) light exactly alike. The shadow tiles' matrices are a second uniform
//! array ([`MAX_SHADOW_TILES`]).
//!
//! # Memory
//!
//! A scene without local lights holds none of their GPU memory, and binds and draws what it did before they
//! existed. The pipelines are made with the renderer (so the first light costs no compile, ADR 0008 §9), but the
//! lights' uniform array (16 KiB), the shadow tiles' (6 KiB) and the cluster texture (232 KiB) are made on the first
//! frame that draws a light, and the shadow atlas (16 MiB at Medium) with its casters' uniform (16 KiB) on the first
//! frame with a shadow, at the size the settings say then.
//!
//! `DirectionalLight` instances (extra suns) are not drawn yet: see #123.

mod atlas;
mod cluster;
mod gpu;
mod select;

use std::time::Duration;

pub use cluster::{CLUSTER_COUNT, CLUSTERS_X, CLUSTERS_Y, CLUSTERS_Z, FIRST_SLICE};
pub(crate) use gpu::{FrameLights, LightBindings, LocalLights};

/// The radiant intensity of `Brightness` 1, in the renderer's linear units: at 4 m it lights a surface facing it
/// with 1 (a white one then reflects 1/π, about the shade at noon).
pub const LIGHT_INTENSITY: f32 = 16.0;

/// The size of a light's source in metres: the falloff is `1 / max(d², MIN_DISTANCE²)`, so it stays finite at the
/// light.
pub const MIN_DISTANCE: f32 = 0.1;

/// The share of a spot light's half-angle over which its cone fades out to its edge.
pub const SPOT_SOFTNESS: f32 = 0.2;

/// How many times its importance a light already in the budget counts, so lights near the budget's edge do not
/// trade places every frame.
pub const BUDGET_HYSTERESIS: f32 = 1.25;

/// The most lights a frame draws: in the budget plus fading out. The size of the lights' uniform array (16 KiB).
pub const MAX_GPU_LIGHTS: usize = 256;

/// The most lights one cluster lists ([`LightSettings::per_cluster`] is clamped to it).
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 64;

/// The most shadow-atlas tiles: the size of the tiles' uniform array.
pub const MAX_SHADOW_TILES: usize = 64;

/// How local lights are drawn, which the graphics presets drive ([`crate::settings::Quality`]): [`LightSettings::LOW`], [`LightSettings::MEDIUM`]
/// (the default) and [`LightSettings::HIGH`]. Presets scale the cost (how many lights a frame draws, how many cast
/// shadows, how sharp the shadows are), never whether a game's lights shine or cast shadows at all: that is each
/// light's `Enabled` and `Shadows`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightSettings {
    /// Draw local lights at all. Off only for tools and measurements: a preset never turns a game's lights off.
    pub enabled: bool,
    /// The lights a frame draws at full strength, the most important ones (at most [`MAX_GPU_LIGHTS`]). Lights
    /// leaving the budget fade out on top of it.
    pub budget: u32,
    /// The most lights one cluster lists (at most [`MAX_LIGHTS_PER_CLUSTER`]): a safety cap, sized so it is not
    /// reached in practice. Where it is, the cluster keeps its most important lights.
    pub per_cluster: u32,
    /// Seconds a light takes to fade in or out when it enters or leaves the budget (and a shadow when it gets or
    /// loses its tiles).
    pub fade_time: f32,
    /// Draw local lights' shadows at all (for lights with `Shadows` on).
    pub shadows: bool,
    /// The most lights with shadows in a frame, the most important ones.
    pub shadowed_lights: u32,
    /// The shadow atlas is this many texels square (clamped to the device's texture limit).
    pub atlas_resolution: u32,
    /// Each tile is this many texels square. The atlas holds `(atlas / tile)²` tiles, at most [`MAX_SHADOW_TILES`].
    pub tile_resolution: u32,
    /// The comparison filter's reach in texels: 0 is one bilinear tap, 1 a 3 × 3 tent of taps.
    pub filter_radius: u32,
    /// The depth bias of a receiving point, in texels of its tile at its distance, at most
    /// [`crate::shadow::MAX_RECEIVER_OFFSET`] metres (against shadow acne).
    ///
    /// Every preset uses 0.5, with [`LightSettings::normal_offset`] 1. The atlas holds the casters' faces towards
    /// the light, drawn with a slope-scaled rasterizer bias of 2 texels of their depth slope (the shadow taps reach
    /// 1.5 texels from the point, so a lit surface is never compared with its own depth more than 1.5 texels of
    /// slope away), and that bias alone keeps every lit surface of the image tests clear of acne at all three
    /// presets (`tests/lights.rs`: a cube's top, a 30° ramp, the floor, under a spot, a near point light and a far
    /// one). The receivers' offsets are the margin on top: half a texel of depth for depth precision, a texel along
    /// the normal for grazing surfaces. They used to be 1.5 and 1.5 against the back faces the atlas held then;
    /// those offsets moved a lookup 15 to 25 cm a few metres from a light with 256² tiles and lit a band where a part
    /// rests on another (#126). High's 512² tiles halve every one of these lengths; Low and Medium share 256² tiles.
    pub depth_bias: f32,
    /// How far a receiving point moves along its normal before the lookup, in texels of its tile at its distance,
    /// scaled by `1 − N·L` (a surface facing the light needs none, a grazing one all of it) and at most
    /// [`crate::shadow::MAX_RECEIVER_OFFSET`] metres. See [`LightSettings::depth_bias`] for the values.
    pub normal_offset: f32,
}

impl LightSettings {
    /// Low (Iris Xe laptops, mid-range phones): 32 lights a frame, 2 of them shadowed in a 1024² atlas of 256²
    /// tiles.
    pub const LOW: Self = Self {
        enabled: true,
        budget: 32,
        per_cluster: 32,
        fade_time: 0.3,
        shadows: true,
        shadowed_lights: 2,
        atlas_resolution: 1024,
        tile_resolution: 256,
        filter_radius: 1,
        depth_bias: 0.5,
        normal_offset: 1.0,
    };

    /// Medium (GTX 1650 / RX 6500 class at 60 fps, the Mid tier of ADR 0008): 128 lights a frame, at most 48 per
    /// cluster, 8 of them shadowed in a 2048² atlas of 256² tiles (16 MiB).
    pub const MEDIUM: Self = Self {
        budget: 128,
        per_cluster: 48,
        shadowed_lights: 8,
        atlas_resolution: 2048,
        ..Self::LOW
    };

    /// High (RTX 3060 Ti class): 192 lights a frame, at most 64 per cluster, 8 of them shadowed in a 4096² atlas of
    /// 512² tiles (64 MiB).
    pub const HIGH: Self = Self {
        budget: 192,
        per_cluster: 64,
        atlas_resolution: 4096,
        tile_resolution: 512,
        ..Self::MEDIUM
    };

    /// No local lights (for tools and measurements).
    pub const OFF: Self = Self {
        enabled: false,
        ..Self::MEDIUM
    };

    /// The budget as drawn: at most [`MAX_GPU_LIGHTS`], 0 when off.
    pub fn light_budget(&self) -> usize {
        if self.enabled {
            (self.budget as usize).min(MAX_GPU_LIGHTS)
        } else {
            0
        }
    }

    /// The per-cluster cap as drawn: 1 to [`MAX_LIGHTS_PER_CLUSTER`].
    pub fn cluster_cap(&self) -> u32 {
        self.per_cluster.clamp(1, MAX_LIGHTS_PER_CLUSTER)
    }
}

impl Default for LightSettings {
    /// [`LightSettings::MEDIUM`].
    fn default() -> Self {
        Self::MEDIUM
    }
}

/// What the last frame did for local lights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightStats {
    /// Local lights in the world.
    pub in_scene: u32,
    /// Of those, the ones that shine and reach into the view: the candidates for the budget.
    pub in_view: u32,
    /// Candidates in the budget.
    pub in_budget: u32,
    /// Lights drawn: in the budget, plus those fading out.
    pub drawn: u32,
    /// Of the drawn lights, those fading in or out.
    pub fading: u32,
    /// Lights drawn with shadows (holding atlas tiles, fading included).
    pub shadowed: u32,
    /// Atlas tiles in use by drawn lights.
    pub shadow_tiles: u32,
    /// Atlas tiles drawn this frame (the others were kept).
    pub tiles_redrawn: u32,
    /// Casters drawn into the atlas this frame.
    pub shadow_casters: u32,
    /// Clusters with at least one light.
    pub lit_clusters: u32,
    /// Light entries over every cluster.
    pub cluster_entries: u32,
    /// The most lights in one cluster.
    pub max_per_cluster: u32,
    /// Light entries dropped because a cluster was full ([`LightSettings::per_cluster`]).
    pub overflow: u32,
    /// GPU memory local lights hold, in bytes: none until a frame first draws a light, and the shadow atlas only
    /// once a drawn light first has a shadow (see the [module documentation](self)).
    pub gpu_bytes: u64,
    /// CPU time of selecting, clustering and packing the lights.
    pub cpu_time: Duration,
    /// CPU time of the shadow tiles' caster culls.
    pub shadow_cull_time: Duration,
}

impl LightStats {
    /// The whole CPU time local lights took: [`LightStats::cpu_time`] and [`LightStats::shadow_cull_time`].
    pub fn total_cpu_time(&self) -> Duration {
        self.cpu_time + self.shadow_cull_time
    }
}

/// The renderer's per-light state, kept beside each light in the scene ([`crate::InstanceScene`]): how far it has
/// faded and which atlas tiles it holds.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LightRuntime {
    /// How much of the light is drawn, 0 to 1.
    pub(crate) fade: f32,
    /// The fade has a value (the light was seen); otherwise it starts at its target.
    pub(crate) seen: bool,
    /// The fade started this frame (the light just appeared): its shadow starts at full strength too.
    pub(crate) fresh: bool,
    /// The light was in the budget last frame.
    pub(crate) in_budget: bool,
    /// How much of its shadow is drawn, 0 to 1.
    pub(crate) shadow_fade: f32,
    /// The atlas tiles it holds, the first `tile_count` of them (one per cube face for a point light).
    pub(crate) tiles: [u16; 6],
    pub(crate) tile_count: u8,
}
