//! The Rotate gizmo (#357): three rings, and a trackball inside them for turning freely. A ring turns the targets
//! about its axis through the gizmo's centre (several swing round their middle), shown with marks round the ring and
//! the angle turned, filled in; the trackball turns them as the pointer rolls it. Primary+R and Primary+T turn the
//! selection a quarter about the up axis or tilt it ([`quarter_turn`]).

use egui::{Mesh, Painter, Shape, Stroke};
use metallic_engine::metallic_classes::base::world_object;
use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{CFrame, DataModel};

use super::{AXIS_NAMES, AXIS_RGB, Frame, HIGHLIGHT_RGB, Handle, Start, Tool, axis_colour, colour, label};
use crate::editor::Scene;
use crate::viewport::gizmo;
use crate::viewport::view::{Ray, View};

/// How many straight pieces a ring is drawn with.
const RING_PIECES: usize = 64;

/// What a Rotate drag keeps.
#[derive(Clone, Debug)]
pub(super) struct RotateDrag {
    /// The ring's axis, or `None` for the trackball.
    ring: Option<usize>,
    /// Where on the ring's plane, or the ball, the pointer took hold, from the centre.
    grab: Vec3,
    /// A ring's raw angle last frame and the angle turned in all (radians).
    angles: (f32, f32),
    /// The trackball: where it was last held, and the turn so far.
    ball: (Vec3, Quat),
    /// The angle turned, in degrees, for the measurement.
    degrees: f32,
}

/// The ring under `ray`, else the trackball inside them.
pub(super) fn hit(frame: &Frame, camera: Vec3, ray: Ray) -> Option<Handle> {
    gizmo::hit_ring(camera, ray, frame.origin, &frame.dirs)
        .map(Handle::Ring)
        .or_else(|| gizmo::inside_trackball(ray, frame.origin, frame.len * 0.9).then_some(Handle::Trackball))
}

/// Takes hold of ring `ring` (`None`: the trackball) with `ray`.
pub(super) fn begin(ring: Option<usize>, frame: &Frame, ray: Ray) -> Option<RotateDrag> {
    let grab = match ring {
        Some(axis) => gizmo::ray_plane(ray, frame.origin, frame.dirs[axis], 1e-5)? - frame.origin,
        None => gizmo::trackball_point(ray, frame.origin, frame.len * 0.9),
    };
    Some(RotateDrag {
        ring,
        grab,
        angles: (0.0, 0.0),
        ball: (grab, Quat::IDENTITY),
        degrees: 0.0,
    })
}

/// Turns every start by `turn` about `origin`.
fn turn_all(dm: &mut DataModel, origin: Vec3, starts: &[Start], turn: Quat) {
    for start in starts {
        let position = origin + turn * (start.pivot.position() - origin);
        let target = CFrame::from_parts(position, (turn * start.pivot.rotation()).normalize());
        if let Err(error) = world_object::pivot_to(dm, start.id, target) {
            log::warn!("tools: could not turn: {error}");
        }
    }
}

/// Turns the targets from how they started by how far `ray` has gone round, in steps of `step` degrees (a ring).
pub(super) fn update(dm: &mut DataModel, drag: &mut RotateDrag, f: &Frame, starts: &[Start], ray: Ray, step: f32) {
    let turn = match drag.ring {
        Some(axis) => {
            let Some(at) = gizmo::ray_plane(ray, f.origin, f.dirs[axis], 1e-5) else {
                return;
            };
            let v = at - f.origin;
            if v.length_squared() < 1e-8 {
                return;
            }
            // A direction only gives the angle up to a whole turn: count the turns.
            let raw = gizmo::signed_angle(f.dirs[axis], drag.grab, v);
            drag.angles = (raw, gizmo::unwrap_angle(drag.angles.1, drag.angles.0, raw));
            drag.degrees = gizmo::snap(drag.angles.1.to_degrees(), step);
            Quat::from_axis_angle(f.dirs[axis], drag.degrees.to_radians())
        }
        None => {
            // The small arcs from frame to frame, added up.
            let now = gizmo::trackball_point(ray, f.origin, f.len * 0.9);
            let arc = gizmo::trackball_rotation(drag.ball.0, now);
            drag.ball = (now, (arc * drag.ball.1).normalize());
            drag.degrees = drag.ball.1.angle_between(Quat::IDENTITY).to_degrees();
            drag.ball.1
        }
    };
    turn_all(dm, f.origin, starts, turn);
}

/// Turns the selection's targets a quarter about the Rotate gizmo's up axis (`tilt`: its X axis), about its centre:
/// as one group, so how they sit to each other is kept. One undo step. Returns whether anything turned.
pub fn quarter_turn(scene: &mut Scene<'_>, local: bool, tilt: bool) -> bool {
    let targets = super::targets(scene.dm, scene.selection);
    let Some(frame) = super::frame_for(scene.dm, Tool::Rotate, local, &targets, Vec3::ZERO) else {
        return false;
    };
    let starts: Vec<Start> = targets
        .iter()
        .filter_map(|&id| world_object::pivot(scene.dm, id).ok().map(|pivot| Start { id, pivot }))
        .collect();
    let axis = frame.dirs[if tilt { 0 } else { 1 }];
    let turn = Quat::from_axis_angle(axis, std::f32::consts::FRAC_PI_2);
    let watched = super::watch_list(scene.dm, &targets, &["CFrame"]);
    scene.watching(&watched, |dm| turn_all(dm, frame.origin, &starts, turn));
    true
}

/// The points of the circle of `radius` about `axis` through `origin`.
fn circle(origin: Vec3, axis: Vec3, radius: f32) -> Vec<Vec3> {
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    (0..=RING_PIECES)
        .map(|i| {
            let a = i as f32 / RING_PIECES as f32 * std::f32::consts::TAU;
            origin + (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

/// A polyline in the world, on the screen.
fn polyline(painter: &Painter, view: &View, points: &[Vec3], stroke: Stroke) {
    for pair in points.windows(2) {
        if let Some(line) = view.segment(pair[0], pair[1]) {
            painter.line_segment(line, stroke);
        }
    }
}

/// The rings, and the trackball's faint disc inside them, the handle under the pointer lit.
pub(super) fn paint(painter: &Painter, view: &View, frame: &Frame, hover: Option<Handle>) {
    let radius = frame.len * 0.9;
    if let Some(centre) = view.project(frame.origin) {
        let on = hover == Some(Handle::Trackball);
        let disc = radius * view.points_per_unit(frame.origin);
        painter.circle(
            centre,
            disc,
            egui::Color32::from_white_alpha(if on { 34 } else { 12 }),
            Stroke::new(if on { 1.5 } else { 0.0 }, colour(HIGHLIGHT_RGB, 180)),
        );
    }
    for axis in 0..3 {
        let on = hover == Some(Handle::Ring(axis));
        let points = circle(frame.origin, frame.dirs[axis], radius);
        polyline(painter, view, &points, Stroke::new(2.5, axis_colour(axis, on, 240)));
    }
}

/// A ring's drag: the ring, its marks from where it was taken hold of, the angle turned filled in, and the angle; the
/// trackball's: its outline and the angle.
pub(super) fn paint_measurement(painter: &Painter, view: &View, f: &Frame, drag: &RotateDrag, increment: f32) {
    let radius = f.len * 0.9;
    let Some(axis) = drag.ring else {
        if let Some(at) = view.project(f.origin) {
            label(
                painter,
                at,
                &format!("Trackball {:.1}\u{B0}", drag.degrees),
                HIGHLIGHT_RGB,
            );
        }
        return;
    };
    let dir = f.dirs[axis];
    let zero = drag.grab.normalize_or_zero();
    let rgb = AXIS_RGB[axis];
    polyline(
        painter,
        view,
        &circle(f.origin, dir, radius),
        Stroke::new(2.0, colour(rgb, 240)),
    );
    let on_screen = radius * view.points_per_unit(f.origin);
    for degrees in gizmo::tick_marks(increment, on_screen) {
        let out = Quat::from_axis_angle(dir, degrees.to_radians()) * zero;
        let long = gizmo::tick_length(degrees);
        if let Some(line) = view.segment(f.origin + out * radius, f.origin + out * (radius * (1.0 + long))) {
            painter.line_segment(line, Stroke::new(1.5, colour(rgb, if long > 0.1 { 255 } else { 170 })));
        }
    }
    // The angle turned, filled in: a fan of triangles, right over a whole turn and more.
    let sweep = drag.degrees.clamp(-360.0, 360.0);
    let steps = ((sweep.abs() / 3.0).ceil() as usize).clamp(2, 120);
    let points: Option<Vec<egui::Pos2>> = std::iter::once(f.origin)
        .chain((0..=steps).map(|k| {
            let angle = (sweep * k as f32 / steps as f32).to_radians();
            f.origin + (Quat::from_axis_angle(dir, angle) * zero) * radius
        }))
        .map(|p| view.project(p))
        .collect();
    if let Some(points) = points.filter(|_| sweep.abs() > 0.05) {
        let mut mesh = Mesh::default();
        for p in &points {
            mesh.colored_vertex(*p, colour(rgb, 62));
        }
        for k in 1..=steps as u32 {
            mesh.add_triangle(0, k, k + 1);
        }
        painter.add(Shape::mesh(mesh));
    }
    let now = Quat::from_axis_angle(dir, drag.degrees.to_radians()) * zero;
    if let Some(at) = view.project(f.origin + now * (radius * 1.2)) {
        label(
            painter,
            at,
            &format!("{} {:+.1}\u{B0}", AXIS_NAMES[axis], drag.degrees),
            rgb,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{Bench, look, position};
    use super::super::*;
    use super::*;

    fn turned(dm: &DataModel, id: metallic_engine::metallic_core::Id) -> Quat {
        world_object::pivot(dm, id).unwrap().rotation()
    }

    #[test]
    fn a_ring_turns_several_about_their_middle_past_half_a_turn() {
        let mut bench = Bench::new();
        let a = bench.part(Vec3::new(-2.0, 0.0, 0.0), Vec3::ONE);
        let b = bench.part(Vec3::new(2.0, 0.0, 0.0), Vec3::ONE);
        bench.select(&[a, b]);
        let view = look(Vec3::new(0.0, 30.0, 0.1), Vec3::ZERO);
        let targets = targets(&bench.dm, &bench.selection);
        let frame = frame_for(&bench.dm, Tool::Rotate, false, &targets, view.pos).unwrap();
        let radius = frame.len * 0.9;
        // Seen from above, the Y ring: take hold half way between +X and +Z (where no other ring passes) and go round
        // an eighth at a time to three quarters.
        let around = |degrees: f32| Quat::from_rotation_y(degrees.to_radians()) * Vec3::X * radius;
        let on_ring = view.project(around(-45.0)).unwrap();
        let handle = super::super::hit(Tool::Rotate, &frame, &view, on_ring).unwrap();
        assert_eq!(handle, Handle::Ring(1));
        let mut drag = begin_drag(&bench.dm, handle, frame, view.ray(on_ring).unwrap(), &targets).unwrap();
        let mut scene = bench.scene();
        for turned in [45.0f32, 90.0, 135.0, 180.0, 225.0, 270.0] {
            let at = around(turned - 45.0);
            update_drag(
                &mut scene,
                &mut drag,
                view.ray(view.project(at).unwrap()).unwrap(),
                Keys::default(),
                Snap::default(),
            );
        }
        // Three quarters about Y, the way the right hand turns.
        let expected = Quat::from_rotation_y(270f32.to_radians());
        assert!(position(scene.dm, a).abs_diff_eq(expected * Vec3::new(-2.0, 0.0, 0.0), 1e-2));
        assert!(position(scene.dm, b).abs_diff_eq(expected * Vec3::new(2.0, 0.0, 0.0), 1e-2));
        assert!(turned(scene.dm, a).angle_between(expected) < 1e-2);
        // The middle of the disc is the trackball.
        let middle = view.project(Vec3::ZERO).unwrap();
        assert_eq!(
            super::super::hit(Tool::Rotate, &frame, &view, middle),
            Some(Handle::Trackball)
        );
    }

    #[test]
    fn a_ring_turns_a_model_without_a_primary_part_by_the_angle_shown() {
        let mut bench = Bench::new();
        let a = bench.part(Vec3::new(-2.0, 0.0, 0.0), Vec3::ONE);
        let b = bench.part(Vec3::new(4.0, 0.0, 0.0), Vec3::ONE);
        let dm = &mut bench.dm;
        let model = dm.create(dm.registry().class_id("Model").unwrap()).unwrap();
        dm.set_parent(model, crate::viewport::pick::workspace(dm)).unwrap();
        dm.set_parent(a, Some(model)).unwrap();
        dm.set_parent(b, Some(model)).unwrap();
        bench.select(&[model]);
        let view = look(Vec3::new(1.0, 30.0, 0.1), Vec3::new(1.0, 0.0, 0.0));
        let targets = targets(&bench.dm, &bench.selection);
        let frame = frame_for(&bench.dm, Tool::Rotate, false, &targets, view.pos).unwrap();
        // The model's pivot is the middle of the box round its parts.
        let centre = frame.origin;
        assert!(centre.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-4));
        let radius = frame.len * 0.9;
        let around = |degrees: f32| centre + Quat::from_rotation_y(degrees.to_radians()) * Vec3::X * radius;
        let on_ring = view.project(around(-45.0)).unwrap();
        let handle = super::super::hit(Tool::Rotate, &frame, &view, on_ring).unwrap();
        let mut drag = begin_drag(&bench.dm, handle, frame, view.ray(on_ring).unwrap(), &targets).unwrap();
        let mut scene = bench.scene();
        // A frame at a time, as the pointer goes round: each must not add to the last.
        for step in 1..=18 {
            let at = around(step as f32 * 5.0 - 45.0);
            update_drag(
                &mut scene,
                &mut drag,
                view.ray(view.project(at).unwrap()).unwrap(),
                Keys::default(),
                Snap::default(),
            );
        }
        let expected = Quat::from_rotation_y(90f32.to_radians());
        for (part, at) in [(a, -2.0), (b, 4.0)] {
            let want = centre + expected * (Vec3::new(at, 0.0, 0.0) - centre);
            assert!(
                position(scene.dm, part).abs_diff_eq(want, 1e-2),
                "{} vs {want}",
                position(scene.dm, part)
            );
            assert!(turned(scene.dm, part).angle_between(expected) < 1e-2);
        }
    }

    #[test]
    fn a_quarter_turn_turns_the_selection_about_the_up_axis_or_tilts_it() {
        let mut bench = Bench::new();
        let part = bench.part(Vec3::new(3.0, 0.0, 0.0), Vec3::ONE);
        bench.select(&[part]);
        let mut scene = bench.scene();
        assert!(quarter_turn(&mut scene, false, false));
        assert!(turned(scene.dm, part).angle_between(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)) < 1e-4);
        assert!(
            position(scene.dm, part).abs_diff_eq(Vec3::new(3.0, 0.0, 0.0), 1e-4),
            "about its own pivot"
        );
        assert!(quarter_turn(&mut scene, false, true));
        scene.commit(std::time::Instant::now(), false);
        assert!(
            scene.undo(std::time::Instant::now()),
            "both turns, one step: nothing came between them"
        );
        assert!(turned(scene.dm, part).angle_between(Quat::IDENTITY) < 1e-4);
    }
}
