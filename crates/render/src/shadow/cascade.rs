//! Fitting the sun's cascades to the camera, texel-snapped, and deciding which cascades must be drawn again.
//!
//! Everything here is CPU maths with no GPU, in the scene's origin-relative coordinates (the instances' own).

use metallic_core::glam::{Mat4, Vec3, Vec4};

use super::{MAX_CASCADES, ShadowSettings};
use crate::cull::{CullView, Frustum};
use crate::lod::LodView;
use crate::parts::Camera;

/// How far towards the light beyond a cascade's receivers casters are still drawn into it, in metres: a part up to
/// this far up-sun of what the camera sees still shadows it. Depth is `Depth32Float`, so the range costs no
/// precision worth noting (a few hundredths of a millimetre).
pub const CASTER_REACH: f32 = 512.0;

/// A cached cascade moves in steps of `resolution / CACHED_STEP_DIVISOR` texels (64 at 2048), and is made that
/// much larger so it still covers its slice of the view: it is drawn again only when the camera has moved a step,
/// at the cost of about 6% of its resolution. Cascades redrawn every frame move one texel at a time.
pub const CACHED_STEP_DIVISOR: u32 = 32;

/// A cached cascade is centred on the camera instead of on its slice of the view when that sphere is at most this
/// much wider: then turning the camera never moves it (wide fields of view, where the slice's sphere reaches back
/// to the camera anyway).
const CAMERA_CENTRED_SLACK: f32 = 1.25;

/// The sun's frame: two axes across the shadow map and the direction towards the light. Stable for a given light
/// direction, so a still sun never moves the maps.
pub(crate) fn light_basis(towards_light: Vec3) -> (Vec3, Vec3, Vec3) {
    let l = towards_light.try_normalize().unwrap_or(Vec3::Y);
    let reference = if l.y.abs() < 0.99 { Vec3::Y } else { Vec3::Z };
    let right = reference.cross(l).normalize();
    let up = l.cross(right);
    (right, up, l)
}

/// Where each cascade ends, as view depths from the camera: `splits[i]` to `splits[i + 1]` is cascade `i`'s slice.
/// A blend of uniform and logarithmic spacing (`lambda` 0 is uniform, 1 logarithmic), the "practical split scheme".
pub(crate) fn split_depths(near: f32, far: f32, count: usize, lambda: f32) -> [f32; MAX_CASCADES + 1] {
    let mut splits = [far; MAX_CASCADES + 1];
    let count = count.clamp(1, MAX_CASCADES);
    let near = near.max(1e-3);
    let far = far.max(near * 2.0);
    let lambda = lambda.clamp(0.0, 1.0);
    splits[0] = near;
    for (i, split) in splits.iter_mut().enumerate().take(count).skip(1) {
        let t = i as f32 / count as f32;
        let log = near * (far / near).powf(t);
        let uniform = near + (far - near) * t;
        *split = lambda * log + (1.0 - lambda) * uniform;
    }
    splits
}

/// Which placement a cascade has, exactly: a cascade is drawn again when this changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlacementKey {
    /// The snapped centre in light space, in steps.
    steps: [i64; 3],
    /// The step and half-width, as bits.
    step: u32,
    half: u32,
    /// The light direction, as bits.
    light: [u32; 3],
}

/// One cascade fitted for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CascadeFit {
    /// Origin-relative positions to the shadow map's clip space: x and y across the map (−1 to 1), depth 0 at the
    /// light's end and 1 at the far end.
    pub(crate) clip: Mat4,
    /// What the cascade holds, for culling its casters.
    pub(crate) frustum: Frustum,
    /// Where the cascade's near plane is centred (origin-relative) and the light's direction away from it, for
    /// ordering casters nearest the light first.
    pub(crate) eye: Vec3,
    pub(crate) forward: Vec3,
    /// The placement, to tell whether a cached map still fits.
    pub(crate) key: PlacementKey,
    /// The view depth this cascade's slice ends at.
    pub(crate) split_far: f32,
    /// The world size of one texel, in metres.
    pub(crate) texel: f32,
    /// The depth range of the map in metres (depth 0 to 1).
    pub(crate) depth_range: f32,
    /// How the cascade chooses its casters' levels of detail ([`crate::lod`]): as the camera would for a receiver
    /// as far from it as a point that looks this cascade up can be, the slice's far corner.
    pub(crate) lod: LodView,
}

impl CascadeFit {
    /// The view that culls this cascade's casters.
    pub(crate) fn cull_view(&self) -> CullView {
        CullView {
            lod: self.lod,
            ..CullView::from_frustum(self.frustum, self.eye, self.forward)
        }
    }
}

impl Default for CascadeFit {
    fn default() -> Self {
        Self {
            clip: Mat4::IDENTITY,
            frustum: Frustum::from_clip(Mat4::IDENTITY),
            eye: Vec3::ZERO,
            forward: Vec3::NEG_Y,
            key: PlacementKey::default(),
            split_far: 0.0,
            texel: 0.0,
            depth_range: 1.0,
            lod: LodView::OFF,
        }
    }
}

/// Fits `settings.cascades` cascades to `camera` (on a target `aspect` wide, in a scene whose instances are
/// relative to `origin`) for a light towards `towards_light`, into `out`, their casters' levels of detail following
/// `lod`, the camera view's ([`crate::cull::CullView::lod`]). Returns how many it fitted.
///
/// Each cascade covers the bounding sphere of its slice of the view, whose size does not depend on where the camera
/// looks, so the map's texels keep their world size. The sphere's centre is snapped in light space to whole texels
/// (whole steps of [`CACHED_STEP_DIVISOR`] for cascades that are cached), so a moving camera moves the map by whole
/// texels and shadow edges do not shimmer; the map is made larger by the snap so the slice always stays inside.
pub(crate) fn fit_cascades(
    camera: &Camera,
    aspect: f32,
    origin: Vec3,
    towards_light: Vec3,
    settings: &ShadowSettings,
    lod: &LodView,
    out: &mut [CascadeFit; MAX_CASCADES],
) -> usize {
    let count = settings.cascade_count();
    if count == 0 {
        return 0;
    }
    let dynamic = settings.dynamic_count();
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let resolution = settings.resolution.max(16);
    let splits = split_depths(
        camera.near.max(0.05),
        settings.distance.max(1.0),
        count,
        settings.split_lambda,
    );
    let (right, up, l) = light_basis(towards_light);
    let eye = camera.position - origin;
    let forward = (camera.rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
    // The squared slope from the view axis to a corner of the view.
    let tan = (camera.fov_y.clamp(1e-3, 3.1) * 0.5).tan();
    let k2 = tan * tan * (1.0 + aspect * aspect);
    let light_bits = l.to_array().map(f32::to_bits);
    for (i, fit) in out.iter_mut().enumerate().take(count) {
        let cached = i >= dynamic;
        let (a, b) = (if i == 0 { 0.0 } else { splits[i] }, splits[i + 1]);
        // The smallest sphere around the slice's corners, on the view axis (see `slice_sphere`).
        let (z, slice_radius) = slice_sphere(a, b, k2);
        let mut center = eye + forward * z;
        let mut radius = slice_radius;
        let around_camera = b * (1.0 + k2).sqrt();
        if cached && around_camera <= slice_radius * CAMERA_CENTRED_SLACK {
            center = eye;
            radius = around_camera;
        }
        // Rounded up to 1/64 m, so the size is the same from frame to frame whatever the rounding of the maths.
        let radius = (radius * 64.0).ceil() / 64.0;
        let steps_texels = if cached {
            (resolution / CACHED_STEP_DIVISOR).max(1)
        } else {
            1
        };
        // The map is `resolution` texels across `2 × half`, where `half` covers the sphere plus one step either way:
        // texel = 2 × half / resolution and half = radius + step, with step = steps_texels × texel.
        let usable = resolution.saturating_sub(2 * steps_texels).max(1) as f32;
        let texel = 2.0 * radius / usable;
        let step = texel * steps_texels as f32;
        let half = radius + step;
        let light_space = Vec3::new(center.dot(right), center.dot(up), center.dot(l));
        let snapped = (light_space / step).round();
        let steps = [snapped.x as i64, snapped.y as i64, snapped.z as i64];
        let s = snapped * step;
        let near_z = s.z + half + CASTER_REACH;
        let far_z = s.z - half;
        let depth_range = near_z - far_z;
        let clip = Mat4::from_cols(
            Vec4::new(right.x / half, up.x / half, -l.x / depth_range, 0.0),
            Vec4::new(right.y / half, up.y / half, -l.y / depth_range, 0.0),
            Vec4::new(right.z / half, up.z / half, -l.z / depth_range, 0.0),
            Vec4::new(-s.x / half, -s.y / half, near_z / depth_range, 1.0),
        );
        *fit = CascadeFit {
            clip,
            frustum: Frustum::from_clip(clip),
            eye: right * s.x + up * s.y + l * near_z,
            forward: -l,
            key: PlacementKey {
                steps,
                step: step.to_bits(),
                half: half.to_bits(),
                light: light_bits,
            },
            split_far: b,
            texel,
            depth_range,
            // A point looks this cascade up while its view depth is below `b`, so it is at most `b` over the cosine
            // of the view's corner angle from the camera: the far corner of the slice.
            lod: lod.for_shadow(around_camera),
        };
    }
    count
}

/// The smallest sphere around a slice of a symmetric view from depth `a` to `b`, whose corners are `k2.sqrt()` times
/// their depth off the axis: its centre's depth on the axis and its radius. The near and far corners are equally far
/// from the centre at depth `(a + b)(1 + k²) / 2`; when that lies beyond `b` the far corners alone set the sphere.
pub(crate) fn slice_sphere(a: f32, b: f32, k2: f32) -> (f32, f32) {
    let z = ((a + b) * (1.0 + k2) * 0.5).min(b);
    let radius = ((b - z) * (b - z) + b * b * k2).sqrt();
    (z, radius)
}

/// What a cascade's map was last drawn for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CascadeCache {
    /// The map holds a drawing at all.
    pub(crate) valid: bool,
    pub(crate) key: PlacementKey,
    /// [`crate::InstanceScene::shadow_epoch`] then.
    pub(crate) epoch: u64,
    /// [`crate::InstanceScene::revision`] then (cascades redrawn every frame are kept while it stays).
    pub(crate) revision: u64,
    /// The layout of the cascades' lists then (a new layout draws everything again).
    pub(crate) layout: u64,
}

/// What a frame knows when it decides which cascades to draw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SceneState<'a> {
    pub(crate) epoch: u64,
    pub(crate) revision: u64,
    pub(crate) layout: u64,
    /// The static casters that changed since the last frame ([`crate::InstanceScene::shadow_changes`]).
    pub(crate) changes: &'a [(Vec3, f32)],
}

impl CascadeCache {
    /// Whether a cascade fitted as `fit` must be drawn this frame. A cascade that holds every caster (`dynamic`) is
    /// drawn whenever anything in the scene changed; a cached one only when a static caster changed inside it. Both
    /// are drawn when they moved, the light turned, or the scene was rebuilt or re-based.
    pub(crate) fn needs_redraw(&self, fit: &CascadeFit, dynamic: bool, scene: &SceneState<'_>) -> bool {
        if !self.valid || self.key != fit.key || self.epoch != scene.epoch || self.layout != scene.layout {
            return true;
        }
        if dynamic {
            return self.revision != scene.revision;
        }
        scene
            .changes
            .iter()
            .any(|&(center, radius)| fit.frustum.contains_sphere(center, radius))
    }

    /// The cache after drawing `fit`.
    pub(crate) fn drawn(fit: &CascadeFit, scene: &SceneState<'_>) -> Self {
        Self {
            valid: true,
            key: fit.key,
            epoch: scene.epoch,
            revision: scene.revision,
            layout: scene.layout,
        }
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::CFrame;

    use super::*;

    fn camera(eye: Vec3, target: Vec3) -> Camera {
        Camera::from_cframe(CFrame::look_at(eye, target, Vec3::Y), 70.0)
    }

    fn fit(camera: &Camera, settings: &ShadowSettings) -> ([CascadeFit; MAX_CASCADES], usize) {
        let mut out = [CascadeFit::default(); MAX_CASCADES];
        let sun = Vec3::new(-0.5, 0.65, 0.58).normalize();
        let n = fit_cascades(camera, 16.0 / 9.0, Vec3::ZERO, sun, settings, &LodView::OFF, &mut out);
        (out, n)
    }

    #[test]
    fn splits_run_from_near_to_the_shadow_distance() {
        let s = split_depths(0.1, 256.0, 3, 0.75);
        assert_eq!(s[0], 0.1);
        assert_eq!(s[3], 256.0);
        assert!(s[0] < s[1] && s[1] < s[2] && s[2] < s[3], "{s:?}");
        // Uniform at 0, logarithmic at 1.
        let uniform = split_depths(0.0, 300.0, 3, 0.0);
        assert!((uniform[1] - 100.0).abs() < 0.01, "{uniform:?}");
        let log = split_depths(1.0, 1000.0, 3, 1.0);
        assert!((log[1] - 10.0).abs() < 0.01 && (log[2] - 100.0).abs() < 0.1, "{log:?}");
    }

    #[test]
    fn the_light_basis_is_orthonormal_and_points_at_the_light() {
        for l in [Vec3::Y, Vec3::new(0.3, 0.9, 0.1), Vec3::X, Vec3::new(0.0, -1.0, 0.001)] {
            let (r, u, t) = light_basis(l);
            assert!((r.length() - 1.0).abs() < 1e-5 && (u.length() - 1.0).abs() < 1e-5);
            assert!(r.dot(u).abs() < 1e-5 && r.dot(t).abs() < 1e-5 && u.dot(t).abs() < 1e-5);
            assert!(t.abs_diff_eq(l.normalize(), 1e-5));
        }
    }

    #[test]
    fn the_slice_sphere_holds_every_corner() {
        for (a, b, k2) in [(0.1, 20.0, 2.0), (20.0, 60.0, 2.0), (60.0, 256.0, 0.1), (5.0, 6.0, 0.5)] {
            let (z, r) = slice_sphere(a, b, k2);
            for (depth, off) in [(a, a * k2.sqrt()), (b, b * k2.sqrt())] {
                let d = ((depth - z) * (depth - z) + off * off).sqrt();
                assert!(d <= r + 1e-3, "corner at {depth} is {d} from the centre, radius {r}");
            }
        }
    }

    /// Every corner of each cascade's slice of the view lands inside its map, depth included.
    #[test]
    fn each_cascade_covers_its_slice_of_the_view() {
        let settings = ShadowSettings::default();
        for (eye, target) in [
            (Vec3::new(0.0, 6.0, 0.0), Vec3::new(100.0, 0.0, 0.0)),
            (Vec3::new(13.7, 40.0, -2.2), Vec3::new(0.0, 0.0, 30.0)),
            (Vec3::new(-300.0, 2.0, 50.0), Vec3::new(-300.0, 2.0, -50.0)),
        ] {
            let camera = camera(eye, target);
            let (fits, n) = fit(&camera, &settings);
            assert_eq!(n, 3);
            let splits = split_depths(camera.near, settings.distance, n, settings.split_lambda);
            let tan = (camera.fov_y * 0.5).tan();
            for (i, fit) in fits.iter().take(n).enumerate() {
                for depth in [if i == 0 { 0.0 } else { splits[i] }, splits[i + 1]] {
                    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        let local = Vec3::new(sx * tan * depth * 16.0 / 9.0, sy * tan * depth, -depth);
                        let world = camera.position + camera.rotation * local;
                        let clip = fit.clip.project_point3(world);
                        assert!(
                            clip.x.abs() <= 1.0 && clip.y.abs() <= 1.0 && (0.0..=1.0).contains(&clip.z),
                            "cascade {i}: corner at depth {depth} maps to {clip}"
                        );
                    }
                }
            }
            // Cascades grow with distance.
            assert!(fits[0].texel < fits[1].texel && fits[1].texel < fits[2].texel);
        }
    }

    /// A camera moving by less than a texel keeps every map where it is (no shimmer); moving further moves the near
    /// cascades by whole texels, and the cached one only once it moved a step.
    #[test]
    fn maps_move_in_whole_texels_and_cached_ones_in_steps() {
        let settings = ShadowSettings::default();
        let start = camera(Vec3::new(0.0, 6.0, 0.0), Vec3::new(100.0, 0.0, 0.0));
        let (before, _) = fit(&start, &settings);
        let nudge = Camera {
            position: start.position + Vec3::new(0.001, 0.0, 0.0),
            ..start
        };
        let (after, _) = fit(&nudge, &settings);
        for i in 0..3 {
            assert_eq!(before[i].key, after[i].key, "cascade {i} moved for a millimetre");
            assert_eq!(before[i].clip, after[i].clip);
        }
        // A metre: the near cascades move (by whole texels: their offset across the map is a whole number of
        // texels), the cached far one does not.
        let walk = Camera {
            position: start.position + Vec3::new(1.0, 0.0, 0.0),
            ..start
        };
        let (walked, _) = fit(&walk, &settings);
        assert_ne!(before[0].key, walked[0].key);
        assert_eq!(before[2].key, walked[2].key, "the cached cascade moves in steps");
        let res = settings.resolution as f32;
        for i in 0..2 {
            let p = Vec3::new(3.0, 1.0, -2.0);
            let shift = (walked[i].clip.project_point3(p) - before[i].clip.project_point3(p)) * 0.5 * res;
            assert!(
                (shift.x - shift.x.round()).abs() < 0.02 && (shift.y - shift.y.round()).abs() < 0.02,
                "cascade {i} moved by {shift} texels"
            );
        }
    }

    #[test]
    fn a_turning_sun_or_a_rebuild_redraws_and_static_frames_keep_cached_cascades() {
        let settings = ShadowSettings::default();
        let cam = camera(Vec3::new(0.0, 6.0, 0.0), Vec3::new(100.0, 0.0, 0.0));
        let (fits, _) = fit(&cam, &settings);
        let scene = SceneState {
            epoch: 1,
            revision: 10,
            layout: 1,
            changes: &[],
        };
        let near = CascadeCache::drawn(&fits[0], &scene);
        let far = CascadeCache::drawn(&fits[2], &scene);
        // A map never drawn is drawn.
        assert!(CascadeCache::default().needs_redraw(&fits[2], false, &scene));
        // Nothing changed: both kept.
        assert!(!near.needs_redraw(&fits[0], true, &scene));
        assert!(!far.needs_redraw(&fits[2], false, &scene));
        // Something moved (anywhere): the near cascade, which holds moving parts, is drawn again; the far one only
        // when a static caster changed inside it.
        let moved = SceneState { revision: 11, ..scene };
        assert!(near.needs_redraw(&fits[0], true, &moved));
        assert!(!far.needs_redraw(&fits[2], false, &moved));
        let far_away = [(Vec3::new(5000.0, 0.0, 5000.0), 2.0)];
        let inside = [(Vec3::new(150.0, 1.0, 0.0), 2.0)];
        assert!(!far.needs_redraw(
            &fits[2],
            false,
            &SceneState {
                changes: &far_away,
                ..moved
            }
        ));
        assert!(far.needs_redraw(
            &fits[2],
            false,
            &SceneState {
                changes: &inside,
                ..moved
            }
        ));
        // A new epoch or layout, or another light: everything.
        assert!(far.needs_redraw(&fits[2], false, &SceneState { epoch: 2, ..scene }));
        assert!(far.needs_redraw(&fits[2], false, &SceneState { layout: 2, ..scene }));
        let mut turned = [CascadeFit::default(); MAX_CASCADES];
        fit_cascades(
            &cam,
            16.0 / 9.0,
            Vec3::ZERO,
            Vec3::new(-0.5, 0.66, 0.58),
            &settings,
            &LodView::OFF,
            &mut turned,
        );
        assert!(far.needs_redraw(&turned[2], false, &scene));
    }

    #[test]
    fn turning_the_camera_keeps_a_camera_centred_cached_cascade() {
        let settings = ShadowSettings::default();
        let a = camera(Vec3::new(0.0, 6.0, 0.0), Vec3::new(100.0, 0.0, 0.0));
        let b = camera(Vec3::new(0.0, 6.0, 0.0), Vec3::new(0.0, 0.0, 100.0));
        let (fa, _) = fit(&a, &settings);
        let (fb, _) = fit(&b, &settings);
        // At 70° on 16:9 the far slice's sphere reaches back to the camera, so the cached cascade is centred on it.
        assert_eq!(fa[2].key, fb[2].key);
        assert_ne!(fa[0].key, fb[0].key);
    }

    #[test]
    fn off_or_no_cascades_fits_nothing() {
        let cam = camera(Vec3::ZERO, Vec3::NEG_Z);
        assert_eq!(fit(&cam, &ShadowSettings::OFF).1, 0);
        let one = ShadowSettings {
            cascades: 1,
            ..ShadowSettings::default()
        };
        assert_eq!(fit(&cam, &one).1, 1);
        let many = ShadowSettings {
            cascades: 99,
            ..ShadowSettings::default()
        };
        assert_eq!(fit(&cam, &many).1, MAX_CASCADES);
    }
}
