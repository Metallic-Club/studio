//! Sun shadows: cascaded shadow maps, texel-snapped, with the far cascades cached (ADR 0008 §6).
//!
//! # Cascades
//!
//! The view from the camera out to [`ShadowSettings::distance`] is cut into [`ShadowSettings::cascades`] slices
//! (uniform and logarithmic spacing blended by [`ShadowSettings::split_lambda`]), and each slice gets its own depth
//! map of [`ShadowSettings::resolution`]² texels, seen along the light: the sun by day, the moon by night (the
//! frame's one directional light, [`crate::lighting`]). A map covers the bounding sphere of its slice, whose size
//! does not change as the camera turns, and its centre is snapped to whole texels in the light's frame, so the map
//! moves by whole texels and shadow edges do not shimmer as the camera moves. A map holds the casters between its
//! slice and [`CASTER_REACH`] metres towards the light. All maps are layers of one `Depth32Float` array
//! texture; their memory is `cascades × resolution² × 4` bytes (48 MiB for the default 3 × 2048).
//!
//! # Casters
//!
//! Each cascade culls its casters with the scene's culling grid ([`crate::InstanceScene::cull_casters`]) against its
//! own volume, into its own [`VisibleSet`], and draws them with one instanced draw per mesh kind from its own segment
//! of a shadow list buffer, like the camera's draws ([`crate::parts`]). A caster is an opaque part with `CastShadow`
//! on ([`crate::scene::is_shadow_caster`]). See-through parts cast no shadow yet. Sun shadows as a whole follow
//! `Lighting.GlobalShadows` ([`crate::SceneLighting::global_shadows`]); `Light.Shadows` is the switch of local
//! lights' shadows, which come with point and spot lights (#83).
//!
//! # Caching
//!
//! Most of a scene does not move, and most of its casters are in the far cascades. So the nearest
//! [`ShadowSettings::dynamic_cascades`] cascades hold **every** caster and are drawn again in any frame in which the
//! scene changed at all, while the farther ones hold only the **static region's** casters ([`crate::scene::Region`])
//! and are **cached**: drawn again only when
//!
//! - the light's direction changes,
//! - the cascade's snapped placement changes (the camera moved far enough), or
//! - a static caster changed inside it ([`crate::InstanceScene::shadow_changes`]: moved, resized, appeared,
//!   disappeared, started or stopped casting, or moved between regions).
//!
//! A cached cascade moves in coarser steps than one redrawn every frame (1/[`CACHED_STEP_DIVISOR`] of its
//! width, for about 6% of its resolution) and, where the view is wide enough that this costs little, is centred on
//! the camera rather than ahead of it, so that walking a few metres or turning around does not redraw it.
//!
//! What this costs a game's look: **moving parts cast shadows only within the dynamic cascades' reach** (about 57 m
//! with the defaults); beyond it, a part the scene sees as moving (in the dynamic region) casts none until it has
//! rested long enough to return to the static region. That reach is a setting a preset scales (all cascades dynamic
//! is the full-quality end), like the shadow distance itself.
//!
//! # Receiving
//!
//! Parts look their cascade up by view depth, offset the point along its normal by
//! [`ShadowSettings::normal_offset`] texels and compare with a depth bias of [`ShadowSettings::depth_bias`] texels
//! (against acne), filtered by a tent of bilinear comparison taps ([`ShadowSettings::filter_radius`]: 3 × 3 taps
//! by default, a smooth edge about 4 texels wide). Casters are drawn with their front faces culled, which keeps lit
//! surfaces off their own depth; shadows fade out over the last tenth of the distance.
//!
//! # Both instance paths
//!
//! On the storage path a cascade's list holds slot entries; on the vertex-buffer fallback (GLES) it holds copies of
//! the instances, gathered when the cascade is drawn, as the camera's lists do. Shadows look the same on both.

pub(crate) mod cascade;
mod maps;

use std::time::Duration;

use metallic_core::glam::Vec3;

pub use cascade::{CACHED_STEP_DIVISOR, CASTER_REACH};
pub(crate) use maps::{FrameShadow, ShadowMaps};

#[cfg(doc)]
use crate::VisibleSet;
use crate::cull::{Casters, CullView};
use crate::parts::Camera;

/// The most cascades a renderer draws.
pub const MAX_CASCADES: usize = 4;

/// The cull views of the cascades `settings` fits to `camera` on a target `aspect` (width / height) wide, in a
/// scene whose instances are relative to `origin`, for a light towards `towards_light`
/// ([`crate::SceneLighting::light_direction`]), with which casters each holds; and how many there are. These are
/// the views the renderer culls each cascade's casters with ([`crate::InstanceScene::cull_casters`]), for tools and
/// tests that mirror it.
pub fn cascade_views(
    settings: &ShadowSettings,
    camera: &Camera,
    aspect: f32,
    origin: Vec3,
    towards_light: Vec3,
) -> ([(CullView, Casters); MAX_CASCADES], usize) {
    let mut fits = [cascade::CascadeFit::default(); MAX_CASCADES];
    let count = cascade::fit_cascades(camera, aspect, origin, towards_light, settings, &mut fits);
    let dynamic = settings.dynamic_count();
    let mut views = [(fits[0].cull_view(), Casters::All); MAX_CASCADES];
    for (i, (view, fit)) in views.iter_mut().zip(&fits).enumerate() {
        let casters = if i < dynamic { Casters::All } else { Casters::Static };
        *view = (fit.cull_view(), casters);
    }
    (views, count)
}

/// How the sun's shadows are drawn, which graphics presets (#84) drive: ADR 0008 sets Low at 1 × 1024 and High at
/// 3 × 2048. The defaults are High's cascades, with the nearest two redrawn every frame. Presets scale the cost (map
/// size, cascades, distance, filter taps), never whether a game has shadows: that is `Lighting.GlobalShadows`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    /// Draw sun shadows at all.
    pub enabled: bool,
    /// How many cascades, 1 to [`MAX_CASCADES`].
    pub cascades: u32,
    /// Each cascade's map is this many texels square (clamped to the device's texture limit).
    pub resolution: u32,
    /// How far from the camera shadows reach, in metres.
    pub distance: f32,
    /// How the view is split between cascades: 0 evenly, 1 logarithmically (the near cascades small and sharp).
    pub split_lambda: f32,
    /// How many of the nearest cascades hold moving casters and are redrawn whenever the scene changes; the rest
    /// hold only static casters and are cached (see the [module documentation](self)). At least 1.
    pub dynamic_cascades: u32,
    /// The comparison filter's reach in texels: 0 is one bilinear tap (hard edges), 1 a 3 × 3 tent of taps, 2 a
    /// 5 × 5 one.
    pub filter_radius: u32,
    /// The depth bias of a receiving point, in texels of its cascade (against shadow acne).
    pub depth_bias: f32,
    /// How far a receiving point is moved along its normal before the lookup, in texels of its cascade (against
    /// acne on surfaces at a grazing angle to the light).
    pub normal_offset: f32,
}

impl ShadowSettings {
    /// No shadows.
    pub const OFF: Self = Self {
        enabled: false,
        cascades: 3,
        resolution: 2048,
        distance: 256.0,
        split_lambda: 0.75,
        dynamic_cascades: 2,
        filter_radius: 1,
        depth_bias: 1.0,
        normal_offset: 1.0,
    };

    /// How many cascades are drawn: 0 when off.
    pub fn cascade_count(&self) -> usize {
        if self.enabled {
            (self.cascades as usize).clamp(1, MAX_CASCADES)
        } else {
            0
        }
    }

    /// How many of them are redrawn whenever the scene changes (the rest are cached).
    pub fn dynamic_count(&self) -> usize {
        (self.dynamic_cascades as usize).clamp(1, self.cascade_count().max(1))
    }
}

impl Default for ShadowSettings {
    /// Three cascades of 2048² out to 256 m, the nearest two redrawn every frame and the far one cached, a 3 × 3
    /// tent filter.
    fn default() -> Self {
        Self {
            enabled: true,
            ..Self::OFF
        }
    }
}

/// One cascade in the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CascadeStats {
    /// The casters its map holds, as of its last drawing.
    pub casters: u32,
    /// It was drawn in the last frame (otherwise its map was kept).
    pub redrawn: bool,
    /// It holds only static casters and is kept while they and its placement stay (see [`ShadowSettings`]).
    pub cached: bool,
}

/// What the last frame did for shadows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadowStats {
    /// Cascades in use (0 when shadows are off, there is no camera or no light).
    pub cascades: u32,
    /// Each cascade, the first [`ShadowStats::cascades`] of them.
    pub cascade: [CascadeStats; MAX_CASCADES],
    /// CPU time of the culls of the cascades drawn.
    pub cull_time: Duration,
}

impl ShadowStats {
    /// How many cascades were drawn in the last frame.
    pub fn redrawn(&self) -> u32 {
        self.cascade.iter().filter(|c| c.redrawn).count() as u32
    }

    /// Bit `i` set when cascade `i` was drawn in the last frame.
    pub fn redrawn_mask(&self) -> u8 {
        self.cascade
            .iter()
            .enumerate()
            .filter(|(_, c)| c.redrawn)
            .fold(0, |mask, (i, _)| mask | 1 << i)
    }
}
