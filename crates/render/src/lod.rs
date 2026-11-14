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
//! that error stays under [`PIXEL_ERROR`] pixels, divided by the [`LodSettings::bias`]. The bounding sphere of a round
//! Ball is √3 times its radius and that of a Cylinder at least √2 times (its box's diagonal), so the thresholds hold
//! for the roundest case of each. A Ball or Cylinder stretched along one side (#384) is rounder across its long
//! side than that, so its outline there may stray a little more before it goes finer.
//!
//! **Hysteresis**: a part goes to a coarser level only once its size is [`HYSTERESIS`] (10%) below the threshold,
//! and back only once it is 10% above it, so a part hovering at a threshold, or a camera stepping back and forth,
//! switches once, not every frame. Each part remembers the level the camera's view drew it at in the culling grid
//! (a part seen for the first time starts at the coarsest level the band allows), where the cull already computes
//! what it needs (its distance), so choosing costs a few multiplies per round part in view and nothing for blocks and
//! wedges: a grid cell that has held no round part culls exactly as before.
//!
//! # Draws
//!
//! A mesh kind's visible instances are listed per level, each level front to back, and the levels of a kind follow
//! each other in its list segment, so a kind draws one instanced draw per level it uses: at most 1 + 4 + 3 + 1 = 9
//! opaque draws for the built-in kinds, whatever the part count. See-through parts take their level the same way;
//! sorted back to front, they are drawn in one draw that pulls each one's mesh at its level (one draw per run of the
//! same mesh and level on the vertex-buffer fallback; `parts/transparent.rs`).
//!
//! # Shadows
//!
//! A shadow map is never finer than the surface that receives it. Every level is inscribed in the finer ones (its
//! vertices are theirs), so a coarser caster lies inside the finer surface and the drawn surface is in front of its
//! own shadow, never behind it; the other way round, a finer caster in the map would stand up to one segment's
//! sagitta in front of a coarse receiver and shadow its own lit side (acne the receivers' small biases cannot hide).
//!
//! So each shadow view draws a caster no finer than the camera draws it:
//!
//! - a **sun cascade drawn again every frame** (the nearest ones, [`crate::ShadowSettings::dynamic_cascades`]) takes
//!   each caster at the level the camera's view would choose for it at its own distance, at the coarse side of the
//!   hysteresis band ([`LodView::seen_from`], #598): the level the camera draws it at, or one coarser while it is in a
//!   band. Choosing by the slice's far end instead drew a ball near the camera as an 8- or 16-sided one, whose
//!   shadow had straight sides and corners. These maps follow the camera already, so they are drawn again when it
//!   moves;
//! - a **cached sun cascade** is looked up by points whose view depth is within its slice, so no farther from the
//!   camera than the slice's far depth over the cosine of the view's corner angle ([`CullView::with_caster_lod`]); a
//!   caster of radius `r` is chosen as if its centre were that far plus `r`, at the coarse side of the hysteresis
//!   band. That needs no camera position, so the cached map stays valid as the camera moves;
//! - a **local light's tile** is looked up by points the light reaches, no farther from the camera than the
//!   light's distance from it plus its range, rounded up to a power of two so a moving camera redraws a cached tile
//!   only when that doubles or halves.
//!
//! The cached levels follow from the cascade's slice and the camera's projection only (and are drawn again when the
//! projection or the settings change). They are as fine as the shadow map can show: a far cascade's texels are about
//! as large as the screen's pixels at the slice's far end.
//!
//! # Debug view
//!
//! [`crate::Renderer::set_lod_tint`] colours each part by the level it is drawn at (green, yellow, orange, red from
//! full detail down). The meshes are laid out in the vertex pool level by level, so the vertex shader tells a
//! vertex's level from its index alone: three numbers in the frame uniform, no per-draw state.

use std::f32::consts::PI;

use metallic_core::glam::Vec3;

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

/// What a level is chosen against when a part has none yet, or for a shadow view: the coarse side of every band,
/// so the coarsest level the camera could be drawing at that size.
pub(crate) const COARSEST: u8 = u8::MAX;
/// A part's level before the camera's view has chosen one: it starts at the coarsest its size allows
/// ([`COARSEST`]).
pub(crate) const NEW: u8 = COARSEST;

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

/// How levels are chosen: a quality setting, which the graphics presets drive ([`crate::settings::Quality`]). Presets scale the cost, never the
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
    /// [`LodSettings::HIGH`]: outlines within a quarter of a pixel, so the renderer's defaults look the same as
    /// without levels (the presets trade some of that for speed). On `parts_100k` it draws 82% fewer
    /// triangles than full detail, against 86% at Medium.
    fn default() -> Self {
        Self::HIGH
    }
}

/// The squared size limits of one class: per boundary between a level and the next coarser one, `(T / S)²` for the
/// threshold `T` in pixels and the view's size scale `S`, at the band's two edges. 0 past the class's last level,
/// which no size crosses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ClassLimits {
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
    /// For a shadow view that chooses each caster by its own distance from the camera (#598): where the camera is,
    /// in the scene's coordinates.
    eye: Option<Vec3>,
    enabled: bool,
}

impl LodView {
    /// No levels: everything at full detail.
    pub const OFF: Self = Self {
        limits: [ClassLimits {
            finer: [0.0; MAX_LOD_LEVELS - 1],
            coarser: [0.0; MAX_LOD_LEVELS - 1],
        }; CLASSES],
        distance: 0.0,
        eye: None,
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
            eye: None,
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

    /// The same shadow view choosing each caster by its own distance from the camera at `eye` (scene coordinates),
    /// at the coarse side of the band (#598): the level the camera draws it at, or one coarser within the band, so
    /// its shadow is as round as the part and still never finer than the part receiving it. For maps drawn again
    /// whenever the camera moves; a cached map keeps [`LodView::for_shadow`]'s levels, which do not depend on where
    /// the camera is.
    pub fn seen_from(self, eye: Vec3) -> Self {
        if !self.enabled || self.distance <= 0.0 || !eye.is_finite() {
            return self;
        }
        Self { eye: Some(eye), ..self }
    }

    /// Whether the view chooses levels at all.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Where a shadow view that chooses each caster by its own distance sees them from ([`LodView::seen_from`]).
    pub fn eye(&self) -> Option<Vec3> {
        self.eye
    }

    /// The distance a shadow view chooses at; 0 for a camera's view.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// The level of a part of class `class` (not [`CLASS_NONE`]) with a bounding sphere of radius² `r2` whose centre
    /// is `d2` squared metres from the eye, last drawn at `previous` (a level, or [`COARSEST`]).
    ///
    /// Each boundary the part is past adds a level. A boundary the part was past last time is crossed back only
    /// at the band's far edge, one it was short of only at the near edge. The thresholds are far enough apart (a
    /// factor of four) that the bands never overlap, so the boundaries passed are always the first few, and the
    /// count is the level, with no branch to mispredict.
    #[inline]
    pub(crate) fn choose(&self, class: u8, r2: f32, d2: f32, previous: u8) -> u8 {
        let Some(limits) = self.limits.get(usize::from(class)) else {
            return 0;
        };
        // Projected size below the threshold: r² S² / (d² − r²) < T², so r² < k (d² − r²). The eye inside the sphere
        // (d² ≤ r²) never passes: full detail. A limit of 0 is never passed either.
        let w = d2 - r2;
        let mut level = 0u8;
        for j in 0..MAX_LOD_LEVELS - 1 {
            let k = if previous > j as u8 {
                limits.finer[j]
            } else {
                limits.coarser[j]
            };
            level += u8::from(r2 < k * w);
        }
        level
    }

    /// The level a shadow view draws a part of class `class` with a bounding sphere of radius `radius` centred at
    /// `center` (scene coordinates) at: as if it were [`LodView::distance`] plus its radius from the camera, or, seen
    /// from an eye ([`LodView::seen_from`]), as far as it is.
    #[inline]
    pub(crate) fn choose_caster(&self, class: u8, radius: f32, center: Vec3) -> u8 {
        let d2 = match self.eye {
            Some(eye) => (center - eye).length_squared(),
            None => {
                let d = self.distance + radius;
                d * d
            }
        };
        self.choose(class, radius * radius, d2, COARSEST)
    }
}

/// The level `view` draws an instance of `kind` with a bounding sphere of `radius` centred at `center` at, when `view`
/// is a shadow view ([`CullView::with_caster_lod`]): for tools and tests that mirror the renderer's shadow draws. A
/// camera's view remembers each instance's level, so its levels come from its cull alone.
pub fn caster_level(view: &CullView, kind: MeshKind, radius: f32, center: Vec3) -> usize {
    let class = lod_class(kind);
    if class == CLASS_NONE || !view.lod.enabled() || view.lod.distance() <= 0.0 {
        return 0;
    }
    usize::from(view.lod.choose_caster(class, radius, center))
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
        // The band's edges, 10% either side of the threshold, from either level.
        let at = |size: f32| distance_for(size, r).powi(2);
        assert_eq!(lod.choose(CLASS_BALL, r * r, at(t1 * 0.91), 0), 0);
        assert_eq!(lod.choose(CLASS_BALL, r * r, at(t1 * 0.89), 0), 1);
        assert_eq!(lod.choose(CLASS_BALL, r * r, at(t1 * 1.09), 1), 1);
        assert_eq!(lod.choose(CLASS_BALL, r * r, at(t1 * 1.11), 1), 0);
        // Far enough, the coarsest level; never past it.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, 1e12, NEW), 3);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, 1e12, NEW), 2);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, 1e12, 0), 2);
        // Inside the sphere: full detail.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, 0.5, NEW), 0);
        // A cylinder's thresholds are lower: its round part is larger for its bounding sphere than a ball's.
        let t1c = 2.0 * 2f32.sqrt() * 0.5 / (1.0 - (PI / 16.0).cos());
        let d2 = at((t1 * 0.9 + t1c * 1.1) / 2.0);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, d2, 0), 1);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, d2, NEW), 0);
        assert_eq!(lod.choose(CLASS_CYLINDER, 1.0, at(t1c * 0.89), 0), 1);
        // Every level of both shapes, coming from far and from near.
        let mut seen = [[false; MAX_LOD_LEVELS]; 2];
        for step in 0..400 {
            let d = 1.05f32.powi(step);
            for previous in [0, NEW] {
                seen[0][usize::from(lod.choose(CLASS_BALL, 1.0, d * d, previous))] = true;
                seen[1][usize::from(lod.choose(CLASS_CYLINDER, 1.0, d * d, previous))] = true;
            }
        }
        assert_eq!(seen, [[true; 4], [true, true, true, false]]);
    }

    #[test]
    fn the_bias_scales_the_distance() {
        let r = 1.0;
        let level_at = |lod: &LodView, d: f32| lod.choose(CLASS_BALL, r * r, d * d, 0);
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
        // A part drifting back and forth across the threshold by up to 5%, slowly getting farther (from 15% nearer to
        // 15% farther than the threshold): 2,000 frames.
        let distance = |frame: i32| {
            let t = frame as f32 / 2_000.0;
            let wobble = 0.05 * (frame as f32 * 0.37).sin();
            threshold * (0.85 + 0.3 * t) * (1.0 + wobble)
        };
        let mut level = 0;
        let mut switches = 0;
        let mut levels = Vec::new();
        for frame in 0..2_000 {
            let d = distance(frame);
            let next = lod.choose(CLASS_BALL, r * r, d * d, level);
            if next != level {
                switches += 1;
            }
            level = next;
            levels.push(level);
        }
        assert_eq!(levels.first(), Some(&0));
        assert_eq!(levels.last(), Some(&1));
        assert_eq!(switches, 1, "one switch, not one per crossing");
        // Without the band, the same drift would switch many times.
        let crossings = (1..2_000)
            .filter(|&frame| (distance(frame - 1) < threshold) != (distance(frame) < threshold))
            .count();
        assert!(crossings > 10, "{crossings}");
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
        // A shadow view, and a part seen for the first time, take the coarse side of the band: never finer than the
        // camera could be drawing.
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.05), COARSEST), 1);
        assert_eq!(lod.choose(CLASS_BALL, 1.0, at(t1 * 1.15), COARSEST), 0);
        assert_eq!(NEW, COARSEST);
    }

    /// #598: seen from the camera, a caster takes the level the camera draws it at, at the band's coarse side, so it is
    /// never finer than the part receiving its own shadow, whatever level that part's hysteresis left it at.
    #[test]
    fn a_shadow_view_seen_from_the_camera_follows_each_caster_s_own_distance() {
        let camera = view(1.0);
        let eye = Vec3::new(3.0, 1.0, -2.0);
        let shadow = camera.for_shadow(120.0).seen_from(eye);
        assert_eq!(shadow.eye(), Some(eye));
        assert_eq!(
            camera.seen_from(eye).eye(),
            None,
            "a camera's view has no caster distance"
        );
        for radius in [0.1f32, 0.5, 1.0, 2.0, 5.0, 20.0] {
            let mut levels = Vec::new();
            for step in 0..=400 {
                let d = (radius + 300.0) * step as f32 / 400.0;
                let center = eye + Vec3::new(0.6, 0.0, 0.8) * d;
                let caster = shadow.choose_caster(CLASS_BALL, radius, center);
                for previous in [COARSEST, 0, 1, 2, 3] {
                    let drawn = camera.choose(CLASS_BALL, radius * radius, d * d, previous);
                    assert!(
                        drawn <= caster,
                        "r {radius} d {d} previous {previous}: {drawn} > {caster}"
                    );
                }
                // At most the band away from what the camera chooses fresh.
                assert!(caster <= camera.choose(CLASS_BALL, radius * radius, d * d, COARSEST));
                levels.push(caster);
            }
            // Nearer is never coarser.
            assert!(
                levels.windows(2).all(|pair| pair[0] <= pair[1]),
                "r {radius}: {levels:?}"
            );
        }
    }

    #[test]
    fn a_shadow_view_is_never_finer_than_the_camera_where_it_is_looked_up() {
        let camera = view(1.0);
        let shadow = camera.for_shadow(120.0);
        assert_eq!(shadow.distance(), 120.0);
        assert_eq!(camera.for_shadow(0.0), LodView::OFF);
        assert_eq!(LodView::OFF.for_shadow(10.0), LodView::OFF);
        for radius in [0.1f32, 0.5, 1.0, 2.0, 5.0, 20.0] {
            let caster = shadow.choose_caster(CLASS_BALL, radius, Vec3::ZERO);
            // Any receiver centre within 120 m + r of the eye, at any level its hysteresis allows, is drawn at the
            // caster's level or finer.
            for step in 0..=200 {
                let d = (120.0 + radius) * step as f32 / 200.0;
                for previous in [COARSEST, 0, 1, 2, 3] {
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
