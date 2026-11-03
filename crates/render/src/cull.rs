//! CPU culling (ADR 0008 §3): which instances a view draws.
//!
//! The scene keeps every drawn instance in a loose grid of bounding spheres ([`crate::scene`]), updated from the
//! same dirty slots its uploads come from, so a moving part costs O(1) and nothing is rebuilt. A **view**
//! ([`CullView`]) is a frustum plus the settings that apply to it; [`crate::InstanceScene::cull`] walks the grid
//! once for it and writes the visible entries into a [`VisibleSet`], in the draw lists' format (one list per mesh
//! kind plus the see-through instances), which [`crate::GpuScene`] uploads into its list segments in place of every
//! live entry. Nothing on the GPU is compacted or moved: only the lists say what is drawn.
//!
//! An instance is culled when its bounding sphere is
//!
//! - outside the view's frustum (any of its planes; the main camera has no far plane),
//! - farther than [`CullSettings::max_distance`] (the sphere's nearest point), or
//! - smaller on screen than [`CullSettings::min_screen_size`] pixels: the sphere's projected diameter through the
//!   view's projection, at the target's height.
//!
//! Views are built from any clip matrix ([`Frustum::from_clip`]), so shadow cascades and local-light shadows
//! (#82, #83) cull against the same grid with their own frusta ([`CullView::from_frustum`]), each into its own
//! [`VisibleSet`].
//!
//! # Order
//!
//! The visible opaque instances of each kind come out **roughly front to back**, so nearer surfaces fill the depth
//! buffer first and hidden fragments behind them fail the depth test before shading (ADR 0008 §7): the grid cells
//! and large items in view are sorted by the view depth of their box's nearest point (a few hundred keys), and
//! each cell's items follow in the order they joined it. Large items, the walls and floors that hide the most, are
//! ordered one by one. Ordering every instance by its own depth was measured too (a counting sort into depth
//! buckets): on 100,000 parts it made the cull 2.1 to 2.7 times as long, for about the same GPU time (`overdraw`:
//! 0.206 against 0.216 ms; `parts_100k`: 0.379 against 0.364 ms; see `docs/perf.md`).
//! See-through instances are sorted back to front per instance afterwards ([`crate::parts`]), so their order here
//! does not matter.

use std::time::Duration;

use metallic_core::glam::{Mat4, Vec3, Vec4};

use crate::parts::Camera;

/// The culling settings of a view, which graphics presets (#84) drive. The defaults cull what cannot be seen and
/// nothing that can: the frustum, and parts smaller than a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullSettings {
    /// Cull at all. Off, every live instance is drawn (the reference the culled image is compared against).
    pub enabled: bool,
    /// The view distance in metres: instances whose bounding sphere starts farther than this are not drawn.
    /// Infinite by default (the projection has no far plane either).
    pub max_distance: f32,
    /// Instances whose bounding sphere projects smaller than this many pixels across (at the target's height)
    /// are not drawn. 0 turns screen-size culling off.
    pub min_screen_size: f32,
}

impl CullSettings {
    /// Culls nothing: every live instance is drawn.
    pub const OFF: Self = Self {
        enabled: false,
        max_distance: f32::INFINITY,
        min_screen_size: 0.0,
    };
}

impl Default for CullSettings {
    /// Frustum culling, no view distance, and sub-pixel parts culled.
    fn default() -> Self {
        Self {
            enabled: true,
            max_distance: f32::INFINITY,
            min_screen_size: 1.0,
        }
    }
}

/// How a box lies against a frustum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Containment {
    /// Wholly outside one plane: nothing in it is visible.
    Outside,
    /// Crossing at least one plane: what is in it must be tested.
    Intersects,
    /// Inside every plane: everything in it passes the frustum.
    Inside,
}

/// The planes of a view volume, each `(normal, d)` as a [`Vec4`] with the inside where `normal · p + d ≥ 0`.
/// Normals are unit length, so the value is a signed distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    planes: [Vec4; 6],
    count: usize,
}

impl Frustum {
    /// The volume `clip` maps into wgpu's clip space (`-w ≤ x, y ≤ w`, `0 ≤ z ≤ w`), in `clip`'s input space.
    /// This holds for standard and reverse-Z, perspective and orthographic projections alike. A plane that
    /// degenerates (the far plane of an infinite projection) is left out.
    pub fn from_clip(clip: Mat4) -> Self {
        let (x, y, z, w) = (clip.row(0), clip.row(1), clip.row(2), clip.row(3));
        let mut frustum = Self {
            planes: [Vec4::ZERO; 6],
            count: 0,
        };
        for plane in [w + x, w - x, w + y, w - y, z, w - z] {
            let length = plane.truncate().length();
            if length > 1e-6 && length.is_finite() {
                frustum.planes[frustum.count] = plane / length;
                frustum.count += 1;
            }
        }
        frustum
    }

    /// The planes.
    pub fn planes(&self) -> &[Vec4] {
        &self.planes[..self.count]
    }

    /// Whether a sphere at `center` with `radius` is at least partly inside.
    #[inline]
    pub fn contains_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes().iter().all(|p| p.truncate().dot(center) + p.w >= -radius)
    }

    /// How the axis-aligned box at `center` with half extents `half` lies against the frustum.
    #[inline]
    pub fn classify_box(&self, center: Vec3, half: Vec3) -> Containment {
        let mut inside = true;
        for p in self.planes() {
            let normal = p.truncate();
            let distance = normal.dot(center) + p.w;
            let reach = normal.abs().dot(half);
            if distance < -reach {
                return Containment::Outside;
            }
            if distance < reach {
                inside = false;
            }
        }
        if inside {
            Containment::Inside
        } else {
            Containment::Intersects
        }
    }
}

/// One view to cull for: its frustum and the distance and screen-size thresholds, all in the scene's coordinates
/// (relative to [`crate::InstanceScene::origin`], like the packed instances).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullView {
    /// The view volume.
    pub frustum: Frustum,
    /// Where distances are measured from.
    pub eye: Vec3,
    /// The view direction (unit length), for the front-to-back order.
    pub forward: Vec3,
    /// Instances whose bounding sphere starts farther than this from `eye` are culled.
    pub max_distance: f32,
    /// The projected diameter in pixels of a sphere of radius 1 seen at distance 1: the projection's vertical scale
    /// times the target's height. 0 turns screen-size culling off (views without a perspective, such as a sun's).
    pub size_scale: f32,
    /// Instances projecting smaller than this many pixels across are culled.
    pub min_screen_size: f32,
}

impl CullView {
    /// The main view of `camera` on a `width` × `height` target, for a scene whose instances are relative to
    /// `origin`, with `settings`' thresholds.
    pub fn for_camera(camera: &Camera, origin: Vec3, width: u32, height: u32, settings: &CullSettings) -> Self {
        let aspect = width as f32 / height.max(1) as f32;
        let eye = camera.position - origin;
        // The camera's matrix takes positions relative to the camera; the scene's are relative to its origin.
        let clip = camera.view_projection(aspect) * Mat4::from_translation(-eye);
        let forward = (camera.rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        // The projection's vertical scale, `1 / tan(fov_y / 2)`, maps a slope to normalised device coordinates,
        // which span 2 over the target's height: a sphere of radius r at distance d covers r / d × scale × height.
        let projection_scale = 1.0 / (camera.fov_y.clamp(1e-3, 3.1) * 0.5).tan();
        Self {
            frustum: Frustum::from_clip(clip),
            eye,
            forward,
            max_distance: if settings.max_distance > 0.0 {
                settings.max_distance
            } else {
                f32::INFINITY
            },
            size_scale: projection_scale * height as f32,
            min_screen_size: settings.min_screen_size.max(0.0),
        }
    }

    /// A view that culls against `frustum` only, measuring depth from `eye` along `forward` (shadow views).
    pub fn from_frustum(frustum: Frustum, eye: Vec3, forward: Vec3) -> Self {
        Self {
            frustum,
            eye,
            forward: forward.normalize_or(Vec3::NEG_Z),
            max_distance: f32::INFINITY,
            size_scale: 0.0,
            min_screen_size: 0.0,
        }
    }

    /// Whether a bounding sphere passes the distance and screen-size tests (not the frustum).
    #[inline]
    pub fn sphere_large_and_near(&self, center: Vec3, radius: f32) -> bool {
        let d2 = (center - self.eye).length_squared();
        if d2 <= radius * radius {
            // The eye is inside the sphere.
            return true;
        }
        let reach = self.max_distance + radius;
        if d2 > reach * reach {
            return false;
        }
        // Projected diameter r·S / √(d² − r²) against the threshold, squared: no square root.
        let size = radius * self.size_scale;
        let min = self.min_screen_size;
        size * size >= min * min * (d2 - radius * radius)
    }

    /// Whether a bounding sphere is drawn: inside the frustum, near enough and large enough.
    #[inline]
    pub fn sphere_visible(&self, center: Vec3, radius: f32) -> bool {
        self.frustum.contains_sphere(center, radius) && self.sphere_large_and_near(center, radius)
    }

    /// The front-to-back key of an axis-aligned box: the view depth of its nearest point.
    #[inline]
    pub(crate) fn box_key(&self, center: Vec3, half: Vec3) -> u32 {
        depth_key((center - self.eye).dot(self.forward) - half.dot(self.forward.abs()))
    }
}

/// A view depth in metres as a key that sorts nearest first: non-negative floats order like their bits, and
/// everything at or behind the eye (or not a number) is 0.
#[inline]
pub(crate) fn depth_key(depth: f32) -> u32 {
    depth.max(0.0).to_bits()
}

/// What one cull did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CullStats {
    /// Instances that could be drawn: live and not fully transparent.
    pub candidates: u32,
    /// Instances left to draw.
    pub visible: u32,
    /// Instances outside the frustum, counted one by one or by their whole grid cell.
    pub outside_frustum: u32,
    /// Instances culled for distance or screen size.
    pub too_far_or_small: u32,
    /// Grid cells with instances in them.
    pub cells: u32,
    /// Of those, cells wholly outside the view, skipped without testing their instances.
    pub cells_outside: u32,
    /// The previous cull was still valid (nothing changed in the scene or the view) and was kept.
    pub reused: bool,
    /// CPU time of the cull, from walking the grid to the filled lists; 0 when the previous cull was kept.
    pub cpu_time: Duration,
}

impl CullStats {
    /// Instances not drawn.
    pub fn culled(&self) -> u32 {
        self.candidates.saturating_sub(self.visible)
    }
}

/// The instances one view draws, in the draw lists' format: one list of entries per mesh kind (front to back),
/// and the see-through instances (unsorted). Reused frame to frame: once its vectors have grown to the scene's
/// size, a cull allocates nothing.
#[derive(Debug, Default)]
pub struct VisibleSet {
    pub(crate) opaque: Vec<Vec<u32>>,
    pub(crate) transparent: Vec<u32>,
    /// The last cull's see-through entries before this one's, to tell whether they changed.
    pub(crate) previous_transparent: Vec<u32>,
    /// Changes whenever the visible see-through entries do.
    pub(crate) transparent_revision: u64,
    pub(crate) stats: CullStats,
    /// The grid cells and large items in view, by depth key ([`depth_key`]), for the front-to-back pass.
    pub(crate) order: Vec<(u32, u32)>,
}

impl VisibleSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The visible opaque entries of mesh kind `kind`, roughly front to back.
    pub fn opaque(&self, kind: crate::scene::MeshKind) -> &[u32] {
        self.opaque.get(usize::from(kind.0)).map_or(&[], Vec::as_slice)
    }

    /// How many opaque lists there are (one per mesh kind up to the highest drawn).
    pub fn opaque_count(&self) -> usize {
        self.opaque.len()
    }

    /// The visible see-through entries, in no particular order.
    pub fn transparent(&self) -> &[u32] {
        &self.transparent
    }

    /// Changes whenever [`VisibleSet::transparent`] does.
    pub fn transparent_revision(&self) -> u64 {
        self.transparent_revision
    }

    /// What the last cull did.
    pub fn stats(&self) -> CullStats {
        self.stats
    }

    /// Every visible entry, opaque and see-through.
    pub fn len(&self) -> usize {
        self.opaque.iter().map(Vec::len).sum::<usize>() + self.transparent.len()
    }

    /// Whether nothing is visible.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Empties the lists for a new cull, keeping their memory. `capacities` gives each opaque list's full length,
    /// `transparent` the see-through list's and `places` how many cells and large items there are, so nothing
    /// grows during the cull.
    pub(crate) fn begin(
        &mut self,
        capacities: impl ExactSizeIterator<Item = usize>,
        transparent: usize,
        places: usize,
    ) {
        let lists = capacities.len();
        if self.opaque.len() < lists {
            // Only when a mesh kind is drawn for the first time.
            self.opaque.resize_with(lists, Vec::new);
        }
        for (list, capacity) in self.opaque.iter_mut().zip(capacities) {
            list.clear();
            list.reserve(capacity);
        }
        std::mem::swap(&mut self.transparent, &mut self.previous_transparent);
        self.transparent.clear();
        // (The previous entries are only compared with, never pushed to.)
        self.transparent.reserve(transparent);
        self.order.clear();
        self.order.reserve(places);
    }

    /// Adds a visible instance of `list` (an opaque list index, or the transparent list).
    #[inline]
    pub(crate) fn push(&mut self, entry: u32, list: u16) {
        if list == crate::scene::TRANSPARENT_LIST {
            self.transparent.push(entry);
        } else if let Some(out) = self.opaque.get_mut(usize::from(list)) {
            // The capacity was reserved for every entry of the kind: this never allocates.
            out.push(entry);
        }
    }

    /// Notes whether the see-through entries changed. Returns how many instances are visible.
    pub(crate) fn finish(&mut self) -> u32 {
        if self.transparent != self.previous_transparent {
            self.transparent_revision = self.transparent_revision.wrapping_add(1);
        }
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::CFrame;

    use super::*;

    fn camera_at(eye: Vec3, target: Vec3, fov: f64) -> Camera {
        Camera::from_cframe(CFrame::look_at(eye, target, Vec3::Y), fov)
    }

    #[test]
    fn the_frustum_of_a_camera_has_five_planes_and_holds_what_it_sees() {
        // Looking down −Z from the origin, 90° vertically on a square target.
        let camera = camera_at(Vec3::ZERO, Vec3::NEG_Z, 90.0);
        let view = CullView::for_camera(&camera, Vec3::ZERO, 100, 100, &CullSettings::OFF);
        // Four sides and the near plane; the infinite far plane is left out.
        assert_eq!(view.frustum.planes().len(), 5);
        for plane in view.frustum.planes() {
            assert!((plane.truncate().length() - 1.0).abs() < 1e-5);
        }
        let f = &view.frustum;
        assert!(f.contains_sphere(Vec3::new(0.0, 0.0, -10.0), 0.1));
        assert!(f.contains_sphere(Vec3::new(0.0, 0.0, -1e6), 0.1), "no far plane");
        assert!(!f.contains_sphere(Vec3::new(0.0, 0.0, 10.0), 1.0), "behind");
        // At 45° the edge: 10 m ahead, a point 11 m to the side is out, unless its sphere reaches back in.
        assert!(f.contains_sphere(Vec3::new(9.9, 0.0, -10.0), 0.0));
        assert!(!f.contains_sphere(Vec3::new(11.0, 0.0, -10.0), 0.5));
        assert!(
            f.contains_sphere(Vec3::new(11.0, 0.0, -10.0), 0.8),
            "√2/2 m from the plane"
        );
        assert!(!f.contains_sphere(Vec3::new(0.0, -11.0, -10.0), 0.5));
        // The near plane at 0.1 m.
        assert!(!f.contains_sphere(Vec3::new(0.0, 0.0, -0.05), 0.01));
    }

    #[test]
    fn the_frustum_follows_the_camera_and_the_scene_origin() {
        // At (100, 0, 0) looking +X, with the scene's origin at (64, 0, 0): positions are relative to it.
        let camera = camera_at(Vec3::new(100.0, 0.0, 0.0), Vec3::new(200.0, 0.0, 0.0), 60.0);
        let origin = Vec3::new(64.0, 0.0, 0.0);
        let view = CullView::for_camera(&camera, origin, 160, 90, &CullSettings::default());
        assert!(view.eye.abs_diff_eq(Vec3::new(36.0, 0.0, 0.0), 1e-5));
        assert!(view.forward.abs_diff_eq(Vec3::X, 1e-5));
        // World (150, 0, 0) is ahead; world (50, 0, 0) is behind.
        assert!(view.frustum.contains_sphere(Vec3::new(150.0, 0.0, 0.0) - origin, 1.0));
        assert!(!view.frustum.contains_sphere(Vec3::new(50.0, 0.0, 0.0) - origin, 1.0));
        // The wide aspect: 16:9 at 60° vertical is about 91° across, so 45° to the side is just inside.
        assert!(view.frustum.contains_sphere(Vec3::new(150.0, 0.0, 48.0) - origin, 0.1));
        assert!(!view.frustum.contains_sphere(Vec3::new(150.0, 0.0, 60.0) - origin, 0.1));
    }

    #[test]
    fn boxes_are_outside_crossing_or_inside() {
        let camera = camera_at(Vec3::ZERO, Vec3::NEG_Z, 90.0);
        let f = CullView::for_camera(&camera, Vec3::ZERO, 1, 1, &CullSettings::OFF).frustum;
        assert_eq!(
            f.classify_box(Vec3::new(0.0, 0.0, -20.0), Vec3::ONE),
            Containment::Inside
        );
        assert_eq!(
            f.classify_box(Vec3::new(20.0, 0.0, -20.0), Vec3::ONE),
            Containment::Intersects
        );
        assert_eq!(
            f.classify_box(Vec3::new(30.0, 0.0, -20.0), Vec3::ONE),
            Containment::Outside
        );
        assert_eq!(
            f.classify_box(Vec3::new(0.0, 0.0, 20.0), Vec3::ONE),
            Containment::Outside
        );
        // A box around the eye crosses the near plane and the sides.
        assert_eq!(f.classify_box(Vec3::ZERO, Vec3::ONE), Containment::Intersects);
    }

    #[test]
    fn distance_culling_uses_the_nearest_point_of_the_sphere() {
        let camera = camera_at(Vec3::ZERO, Vec3::NEG_Z, 70.0);
        let settings = CullSettings {
            max_distance: 100.0,
            min_screen_size: 0.0,
            ..CullSettings::default()
        };
        let view = CullView::for_camera(&camera, Vec3::ZERO, 1920, 1080, &settings);
        assert!(view.sphere_visible(Vec3::new(0.0, 0.0, -99.0), 0.5));
        assert!(!view.sphere_visible(Vec3::new(0.0, 0.0, -101.0), 0.5));
        // A large sphere whose near side is within the distance stays.
        assert!(view.sphere_visible(Vec3::new(0.0, 0.0, -110.0), 20.0));
        // No limit by default.
        let far = CullView::for_camera(&camera, Vec3::ZERO, 1920, 1080, &CullSettings::default());
        assert!(far.sphere_visible(Vec3::new(0.0, 0.0, -1e5), 1e3));
    }

    #[test]
    fn screen_size_culling_projects_the_sphere() {
        // 90° vertically on a 1000-pixel-high target: the projection's scale is 1, so a sphere of radius r at
        // distance d covers about 1000 × r / d pixels (r / √(d² − r²) exactly).
        let camera = camera_at(Vec3::ZERO, Vec3::NEG_Z, 90.0);
        let settings = CullSettings {
            min_screen_size: 4.0,
            ..CullSettings::default()
        };
        let view = CullView::for_camera(&camera, Vec3::ZERO, 1000, 1000, &settings);
        assert!((view.size_scale - 1000.0).abs() < 1e-2, "{}", view.size_scale);
        // Radius 1: 4 pixels at 250 m.
        assert!(view.sphere_visible(Vec3::new(0.0, 0.0, -240.0), 1.0));
        assert!(!view.sphere_visible(Vec3::new(0.0, 0.0, -260.0), 1.0));
        // Twice the height, twice the pixels: visible to twice the distance.
        let tall = CullView::for_camera(&camera, Vec3::ZERO, 2000, 2000, &settings);
        assert!(tall.sphere_visible(Vec3::new(0.0, 0.0, -490.0), 1.0));
        assert!(!tall.sphere_visible(Vec3::new(0.0, 0.0, -510.0), 1.0));
        // A narrower field of view magnifies: at 2 × atan(0.5) the scale is 2.
        let zoomed = Camera {
            fov_y: 2.0 * 0.5f32.atan(),
            ..camera
        };
        let zoom = CullView::for_camera(&zoomed, Vec3::ZERO, 1000, 1000, &settings);
        assert!(zoom.sphere_visible(Vec3::new(0.0, 0.0, -490.0), 1.0));
        // Off at 0, and never for a sphere around the eye.
        let off = CullView::for_camera(&camera, Vec3::ZERO, 1000, 1000, &CullSettings::OFF);
        assert!(off.sphere_large_and_near(Vec3::new(0.0, 0.0, -1e6), 1e-3));
        assert!(view.sphere_large_and_near(Vec3::ZERO, 0.5));
    }

    #[test]
    fn depth_keys_sort_nearest_first() {
        assert_eq!(depth_key(-5.0), 0);
        assert_eq!(depth_key(0.0), 0);
        assert_eq!(depth_key(f32::NAN), 0);
        let mut last = 0;
        for i in 1..10_000 {
            let key = depth_key(i as f32 * 0.37);
            assert!(key > last);
            last = key;
        }
        // A box is keyed by its nearest point along the view: a wide wall 10 m ahead starts at 10 m.
        let camera = camera_at(Vec3::ZERO, Vec3::NEG_Z, 70.0);
        let view = CullView::for_camera(&camera, Vec3::ZERO, 100, 100, &CullSettings::default());
        let wall = view.box_key(Vec3::new(30.0, 0.0, -10.5), Vec3::new(50.0, 20.0, 0.5));
        assert_eq!(wall, depth_key(10.0));
        assert!(view.box_key(Vec3::new(0.0, 0.0, -20.0), Vec3::ONE) > wall);
    }

    #[test]
    fn a_visible_set_lists_entries_by_kind_and_notes_see_through_changes() {
        let mut set = VisibleSet::new();
        set.begin([4usize, 2].into_iter(), 1, 0);
        for (entry, list) in [(10, 0), (20, 1), (11, 0), (30, crate::scene::TRANSPARENT_LIST), (21, 1)] {
            set.push(entry, list);
        }
        assert_eq!(set.finish(), 5);
        assert_eq!(set.opaque[0], [10, 11]);
        assert_eq!(set.opaque[1], [20, 21]);
        assert_eq!(set.transparent(), [30]);
        let revision = set.transparent_revision();
        // The same see-through entries again: same revision; different ones: a new one.
        set.begin([4usize, 2].into_iter(), 1, 0);
        set.push(30, crate::scene::TRANSPARENT_LIST);
        set.finish();
        assert_eq!(set.transparent_revision(), revision);
        assert!(set.opaque[0].is_empty());
        set.begin([4usize, 2].into_iter(), 1, 0);
        set.finish();
        assert_ne!(set.transparent_revision(), revision);
    }
}
