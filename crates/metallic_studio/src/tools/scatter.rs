//! Brush scatter tool core (B; spec section 9, issue #219).
//!
//! A [`BrushSet`] holds up to 16 weighted items. A [`Stroke`] paints or erases with one set
//! and one [`BrushSettings`]; each [`Stroke::dab`] is one pointer sample. When the stroke ends,
//! [`Stroke::finish`] returns one [`StrokeCommit`]: the whole stroke is one undo step.
//!
//! **Placement.** The viewport casts the cursor ray down the view direction and passes the hit
//! point and normal to [`Stroke::dab`]. Points are drawn by Poisson-disk dart throwing inside
//! the brush disk, which lies in the plane through the hit point at right angles to its
//! normal. Each point is projected onto the scene with a ray along the negative normal (the
//! [`SurfaceProbe`]), then kept only if
//! - the surface passes the [`SurfaceFilter`] and its slope is at most `max_slope_deg`, and
//! - it is at least the minimum spacing away from every instance this brush has placed,
//!   including the ones from earlier dabs and earlier strokes.
//!
//! **Density and dwell.** The disk's target is `density x area / 100` instances (density is
//! per 100 square studs). One dab adds at most the target per 0.1 s of dwell, never more than
//! 64, and never more than the disk still lacks, so holding still fills the disk to density
//! and stops.
//!
//! **Determinism.** Dab `k` of a stroke draws from [`SeededRng::for_stream`]`(seed, k)`: the
//! same seed, inputs and scene give the same placements.
//!
//! **Erase.** A stroke begun with [`StrokeMode::Erase`] removes only instances that carry
//! this set's tag, [`BrushSet::tag`] (`Brush:<set name>`), inside the brush radius.

use std::collections::HashMap;

use glam::{DQuat, DVec2, DVec3, EulerRot};

use super::geom::PartSnapshot;
use super::rng::SeededRng;

/// Most items one brush set holds.
pub const MAX_BRUSH_ITEMS: usize = 16;
/// Most placements one dab (one frame) adds.
pub const MAX_PLACEMENTS_PER_DAB: usize = 64;
/// Dwell time that allows one full disk of placements.
pub const DWELL_STEP_SECONDS: f64 = 0.1;
/// Dart-throwing attempts per wanted placement before a dab gives up.
const ATTEMPTS_PER_PLACEMENT: usize = 30;

/// Errors from building a brush set.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScatterError {
    /// A brush set needs at least one item.
    #[error("the brush set is empty")]
    EmptySet,
    /// A brush set holds at most [`MAX_BRUSH_ITEMS`] items.
    #[error("a brush set holds at most {MAX_BRUSH_ITEMS} items, got {0}")]
    TooManyItems(usize),
}

/// One Model or Part in the brush tray.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushItem {
    /// What to place (an Explorer path or asset reference; the caller resolves it).
    pub source: String,
    /// Pick weight, 1 to 10 (clamped).
    pub weight: u8,
    /// Bounding-box size in studs at scale 1. The item's pivot is the box centre.
    pub size: DVec3,
}

impl BrushItem {
    /// Creates an item; the weight is clamped to 1..=10.
    pub fn new(source: impl Into<String>, weight: u8, size: DVec3) -> Self {
        Self {
            source: source.into(),
            weight: weight.clamp(1, 10),
            size,
        }
    }

    /// Radius of the sphere around the bounding box.
    pub fn bounding_radius(&self) -> f64 {
        self.size.abs().length() * 0.5
    }
}

/// A named, weighted set of up to 16 items.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushSet {
    name: String,
    items: Vec<BrushItem>,
}

impl BrushSet {
    /// Creates a set from 1 to [`MAX_BRUSH_ITEMS`] items.
    pub fn new(name: impl Into<String>, items: Vec<BrushItem>) -> Result<Self, ScatterError> {
        if items.is_empty() {
            return Err(ScatterError::EmptySet);
        }
        if items.len() > MAX_BRUSH_ITEMS {
            return Err(ScatterError::TooManyItems(items.len()));
        }
        Ok(Self {
            name: name.into(),
            items,
        })
    }

    /// The set's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The items, in tray order.
    pub fn items(&self) -> &[BrushItem] {
        &self.items
    }

    /// Tag carried by every instance this set places: `Brush:<name>`.
    pub fn tag(&self) -> String {
        format!("Brush:{}", self.name)
    }

    /// Whether a tag list marks an instance as placed by this set.
    pub fn owns(&self, tags: &[String]) -> bool {
        let tag = self.tag();
        tags.contains(&tag)
    }

    /// Default minimum spacing: half the smallest bounding radius in the set.
    pub fn default_min_spacing(&self) -> f64 {
        self.items
            .iter()
            .map(BrushItem::bounding_radius)
            .fold(f64::INFINITY, f64::min)
            * 0.5
    }

    /// Picks an item index by weight.
    pub fn pick(&self, rng: &mut SeededRng) -> usize {
        let total: u32 = self.items.iter().map(|i| u32::from(i.weight)).sum();
        let mut roll = (rng.next_f64() * f64::from(total)) as u32;
        for (index, item) in self.items.iter().enumerate() {
            let w = u32::from(item.weight);
            if roll < w {
                return index;
            }
            roll -= w;
        }
        self.items.len() - 1
    }
}

/// Which surfaces the brush may place on.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SurfaceFilter {
    /// Any surface.
    #[default]
    Any,
    /// Only surfaces of this material.
    Material(String),
    /// Only surfaces of parts with this tag.
    Tag(String),
}

impl SurfaceFilter {
    /// Whether a hit passes the filter.
    pub fn accepts(&self, hit: &SurfaceHit) -> bool {
        match self {
            SurfaceFilter::Any => true,
            SurfaceFilter::Material(m) => hit.material == *m,
            SurfaceFilter::Tag(t) => hit.tags.iter().any(|x| x == t),
        }
    }
}

/// Brush settings shown in the viewport header. [`BrushSettings::clamped`] keeps them in the
/// ranges of spec section 9.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushSettings {
    /// Brush radius in studs, 1 to 256.
    pub radius: f64,
    /// Instances per 100 square studs, 0.1 to 50.
    pub density: f64,
    /// Minimum distance between instances; `None` uses [`BrushSet::default_min_spacing`].
    pub min_spacing: Option<f64>,
    /// Smallest random uniform scale.
    pub scale_min: f64,
    /// Largest random uniform scale.
    pub scale_max: f64,
    /// Random yaw range in degrees, low end.
    pub yaw_min_deg: f64,
    /// Random yaw range in degrees, high end.
    pub yaw_max_deg: f64,
    /// How far an instance's up axis turns from world up towards the surface normal, 0 to 1.
    pub align_to_normal: f64,
    /// Steepest surface accepted, 0 to 90 degrees from flat.
    pub max_slope_deg: f64,
    /// Which surfaces are accepted.
    pub filter: SurfaceFilter,
    /// Parent folder for new instances.
    pub parent: String,
    /// Random seed of the stroke.
    pub seed: u64,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            radius: 16.0,
            density: 2.0,
            min_spacing: None,
            scale_min: 0.8,
            scale_max: 1.2,
            yaw_min_deg: 0.0,
            yaw_max_deg: 360.0,
            align_to_normal: 0.0,
            max_slope_deg: 35.0,
            filter: SurfaceFilter::Any,
            parent: "Workspace.Brush".into(),
            seed: 0,
        }
    }
}

impl BrushSettings {
    /// A copy with every value in its allowed range (non-finite values fall back to defaults).
    pub fn clamped(&self) -> Self {
        let d = Self::default();
        let fin = |v: f64, fallback: f64| if v.is_finite() { v } else { fallback };
        let scale_a = fin(self.scale_min, d.scale_min).max(0.01);
        let scale_b = fin(self.scale_max, d.scale_max).max(0.01);
        let yaw_a = fin(self.yaw_min_deg, d.yaw_min_deg);
        let yaw_b = fin(self.yaw_max_deg, d.yaw_max_deg);
        Self {
            radius: fin(self.radius, d.radius).clamp(1.0, 256.0),
            density: fin(self.density, d.density).clamp(0.1, 50.0),
            min_spacing: self
                .min_spacing
                .filter(|s| s.is_finite())
                .map(|s| s.max(0.0)),
            scale_min: scale_a.min(scale_b),
            scale_max: scale_a.max(scale_b),
            yaw_min_deg: yaw_a.min(yaw_b),
            yaw_max_deg: yaw_a.max(yaw_b),
            align_to_normal: fin(self.align_to_normal, d.align_to_normal).clamp(0.0, 1.0),
            max_slope_deg: fin(self.max_slope_deg, d.max_slope_deg).clamp(0.0, 90.0),
            filter: self.filter.clone(),
            parent: self.parent.clone(),
            seed: self.seed,
        }
    }

    /// Instances the full brush disk should hold: `density x pi r^2 / 100`.
    pub fn target_count(&self) -> f64 {
        self.density * std::f64::consts::PI * self.radius * self.radius / 100.0
    }
}

/// A ray hit reported by the scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceHit {
    /// Hit point in world space.
    pub point: DVec3,
    /// Surface normal at the hit (need not be unit length).
    pub normal: DVec3,
    /// Material of the hit part.
    pub material: String,
    /// Tags of the hit part.
    pub tags: Vec<String>,
}

/// Ray queries against the scene. The viewport implements it with its picker; tests use
/// planes.
pub trait SurfaceProbe {
    /// The first surface hit along `origin + t * direction` for `0 <= t <= max_distance`.
    /// `direction` is unit length.
    fn raycast(&self, origin: DVec3, direction: DVec3, max_distance: f64) -> Option<SurfaceHit>;
}

/// One instance to create.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Index into [`BrushSet::items`].
    pub item: usize,
    /// Pivot (bounding-box centre) in world space; the box's base sits on the surface.
    pub position: DVec3,
    /// World rotation.
    pub rotation: DQuat,
    /// Uniform scale.
    pub scale: f64,
    /// Surface point the instance stands on.
    pub surface_point: DVec3,
}

impl Placement {
    /// Rotation as a Roblox-style `Orientation` in degrees: applied Y, then X, then Z, the
    /// order `metallic_render` reads it in.
    pub fn orientation_degrees(&self) -> DVec3 {
        let (y, x, z) = self.rotation.to_euler(EulerRot::YXZ);
        DVec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
    }

    /// The instance's up axis in world space.
    pub fn up(&self) -> DVec3 {
        self.rotation * DVec3::Y
    }
}

/// Slope of a surface in degrees: 0 is flat ground, 90 a wall, 180 a ceiling.
pub fn slope_degrees(normal: DVec3) -> f64 {
    let n = normal.normalize_or_zero();
    if n == DVec3::ZERO {
        return 90.0;
    }
    n.y.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Up axis after turning world up a fraction `align` (0 to 1) of the way to `normal`, along
/// the great circle between them.
pub fn aligned_up(normal: DVec3, align: f64) -> DVec3 {
    alignment_rotation(normal, align) * DVec3::Y
}

/// The rotation that turns world up a fraction `align` of the way to `normal`.
pub fn alignment_rotation(normal: DVec3, align: f64) -> DQuat {
    let n = normal.normalize_or_zero();
    if n == DVec3::ZERO {
        return DQuat::IDENTITY;
    }
    let full = DQuat::from_rotation_arc(DVec3::Y, n);
    DQuat::IDENTITY
        .slerp(full, align.clamp(0.0, 1.0))
        .normalize()
}

/// Rotation of a placed instance: spin `yaw_deg` about its own up axis, then align that axis
/// to the surface by `align`.
pub fn surface_rotation(normal: DVec3, align: f64, yaw_deg: f64) -> DQuat {
    (alignment_rotation(normal, align) * DQuat::from_rotation_y(yaw_deg.to_radians())).normalize()
}

/// Two unit vectors that, with `normal`, form a right-handed orthonormal basis.
pub fn tangent_basis(normal: DVec3) -> (DVec3, DVec3) {
    let n = normal.normalize_or(DVec3::Y);
    let helper = if n.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
    let u = helper.cross(n).normalize();
    let v = n.cross(u);
    (u, v)
}

/// A uniform point in the unit disk.
fn unit_disk_point(rng: &mut SeededRng) -> DVec2 {
    let r = rng.next_f64().sqrt();
    let a = rng.next_f64() * std::f64::consts::TAU;
    DVec2::new(r * a.cos(), r * a.sin())
}

/// Spatial hash of points for minimum-distance checks.
#[derive(Debug, Clone)]
struct SpacingGrid {
    cell: f64,
    min_sq: f64,
    cells: HashMap<(i64, i64, i64), Vec<DVec3>>,
}

impl SpacingGrid {
    fn new(min_spacing: f64) -> Self {
        Self {
            cell: min_spacing.max(1e-3),
            min_sq: min_spacing * min_spacing,
            cells: HashMap::new(),
        }
    }

    fn key(&self, p: DVec3) -> (i64, i64, i64) {
        let k = (p / self.cell).floor();
        (k.x as i64, k.y as i64, k.z as i64)
    }

    fn insert(&mut self, p: DVec3) {
        let key = self.key(p);
        self.cells.entry(key).or_default().push(p);
    }

    fn remove(&mut self, p: DVec3) {
        let key = self.key(p);
        if let Some(list) = self.cells.get_mut(&key)
            && let Some(i) = list.iter().position(|q| *q == p)
        {
            list.swap_remove(i);
        }
    }

    fn is_clear(&self, p: DVec3) -> bool {
        if self.min_sq <= 0.0 {
            return true;
        }
        let (kx, ky, kz) = self.key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = self.cells.get(&(kx + dx, ky + dy, kz + dz))
                        && list.iter().any(|q| q.distance_squared(p) < self.min_sq)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Poisson-disk dart throwing in a flat disk: up to `count` points in a disk of `radius`
/// around the origin, no two closer than `min_spacing`. Stops early when 30 throws per point
/// fail.
pub fn poisson_disk(
    rng: &mut SeededRng,
    radius: f64,
    min_spacing: f64,
    count: usize,
) -> Vec<DVec2> {
    let mut grid = SpacingGrid::new(min_spacing);
    let mut out = Vec::with_capacity(count);
    let mut attempts = count * ATTEMPTS_PER_PLACEMENT;
    while out.len() < count && attempts > 0 {
        attempts -= 1;
        let p = unit_disk_point(rng) * radius;
        let p3 = DVec3::new(p.x, p.y, 0.0);
        if grid.is_clear(p3) {
            grid.insert(p3);
            out.push(p);
        }
    }
    out
}

/// An instance already in the scene, as a stroke sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushInstance<I> {
    /// The instance.
    pub id: I,
    /// Its pivot position in world space.
    pub position: DVec3,
    /// Its tags. Only instances tagged by the stroke's set ([`BrushSet::owns`]) take part.
    pub tags: Vec<String>,
}

impl<I: Copy> BrushInstance<I> {
    /// An instance from a [`PartSnapshot`] and the part's tags.
    pub fn from_snapshot(part: &PartSnapshot<I>, tags: Vec<String>) -> Self {
        Self {
            id: part.id,
            position: part.position,
            tags,
        }
    }
}

/// Paint or erase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeMode {
    /// Add instances (left-drag).
    Paint,
    /// Remove this brush's instances (Shift-drag).
    Erase,
}

/// Everything one stroke did: the single undo step the history records.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeCommit<I> {
    /// Brush set name (for the tag and the undo name).
    pub set_name: String,
    /// Tag to give each new instance.
    pub tag: String,
    /// Parent folder for new instances.
    pub parent: String,
    /// Instances to create, in placement order.
    pub added: Vec<Placement>,
    /// Existing instances to remove.
    pub removed: Vec<I>,
}

impl<I> StrokeCommit<I> {
    /// Undo name in the spec's "Verb object (count)" form.
    pub fn undo_name(&self) -> String {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        if self.removed.is_empty() {
            let n = self.added.len();
            format!("Brush {} ({n} instance{})", self.set_name, plural(n))
        } else {
            let n = self.removed.len();
            format!("Erase {} ({n} instance{})", self.set_name, plural(n))
        }
    }
}

/// One brush stroke in progress. The world is not touched until the history applies the
/// [`StrokeCommit`] from [`Stroke::finish`]; the viewport draws [`Stroke::added`] as a preview.
#[derive(Debug, Clone)]
pub struct Stroke<I> {
    set: BrushSet,
    settings: BrushSettings,
    mode: StrokeMode,
    min_spacing: f64,
    grid: SpacingGrid,
    /// This brush's instances that existed when the stroke began and are still kept.
    existing: Vec<(I, DVec3)>,
    added: Vec<Placement>,
    removed: Vec<I>,
    dabs: u64,
    allowance: f64,
}

impl<I: Copy + PartialEq> Stroke<I> {
    /// Begins a stroke. `existing` may list any scene instances near the brush: the stroke
    /// keeps only those [`BrushSet::owns`] (tag `Brush:<set name>`), so erase never removes,
    /// and spacing never counts, an instance this set did not place.
    pub fn begin(
        set: BrushSet,
        settings: &BrushSettings,
        mode: StrokeMode,
        existing: impl IntoIterator<Item = BrushInstance<I>>,
    ) -> Self {
        let settings = settings.clamped();
        let min_spacing = settings
            .min_spacing
            .unwrap_or_else(|| set.default_min_spacing());
        let existing: Vec<(I, DVec3)> = existing
            .into_iter()
            .filter(|inst| set.owns(&inst.tags))
            .map(|inst| (inst.id, inst.position))
            .collect();
        let mut grid = SpacingGrid::new(min_spacing);
        for (_, p) in &existing {
            grid.insert(*p);
        }
        Self {
            set,
            settings,
            mode,
            min_spacing,
            grid,
            existing,
            added: Vec::new(),
            removed: Vec::new(),
            dabs: 0,
            allowance: 0.0,
        }
    }

    /// The settings in use (clamped).
    pub fn settings(&self) -> &BrushSettings {
        &self.settings
    }

    /// The minimum spacing in use.
    pub fn min_spacing(&self) -> f64 {
        self.min_spacing
    }

    /// Placements so far (the live preview).
    pub fn added(&self) -> &[Placement] {
        &self.added
    }

    /// Instances erased so far.
    pub fn removed(&self) -> &[I] {
        &self.removed
    }

    /// Brush instances inside the disk around `centre`, counting this stroke's placements.
    fn count_in_disk(&self, centre: DVec3) -> usize {
        let r_sq = self.settings.radius * self.settings.radius;
        let kept = self
            .existing
            .iter()
            .filter(|(_, p)| p.distance_squared(centre) <= r_sq)
            .count();
        let new = self
            .added
            .iter()
            .filter(|p| p.surface_point.distance_squared(centre) <= r_sq)
            .count();
        kept + new
    }

    /// One pointer sample: the surface point and normal under the cursor, and the time since
    /// the last dab in seconds (use [`DWELL_STEP_SECONDS`] for the press). Returns how many
    /// instances this dab added or erased.
    pub fn dab(
        &mut self,
        probe: &dyn SurfaceProbe,
        centre: DVec3,
        normal: DVec3,
        dt: f64,
    ) -> usize {
        let k = self.dabs;
        self.dabs += 1;
        match self.mode {
            StrokeMode::Erase => self.erase_dab(centre),
            StrokeMode::Paint => self.paint_dab(probe, centre, normal, dt, k),
        }
    }

    fn erase_dab(&mut self, centre: DVec3) -> usize {
        let r_sq = self.settings.radius * self.settings.radius;
        let mut erased = 0;
        let mut i = 0;
        while i < self.existing.len() {
            let (id, p) = self.existing[i];
            if p.distance_squared(centre) <= r_sq {
                self.existing.swap_remove(i);
                self.grid.remove(p);
                self.removed.push(id);
                erased += 1;
            } else {
                i += 1;
            }
        }
        erased
    }

    fn paint_dab(
        &mut self,
        probe: &dyn SurfaceProbe,
        centre: DVec3,
        normal: DVec3,
        dt: f64,
        k: u64,
    ) -> usize {
        let s = &self.settings;
        let target = s.target_count();
        self.allowance = (self.allowance + target * (dt.max(0.0) / DWELL_STEP_SECONDS)).min(target);
        let lacking = (target.round() as usize).saturating_sub(self.count_in_disk(centre));
        let wanted = lacking
            .min(self.allowance.floor() as usize)
            .min(MAX_PLACEMENTS_PER_DAB);
        if wanted == 0 {
            return 0;
        }
        let mut rng = SeededRng::for_stream(s.seed, k);
        let n = normal.normalize_or(DVec3::Y);
        let (u, v) = tangent_basis(n);
        let radius = s.radius;
        let mut placed = 0;
        let mut attempts = wanted * ATTEMPTS_PER_PLACEMENT;
        while placed < wanted && attempts > 0 {
            attempts -= 1;
            let d = unit_disk_point(&mut rng) * radius;
            // Draw every random value of the throw up front so rejections do not shift the
            // stream differently on different scenes more than needed.
            let item = self.set.pick(&mut rng);
            let scale = rng.range(s.scale_min, s.scale_max);
            let yaw = rng.range(s.yaw_min_deg, s.yaw_max_deg);
            let origin = centre + u * d.x + v * d.y + n * radius;
            let Some(hit) = probe.raycast(origin, -n, radius * 2.0) else {
                continue;
            };
            if !s.filter.accepts(&hit) || slope_degrees(hit.normal) > s.max_slope_deg + 1e-9 {
                continue;
            }
            if !self.grid.is_clear(hit.point) {
                continue;
            }
            let rotation = surface_rotation(hit.normal, s.align_to_normal, yaw);
            let half_height = self.set.items[item].size.y * scale * 0.5;
            let position = hit.point + rotation * DVec3::Y * half_height;
            self.grid.insert(hit.point);
            self.added.push(Placement {
                item,
                position,
                rotation,
                scale,
                surface_point: hit.point,
            });
            placed += 1;
        }
        self.allowance -= placed as f64;
        placed
    }

    /// Ends the stroke. Returns `None` when it changed nothing (no undo entry).
    pub fn finish(self) -> Option<StrokeCommit<I>> {
        if self.added.is_empty() && self.removed.is_empty() {
            return None;
        }
        Some(StrokeCommit {
            tag: self.set.tag(),
            set_name: self.set.name,
            parent: self.settings.parent,
            added: self.added,
            removed: self.removed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An infinite plane through `point` with `normal`.
    struct Plane {
        point: DVec3,
        normal: DVec3,
        material: &'static str,
    }

    impl Plane {
        fn ground() -> Self {
            Self {
                point: DVec3::ZERO,
                normal: DVec3::Y,
                material: "Grass",
            }
        }
        fn tilted(deg: f64) -> Self {
            Self {
                point: DVec3::ZERO,
                normal: DQuat::from_rotation_z(deg.to_radians()) * DVec3::Y,
                material: "Grass",
            }
        }
    }

    impl SurfaceProbe for Plane {
        fn raycast(&self, origin: DVec3, dir: DVec3, max: f64) -> Option<SurfaceHit> {
            let n = self.normal.normalize();
            let denom = n.dot(dir);
            if denom.abs() < 1e-12 {
                return None;
            }
            let t = n.dot(self.point - origin) / denom;
            (0.0..=max).contains(&t).then(|| SurfaceHit {
                point: origin + dir * t,
                normal: n,
                material: self.material.into(),
                tags: vec!["Ground".into()],
            })
        }
    }

    fn rock_set() -> BrushSet {
        BrushSet::new(
            "Rocks",
            vec![
                BrushItem::new("Rock A", 3, DVec3::splat(2.0)),
                BrushItem::new("Rock B", 1, DVec3::new(1.0, 2.0, 1.0)),
            ],
        )
        .unwrap()
    }

    fn min_pair_distance(points: &[DVec3]) -> f64 {
        let mut best = f64::INFINITY;
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                best = best.min(points[i].distance(points[j]));
            }
        }
        best
    }

    fn paint_once(settings: &BrushSettings, probe: &dyn SurfaceProbe) -> Vec<Placement> {
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), settings, StrokeMode::Paint, Vec::new());
        stroke.dab(probe, DVec3::ZERO, DVec3::Y, DWELL_STEP_SECONDS);
        stroke.added().to_vec()
    }

    #[test]
    fn set_size_limits() {
        assert_eq!(BrushSet::new("x", vec![]), Err(ScatterError::EmptySet));
        let many = vec![BrushItem::new("p", 1, DVec3::ONE); 17];
        assert_eq!(
            BrushSet::new("x", many),
            Err(ScatterError::TooManyItems(17))
        );
        assert_eq!(BrushItem::new("p", 0, DVec3::ONE).weight, 1);
        assert_eq!(BrushItem::new("p", 99, DVec3::ONE).weight, 10);
    }

    #[test]
    fn settings_clamp_to_spec_ranges() {
        let s = BrushSettings {
            radius: 1000.0,
            density: 0.0,
            scale_min: 2.0,
            scale_max: 0.5,
            align_to_normal: 3.0,
            max_slope_deg: f64::NAN,
            ..BrushSettings::default()
        }
        .clamped();
        assert_eq!(s.radius, 256.0);
        assert_eq!(s.density, 0.1);
        assert_eq!((s.scale_min, s.scale_max), (0.5, 2.0));
        assert_eq!(s.align_to_normal, 1.0);
        assert_eq!(s.max_slope_deg, 35.0);
    }

    #[test]
    fn default_min_spacing_is_half_smallest_bounding_radius() {
        let set = rock_set();
        let smallest = DVec3::new(1.0, 2.0, 1.0).length() * 0.5;
        assert!((set.default_min_spacing() - smallest * 0.5).abs() < 1e-12);
    }

    #[test]
    fn flat_poisson_disk_respects_spacing_and_count() {
        let mut rng = SeededRng::new(9);
        let pts = poisson_disk(&mut rng, 16.0, 2.0, 60);
        assert_eq!(pts.len(), 60);
        for (i, a) in pts.iter().enumerate() {
            assert!(a.length() <= 16.0 + 1e-9);
            for b in &pts[i + 1..] {
                assert!(a.distance(*b) >= 2.0);
            }
        }
    }

    /// Issue #219 "done when": a 32-stud disk at density 2 gets the expected count within 10%
    /// with no pair closer than the minimum spacing.
    #[test]
    fn disk_of_32_studs_at_density_2() {
        let settings = BrushSettings {
            radius: 16.0,
            density: 2.0,
            min_spacing: Some(3.0),
            seed: 1234,
            ..BrushSettings::default()
        };
        let expected = settings.target_count();
        let placed = paint_once(&settings, &Plane::ground());
        let n = placed.len() as f64;
        assert!(
            (n - expected).abs() <= expected * 0.1,
            "placed {n}, expected {expected}"
        );
        let points: Vec<DVec3> = placed.iter().map(|p| p.surface_point).collect();
        assert!(min_pair_distance(&points) >= 3.0);
        for p in &placed {
            assert!(p.surface_point.length() <= 16.0 + 1e-9);
            assert!(p.surface_point.y.abs() < 1e-9);
        }
    }

    #[test]
    fn dwell_fills_to_density_then_stops() {
        // A large disk needs several dabs because one dab adds at most 64.
        let settings = BrushSettings {
            radius: 32.0,
            density: 4.0,
            min_spacing: Some(1.5),
            seed: 5,
            ..BrushSettings::default()
        };
        let target = settings.target_count();
        let probe = Plane::ground();
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &settings, StrokeMode::Paint, Vec::new());
        let first = stroke.dab(&probe, DVec3::ZERO, DVec3::Y, DWELL_STEP_SECONDS);
        assert_eq!(first, MAX_PLACEMENTS_PER_DAB);
        for _ in 0..20 {
            stroke.dab(&probe, DVec3::ZERO, DVec3::Y, DWELL_STEP_SECONDS);
        }
        let n = stroke.added().len() as f64;
        assert!(
            (n - target).abs() <= target * 0.1,
            "placed {n}, target {target}"
        );
        assert_eq!(stroke.dab(&probe, DVec3::ZERO, DVec3::Y, 1.0), 0);
        let points: Vec<DVec3> = stroke.added().iter().map(|p| p.surface_point).collect();
        assert!(min_pair_distance(&points) >= 1.5);
    }

    #[test]
    fn short_dwell_adds_proportionally_less() {
        let settings = BrushSettings {
            min_spacing: Some(1.0),
            ..BrushSettings::default()
        };
        let probe = Plane::ground();
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &settings, StrokeMode::Paint, Vec::new());
        // 0.05 s allows half the disk's ~16 instances.
        let n = stroke.dab(&probe, DVec3::ZERO, DVec3::Y, 0.05);
        assert_eq!(n, 8);
    }

    #[test]
    fn spacing_holds_against_earlier_strokes() {
        let settings = BrushSettings {
            min_spacing: Some(3.0),
            seed: 77,
            ..BrushSettings::default()
        };
        let probe = Plane::ground();
        let first = paint_once(&settings, &probe);
        let tag = rock_set().tag();
        let existing: Vec<BrushInstance<u32>> = first
            .iter()
            .enumerate()
            .map(|(i, p)| BrushInstance {
                id: i as u32,
                position: p.surface_point,
                tags: vec![tag.clone()],
            })
            .collect();
        let mut stroke = Stroke::begin(rock_set(), &settings, StrokeMode::Paint, existing);
        // The disk is already at density, so a dab at the same spot adds nothing;
        // a dab half a radius away adds only where there is room.
        assert_eq!(stroke.dab(&probe, DVec3::ZERO, DVec3::Y, 0.1), 0);
        stroke.dab(&probe, DVec3::new(8.0, 0.0, 0.0), DVec3::Y, 0.1);
        let mut all: Vec<DVec3> = first.iter().map(|p| p.surface_point).collect();
        all.extend(stroke.added().iter().map(|p| p.surface_point));
        assert!(min_pair_distance(&all) >= 3.0);
    }

    #[test]
    fn same_seed_same_placements_and_different_seed_differs() {
        let settings = BrushSettings {
            seed: 42,
            ..BrushSettings::default()
        };
        let a = paint_once(&settings, &Plane::ground());
        let b = paint_once(&settings, &Plane::ground());
        assert!(!a.is_empty());
        assert_eq!(a, b);
        let other = BrushSettings {
            seed: 43,
            ..settings
        };
        assert_ne!(a, paint_once(&other, &Plane::ground()));
    }

    /// Issue #219 "done when": the slope filter rejects a 60-degree face.
    #[test]
    fn slope_filter_rejects_60_degree_face() {
        let settings = BrushSettings::default();
        let face = Plane::tilted(60.0);
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &settings, StrokeMode::Paint, Vec::new());
        assert_eq!(stroke.dab(&face, DVec3::ZERO, face.normal, 0.1), 0);
        assert!(stroke.finish().is_none());
        // The same face is accepted when the limit allows it.
        let steep_ok = BrushSettings {
            max_slope_deg: 61.0,
            ..BrushSettings::default()
        };
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &steep_ok, StrokeMode::Paint, Vec::new());
        assert!(stroke.dab(&face, DVec3::ZERO, face.normal, 0.1) > 0);
        assert!((slope_degrees(face.normal) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn surface_filter_by_material_and_tag() {
        let probe = Plane::ground();
        for (filter, ok) in [
            (SurfaceFilter::Material("Grass".into()), true),
            (SurfaceFilter::Material("Sand".into()), false),
            (SurfaceFilter::Tag("Ground".into()), true),
            (SurfaceFilter::Tag("Road".into()), false),
        ] {
            let s = BrushSettings {
                filter,
                ..BrushSettings::default()
            };
            assert_eq!(!paint_once(&s, &probe).is_empty(), ok);
        }
    }

    #[test]
    fn scale_and_yaw_stay_in_range_and_base_sits_on_surface() {
        let settings = BrushSettings {
            scale_min: 0.5,
            scale_max: 1.5,
            yaw_min_deg: 10.0,
            yaw_max_deg: 20.0,
            seed: 3,
            ..BrushSettings::default()
        };
        let set = rock_set();
        for p in paint_once(&settings, &Plane::ground()) {
            assert!((0.5..1.5).contains(&p.scale));
            let yaw = p.orientation_degrees().y;
            assert!((10.0 - 1e-9..20.0 + 1e-9).contains(&yaw), "yaw {yaw}");
            let half = set.items()[p.item].size.y * p.scale * 0.5;
            assert!((p.position - p.surface_point - DVec3::Y * half).length() < 1e-9);
        }
    }

    #[test]
    fn weighted_pick_follows_weights() {
        let set = rock_set(); // weights 3 and 1
        let mut rng = SeededRng::new(11);
        let n = 40_000;
        let a = (0..n).filter(|_| set.pick(&mut rng) == 0).count() as f64 / n as f64;
        assert!((a - 0.75).abs() < 0.01, "share {a}");
    }

    #[test]
    fn alignment_maths() {
        let n = DVec3::new(1.0, 1.0, 0.0).normalize(); // 45-degree slope
        assert!((aligned_up(n, 0.0) - DVec3::Y).length() < 1e-12);
        assert!((aligned_up(n, 1.0) - n).length() < 1e-12);
        // Half alignment is half the angle, on the great circle between up and the normal.
        let half = aligned_up(n, 0.5);
        assert!((half.angle_between(DVec3::Y).to_degrees() - 22.5).abs() < 1e-9);
        assert!((half.angle_between(n).to_degrees() - 22.5).abs() < 1e-9);
        assert!(half.z.abs() < 1e-12);
        // Yaw spins about the aligned up axis and does not move it.
        for yaw in [0.0, 37.0, 180.0, 300.0] {
            let r = surface_rotation(n, 1.0, yaw);
            assert!((r * DVec3::Y - n).length() < 1e-12);
            assert!((r.length() - 1.0).abs() < 1e-12);
        }
        // An upside-down normal still gives a valid rotation.
        assert!((aligned_up(-DVec3::Y, 1.0) + DVec3::Y).length() < 1e-9);
        // Unnormalised and zero normals.
        assert!((aligned_up(DVec3::new(0.0, 0.0, 5.0), 1.0) - DVec3::Z).length() < 1e-12);
        assert_eq!(alignment_rotation(DVec3::ZERO, 1.0), DQuat::IDENTITY);
    }

    #[test]
    fn tangent_basis_is_orthonormal() {
        for n in [DVec3::Y, -DVec3::Y, DVec3::X, DVec3::new(1.0, 2.0, 3.0)] {
            let (u, v) = tangent_basis(n);
            let n = n.normalize();
            assert!(u.dot(v).abs() < 1e-12 && u.dot(n).abs() < 1e-12 && v.dot(n).abs() < 1e-12);
            assert!((u.length() - 1.0).abs() < 1e-12 && (v.length() - 1.0).abs() < 1e-12);
            assert!((u.cross(v) - n).length() < 1e-12);
        }
    }

    #[test]
    fn aligned_placements_on_a_slope_follow_the_normal() {
        let face = Plane::tilted(30.0);
        let settings = BrushSettings {
            align_to_normal: 1.0,
            ..BrushSettings::default()
        };
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &settings, StrokeMode::Paint, Vec::new());
        assert!(stroke.dab(&face, DVec3::ZERO, face.normal, 0.1) > 0);
        for p in stroke.added() {
            assert!((p.up() - face.normal).length() < 1e-9);
            // On the plane.
            assert!(face.normal.dot(p.surface_point).abs() < 1e-9);
        }
    }

    #[test]
    fn orientation_degrees_round_trip() {
        let r = surface_rotation(DVec3::new(0.3, 1.0, -0.2), 1.0, 70.0);
        let o = Placement {
            item: 0,
            position: DVec3::ZERO,
            rotation: r,
            scale: 1.0,
            surface_point: DVec3::ZERO,
        }
        .orientation_degrees();
        let back = DQuat::from_euler(
            EulerRot::YXZ,
            o.y.to_radians(),
            o.x.to_radians(),
            o.z.to_radians(),
        );
        assert!(back.dot(r).abs() > 1.0 - 1e-12);
    }

    #[test]
    fn erase_removes_only_this_brush_inside_radius() {
        let set = rock_set();
        let tag = set.tag();
        assert_eq!(tag, "Brush:Rocks");
        // Scene: (id, position, tags).
        let scene: Vec<(u32, DVec3, Vec<String>)> = vec![
            (1, DVec3::new(1.0, 0.0, 0.0), vec![tag.clone()]),
            (2, DVec3::new(5.0, 0.0, 0.0), vec!["Brush:Trees".into()]),
            (3, DVec3::new(40.0, 0.0, 0.0), vec![tag.clone()]),
            (
                4,
                DVec3::new(0.0, 0.0, -10.0),
                vec![tag.clone(), "Keep".into()],
            ),
            (5, DVec3::new(2.0, 0.0, 2.0), vec![]),
        ];
        // The whole scene goes in; the stroke itself keeps only this set's instances.
        let all = scene.iter().map(|(id, p, tags)| BrushInstance {
            id: *id,
            position: *p,
            tags: tags.clone(),
        });
        let mut stroke = Stroke::begin(set, &BrushSettings::default(), StrokeMode::Erase, all);
        let probe = Plane::ground();
        assert_eq!(stroke.dab(&probe, DVec3::ZERO, DVec3::Y, 0.1), 2);
        // A second dab on the same spot finds nothing left.
        assert_eq!(stroke.dab(&probe, DVec3::ZERO, DVec3::Y, 0.1), 0);
        let commit = stroke.finish().unwrap();
        let mut removed = commit.removed.clone();
        removed.sort();
        assert_eq!(removed, vec![1, 4]);
        assert!(commit.added.is_empty());
        assert_eq!(commit.undo_name(), "Erase Rocks (2 instances)");
    }

    #[test]
    fn erase_never_removes_parts_the_brush_did_not_place() {
        let set = rock_set();
        // Untagged, other-brush and look-alike tags, all inside the radius.
        let scene = vec![
            BrushInstance {
                id: 1u32,
                position: DVec3::ZERO,
                tags: vec![],
            },
            BrushInstance {
                id: 2,
                position: DVec3::X,
                tags: vec!["Brush:Trees".into()],
            },
            BrushInstance {
                id: 3,
                position: DVec3::Z,
                tags: vec!["Brush:Rocks2".into()],
            },
            BrushInstance {
                id: 4,
                position: DVec3::NEG_X,
                tags: vec!["brush:rocks".into()],
            },
        ];
        let mut stroke = Stroke::begin(
            set.clone(),
            &BrushSettings::default(),
            StrokeMode::Erase,
            scene.clone(),
        );
        assert_eq!(stroke.dab(&Plane::ground(), DVec3::ZERO, DVec3::Y, 0.1), 0);
        assert!(stroke.finish().is_none());
        // Foreign instances do not block painting either: spacing is only against this set.
        let crowded = BrushSettings {
            min_spacing: Some(3.0),
            ..BrushSettings::default()
        };
        let mut paint = Stroke::begin(set, &crowded, StrokeMode::Paint, scene);
        assert_eq!(paint.dab(&Plane::ground(), DVec3::ZERO, DVec3::Y, 0.1), 16);
    }

    /// A toy history target: the scene as a list of brush instances.
    #[derive(Debug, Clone, PartialEq)]
    struct ToyScene {
        next_id: u32,
        items: Vec<(u32, DVec3)>,
    }

    impl ToyScene {
        /// Applies a commit; returns the ids it created (what a history keeps to undo).
        fn apply(
            &mut self,
            c: &StrokeCommit<u32>,
            removed_store: &mut Vec<(u32, DVec3)>,
        ) -> Vec<u32> {
            removed_store.clear();
            self.items.retain(|it| {
                let gone = c.removed.contains(&it.0);
                if gone {
                    removed_store.push(*it);
                }
                !gone
            });
            c.added
                .iter()
                .map(|p| {
                    self.next_id += 1;
                    self.items.push((self.next_id, p.surface_point));
                    self.next_id
                })
                .collect()
        }
        fn revert(&mut self, created: &[u32], removed_store: &[(u32, DVec3)]) {
            self.items.retain(|it| !created.contains(&it.0));
            self.items.extend_from_slice(removed_store);
        }
    }

    #[test]
    fn a_stroke_of_many_dabs_is_one_undo_step() {
        let settings = BrushSettings {
            seed: 8,
            min_spacing: Some(2.0),
            ..BrushSettings::default()
        };
        let probe = Plane::ground();
        let mut stroke: Stroke<u32> =
            Stroke::begin(rock_set(), &settings, StrokeMode::Paint, Vec::new());
        let mut total = 0;
        for i in 0..10 {
            total += stroke.dab(&probe, DVec3::new(i as f64 * 4.0, 0.0, 0.0), DVec3::Y, 0.1);
        }
        let commit = stroke.finish().expect("stroke placed something");
        assert_eq!(commit.added.len(), total);
        assert_eq!(commit.tag, "Brush:Rocks");
        assert_eq!(commit.parent, "Workspace.Brush");
        assert_eq!(
            commit.undo_name(),
            format!("Brush Rocks ({total} instances)")
        );

        let mut scene = ToyScene {
            next_id: 100,
            items: vec![(7, DVec3::new(-50.0, 0.0, 0.0))],
        };
        let before = scene.items.clone();
        let mut removed = Vec::new();
        let created = scene.apply(&commit, &mut removed);
        assert_eq!(scene.items.len(), before.len() + total);
        scene.revert(&created, &removed);
        assert_eq!(scene.items, before);

        // An erase stroke over the painted area, then undo, restores the painted scene.
        let created_now = scene.apply(&commit, &mut removed);
        let painted = scene.items.clone();
        let tag = rock_set().tag();
        let everything: Vec<BrushInstance<u32>> = scene
            .items
            .iter()
            .map(|&(id, position)| BrushInstance {
                id,
                position,
                tags: if created_now.contains(&id) {
                    vec![tag.clone()]
                } else {
                    vec![]
                },
            })
            .collect();
        let mut erase = Stroke::begin(rock_set(), &settings, StrokeMode::Erase, everything);
        erase.dab(&probe, DVec3::new(10.0, 0.0, 0.0), DVec3::Y, 0.1);
        let erase_commit = erase.finish().expect("erased something");
        let mut erased_store = Vec::new();
        let none = scene.apply(&erase_commit, &mut erased_store);
        assert!(none.is_empty());
        assert!(scene.items.len() < painted.len());
        assert!(
            scene.items.iter().any(|it| it.0 == 7),
            "foreign instance kept"
        );
        scene.revert(&none, &erased_store);
        let mut a = scene.items.clone();
        let mut b = painted;
        a.sort_by_key(|it| it.0);
        b.sort_by_key(|it| it.0);
        assert_eq!(a, b);
    }

    #[test]
    fn ray_misses_place_nothing() {
        struct Nothing;
        impl SurfaceProbe for Nothing {
            fn raycast(&self, _: DVec3, _: DVec3, _: f64) -> Option<SurfaceHit> {
                None
            }
        }
        let mut stroke: Stroke<u32> = Stroke::begin(
            rock_set(),
            &BrushSettings::default(),
            StrokeMode::Paint,
            Vec::new(),
        );
        assert_eq!(stroke.dab(&Nothing, DVec3::ZERO, DVec3::Y, 0.1), 0);
        assert!(stroke.finish().is_none());
    }
}
