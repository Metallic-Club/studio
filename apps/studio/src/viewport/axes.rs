//! The axis gizmo (#386): in the viewport's top right corner, which way the world's X, Y and Z point from the editor
//! camera, drawn as the Move gizmo's arrows (a line ending in a solid cone, in the axes' colours) with their letters.
//! It turns with the camera and never moves.

use egui::{Align2, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use metallic_engine::metallic_core::glam::{Quat, Vec2, Vec3};

use super::gizmo::{self, AXES};
use super::tools::{AXIS_NAMES, axis_colour};

/// How long an arrow is, in points.
const LENGTH: f32 = 32.0;
/// How far the gizmo's centre is from the corner, in points.
const INSET: f32 = 66.0;
/// Where along an arrow its cone starts, and how wide the cone is, in arrow lengths.
const CONE: (f32, f32) = (0.7, 0.11);

/// Where `point` (in world axes, the gizmo's centre at the origin, its arrows one unit long) is on the screen seen from
/// a camera turned by `rotation`, round `centre`, and how far toward the camera it is.
fn place(rotation: Quat, centre: Pos2, point: Vec3) -> (Pos2, f32) {
    let seen = rotation.inverse() * point;
    (centre + vec2(seen.x, -seen.y) * LENGTH, seen.z)
}

/// The axes in the order they are drawn: the one pointing furthest from the camera first, so the nearer ones cover it.
pub fn back_to_front(rotation: Quat) -> [usize; 3] {
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| {
        let depth = |axis: usize| (rotation.inverse() * AXES[axis]).z;
        depth(a).total_cmp(&depth(b))
    });
    order
}

/// Draws the gizmo in the top right corner of `viewport`, seen from a camera turned by `rotation`.
pub fn paint(painter: &Painter, viewport: Rect, rotation: Quat) {
    let centre = viewport.right_top() + vec2(-INSET, INSET);
    for axis in back_to_front(rotation) {
        let dir = AXES[axis];
        let ink = axis_colour(axis, false, 255);
        let (shaft, _) = place(rotation, centre, dir * CONE.0);
        painter.line_segment([centre, shaft], Stroke::new(2.0, ink));
        let cone: Vec<Vec2> = gizmo::cone_points(dir * CONE.0, dir, CONE.1, 14)
            .into_iter()
            .map(|point| {
                let (at, _) = place(rotation, centre, point);
                Vec2::new(at.x, at.y)
            })
            .collect();
        let hull = gizmo::convex_hull(&cone);
        if hull.len() >= 3 {
            painter.add(Shape::convex_polygon(
                hull.iter().map(|p| pos2(p.x, p.y)).collect(),
                ink,
                Stroke::NONE,
            ));
        }
        let (letter, _) = place(rotation, centre, dir * 1.4);
        painter.text(
            letter,
            Align2::CENTER_CENTER,
            AXIS_NAMES[axis],
            FontId::proportional(12.0),
            ink,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::EditorCamera;

    #[test]
    fn the_arrows_point_the_way_the_axes_do_from_the_camera() {
        // Looking along -Z: X right, Y up, Z toward the camera, drawn last.
        let ahead = EditorCamera::looking_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO).rotation();
        let centre = pos2(100.0, 100.0);
        let (x, _) = place(ahead, centre, Vec3::X);
        let (y, _) = place(ahead, centre, Vec3::Y);
        let (z, toward) = place(ahead, centre, Vec3::Z);
        assert!(x.x > centre.x + 31.0 && (x.y - centre.y).abs() < 1e-3);
        assert!(y.y < centre.y - 31.0, "up is up on the screen");
        assert!(z.distance(centre) < 1e-3 && toward > 0.99);
        assert_eq!(back_to_front(ahead)[2], 2);
        // Facing +X: X points away and is drawn first, and +Z is to the right.
        let side = EditorCamera::looking_at(Vec3::ZERO, Vec3::X).rotation();
        assert_eq!(back_to_front(side)[0], 0);
        let (z, _) = place(side, centre, Vec3::Z);
        assert!(z.x > centre.x + 31.0, "facing +X, +Z is to the right");
    }

    #[test]
    fn it_draws_three_arrows_and_their_letters_in_the_corner() {
        let ctx = egui::Context::default();
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let rotation = EditorCamera::looking_at(Vec3::new(5.0, 8.0, 10.0), Vec3::ZERO).rotation();
        let mut output = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), viewport, rotation));
        output.textures_delta.clear();
        let count = |want: fn(&Shape) -> bool| output.shapes.iter().filter(|s| want(&s.shape)).count();
        assert_eq!(count(|s| matches!(s, Shape::LineSegment { .. })), 3);
        assert_eq!(count(|s| matches!(s, Shape::Path(_))), 3, "a cone each");
        assert!(
            output
                .shapes
                .iter()
                .all(|s| s.shape.visual_bounding_rect().min.x > 700.0),
            "in the top right corner"
        );
    }
}
