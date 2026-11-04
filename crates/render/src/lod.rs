//! Level of detail for the primitive meshes (ADR 0008 §5): a round part far away is drawn with fewer triangles.
//!
//! # Levels
//!
//! Only the round primitives have levels; a Block (12 triangles) and a Wedge (8) have nothing to drop. Each level
//! halves the segments around (and a Ball's rings), so every level's vertices are a subset of the finer level's:
//!
//! | Kind | Levels: segments (triangles) |
//! |---|---|
//! | Ball | 32 × 16 rings (960), 16 × 8 (224), 8 × 4 (48), 4 × 2 (8) |
//! | Cylinder | 32 (128), 16 (64), 8 (32) |
//! | Block, Wedge | one level |
//!
//! A cylinder's fourth level (a square prism, 16 triangles) would save 16 triangles a part and cost a draw, so it
//! has three.
//!
//! # Choosing a level
//!
//! A level is chosen by the part's **projected size**, the same measure screen-size culling uses ([`crate::cull`]):
//! the projected diameter in pixels of its bounding sphere. A level with `n` segments around a circle of radius `R`
//! pixels strays at most `R (1 − cos(π / n))` pixels from it (the sagitta of one segment), so each level is used while
//! that error stays under [`PIXEL_ERROR`] pixels, divided by the [`LodSettings::bias`]. The bounding sphere of a Ball
//! is √3 times its radius and that of a Cylinder at least √2 times (its box's diagonal), so the thresholds hold for
//! the roundest case of each.
//!
//! **Hysteresis**: a part goes to a coarser level only once its size is [`HYSTERESIS`] (10%) below the threshold,
//! and back only once it is 10% above it, so a part hovering at a threshold, or a camera stepping back and forth,
//! switches once, not every frame. Each part remembers the level the camera's view drew it at in the culling grid,
//! where the cull already computes what it needs (its distance), so choosing costs a few multiplies per round part
//! in view and nothing for blocks and wedges: a grid cell that has held no round part culls exactly as before.
//!
//! # Draws
//!
//! A mesh kind's visible instances are listed per level, each level front to back, and the levels of a kind follow
//! each other in its list segment, so a kind draws one instanced draw per level it uses: at most 1 + 4 + 3 + 1 = 9
//! opaque draws for the built-in kinds, whatever the part count. See-through parts take their level the same way;
//! sorted back to front, they draw one draw per run of the same mesh and level.
//!
//! # Shadows
//!
//! A shadow map is never finer than the surface that receives it. Every level is inscribed in the finer ones (its
//! vertices are theirs), so a coarser caster lies inside the finer surface and the drawn surface is in front of its
//! own shadow, never behind it; the other way round, a finer caster in the map would stand up to one segment's
//! sagitta in front of a coarse receiver and shadow its own lit side (acne the receivers' small biases cannot hide).
//!
//! So each shadow view draws a caster at the **coarsest level the camera could be drawing it at** where that view's
//! shadow is looked up, which needs no state and no camera position:
//!
//! - a **sun cascade** is looked up by points whose view depth is within its slice, so no farther from the camera
//!   than the slice's far depth over the cosine of the view's corner angle ([`CullView::with_caster_lod`]); a caster
//!   of radius `r` is chosen as if its centre were that far plus `r`, at the coarse side of the hysteresis band;
//! - a **local light's tile** is looked up by points the light reaches, no farther from the camera than the
//!   light's distance from it plus its range, rounded up to a power of two so a moving camera redraws a cached tile
//!   only when that doubles or halves.
//!
//! These levels follow from the cascade's slice and the camera's projection only, so a cached cascade stays valid as
//! the camera moves (and is drawn again when the projection or the settings change). They are as fine as the shadow
//! map can show: a cascade's texels are about as large as the screen's pixels at the slice's far end.
//!
//! # Debug view
//!
//! [`crate::Renderer::set_lod_tint`] colours each part by the level it is drawn at (green, yellow, orange, red from
//! full detail down). The meshes are laid out in the vertex pool level by level, so the vertex shader tells a
//! vertex's level from its index alone: three numbers in the frame uniform, no per-draw state.

use std::f32::consts::PI;

use crate::cull::CullView;
use crate::scene::MeshKind;

/// The most levels a mesh has.
pub const MAX_LOD_LEVELS: usize = 4;

/// Segments around a Ball at each level (it has half as many rings).
pub const BALL_SEGMENTS: [u32; 4] = [32, 16, 8, 4];
/// Segments around a Cylinder at each level.
pub const CYLINDER_SEGMENTS: [u32; 3] = [32, 16, 8];

/// The most a level's outline strays from the true shape, in pixels, at [`LodSettings::bias`] 1.
pub const PIXEL_ERROR: f32 = 0.5;

/// How far past a threshold, as a fraction of it, a part's projected size must go before its level changes.
pub const HYSTERESIS: f32 = 0.1;

/// A part's level before the camera's view has chosen one: chosen without hysteresis.
pub(crate) const NEW: u8 = u8::MAX;
/// The level a shadow view asks for: the coarsest the camera could be drawing (the coarse side of every band).
pub(crate) const COARSEST: u8 = u8::MAX - 1;

/// The shape classes that have levels: what the culling grid keeps per item ([`lod_class`]).
const CLASS_NONE: u8 = 0;
const CLASS_BALL: u8 = 1;
const CLASS_CYLINDER: u8 = 2;
const CLASSES: usize = 3;

/// The levels of each class.
const CLASS_LEVELS: [u8; CLASSES] = [1, BALL_SEGMENTS.len() as u8, CYLINDER_SEGMENTS.len() as u8];

/// How many levels mesh `kind` has: 1 for every kind without levels (Block, Wedge, mesh assets).
#[inline]
pub fn levels(kind: MeshKind) -> usize {
    usize::from(CLASS_LEVELS[usize::from(lod_class(kind))])
}

/// The class of `kind` for choosing levels: 0 for kinds with one level.
#[inline]
pub(crate) fn lod_class(kind: MeshKind) -> u8 {
    match kind {
        MeshKind::BALL => CLASS_BALL,
        MeshKind::CYLINDER => CLASS_CYLINDER,
        _ => CLASS_NONE,
    }
}

/// How levels are chosen: a quality setting, which graphics presets (#84) drive. Presets scale the cost, never the
/// look: every preset keeps a level's outline within a fraction of a pixel of the true shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodSettings {
    /// Choose levels at all. Off, every part is drawn at full detail (the reference the levels are compared
    /// against).
    pub enabled: bool,
    /// The LOD bias: multiplies the projected size levels are chosen by. 2 keeps each level to twice the distance
    /// (outlines within a quarter of a pixel), 0.5 to half (within a pixel).
    pub bias: f32,
}

impl LodSettings {
    /// Low (Iris Xe laptops, mid-range phones): outlines within a pixel.
    pub const LOW: Self = Self {
        enabled: true,
        bias: 0.5,
    };

    /// Medium (GTX 1650 / RX 6500 class): outlines within half a pixel.
    pub const MEDIUM: Self = Self {
        enabled: true,
        bias: 1.0,
    };

    /// High (RTX 3060 Ti class): outlines within a quarter of a pixel.
    pub const HIGH: Self = Self {
        enabled: true,
        bias: 2.0,
    };

    /// Every part at full detail.
    pub const OFF: Self = Self {
        enabled: false,
        bias: 1.0,
    };
}

impl Default for LodSettings {
    /// [`LodSettings::MEDIUM`].
    fn default() -> Self {
        Self::MEDIUM
    }
}

/// The squared size limits of one class: per boundary between a level and the next coarser one, `(T / S)²` for the
/// threshold `T` in pixels and the view's size scale `S`, without the band and at its two edges.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ClassLimits {
    plain: [f32; MAX_LOD_LEVELS - 1],
    /// Back to the finer level only above `(1 + HYSTERESIS) T`.
    finer: [f32; MAX_LOD_LEVELS - 1],
    /// On to the coarser level only below `(1 − HYSTERESIS) T`.
    coarser: [f32; MAX_LOD_LEVELS - 1],
}

/// How a view chooses levels ([`CullView::lod`]): the limits for its projection and settings, and whether it is a
/// camera's view (each instance at its own distance, remembering its level) or a shadow view (every instance as far
/// as [`LodView::distance`] plus its radius, at the coarse side of the band).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LodView {
    limits: [ClassLimits; CLASSES],
    /// 0 for a camera's view; otherwise the distance a shadow view chooses at.
    distance: f32,
    enabled: bool,
}

impl LodView {
    /// No levels: everything at full detail.
    pub const OFF: Self = Self {
        limits: [ClassLimits {
            plain: [0.0; MAX_LOD_LEVELS - 1],
            finer: [0.0; MAX_LOD_LEVELS - 1],
            coarser: [0.0; MAX_LOD_LEVELS - 1],
        }; CLASSES],
        distance: 0.0,
        enabled: false,
    };

    /// The levels of a camera whose projected diameter of a sphere of radius 1 at distance 1 is `size_scale` pixels
    /// ([`CullView::size_scale`]), with `settings`.
    pub fn for_camera(size_scale: f32, settings: &LodSettings) -> Self {
        let scale = size_scale * settings.bias;
        if !settings.enabled || !(scale.is_finite() && scale > 0.0) {
            return Self::OFF;
        }
        let mut limits = [ClassLimits::default(); CLASSES];
        let round = |segments: &[u32], ratio: f32, limits: &mut ClassLimits| {
            for (j, &n) in segments.iter().skip(1).enumerate() {
                // The coarser level `j + 1` is used while R (1 − cos(π / n)) ≤ PIXEL_ERROR, with the bounding
                // diameter `2 ratio R`.
                let threshold = 2.0 * ratio * PIXEL_ERROR / (1.0 - (PI / n as f32).cos());
                let k = (threshold / scale) * (threshold / scale);
                limits.plain[j] = k;
                limits.finer[j] = k * (1.0 + HYSTERESIS) * (1.0 + HYSTERESIS);
                limits.coarser[j] = k * (1.0 - HYSTERESIS) * (1.0 - HYSTERESIS);
            }
        };
        round(&BALL_SEGMENTS, 3f32.sqrt(), &mut limits[usize::from(CLASS_BALL)]);
        round(
            &CYLINDER_SEGMENTS,
            2f32.sqrt(),
            &mut limits[usize::from(CLASS_CYLINDER)],
        );
        Self {
            limits,
            distance: 0.0,
            enabled: true,
        }
    }

    /// The same limits for a shadow view whose receivers are at most `distance` metres from the camera.
    pub fn for_shadow(self, distance: f32) -> Self {
        if !self.enabled || !(distance.is_finite() && distance > 0.0) {
            return Self::OFF;
        }
        Self { distance, ..self }
    }

    /// Whether the view chooses levels at all.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The distance a shadow view chooses at; 0 for a camera's view.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// The level of a part of class `class` (not [`CLASS_NONE`]) with a bounding sphere of radius² `r2` whose centre
    /// is `d2` squared metres from the eye, last drawn at `previous` ([`NEW`], [`COARSEST`] or a level).
    #[inline]
    pub(crate) fn choose(&self, class: u8, r2: f32, d2: f32, previous: u8) -> u8 {
        let Some(limits) = self.limits.get(usize::from(class)) else {
            return 0;
        };
        let last = CLASS_LEVELS[usize::from(class)] - 1;
        // Projected size below the threshold: r² S² / (d² − r²) < T², so r² < k (d² − r²). The eye inside the sphere
        // (d² ≤ r²) never passes: full detail.
        let w = d2 - r2;
        let mut level = 0u8;
        while level < last {
            let j = usize::from(level);
            let k = if previous == NEW {
                limits.plain[j]
            } else if previous > level {
                limits.finer[j]
            } else {
                limits.coarser[j]
            };
            if r2 < k * w {
                level += 1;
            } else {
                break;
            }
        }
        level
    }

    /// The level a shadow view draws a part of class `class` with a bounding sphere of radius `radius` at.
    #[inline]
    pub(crate) fn choose_caster(&self, class: u8, radius: f32) -> u8 {
        let d = self.distance + radius;
        self.choose(class, radius * radius, d * d, COARSEST)
    }
}

/// The level `view` draws an instance of `kind` with a bounding sphere of `radius` at, when `view` is a shadow view
/// ([`CullView::with_caster_lod`]): for tools and tests that mirror the renderer's shadow draws. A camera's view
/// remembers each instance's level, so its levels come from its cull alone.
pub fn caster_level(view: &CullView, kind: MeshKind, radius: f32) -> usize {
    let class = lod_class(kind);
    if class == CLASS_NONE || !view.lod.enabled() || view.lod.distance() <= 0.0 {
        return 0;
    }
    usize::from(view.lod.choose_caster(class, radius))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 90° camera on a 1000-pixel-high target: a sphere of radius r at distance d is 1000 r / d pixels across.
    fn view(bias: f32) -> LodView {
        LodView::for_camera(1000.0, &LodSettings { enabled: true, bias })
    }

    /// The projected size (bounding diameter, pixels) of a sphere of radius `r` at distance `d` under [`view`].
    fn distance_for(size: f32, r: f32) -> f32 {
        // size = 1000 r / √(d² − r²)
        ((1000.0 * r / size).powi(2) + r * r).sqrt()
    }

    #[test]
    fn the_thresholds_keep_the_outline_within_half_a_pixel() {
        let lod = view(1.0);
        let r = 1.0;
        // A ball of bounding radius 1 has radius 1/√3; at 16 segments it may stray R (1 − cos(π/16)) ≤ 0.5 px.
        let t1 = 2.0 * 3f32.sqrt() * 0.5 / (1.0 - (PI / 16.0).cos());
        assert!((t1 - 90.1).abs() < 0.1, "{t1}");
        let just_above = distance_for(t1 * 1.01, r);
        let just_below = distance_for(t1 * 0.99, r);
        assert_eq!(lod.choose(CLASS_BALL, r * r, just_above * just_above, NEW), 0);
        assert_eq!(lod.choose(CLASS_BALL, r * r, just_below * just_below, NEW), 1);
        // Far enough, the coarsest level; never past it.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, 1e12, NEW), 3);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, 1e12, NEW), 2);
        // Inside the sphere: full detail.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, 0.5, NEW), 0);
        // A cylinder's thresholds are lower: its round part is larger for its bounding sphere than a ball's.
        let t1c = 2.0 * 2f32.sqrt() * 0.5 / (1.0 - (PI / 16.0).cos());
        let d = distance_for((t1 + t1c) / 2.0, r);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, d * d, NEW), 1);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, d * d, NEW), 0);
        let d = distance_for(t1c * 0.99, r);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, d * d, NEW), 1);
    }

    #[test]
    fn the_bias_scales_the_distance() {
        let r = 1.0;
        let level_at = |lod: &LodView, d: f32| lod.choose(CLASS_BALL, r * r, d * d, NEW);
        let medium = view(1.0);
        let high = view(2.0);
        let low = view(0.5);
        // Find the distance where Medium first leaves full detail.
        let mut d = 1.0;
        while level_at(&medium, d) == 0 {
            d *= 1.01;
        }
        assert_eq!(level_at(&high, d), 0, "High keeps full detail farther");
        assert_eq!(level_at(&high, d * 2.05), 1);
        assert_eq!(level_at(&low, d * 0.55), 1, "Low drops it nearer");
        assert_eq!(level_at(&LodView::OFF, 1e6), 0);
        assert_eq!(LodView::for_camera(0.0, &LodSettings::MEDIUM), LodView::OFF);
        assert_eq!(LodView::for_camera(1000.0, &LodSettings::OFF), LodView::OFF);
    }

    #[test]
    fn a_part_slowly_crossing_a_threshold_switches_once() {
        let lod = view(1.0);
        let r = 1.0;
        let t1 = 2.0 * 3f32.sqrt() * 0.5 / (1.0 - (PI / 16.0).cos());
        let threshold = distance_for(t1, r);
        // A part drifting back and forth across the threshold by up to 5%, slowly getting farther: 2,000 frames.
        let mut level = NEW;
        let mut switches = 0;
        let mut levels = Vec::new();
        for frame in 0..2_000 {
            let t = frame as f32 / 2_000.0;
            let wobble = 0.05 * (frame as f32 * 0.37).sin();
            let d = threshold * (0.9 + 0.2 * t) * (1.0 + wobble);
            let next = lod.choose(CLASS_BALL, r * r, d * d, level);
            if level != NEW && next != level {
                switches += 1;
            }
            level = next;
            levels.push(level);
        }
        assert_eq!(levels.first(), Some(&0));
        assert_eq!(levels.last(), Some(&1));
        assert_eq!(switches, 1, "one switch, not one per crossing");
        // Without the band, the same drift switches many times.
        let mut flips = 0;
        let mut last = 0;
        for frame in 0..2_000 {
            let t = frame as f32 / 2_000.0;
            let wobble = 0.05 * (frame as f32 * 0.37).sin();
            let d = threshold * (0.9 + 0.2 * t) * (1.0 + wobble);
            let plain = lod.choose(CLASS_BALL, r * r, d * d, NEW);
            if plain != last {
                flips += 1;
            }
            last = plain;
        }
        assert!(flips > 10, "{flips}");
    }

    #[test]
    fn a_level_is_kept_inside_the_band_and_left_past_it() {
        let lod = view(1.0);
        let t1 = 2.0 * 3f32.sqrt() * 0.5 / (1.0 - (PI / 16.0).cos());
        let at = |size: f32| {
            let d = distance_for(size, 1.0);
            d * d
        };
        // Inside the band either level stays.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 0.95), 0), 0);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.05), 1), 1);
        // Past it, the level follows.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 0.85), 0), 1);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.15), 1), 0);
        // A shadow view takes the coarse side of the band: never finer than the camera could be.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.05), COARSEST), 1);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.15), COARSEST), 0);
    }

    #[test]
    fn a_shadow_view_is_never_finer_than_the_camera_where_it_is_looked_up() {
        let camera = view(1.0);
        let shadow = camera.for_shadow(120.0);
        assert_eq!(shadow.distance(), 120.0);
        assert_eq!(camera.for_shadow(0.0), LodView::OFF);
        assert_eq!(LodView::OFF.for_shadow(10.0), LodView::OFF);
        for radius in [0.1f32, 0.5, 1.0, 2.0, 5.0, 20.0] {
            let caster = shadow.choose_caster(CLASS_BALL, radius);
            // Any receiver centre within 120 m + r of the eye, at any level its hysteresis allows, is drawn at the
            // caster's level or finer.
            for step in 0..=200 {
                let d = (120.0 + radius) * step as f32 / 200.0;
                for previous in [NEW, 0, 1, 2, 3] {
                    let drawn = camera.choose(CLASS_BALL, radius * radius, d * d, previous);
                    assert!(
                        drawn <= caster,
                        "r {radius} d {d} previous {previous}: {drawn} > {caster}"
                    );
                }
            }
        }
    }

    #[test]
    fn kinds_have_their_levels() {
        assert_eq!(levels(MeshKind::BLOCK), 1);
        assert_eq!(levels(MeshKind::WEDGE), 1);
        assert_eq!(levels(MeshKind::BALL), 4);
        assert_eq!(levels(MeshKind::CYLINDER), 3);
        assert_eq!(levels(MeshKind(MeshKind::BUILTIN_COUNT)), 1);
        assert!(levels(MeshKind::BALL) <= MAX_LOD_LEVELS);
    }
}
