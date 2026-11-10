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
//! texture, and each cached cascade (below) has a static map of the same size besides: their memory is
//! `(cascades + cached cascades) × resolution² × 4` bytes (64 MiB for the default 3 × 2048, one of them cached).
//! On OpenGL a single cascade's array gets a second, unused layer, because wgpu's GL backend cannot sample a
//! one-layer texture as an array (#155): Low's one 1024² cascade takes 8 MiB there rather than 4. A new size or
//! cascade count after a scene's first frame makes its maps off the frame (`textures`, #152): frames draw
//! with the maps before it until all the new ones exist, then every cascade is drawn into them at once (about 0.4 ms
//! of GPU at Ultra; making them is what costs).
//!
//! # Casters
//!
//! Each cascade culls its casters with the scene's culling grid ([`crate::InstanceScene::cull_casters`]) against its
//! own volume, into its own [`VisibleSet`], and draws them with one instanced draw per mesh kind and level of detail
//! from its own segment of a shadow list buffer, like the camera's draws ([`crate::parts`]). Its round casters take
//! the coarsest level the camera could draw them at anywhere the cascade is looked up (its slice's far corner): never
//! finer than the surface receiving the shadow, so levels cause no acne, and independent of where the camera stands,
//! so a cached cascade stays valid (the reasoning is in [`crate::lod`]). A caster is a drawn part with `CastShadow`
//! on ([`crate::scene::is_shadow_caster`]). A partly see-through part casts a shadow that lets light through by its
//! Transparency (#122, [`ShadowSettings::see_through`]; off at Low, where it casts none): its casters are drawn after
//! the solid ones, sorted into one draw per mesh kind and level, by a pipeline whose fragment stage leaves out that
//! share of its texels (spread by interleaved gradient noise), so the comparison filter averages them into a partial
//! shadow. Solid casters keep a pipeline without a fragment stage. Sun shadows as a whole follow
//! `Lighting.GlobalShadows` ([`crate::SceneLighting::global_shadows`]); `Light.Shadows` is the switch of local
//! lights' shadows, which are drawn into a shadow atlas ([`crate::lights`]).
//!
//! # Caching
//!
//! Most of a scene does not move, and most of its casters are in the far cascades. So the nearest
//! [`ShadowSettings::dynamic_cascades`] cascades hold **every** caster and are drawn again in any frame in which the
//! scene changed at all, while the farther ones are **cached**: the **static region's** casters
//! ([`crate::scene::Region`]) are drawn into a static map of the cascade's own, drawn again only when
//!
//! - the light's direction changes,
//! - the cascade's snapped placement changes (the camera moved far enough),
//! - the camera's levels of detail change (its field of view, the target's height or the LOD bias), or
//! - a static caster changed inside it ([`crate::InstanceScene::shadow_changes`]: moved, resized, appeared,
//!   disappeared, started or stopped casting, or moved between regions);
//!
//! and the cascade's map is **built** from it: the static map copied in (a full-screen draw writing each texel's
//! depth, which works on every backend, GLES included, where depth textures cannot be copied), then the **moving
//! casters** (the dynamic region's) inside the cascade drawn on top, in one pass. A map is built only when its static
//! map was drawn again or its moving casters changed; a frame that moves nothing keeps it, and so does a frame in
//! which a cascade without moving casters still holds its static map alone. The moving casters are few, so each is
//! tested against the cascade's volume directly rather than with the culling grid.
//!
//! The whole static map is copied only when the map does not already hold it: after the static map was drawn
//! again, or when the map's contents are not known. Otherwise the map holds the static map and the moving casters
//! it was last built with, and the build **restores** only their footprints (#133): its pass loads the map, copies
//! the static map back under one rectangle per previous moving caster (the texels its bounding sphere can cover,
//! which bounds every level of detail, and a texel of margin), then draws this frame's moving casters. So a build
//! costs what its moving casters cover rather than the map's size. When the footprints add up to more texels than
//! the map holds, the whole map is copied instead.
//!
//! So moving parts cast shadows out to the shadow distance whatever the settings: the static map is what is
//! cached, never a part of the look (ADR 0008: presets scale cost, they never remove a game's look). A cached
//! cascade moves in coarser steps than one redrawn every frame (1/[`CACHED_STEP_DIVISOR`] of its width, for about 6%
//! of its resolution) and, where the view is wide enough that this costs little, is centred on the camera rather than
//! ahead of it, so that walking a few metres or turning around does not draw its static casters again.
//!
//! # Receiving
//!
//! A map holds its casters' faces **towards the light** (back faces culled), pushed away from the light by a
//! slope-scaled rasterizer bias of 3 texels of their depth slope, so a part resting on another shadows it right up to
//! where they touch (#126). Parts look their cascade up by view depth, offset the point along its normal by
//! [`ShadowSettings::normal_offset`] texels scaled by `1 − N·L` and compare with a depth bias of
//! [`ShadowSettings::depth_bias`] texels (at most [`MAX_RECEIVER_OFFSET`]), filtered by a tent of bilinear
//! comparison taps ([`ShadowSettings::filter_radius`]: 3 × 3 taps by default, a smooth edge about 4 texels wide);
//! shadows fade out over the last tenth of the distance.
//!
//! # Both instance paths
//!
//! On the storage path a cascade's list holds slot entries; on the vertex-buffer fallback (GLES) it holds copies of
//! the instances, gathered when the cascade is drawn, as the camera's lists do. Shadows look the same on both.

pub(crate) mod cascade;
mod maps;
mod see_through;

use std::time::Duration;

use metallic_core::glam::Vec3;

pub use cascade::{CACHED_STEP_DIVISOR, CASTER_REACH};
pub(crate) use maps::{FrameShadow, SHADOW_FORMAT, ShadowMaps, caster_pipeline};
pub(crate) use see_through::SeeThroughCasters;

#[cfg(doc)]
use crate::VisibleSet;
use crate::cull::{Casters, CullView};
use crate::lod::LodView;
use crate::parts::Camera;

/// The most cascades a renderer draws.
pub const MAX_CASCADES: usize = 4;

/// The most a receiving point's depth bias reaches, in metres, in the sun's cascades and the local lights' atlas, and
/// the most its normal offset reaches in the atlas. Both are set in texels, which grow with a coarse cascade or with
/// the distance from a local light (a 256² tile's texel is 8 cm 10 m from a point light): uncapped, they could move a
/// lookup tens of centimetres, past any caster thinner than that, and light would leak under a thin part resting on
/// another. Capped, only a caster thinner than 5 cm can be looked past. The sun's normal offset has no cap: see
/// [`ShadowSettings::depth_bias`].
pub const MAX_RECEIVER_OFFSET: f32 = 0.05;

/// The cull views of the cascades `settings` fits to `camera` on a target `aspect` (width / height) wide, in a
/// scene whose instances are relative to `origin`, for a light towards `towards_light`
/// ([`crate::SceneLighting::light_direction`]), whose casters' levels of detail follow `lod` (the camera view's,
/// [`CullView::lod`]), with which casters each culls; and how many there are. These are the views the renderer culls
/// each cascade's casters with ([`crate::InstanceScene::cull_casters`]), for tools and tests that mirror it. A cached
/// cascade culls its static casters (into its static map); its moving casters are those of the dynamic region whose
/// bounding sphere its view holds, each at [`crate::lod::caster_level`] for the view.
pub fn cascade_views(
    settings: &ShadowSettings,
    camera: &Camera,
    aspect: f32,
    origin: Vec3,
    towards_light: Vec3,
    lod: &LodView,
) -> ([(CullView, Casters); MAX_CASCADES], usize) {
    let mut fits = [cascade::CascadeFit::default(); MAX_CASCADES];
    let count = cascade::fit_cascades(camera, aspect, origin, towards_light, settings, lod, &mut fits);
    let dynamic = settings.dynamic_count();
    let mut views = [(fits[0].cull_view(), Casters::All); MAX_CASCADES];
    for (i, (view, fit)) in views.iter_mut().zip(&fits).enumerate() {
        let casters = if i < dynamic { Casters::All } else { Casters::Static };
        *view = (fit.cull_view().with_see_through(settings.see_through), casters);
    }
    (views, count)
}

/// How the sun's shadows are drawn, which the graphics presets drive ([`crate::settings::Quality`]): ADR 0008 sets Low at 1 × 1024 and High at
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
    /// How many of the nearest cascades are drawn again whole whenever the scene changes; the rest cache their
    /// static casters and draw only their moving ones on a copy of them (see the [module documentation](self)). At
    /// least 1. Both hold every caster: this trades the cached cascades' memory and copies against drawing all
    /// their casters whenever anything moves.
    pub dynamic_cascades: u32,
    /// The comparison filter's reach in texels: 0 is one bilinear tap (hard edges), 1 a 3 × 3 tent of taps, 2 a
    /// 5 × 5 one.
    pub filter_radius: u32,
    /// The depth bias of a receiving point, in texels of its cascade, at most [`MAX_RECEIVER_OFFSET`] metres
    /// (against shadow acne).
    ///
    /// 0.5 by default, with [`ShadowSettings::normal_offset`] 1.5. The maps hold the casters' faces towards the
    /// light, drawn with a slope-scaled rasterizer bias of 3 texels of their depth slope: the 3 × 3 tent's bilinear
    /// taps compare a point with depths up to 2 texels away, and on a sloped surface those differ from its own by 2
    /// texels of slope. With that bias the default cascades show no acne on a cube's top, a ramp the sun meets at 15°
    /// or the floor it meets at 17° (`tests/shadows.rs`), and the receivers' offsets are a margin: half a texel of
    /// depth for depth precision, capped so a coarse cascade cannot look past a thin caster lit from above, and a
    /// texel and a half along the normal for grazing surfaces near a part's edge, where the taps reach the texels of
    /// the edge's other face. Measured on one 1024² cascade (ADR 0008's Low, about 30 cm a texel): capping the
    /// normal offset at 5 or 15 cm brought acne back on the cube's top (7.7 and 6.0 levels of 255), so it has no
    /// cap; scaled by `1 − N·L` it is nothing on surfaces facing the sun. The offsets used to be 1 and 1 texels
    /// against the back faces the maps held then, which lit a band where a part rests on another (#126).
    pub depth_bias: f32,
    /// How far a receiving point is moved along its normal before the lookup, in texels of its cascade, scaled by
    /// `1 − N·L` (against acne on surfaces at a grazing angle to the light). See [`ShadowSettings::depth_bias`] for
    /// the values.
    pub normal_offset: f32,
    /// Partly see-through parts cast shadows that let light through by their Transparency (#122). Off at Low, where
    /// they cast none (the see-through casters cost a fragment stage in the maps).
    pub see_through: bool,
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
        depth_bias: 0.5,
        normal_offset: 1.5,
        see_through: true,
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
    /// Three cascades of 2048² out to 256 m, the nearest two redrawn every frame and the far one's static casters
    /// cached, a 3 × 3 tent filter.
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
    /// The casters its map holds, as of its last drawing (a cached cascade's: its static map's).
    pub casters: u32,
    /// Its casters were drawn in the last frame (a cached cascade's: its static map; otherwise the map was kept).
    pub redrawn: bool,
    /// Its static casters are cached in a static map, kept while they and its placement stay, and its moving
    /// casters are drawn on a copy of it (see the [module documentation](self)).
    pub cached: bool,
    /// A cached cascade's moving casters in the last frame, drawn on its map over the copy of its static map.
    pub moving: u32,
    /// A cached cascade's map was built in the last frame: its static map copied in and its moving casters drawn on
    /// top (otherwise the map was kept: its static map and moving casters stayed).
    pub built: bool,
    /// A cached cascade's map was built in the last frame by restoring the static map only under the footprints of
    /// the moving casters it was built with before, not by copying all of it (see the [module documentation](self)).
    pub restored: bool,
    /// The texels that restore drew (footprints that overlap count as often as they are drawn); 0 for a full copy.
    pub restored_texels: u32,
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
        mask(self.cascade.iter().map(|c| c.redrawn))
    }

    /// Bit `i` set when cached cascade `i`'s map was built in the last frame.
    pub fn built_mask(&self) -> u8 {
        mask(self.cascade.iter().map(|c| c.built))
    }

    /// Bit `i` set when cached cascade `i`'s map was built in the last frame by restoring footprints
    /// ([`CascadeStats::restored`]).
    pub fn restored_mask(&self) -> u8 {
        mask(self.cascade.iter().map(|c| c.restored))
    }
}

fn mask(bits: impl Iterator<Item = bool>) -> u8 {
    bits.enumerate()
        .filter(|&(_, bit)| bit)
        .fold(0, |mask, (i, _)| mask | 1 << i)
}

#[cfg(test)]
mod tests {
    use super::MAX_RECEIVER_OFFSET;

    /// lights.wgsl caps the local lights' offsets with its own copy of the constant.
    #[test]
    fn the_light_shader_caps_receiver_offsets_alike() {
        let wgsl = include_str!("../parts/lights.wgsl");
        let line = format!("const MAX_RECEIVER_OFFSET: f32 = {MAX_RECEIVER_OFFSET:?};");
        assert!(wgsl.contains(&line), "lights.wgsl has no `{line}`");
    }
}
