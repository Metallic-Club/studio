//! The Scale gizmo (#356): six spheres standing off the faces of a box: one part's own, or the world-aligned box round
//! several (shown faintly). Dragging a sphere moves that face and the face opposite stays; with Ctrl both faces move,
//! about the centre; with Shift all three sizes change by one factor. Every part inside is resized and moved with it:
//! a part turned between the box's axes cannot be stretched without shearing, so for a scale along one axis only its
//! place follows. Shown with a ruler along the axis and the new size (or the factor, with Shift).

use egui::{Painter, Stroke};
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, DataModel, Id, Value};

use super::{AXIS_NAMES, AXIS_RGB, Frame, Handle, Keys, axis_colour, label, ruler};
use crate::viewport::gizmo::{self, AXES};
use crate::viewport::pick::{self, EDGES};
use crate::viewport::view::{Ray, View};

/// The smallest a box is made along the axis being dragged.
const MIN_EXTENT: f32 = 0.05;

/// A part a Scale drag resizes, as it was when the drag began.
#[derive(Clone, Copy, Debug)]
struct PartStart {
    id: Id,
    frame: CFrame,
    size: Vec3,
}

/// What a Scale drag keeps.
#[derive(Clone, Debug)]
pub(super) struct ScaleDrag {
    axis: usize,
    sign: f32,
    /// Where along the axis, through the face, the pointer took hold.
    t0: f32,
    parts: Vec<PartStart>,
    /// The box's size along the axis now.
    extent: f32,
    /// What that is of the size it was.
    factor: f32,
    /// How far the grabbed face has moved out (negative: in).
    pull: f32,
    /// Whether Shift scaled all three sizes.
    proportional: bool,
}

/// Takes hold of the sphere on the `sign` side of `axis` with `ray`: every part in and under `targets` is resized.
pub(super) fn begin(
    dm: &DataModel,
    axis: usize,
    sign: f32,
    frame: &Frame,
    ray: Ray,
    targets: &[Id],
) -> Option<ScaleDrag> {
    let bounds = frame.bounds?;
    let t0 = gizmo::ray_line(ray, bounds.face(axis, sign), bounds.outward(axis, sign))?.1;
    let parts = targets
        .iter()
        .flat_map(|&target| std::iter::once(target).chain(dm.descendants(target).into_iter().flatten()))
        .filter_map(|id| {
            let own = pick::part_box(dm, id)?;
            Some(PartStart {
                id,
                frame: CFrame::from_parts(own.center, own.rot),
                size: own.half * 2.0,
            })
        })
        .collect();
    Some(ScaleDrag {
        axis,
        sign,
        t0,
        parts,
        extent: bounds.half[axis] * 2.0,
        factor: 1.0,
        pull: 0.0,
        proportional: false,
    })
}

/// Resizes the parts from how they started by where `ray` is along the axis, with `keys`, in steps of `step`.
pub(super) fn update(dm: &mut DataModel, drag: &mut ScaleDrag, f: &Frame, ray: Ray, keys: Keys, step: f32) {
    let Some(bounds) = f.bounds else {
        return;
    };
    let (axis, sign) = (drag.axis, drag.sign);
    let outward = bounds.outward(axis, sign);
    let Some((_, t)) = gizmo::ray_line(ray, bounds.face(axis, sign), outward) else {
        return;
    };
    let extent0 = bounds.half[axis] * 2.0;
    // About the centre a face moves by half the growth.
    let reach = if keys.ctrl { 2.0 } else { 1.0 };
    let extent = (extent0 + reach * gizmo::snap(t - drag.t0, step)).max(MIN_EXTENT);
    let factor = extent / extent0.max(1e-4);
    drag.extent = extent;
    drag.factor = factor;
    drag.pull = (extent - extent0) / reach;
    drag.proportional = keys.shift;
    let scale = if keys.shift {
        Vec3::splat(factor)
    } else {
        let mut scale = Vec3::ONE;
        scale[axis] = factor;
        scale
    };
    let anchor = if keys.ctrl {
        bounds.center
    } else {
        bounds.face(axis, -sign)
    };
    let axes = AXES.map(|a| bounds.rot * a);
    for part in &drag.parts {
        let rel = part.frame.position() - anchor;
        let at = anchor + (0..3).map(|k| axes[k] * (rel.dot(axes[k]) * scale[k])).sum::<Vec3>();
        let mut size = part.size;
        for (j, side) in size.as_mut().iter_mut().enumerate() {
            let own = part.frame.rotation() * AXES[j];
            let along = if keys.shift {
                Some(factor)
            } else {
                (0..3).find(|&k| own.dot(axes[k]).abs() > 0.98).map(|k| scale[k])
            };
            if let Some(along) = along {
                *side = part.size[j] * along;
            }
        }
        let moved = CFrame::from_parts(at, part.frame.rotation());
        for (name, value) in [("Size", Value::Vector3(size)), ("CFrame", Value::CFrame(moved))] {
            if let Err(error) = dm.set_by_name(part.id, name, value) {
                log::warn!("tools: could not scale: {error}");
            }
        }
    }
}

/// The spheres, the one under the pointer lit, and for several instances (`around`) the box they share, faintly.
pub(super) fn paint(painter: &Painter, view: &View, frame: &Frame, hover: Option<Handle>, around: bool) {
    let Some(bounds) = frame.bounds else {
        return;
    };
    if around {
        let corners = bounds.corners();
        for (a, b) in EDGES {
            if let Some(line) = view.segment(corners[a], corners[b]) {
                painter.line_segment(line, Stroke::new(1.0, egui::Color32::from_white_alpha(60)));
            }
        }
    }
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            let at = gizmo::scale_handle(&bounds, view.pos, axis, sign);
            let Some(centre) = view.project(at) else {
                continue;
            };
            let radius = (gizmo::sphere_radius(view.pos, at) * view.points_per_unit(at)).max(3.0);
            let on = hover == Some(Handle::Scale(axis, sign));
            painter.circle(
                centre,
                radius,
                axis_colour(axis, on, if on { 255 } else { 230 }),
                Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200)),
            );
        }
    }
}

/// A ruler along the axis from where the grabbed face was, with the new size (or, with Shift, the factor).
pub(super) fn paint_measurement(painter: &Painter, view: &View, f: &Frame, drag: &ScaleDrag) {
    let Some(bounds) = f.bounds else {
        return;
    };
    let (axis, sign) = (drag.axis, drag.sign);
    let outward = bounds.outward(axis, sign);
    let face = bounds.face(axis, sign);
    ruler(painter, view, face, outward, AXIS_RGB[axis], f.len, drag.pull);
    let text = if drag.proportional {
        format!("\u{D7}{:.2}", drag.factor)
    } else {
        format!("{} {:.2}", AXIS_NAMES[axis], drag.extent)
    };
    if let Some(at) = view.project(face + outward * drag.pull) {
        label(painter, at, &text, AXIS_RGB[axis]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{Bench, look, position};
    use super::super::*;
    use super::*;
    use metallic_engine::metallic_core::glam::Quat;

    fn size(dm: &DataModel, id: Id) -> Vec3 {
        match dm.get_by_name(id, "Size").unwrap() {
            Value::Vector3(size) => size,
            other => panic!("{other:?}"),
        }
    }

    /// Drags the sphere on the `sign` side of `axis` out by `by` studs, with `keys`, and ends the step.
    fn pull(bench: &mut Bench, axis: usize, sign: f32, by: f32, keys: Keys) {
        let view = look(Vec3::new(20.0, 25.0, 30.0), Vec3::ZERO);
        let targets = targets(&bench.dm, &bench.selection);
        let frame = frame_for(&bench.dm, Tool::Scale, false, &targets, view.pos).unwrap();
        let bounds = frame.bounds.unwrap();
        let at = gizmo::scale_handle(&bounds, view.pos, axis, sign);
        let pointer = view.project(at).unwrap();
        assert_eq!(
            hit(Tool::Scale, &frame, &view, pointer),
            Some(Handle::Scale(axis, sign))
        );
        let mut drag = begin_drag(
            &bench.dm,
            Handle::Scale(axis, sign),
            frame,
            view.ray(pointer).unwrap(),
            &targets,
        )
        .unwrap();
        let to = view.project(at + bounds.outward(axis, sign) * by).unwrap();
        let mut scene = bench.scene();
        update_drag(&mut scene, &mut drag, view.ray(to).unwrap(), keys, Snap::default());
        scene.commit(std::time::Instant::now(), false);
    }

    #[test]
    fn a_face_follows_the_pointer_and_the_opposite_one_stays() {
        let mut bench = Bench::new();
        let part = bench.part(Vec3::ZERO, Vec3::new(4.0, 2.0, 2.0));
        bench.select(&[part]);
        pull(&mut bench, 0, 1.0, 2.0, Keys::default());
        assert!(size(&bench.dm, part).abs_diff_eq(Vec3::new(6.0, 2.0, 2.0), 1e-2));
        assert!(
            position(&bench.dm, part).abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-2),
            "the -X face stayed"
        );
        // Ctrl: about the centre, both faces out.
        pull(
            &mut bench,
            0,
            1.0,
            1.0,
            Keys {
                shift: false,
                ctrl: true,
            },
        );
        assert!(size(&bench.dm, part).abs_diff_eq(Vec3::new(8.0, 2.0, 2.0), 1e-2));
        assert!(position(&bench.dm, part).abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-2));
        // Shift: all three by one factor.
        pull(
            &mut bench,
            1,
            1.0,
            2.0,
            Keys {
                shift: true,
                ctrl: false,
            },
        );
        assert!(size(&bench.dm, part).abs_diff_eq(Vec3::new(16.0, 4.0, 4.0), 1e-2));
        // Each drag one undo step.
        let mut scene = bench.scene();
        let now = std::time::Instant::now();
        assert!(scene.undo(now) && scene.undo(now) && scene.undo(now));
        assert!(size(scene.dm, part).abs_diff_eq(Vec3::new(4.0, 2.0, 2.0), 1e-4));
        assert!(position(scene.dm, part).abs_diff_eq(Vec3::ZERO, 1e-4));
    }

    #[test]
    fn several_parts_stretch_with_the_box_round_them() {
        let mut bench = Bench::new();
        let a = bench.part(Vec3::ZERO, Vec3::new(2.0, 2.0, 2.0));
        let b = bench.part(Vec3::new(4.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        // A part turned a quarter: its own Z lies along the box's X.
        bench
            .dm
            .set_by_name(
                b,
                "CFrame",
                Value::CFrame(CFrame::from_parts(
                    Vec3::new(4.0, 0.0, 0.0),
                    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                )),
            )
            .unwrap();
        bench.select(&[a, b]);
        // The box spans X from -1 to 5; pull its +X face out by 6: twice as wide, from -1.
        pull(&mut bench, 0, 1.0, 6.0, Keys::default());
        assert!(position(&bench.dm, a).abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-2));
        assert!(position(&bench.dm, b).abs_diff_eq(Vec3::new(9.0, 0.0, 0.0), 1e-2));
        assert!(size(&bench.dm, a).abs_diff_eq(Vec3::new(4.0, 2.0, 2.0), 1e-2));
        assert!(
            size(&bench.dm, b).abs_diff_eq(Vec3::new(2.0, 2.0, 4.0), 1e-2),
            "stretched along its own Z"
        );
    }
}
