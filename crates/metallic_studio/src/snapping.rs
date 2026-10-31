//! The snapping engine (spec section 5, #215).
//!
//! # Targets and priority
//!
//! During a drag the snapper collects point candidates within [`SNAP_RADIUS_PX`] of the cursor
//! and takes the first kind that exists, in [`SnapKind`] order: vertex, edge midpoint, face
//! centre, point on edge. Without a point candidate the body drag follows the surface under the
//! cursor ([`place_on_surface`]), and the grid applies in that surface's own plane. With every snap
//! chip off, soft snapping still pulls to a vertex or edge midpoint within [`SOFT_RADIUS_PX`]
//! unless `Snapping::soft` is off.
//!
//! # Sources
//!
//! Candidates come from [`SnapSource`]s. [`PickCache`] is one (box corners, edge midpoints, face
//! centres and the nearest point on each edge of every part near the cursor); the modeller (#136)
//! adds its own vertices and edges by implementing the trait. Sources write into a caller-owned
//! `Vec` so a drag allocates nothing per frame. Parts far from the cursor on screen are skipped
//! with one projection each, so 5,000 parts cost well under the 0.5 ms budget (section 17).
//!
//! # Glyphs
//!
//! [`paint_glyph`] draws the snapped target's shape (filled square, triangle, ring, diamond,
//! crosshair), silver with a graphite ring, so colour is never the only signal; the status line
//! names it ("Vertex of Wall_03", [`SnapKind::label`]). [`paint_grid_patch`] brightens a 4-cell
//! grid patch in the surface plane.
use eframe::egui::{self, Pos2, Shape, Stroke};
use metallic_core::InstanceId;
use metallic_render::glam::{DQuat, DVec3};

use crate::camera::ViewProjection;
use crate::editor::Snapping;
use crate::picking::PickCache;
use crate::tokens::ui_tokens;
use crate::transform::{PartTransform, Ray, snap};

/// Screen radius (points) of the snapper.
pub const SNAP_RADIUS_PX: f64 = 12.0;
/// Screen radius (points) of soft snapping.
pub const SOFT_RADIUS_PX: f64 = 6.0;

/// What a candidate is, in priority order (first wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapKind {
    /// A box corner or mesh vertex.
    Vertex,
    /// The middle of an edge.
    EdgeMid,
    /// The centre of a face.
    FaceCentre,
    /// The point of an edge nearest the cursor.
    OnEdge,
    /// The surface under the cursor.
    Surface,
    /// A grid point.
    Grid,
}

impl SnapKind {
    /// Status line name.
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Vertex => "Vertex",
            SnapKind::EdgeMid => "Edge midpoint",
            SnapKind::FaceCentre => "Face centre",
            SnapKind::OnEdge => "Edge",
            SnapKind::Surface => "Surface",
            SnapKind::Grid => "Grid",
        }
    }
}

/// Which point kinds a query wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapKinds {
    /// Box corners.
    pub vertex: bool,
    /// Edge midpoints.
    pub edge_mid: bool,
    /// Face centres.
    pub face_centre: bool,
    /// Nearest point on an edge.
    pub on_edge: bool,
}

impl SnapKinds {
    /// Every kind.
    pub const ALL: SnapKinds = SnapKinds {
        vertex: true,
        edge_mid: true,
        face_centre: true,
        on_edge: true,
    };

    /// Only vertices.
    pub const VERTEX: SnapKinds = SnapKinds {
        vertex: true,
        edge_mid: false,
        face_centre: false,
        on_edge: false,
    };

    /// True when no kind is wanted.
    pub fn is_empty(&self) -> bool {
        !(self.vertex || self.edge_mid || self.face_centre || self.on_edge)
    }

    /// What the header chips and the V key ask for, and the radius to search (spec 5.1, 5.2).
    /// V held forces vertex, edge midpoint and face centre; the Vertex chip adds vertices; the
    /// Edge chip adds edge midpoints, face centres and points on edges. With every chip off, soft
    /// snapping looks for vertices and edge midpoints within the smaller radius.
    pub fn for_settings(s: &Snapping, v_held: bool) -> (SnapKinds, f64) {
        let mut k = SnapKinds {
            vertex: v_held || s.vertex,
            edge_mid: v_held || s.edge,
            face_centre: v_held || s.edge,
            on_edge: s.edge,
        };
        if k.is_empty() && !(s.grid || s.surface) && s.soft {
            k.vertex = true;
            k.edge_mid = true;
            return (k, SOFT_RADIUS_PX);
        }
        (k, SNAP_RADIUS_PX)
    }
}

/// One snap target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapCandidate {
    /// World position.
    pub point: DVec3,
    /// Its kind.
    pub kind: SnapKind,
    /// The instance it belongs to.
    pub owner: Option<InstanceId>,
    /// Distance to the cursor in points.
    pub screen_dist: f64,
}

/// A candidate search around the cursor.
pub struct SnapQuery<'a> {
    /// The camera.
    pub view: &'a ViewProjection,
    /// Cursor in viewport points.
    pub cursor: (f64, f64),
    /// Search radius in points.
    pub radius_px: f64,
    /// Kinds wanted.
    pub kinds: SnapKinds,
    /// Instances to leave out (the dragged selection).
    pub skip: &'a dyn Fn(InstanceId) -> bool,
}

/// Something that offers snap points (parts, the modeller's vertices and edges).
pub trait SnapSource {
    /// Appends the candidates within `q.radius_px` of the cursor to `out`.
    fn candidates(&self, q: &SnapQuery<'_>, out: &mut Vec<SnapCandidate>);
}

const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Pixels per stud at distance 1 from the eye, for cheap screen-size estimates.
fn px_per_stud_at_unit(view: &ViewProjection) -> f64 {
    let centre = view.ray(view.size.0 * 0.5, view.size.1 * 0.5);
    let spp = view.studs_per_point(centre.at(1.0));
    if spp > 0.0 { 1.0 / spp } else { 0.0 }
}

/// Appends the snap points of one box `t` near the cursor.
pub fn box_candidates(
    t: &PartTransform,
    owner: Option<InstanceId>,
    q: &SnapQuery<'_>,
    out: &mut Vec<SnapCandidate>,
) {
    let corners = t.corners();
    let mut push = |p: DVec3, kind: SnapKind| {
        if let Some((x, y)) = q.view.project(p) {
            let d = ((x - q.cursor.0).powi(2) + (y - q.cursor.1).powi(2)).sqrt();
            if d <= q.radius_px {
                out.push(SnapCandidate {
                    point: p,
                    kind,
                    owner,
                    screen_dist: d,
                });
            }
        }
    };
    if q.kinds.vertex {
        for c in corners {
            push(c, SnapKind::Vertex);
        }
    }
    if q.kinds.edge_mid {
        for (a, b) in EDGES {
            push((corners[a] + corners[b]) * 0.5, SnapKind::EdgeMid);
        }
    }
    if q.kinds.face_centre {
        for axis in 0..3 {
            for s in [-1.0, 1.0] {
                push(
                    t.position + t.axis(axis) * (t.size[axis] * 0.5 * s),
                    SnapKind::FaceCentre,
                );
            }
        }
    }
    if q.kinds.on_edge {
        let ray = q.view.ray(q.cursor.0, q.cursor.1);
        for (a, b) in EDGES {
            let (pa, pb) = (corners[a], corners[b]);
            let len = (pb - pa).length();
            if len < 1e-9 {
                continue;
            }
            let dir = (pb - pa) / len;
            if let Some(s) = ray.closest_on_line(pa, dir) {
                push(pa + dir * s.clamp(0.0, len), SnapKind::OnEdge);
            }
        }
    }
}

impl SnapSource for PickCache {
    fn candidates(&self, q: &SnapQuery<'_>, out: &mut Vec<SnapCandidate>) {
        if q.kinds.is_empty() {
            return;
        }
        let k = px_per_stud_at_unit(q.view);
        for (id, t, _) in self.iter() {
            if (q.skip)(id) {
                continue;
            }
            // Cull by the bounding sphere on screen (1.5x margin for perspective at the edges).
            let Some((x, y)) = q.view.project(t.position) else {
                continue;
            };
            let depth = (t.position - q.view.eye).length().max(1e-6);
            let r_px = t.size.length() * 0.5 * k / depth * 1.5;
            let d = ((x - q.cursor.0).powi(2) + (y - q.cursor.1).powi(2)).sqrt();
            if d - r_px > q.radius_px {
                continue;
            }
            box_candidates(t, Some(id), q, out);
        }
    }
}

/// The winning candidate: lowest [`SnapKind`] first, then nearest the cursor.
pub fn best(candidates: &[SnapCandidate]) -> Option<SnapCandidate> {
    candidates.iter().copied().min_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.screen_dist.total_cmp(&b.screen_dist))
    })
}

/// Runs `sources` for `q` and returns the winner. `buf` is reused between frames.
pub fn find(
    sources: &[&dyn SnapSource],
    q: &SnapQuery<'_>,
    buf: &mut Vec<SnapCandidate>,
) -> Option<SnapCandidate> {
    buf.clear();
    for s in sources {
        s.candidates(q, buf);
    }
    best(buf)
}

/// A surface to place on: the hit point, its normal and the grid frame in its plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    /// Point on the surface.
    pub point: DVec3,
    /// Outward unit normal.
    pub normal: DVec3,
    /// Grid origin (the hit part's pivot; world zero for the virtual ground plane).
    pub origin: DVec3,
    /// Grid axes (the hit part's rotation; identity for the ground plane).
    pub rotation: DQuat,
    /// The part hit; `None` for a virtual surface.
    pub part: Option<InstanceId>,
}

impl Surface {
    /// A horizontal virtual plane at height `y` with the world grid.
    pub fn ground(y: f64) -> Surface {
        Surface {
            point: DVec3::new(0.0, y, 0.0),
            normal: DVec3::Y,
            origin: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
            part: None,
        }
    }

    /// The same plane through the point where `ray` meets it (virtual surface), if in front.
    pub fn extend(&self, ray: &Ray) -> Option<Surface> {
        let t = ray.plane_hit(self.point, self.normal)?;
        Some(Surface {
            point: ray.at(t),
            part: None,
            ..*self
        })
    }

    /// The two grid axes in the surface plane: the frame axes least aligned with the normal.
    pub fn plane_axes(&self) -> (DVec3, DVec3) {
        let axes = [
            self.rotation * DVec3::X,
            self.rotation * DVec3::Y,
            self.rotation * DVec3::Z,
        ];
        let along = (0..3)
            .max_by(|&a, &b| {
                axes[a]
                    .dot(self.normal)
                    .abs()
                    .total_cmp(&axes[b].dot(self.normal).abs())
            })
            .unwrap_or(1);
        (axes[(along + 1) % 3], axes[(along + 2) % 3])
    }
}

fn is_multiple(size: f64, step: f64) -> bool {
    let k = size / step;
    (k - k.round()).abs() < 1e-6
}

/// Spec 2.2 grid rule along one axis: the box `[lo, hi]` (coordinates on the axis, relative to
/// the grid origin) snaps by its centre when its size is a multiple of `step`, else by its min
/// corner. Returns the shift to add.
pub fn grid_shift(lo: f64, hi: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return 0.0;
    }
    if is_multiple(hi - lo, step) {
        let mid = (lo + hi) * 0.5;
        snap(mid, step) - mid
    } else {
        snap(lo, step) - lo
    }
}

/// Where the reference point of a dragged selection goes on `surface` (spec 5.2).
///
/// `corners` are the selection's box corners relative to its reference point (already rotated as
/// it will be placed); `grab` is the grab point relative to the reference point. The selection's
/// face toward the surface touches it, the grab point sits over the surface point, and with
/// `step` the position snaps to the grid in the surface's own plane (spec 2.2 rule on each axis).
pub fn place_on_surface(
    corners: &[DVec3],
    grab: DVec3,
    surface: &Surface,
    step: Option<f64>,
) -> DVec3 {
    let n = surface.normal;
    let lift = corners
        .iter()
        .map(|c| -c.dot(n))
        .fold(f64::NEG_INFINITY, f64::max);
    let lift = if lift.is_finite() { lift } else { 0.0 };
    let grab_t = grab - n * grab.dot(n);
    let mut r = surface.point - grab_t;
    // Put the reference on the surface plane, then lift it so the nearest face touches.
    r += n * ((surface.point - r).dot(n) + lift);
    if let Some(step) = step {
        let (u, v) = surface.plane_axes();
        for w in [u, v] {
            let base = (r - surface.origin).dot(w);
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for c in corners {
                let x = c.dot(w);
                lo = lo.min(x);
                hi = hi.max(x);
            }
            if !lo.is_finite() {
                lo = 0.0;
                hi = 0.0;
            }
            r += w * grid_shift(base + lo, base + hi, step);
        }
    }
    r
}

/// Draws the glyph of `kind` at `at` (8 points, silver with a graphite ring).
pub fn paint_glyph(painter: &egui::Painter, at: Pos2, kind: SnapKind) {
    let t = ui_tokens();
    let fill = t.icon;
    let ring = Stroke::new(1.0, t.icon_hole);
    let r = 4.0;
    match kind {
        SnapKind::Vertex => {
            painter.rect(
                egui::Rect::from_center_size(at, egui::vec2(r * 2.0, r * 2.0)),
                0.0,
                fill,
                ring,
                egui::StrokeKind::Outside,
            );
        }
        SnapKind::EdgeMid => {
            painter.add(Shape::convex_polygon(
                vec![
                    at + egui::vec2(0.0, -r - 1.0),
                    at + egui::vec2(r + 1.0, r),
                    at + egui::vec2(-r - 1.0, r),
                ],
                fill,
                ring,
            ));
        }
        SnapKind::FaceCentre => {
            painter.circle_stroke(at, r + 0.5, Stroke::new(3.0, t.icon_hole));
            painter.circle_stroke(at, r, Stroke::new(2.0, fill));
        }
        SnapKind::OnEdge => {
            let s = r * 0.8;
            painter.add(Shape::convex_polygon(
                vec![
                    at + egui::vec2(0.0, -s),
                    at + egui::vec2(s, 0.0),
                    at + egui::vec2(0.0, s),
                    at + egui::vec2(-s, 0.0),
                ],
                fill,
                ring,
            ));
        }
        SnapKind::Surface => {
            for (a, b) in [
                (egui::vec2(-r - 2.0, 0.0), egui::vec2(r + 2.0, 0.0)),
                (egui::vec2(0.0, -r - 2.0), egui::vec2(0.0, r + 2.0)),
            ] {
                painter.line_segment([at + a, at + b], Stroke::new(3.0, t.icon_hole));
                painter.line_segment([at + a, at + b], Stroke::new(1.5, fill));
            }
        }
        SnapKind::Grid => {
            painter.circle_filled(at, 2.5, fill);
        }
    }
}

/// Brightens a 4 x 4 cell grid patch around `centre` in the plane of `surface`.
pub fn paint_grid_patch(
    painter: &egui::Painter,
    to: &dyn Fn(DVec3) -> Option<Pos2>,
    centre: DVec3,
    surface: &Surface,
    step: f64,
) {
    let (u, v) = surface.plane_axes();
    let stroke = Stroke::new(1.0, ui_tokens().icon.gamma_multiply(0.55));
    for i in -2..=2 {
        let o = f64::from(i) * step;
        for (a, b) in [(u, v), (v, u)] {
            let p0 = centre + a * o - b * (2.0 * step);
            let p1 = centre + a * o + b * (2.0 * step);
            if let (Some(s0), Some(s1)) = (to(p0), to(p1)) {
                painter.line_segment([s0, s1], stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::EditorCamera;
    use metallic_core::{Value, Vec3, World};

    fn cube_corners(half: f64, rotation: DQuat) -> Vec<DVec3> {
        let t = PartTransform {
            position: DVec3::ZERO,
            rotation,
            size: DVec3::splat(half * 2.0),
        };
        t.corners().to_vec()
    }

    #[test]
    fn grid_applies_in_the_plane_of_a_30_degree_slope() {
        let rot = DQuat::from_rotation_x(30f64.to_radians());
        let ramp = PartTransform {
            position: DVec3::new(2.0, 1.0, 0.0),
            rotation: rot,
            size: DVec3::new(20.0, 1.0, 20.0),
        };
        let ray = Ray {
            origin: DVec3::new(3.37, 30.0, 1.61),
            dir: DVec3::NEG_Y,
        };
        let (d, normal) = ramp.ray_hit_normal(&ray).unwrap();
        let surface = Surface {
            point: ray.at(d),
            normal,
            origin: ramp.position,
            rotation: ramp.rotation,
            part: None,
        };
        // A 1-stud cube aligned to the slope (Align to surface on).
        let corners = cube_corners(0.5, rot);
        let r = place_on_surface(&corners, DVec3::ZERO, &surface, Some(1.0));
        let local = rot.inverse() * (r - ramp.position);
        // On the ramp's own grid: whole studs along its X and Z, resting on its top face.
        assert!((local.x - local.x.round()).abs() < 1e-9, "{local:?}");
        assert!((local.z - local.z.round()).abs() < 1e-9, "{local:?}");
        assert!((local.y - 1.0).abs() < 1e-9, "{local:?}");
        // Not on world stud lines.
        assert!((r.y - r.y.round()).abs() > 1e-3);
    }

    #[test]
    fn odd_sizes_snap_by_min_corner_and_even_by_centre() {
        // A 4-wide plate centred on a grid line; a 1.5-wide one snaps its min corner.
        assert!((grid_shift(-2.2, 1.8, 1.0) - 0.2).abs() < 1e-9);
        assert!((grid_shift(0.3, 1.8, 1.0) - -0.3).abs() < 1e-9);
        let corners = cube_corners(0.5, DQuat::IDENTITY);
        let r = place_on_surface(
            &corners,
            DVec3::ZERO,
            &Surface::ground(0.0)
                .extend(&Ray {
                    origin: DVec3::new(3.3, 10.0, -1.6),
                    dir: DVec3::NEG_Y,
                })
                .unwrap(),
            Some(1.0),
        );
        assert!((r - DVec3::new(3.0, 0.5, -2.0)).length() < 1e-9, "{r:?}");
    }

    #[test]
    fn priority_order_beats_distance() {
        let c = |kind, d| SnapCandidate {
            point: DVec3::ZERO,
            kind,
            owner: None,
            screen_dist: d,
        };
        let list = [
            c(SnapKind::OnEdge, 0.5),
            c(SnapKind::FaceCentre, 1.0),
            c(SnapKind::EdgeMid, 2.0),
            c(SnapKind::Vertex, 11.0),
            c(SnapKind::Vertex, 9.0),
        ];
        let b = best(&list).unwrap();
        assert_eq!((b.kind, b.screen_dist), (SnapKind::Vertex, 9.0));
        assert_eq!(best(&list[..3]).map(|b| b.kind), Some(SnapKind::EdgeMid));
        assert!(SnapKind::FaceCentre < SnapKind::OnEdge);
        assert!(SnapKind::Surface < SnapKind::Grid);
    }

    #[test]
    fn soft_snap_only_with_every_chip_off() {
        let mut s = Snapping::default();
        let (k, r) = SnapKinds::for_settings(&s, false);
        assert!(k.is_empty() && r == SNAP_RADIUS_PX);
        s.grid = false;
        s.surface = false;
        let (k, r) = SnapKinds::for_settings(&s, false);
        assert!(k.vertex && k.edge_mid && !k.on_edge && r == SOFT_RADIUS_PX);
        s.soft = false;
        assert!(SnapKinds::for_settings(&s, false).0.is_empty());
        let (k, _) = SnapKinds::for_settings(&s, true);
        assert!(k.vertex && k.edge_mid && k.face_centre);
    }

    fn grid_world(n: usize) -> World {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let side = (n as f64).sqrt().ceil() as usize;
        for i in 0..n {
            let p = w.create("Part", Some(ws)).unwrap();
            let (x, z) = ((i % side) as f64 * 6.0, (i / side) as f64 * 6.0);
            w.set(p, "Position", Value::Vector3(Vec3::new(x, 0.0, z)))
                .unwrap();
            w.set(p, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 4.0)))
                .unwrap();
        }
        w
    }

    #[test]
    fn candidate_search_over_5000_parts_is_within_budget() {
        let world = grid_world(5000);
        let mut picks = PickCache::new();
        picks.sync(&world);
        let mut cam = EditorCamera {
            position: DVec3::new(-40.0, 60.0, -40.0),
            ..EditorCamera::default()
        };
        cam.look_at(DVec3::new(200.0, 0.0, 200.0));
        let view = cam.view((1600.0, 900.0));
        let skip = |_| false;
        let q = SnapQuery {
            view: &view,
            cursor: (800.0, 450.0),
            radius_px: SNAP_RADIUS_PX,
            kinds: SnapKinds::ALL,
            skip: &skip,
        };
        let mut buf = Vec::with_capacity(256);
        let sources: [&dyn SnapSource; 1] = [&picks];
        let found = find(&sources, &q, &mut buf);
        assert!(found.is_some(), "the cursor is over the grid of parts");
        let runs = 50;
        let start = std::time::Instant::now();
        for _ in 0..runs {
            find(&sources, &q, &mut buf);
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        // Section 17: 0.5 ms per frame (release build). Debug builds are about 20x slower.
        let budget = if cfg!(debug_assertions) { 10.0 } else { 0.5 };
        assert!(per < budget, "{per:.3} ms per search");
    }

    #[test]
    fn vertex_candidates_are_exact_corners() {
        let world = grid_world(1);
        let mut picks = PickCache::new();
        picks.sync(&world);
        let cam = EditorCamera::default();
        let view = cam.view((800.0, 600.0));
        let corner = DVec3::new(2.0, 0.5, 2.0);
        let (x, y) = view.project(corner).unwrap();
        let skip = |_| false;
        let q = SnapQuery {
            view: &view,
            cursor: (x + 3.0, y - 2.0),
            radius_px: SNAP_RADIUS_PX,
            kinds: SnapKinds::ALL,
            skip: &skip,
        };
        let mut buf = Vec::new();
        let hit = find(&[&picks as &dyn SnapSource], &q, &mut buf).unwrap();
        assert_eq!(hit.kind, SnapKind::Vertex);
        assert!((hit.point - corner).length() < 1e-12);
    }
}
