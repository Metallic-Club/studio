//! The Move gizmo (#355): six arrows (both ways along each axis, starting a gap out from the centre and ending in a
//! solid cone) and three plane handles, on the side of the axes that faces the camera. A drag moves every target by
//! its pivot (a model with everything in it) along an arrow or across a plane, shown with a ruler and the distance.

use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2};
use metallic_engine::metallic_classes::base::world_object;
use metallic_engine::metallic_core::glam::{Vec2, Vec3};
use metallic_engine::metallic_core::{CFrame, DataModel};

use super::{AXIS_NAMES, AXIS_RGB, Frame, HIGHLIGHT_RGB, Handle, Start, axis_colour, colour, label, ruler};
use crate::viewport::gizmo::{self, MoveHandle};
use crate::viewport::view::{Ray, View};

/// What a Move drag keeps.
#[derive(Clone, Debug)]
pub(super) struct MoveDrag {
    handle: MoveHandle,
    /// Where along the arrow's axis the pointer took hold.
    t0: f32,
    /// Where on the plane it took hold, from the origin.
    grab: Vec3,
    /// How far the targets have moved along each axis.
    moved: [f32; 3],
}

/// Takes hold of `handle` with `ray`.
pub(super) fn begin(handle: MoveHandle, frame: &Frame, ray: Ray) -> Option<MoveDrag> {
    let mut drag = MoveDrag {
        handle,
        t0: 0.0,
        grab: Vec3::ZERO,
        moved: [0.0; 3],
    };
    match handle {
        MoveHandle::Arrow(axis, _) => drag.t0 = gizmo::ray_line(ray, frame.origin, frame.dirs[axis])?.1,
        MoveHandle::Plane(normal) => {
            let at = gizmo::ray_plane(ray, frame.origin, frame.dirs[normal], gizmo::MIN_PLANE_FACING)?;
            drag.grab = at - frame.origin;
        }
    }
    Some(drag)
}

/// Moves the targets from where they `starts`ed by where `ray` is along the handle, in steps of `step`.
pub(super) fn update(dm: &mut DataModel, drag: &mut MoveDrag, f: &Frame, starts: &[Start], ray: Ray, step: f32) {
    let delta = match drag.handle {
        MoveHandle::Arrow(axis, _) => {
            let Some((_, t)) = gizmo::ray_line(ray, f.origin, f.dirs[axis]) else {
                return;
            };
            let amount = gizmo::snap(t - drag.t0, step);
            drag.moved = [0.0; 3];
            drag.moved[axis] = amount;
            f.dirs[axis] * amount
        }
        MoveHandle::Plane(normal) => {
            // Seen edge-on the plane cannot be followed: the targets stay until the view allows it.
            let Some(at) = gizmo::ray_plane(ray, f.origin, f.dirs[normal], gizmo::MIN_PLANE_FACING * 0.6) else {
                return;
            };
            let rel = at - f.origin - drag.grab;
            let (a, b) = gizmo::plane_axes(normal);
            drag.moved = [0.0; 3];
            drag.moved[a] = gizmo::snap(rel.dot(f.dirs[a]), step);
            drag.moved[b] = gizmo::snap(rel.dot(f.dirs[b]), step);
            f.dirs[a] * drag.moved[a] + f.dirs[b] * drag.moved[b]
        }
    };
    for start in starts {
        let target = CFrame::from_parts(start.pivot.position() + delta, start.pivot.rotation());
        if let Err(error) = world_object::pivot_to(dm, start.id, target) {
            log::warn!("tools: could not move: {error}");
        }
    }
}

/// The handles: plane squares, then arrows (a shaft from the gap to a solid cone), the one under the pointer lit.
pub(super) fn paint(painter: &Painter, view: &View, frame: &Frame, hover: Option<Handle>) {
    let lit = |handle: MoveHandle| hover == Some(Handle::Move(handle));
    let signs = gizmo::facing_signs(view.pos, frame.origin, &frame.dirs);
    for axis in 0..3 {
        let corners = gizmo::plane_corners(frame.origin, &frame.dirs, &signs, axis, frame.len);
        if let Some(points) = corners.iter().map(|c| view.project(*c)).collect::<Option<Vec<Pos2>>>() {
            let on = lit(MoveHandle::Plane(axis));
            painter.add(Shape::convex_polygon(
                points,
                axis_colour(axis, on, if on { 190 } else { 110 }),
                Stroke::new(1.0, axis_colour(axis, on, 220)),
            ));
        }
    }
    for (axis, &facing) in signs.iter().enumerate() {
        for sign in [1.0f32, -1.0] {
            let on = lit(MoveHandle::Arrow(axis, sign));
            // The half pointing away from the camera is a little fainter.
            let alpha = if sign == facing { 255 } else { 170 };
            let dir = frame.dirs[axis] * sign;
            let start = frame.origin + dir * (frame.len * gizmo::ARROW_GAP);
            let base = frame.origin + dir * (frame.len * (1.0 - gizmo::CONE_LEN));
            let tip = frame.origin + dir * frame.len;
            if let Some(line) = view.segment(start, base) {
                painter.line_segment(line, Stroke::new(2.5, axis_colour(axis, on, alpha)));
            }
            cone(
                painter,
                view,
                base,
                tip,
                frame.len * gizmo::CONE_RADIUS,
                axis_colour(axis, on, alpha),
            );
        }
    }
}

/// A solid cone, flat: the outline of its base circle and tip on the screen.
fn cone(painter: &Painter, view: &View, base: Vec3, tip: Vec3, radius: f32, fill: Color32) {
    let points = gizmo::cone_points(base, tip, radius, 14);
    let Some(screen) = points
        .iter()
        .map(|p| view.project(*p).map(|s| Vec2::new(s.x, s.y)))
        .collect::<Option<Vec<Vec2>>>()
    else {
        return;
    };
    let hull = gizmo::convex_hull(&screen);
    if hull.len() >= 3 {
        painter.add(Shape::convex_polygon(
            hull.iter().map(|p| pos2(p.x, p.y)).collect(),
            fill,
            Stroke::NONE,
        ));
    }
}

/// A ruler along each axis the drag moves on, with the distance where the targets are now.
pub(super) fn paint_measurement(painter: &Painter, view: &View, f: &Frame, drag: &MoveDrag) {
    let axes = match drag.handle {
        MoveHandle::Arrow(axis, _) => vec![axis],
        MoveHandle::Plane(normal) => {
            let (a, b) = gizmo::plane_axes(normal);
            vec![a, b]
        }
    };
    for axis in axes {
        let moved = drag.moved[axis];
        ruler(painter, view, f.origin, f.dirs[axis], AXIS_RGB[axis], f.len, moved);
        if let Some(at) = view.project(f.origin + f.dirs[axis] * moved) {
            label(
                painter,
                at,
                &format!("{} {moved:+.2}", AXIS_NAMES[axis]),
                AXIS_RGB[axis],
            );
        }
    }
    if let Some(at) = view.project(f.origin) {
        painter.circle_filled(at, 3.5, colour(HIGHLIGHT_RGB, 255));
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{Bench, look, position};
    use super::super::*;

    #[test]
    fn a_move_drag_along_an_arrow_moves_every_target_from_where_it_started_as_one_step() {
        let mut bench = Bench::new();
        let a = bench.part(Vec3::ZERO, Vec3::ONE);
        let b = bench.part(Vec3::new(4.0, 0.0, 0.0), Vec3::ONE);
        bench.select(&[a, b]);
        let view = look(Vec3::new(2.0, 10.0, 30.0), Vec3::new(2.0, 0.0, 0.0));
        let targets = targets(&bench.dm, &bench.selection);
        let frame = frame_for(&bench.dm, Tool::Move, false, &targets, view.pos).unwrap();
        assert!(
            frame.origin.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-4),
            "the middle of both"
        );
        assert_eq!(frame.dirs, gizmo::AXES);
        assert_eq!(frame_for(&bench.dm, Tool::Select, false, &targets, view.pos), None);
        // Take hold of the +X arrow and pull it along X.
        let on_arrow = view.project(frame.origin + Vec3::X * frame.len * 0.6).unwrap();
        let handle = hit(Tool::Move, &frame, &view, on_arrow).unwrap();
        assert_eq!(handle, Handle::Move(MoveHandle::Arrow(0, 1.0)));
        let mut drag = begin_drag(&bench.dm, handle, frame, view.ray(on_arrow).unwrap(), &targets).unwrap();
        let pulled = view.project(frame.origin + Vec3::X * (frame.len * 0.6 + 3.4)).unwrap();
        let mut scene = bench.scene();
        for _ in 0..3 {
            // The same pointer, frame after frame: the same place, nothing added up.
            update_drag(
                &mut scene,
                &mut drag,
                view.ray(pulled).unwrap(),
                Keys::default(),
                Snap::default(),
            );
        }
        assert!(position(scene.dm, a).abs_diff_eq(Vec3::new(3.4, 0.0, 0.0), 1e-3));
        // With a step, it lands on whole steps.
        let snap = Snap {
            move_step: 2.0,
            turn_step: 0.0,
        };
        update_drag(&mut scene, &mut drag, view.ray(pulled).unwrap(), Keys::default(), snap);
        let now = std::time::Instant::now();
        scene.commit(now, false);
        assert!(position(scene.dm, a).abs_diff_eq(Vec3::new(4.0, 0.0, 0.0), 1e-3));
        assert!(position(scene.dm, b).abs_diff_eq(Vec3::new(8.0, 0.0, 0.0), 1e-3));
        // One undo step takes both back.
        assert!(scene.undo(now));
        assert!(position(scene.dm, a).abs_diff_eq(Vec3::ZERO, 1e-4));
        assert!(position(scene.dm, b).abs_diff_eq(Vec3::new(4.0, 0.0, 0.0), 1e-4));
    }
}
