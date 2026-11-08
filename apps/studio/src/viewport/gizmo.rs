//! The maths behind the transform gizmos (#355), ported from Forge-Engine's `gizmo.rs`: where the handles are, which
//! one is under the pointer and what a drag along them measures. Plain functions of the [`View`], so they are tested
//! without a window; [`super::tools`] draws the handles and runs the drags.
//!
//! Handles keep one size on the screen whatever the distance ([`handle_len`]), and are picked on the screen, where
//! the pointer is: an arrow within a few points of its line, a plane handle from its square and a margin round it.

use egui::Pos2;
use metallic_engine::metallic_core::glam::{Quat, Vec2, Vec3};

use super::pick::Bounds;
use super::view::{Ray, View};

/// The world's axes.
pub const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

/// The three handle directions: the world's axes, or turned by `rotation` (local space).
pub fn axis_dirs(local: bool, rotation: Quat) -> [Vec3; 3] {
    AXES.map(|axis| if local { (rotation * axis).normalize() } else { axis })
}

/// How long the handles are, in world units: the same size on the screen whatever the distance.
pub fn handle_len(camera: Vec3, origin: Vec3) -> f32 {
    ((origin - camera).length() * 0.18).clamp(0.5, 60.0)
}

/// A handle of the Move gizmo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveHandle {
    /// One of the six arrows: its axis (0..3) and direction (+1 or -1).
    Arrow(usize, f32),
    /// A plane handle, named by the axis it does *not* move along (`Plane(1)` moves along X and Z).
    Plane(usize),
}

/// The two axes plane handle `normal` moves along, in order.
pub fn plane_axes(normal: usize) -> (usize, usize) {
    match normal {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// The side of each axis that faces the camera: the plane handles sit there, never behind the object.
pub fn facing_signs(camera: Vec3, origin: Vec3, dirs: &[Vec3; 3]) -> [f32; 3] {
    let to_camera = camera - origin;
    dirs.map(|dir| if dir.dot(to_camera) >= 0.0 { 1.0 } else { -1.0 })
}

/// The empty middle of the Move gizmo, as a share of the handle's length: the arrows start this far out.
pub const ARROW_GAP: f32 = 0.22;
/// The cone at an arrow's end: its length and its base's radius, as shares of the handle's length.
pub const CONE_LEN: f32 = 0.2;
/// See [`CONE_LEN`].
pub const CONE_RADIUS: f32 = 0.07;
/// Where a plane handle's square starts and ends along each of its axes, as shares of the handle's length.
pub const PLANE_NEAR: f32 = 0.30;
/// See [`PLANE_NEAR`].
pub const PLANE_FAR: f32 = 0.46;
/// A plane seen more edge-on than this (the cosine between the view and its normal) cannot be picked or dragged:
/// where the pointer's ray meets it is far off and jumps about.
pub const MIN_PLANE_FACING: f32 = 0.08;
/// Right on an arrow's line (this close, in points) the arrow wins over a plane's square.
const ARROW_CORE: f32 = 4.0;

/// The four corners of plane handle `normal`, in order round the square.
pub fn plane_corners(origin: Vec3, dirs: &[Vec3; 3], signs: &[f32; 3], normal: usize, len: f32) -> [Vec3; 4] {
    let (a, b) = plane_axes(normal);
    let (da, db) = (dirs[a] * signs[a], dirs[b] * signs[b]);
    let at = |u: f32, v: f32| origin + da * (u * len) + db * (v * len);
    [
        at(PLANE_NEAR, PLANE_NEAR),
        at(PLANE_FAR, PLANE_NEAR),
        at(PLANE_FAR, PLANE_FAR),
        at(PLANE_NEAR, PLANE_FAR),
    ]
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// The distance from `p` to the segment from `a` to `b`.
pub fn point_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 < 1e-12 {
        0.0
    } else {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    };
    p.distance(a + ab * t)
}

/// The distance from `p` to a convex polygon: 0 inside it.
pub fn point_polygon_distance(p: Vec2, poly: &[Vec2]) -> f32 {
    let n = poly.len();
    let sides: Vec<f32> = (0..n)
        .map(|i| (poly[(i + 1) % n] - poly[i]).perp_dot(p - poly[i]))
        .collect();
    if n >= 3 && (sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)) {
        return 0.0;
    }
    (0..n)
        .map(|i| point_segment_distance(p, poly[i], poly[(i + 1) % n]))
        .fold(f32::MAX, f32::min)
}

/// The Move handle under `pointer`, for a gizmo at `origin` along `dirs`.
pub fn hit_move(view: &View, pointer: Pos2, origin: Vec3, dirs: &[Vec3; 3]) -> Option<MoveHandle> {
    let len = handle_len(view.pos, origin);
    let on_screen = len * view.points_per_unit(origin);
    let pointer = v2(pointer);
    // The nearest arrow, within a few points of its line.
    let tolerance = (on_screen * 0.11).clamp(6.0, 12.0);
    let mut arrow: Option<(MoveHandle, f32)> = None;
    for (axis, dir) in dirs.iter().enumerate() {
        for sign in [1.0f32, -1.0] {
            let from = origin + *dir * (sign * len * ARROW_GAP);
            let to = origin + *dir * (sign * len);
            let (Some(a), Some(b)) = (view.project(from), view.project(to)) else {
                continue;
            };
            let distance = point_segment_distance(pointer, v2(a), v2(b));
            if distance <= tolerance && arrow.is_none_or(|(_, best)| distance < best) {
                arrow = Some((MoveHandle::Arrow(axis, sign), distance));
            }
        }
    }
    // The nearest plane handle, from its square and a margin round it.
    let pad = (on_screen * 0.16).clamp(6.0, 18.0);
    let to_origin = (origin - view.pos).normalize_or_zero();
    let signs = facing_signs(view.pos, origin, dirs);
    let mut plane: Option<(usize, f32)> = None;
    for normal in 0..3 {
        if to_origin.dot(dirs[normal]).abs() < MIN_PLANE_FACING {
            continue;
        }
        let corners = plane_corners(origin, dirs, &signs, normal, len);
        let Some(screen) = corners
            .iter()
            .map(|corner| view.project(*corner).map(v2))
            .collect::<Option<Vec<Vec2>>>()
        else {
            continue;
        };
        let distance = point_polygon_distance(pointer, &screen);
        if distance <= pad && plane.is_none_or(|(_, best)| distance < best) {
            plane = Some((normal, distance));
        }
    }
    match (arrow, plane) {
        // Inside a square and off an arrow's very line: the plane.
        (Some((_, on_arrow)), Some((normal, 0.0))) if on_arrow > ARROW_CORE => Some(MoveHandle::Plane(normal)),
        (Some((handle, _)), _) => Some(handle),
        (None, Some((normal, _))) => Some(MoveHandle::Plane(normal)),
        (None, None) => None,
    }
}

/// How big a Scale handle's sphere is at `point`: the same size on the screen whatever the distance.
pub fn sphere_radius(camera: Vec3, point: Vec3) -> f32 {
    ((point - camera).length() * 0.017).max(0.05)
}

/// Where the Scale handle of the face on the `sign` side of `axis` is: a little out from the face, so the handles do
/// not sit on the object.
pub fn scale_handle(bounds: &Bounds, camera: Vec3, axis: usize, sign: f32) -> Vec3 {
    let face = bounds.face(axis, sign);
    face + bounds.outward(axis, sign) * (sphere_radius(camera, face) * 2.6)
}

/// Where `ray` meets the sphere at `center`, if it does (0 from inside).
fn ray_sphere((origin, dir): Ray, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let disc = b * b - (oc.length_squared() - radius * radius);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0)
        .then_some(t)
        .or_else(|| (-b + disc.sqrt() > 0.0).then_some(0.0))
}

/// The Scale handle (axis, side) `ray` meets first, picked a little bigger than it is drawn.
pub fn hit_scale(camera: Vec3, ray: Ray, bounds: &Bounds) -> Option<(usize, f32)> {
    let mut best: Option<((usize, f32), f32)> = None;
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            let at = scale_handle(bounds, camera, axis, sign);
            if let Some(t) = ray_sphere(ray, at, sphere_radius(camera, at) * 1.6)
                && best.is_none_or(|(_, nearest)| t < nearest)
            {
                best = Some(((axis, sign), t));
            }
        }
    }
    best.map(|(handle, _)| handle)
}

/// The closest points of a ray `(o, d)` and an infinite line through `q` along `e`: their parameters along each.
pub fn ray_line((o, d): Ray, q: Vec3, e: Vec3) -> Option<(f32, f32)> {
    let w0 = o - q;
    let (a, b, c) = (d.dot(d), d.dot(e), e.dot(e));
    let (dd, ee) = (d.dot(w0), e.dot(w0));
    let denom = a * c - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some(((b * ee - c * dd) / denom, (a * ee - b * dd) / denom))
}

/// Where `ray` meets the plane through `origin` with `normal`, in front of it, unless it runs within `min_facing`
/// (a cosine) of along the plane.
pub fn ray_plane(ray: Ray, origin: Vec3, normal: Vec3, min_facing: f32) -> Option<Vec3> {
    let denom = ray.1.dot(normal);
    if denom.abs() < min_facing.max(1e-5) {
        return None;
    }
    let t = (origin - ray.0).dot(normal) / denom;
    (t > 0.0).then(|| ray.0 + ray.1 * t)
}

/// `value` rounded to a multiple of `step`; as it is when `step` is not positive.
pub fn snap(value: f32, step: f32) -> f32 {
    if step > 0.0 {
        (value / step).round() * step
    } else {
        value
    }
}

/// The base circle of a cone (`count` points) and its tip, last: what a cone's outline is made from.
pub fn cone_points(base: Vec3, tip: Vec3, radius: f32, count: usize) -> Vec<Vec3> {
    let axis = (tip - base).normalize_or_zero();
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let mut points: Vec<Vec3> = (0..count)
        .map(|i| {
            let a = i as f32 / count as f32 * std::f32::consts::TAU;
            base + (u * a.cos() + v * a.sin()) * radius
        })
        .collect();
    points.push(tip);
    points
}

/// The convex hull of `points`, counter-clockwise: the outline of a cone on the screen.
pub fn convex_hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Vec<Vec2> = if pass == 0 {
            pts.clone()
        } else {
            pts.iter().rev().copied().collect()
        };
        for p in ordered {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

#[cfg(test)]
mod tests {
    use egui::Rect;

    use super::*;
    use crate::viewport::EditorCamera;

    /// A view from `camera` looking at `target`.
    fn look(camera: Vec3, target: Vec3) -> View {
        let camera = EditorCamera::looking_at(camera, target);
        View::new(&camera, Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0)))
    }

    fn move_hit(camera: Vec3, origin: Vec3, world: Vec3) -> Option<MoveHandle> {
        let view = look(camera, origin);
        hit_move(&view, view.project(world)?, origin, &AXES)
    }

    #[test]
    fn local_axes_follow_the_rotation() {
        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert_eq!(axis_dirs(false, turned), AXES);
        assert!(axis_dirs(true, turned)[0].abs_diff_eq(Vec3::NEG_Z, 1e-5));
    }

    #[test]
    fn the_move_gizmo_has_six_arrows_and_three_planes_facing_the_camera() {
        let camera = Vec3::new(10.0, 12.0, 30.0);
        let len = handle_len(camera, Vec3::ZERO);
        for (axis, dir) in AXES.iter().enumerate() {
            for sign in [1.0f32, -1.0] {
                let on = *dir * (sign * len * 0.6);
                assert_eq!(
                    move_hit(camera, Vec3::ZERO, on),
                    Some(MoveHandle::Arrow(axis, sign)),
                    "{axis} {sign}"
                );
            }
        }
        let signs = facing_signs(camera, Vec3::ZERO, &AXES);
        assert_eq!(signs, [1.0, 1.0, 1.0]);
        for normal in 0..3 {
            let c = plane_corners(Vec3::ZERO, &AXES, &signs, normal, len);
            assert_eq!(
                move_hit(camera, Vec3::ZERO, (c[0] + c[2]) * 0.5),
                Some(MoveHandle::Plane(normal))
            );
        }
        let diagonal = Vec3::splat(25.0);
        assert_eq!(move_hit(diagonal, Vec3::ZERO, Vec3::ZERO), None, "the middle is empty");
        assert_eq!(plane_axes(1), (0, 2));
    }

    #[test]
    fn scale_spheres_stand_off_the_faces_of_the_turned_box() {
        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let bounds = Bounds {
            center: Vec3::new(10.0, 2.0, 0.0),
            half: Vec3::new(4.0, 1.0, 2.0),
            rot: turned,
        };
        assert!(
            bounds.face(0, 1.0).abs_diff_eq(Vec3::new(10.0, 2.0, -4.0), 1e-4),
            "+X of a quarter turn faces -Z"
        );
        let camera = Vec3::new(10.0, 40.0, 0.0);
        let top = scale_handle(&bounds, camera, 1, 1.0);
        assert!(top.y > bounds.face(1, 1.0).y, "outside the face");
        assert_eq!(
            hit_scale(camera, (camera, (top - camera).normalize()), &bounds),
            Some((1, 1.0))
        );
        assert_eq!(
            hit_scale(camera, (camera, Vec3::new(1.0, -1.0, 0.0).normalize()), &bounds),
            None
        );
    }

    #[test]
    fn rays_meet_lines_and_planes_in_front_of_them() {
        let (s, t) = ray_line((Vec3::new(2.0, 5.0, 0.0), Vec3::NEG_Y), Vec3::ZERO, Vec3::X).unwrap();
        assert!((s - 5.0).abs() < 1e-5 && (t - 2.0).abs() < 1e-5);
        let ray = (Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -1.0, 0.0).normalize());
        assert!(
            ray_plane(ray, Vec3::ZERO, Vec3::Y, 0.05)
                .unwrap()
                .abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-4)
        );
        assert!(
            ray_plane((Vec3::Y, Vec3::Y), Vec3::ZERO, Vec3::Y, 0.05).is_none(),
            "behind"
        );
        let grazing = (Vec3::new(0.0, 10.0, 0.0), Vec3::new(1.0, -0.01, 0.0).normalize());
        assert!(
            ray_plane(grazing, Vec3::ZERO, Vec3::Y, 0.05).is_none(),
            "along the plane"
        );
    }

    #[test]
    fn helpers_snap_measure_and_outline() {
        assert_eq!(snap(1.6, 1.0), 2.0);
        assert_eq!(snap(0.37, 0.0), 0.37);
        let square = [
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        assert_eq!(point_polygon_distance(Vec2::new(5.0, 5.0), &square), 0.0);
        assert!((point_polygon_distance(Vec2::new(14.0, 5.0), &square) - 4.0).abs() < 1e-5);
        let mut inside = square.to_vec();
        inside.push(Vec2::new(5.0, 5.0));
        assert_eq!(convex_hull(&inside).len(), 4);
        assert_eq!(cone_points(Vec3::ZERO, Vec3::Z, 1.0, 8).len(), 9);
    }
}
