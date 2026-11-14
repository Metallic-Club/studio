//! Carrying parts (#358), as Forge's Select tool and Roblox Studio do: a drag that starts on a part (not on a
//! handle) picks the selection up. Over another part the box round what is carried is set down on its surface,
//! centred on the pointer, touching it, neither sunk in nor hovering (along a surface lined up with an axis, its place
//! snaps to the move increment, so things land on one grid). Over nothing it slides on a plane through where it was
//! grabbed: level, unless the view is nearly level too, when it faces the camera.

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, DataModel, Id};

use super::{Start, place, span};
use crate::viewport::gizmo;
use crate::viewport::pick::{self, Bounds};
use crate::viewport::view::Ray;

/// What carrying keeps.
#[derive(Clone, Debug)]
pub(super) struct CarryDrag {
    /// Where the part was grabbed.
    grab: Vec3,
    /// The plane it slides on over nothing.
    plane: Vec3,
    /// The boxes it can be set down on: every other part's when the drag began.
    surfaces: Vec<Bounds>,
    /// The world-aligned box round what is carried, as it began.
    carried: Bounds,
}

/// The plane a part picked up along `dir` slides on: level, unless the view is nearly level (a level plane is then
/// seen edge-on and the part would fly off), when it faces the camera.
pub(super) fn slide_normal(dir: Vec3) -> Vec3 {
    if dir.y.abs() > 0.12 {
        Vec3::Y
    } else {
        -dir.normalize_or_zero()
    }
}

/// Picks up `targets` at `grab`, the point of a part `ray` met.
pub(super) fn begin(dm: &DataModel, targets: &[Id], grab: Vec3, ray: Ray) -> Option<CarryDrag> {
    let carried: Vec<Id> = targets
        .iter()
        .flat_map(|&target| std::iter::once(target).chain(dm.descendants(target).into_iter().flatten()))
        .collect();
    let world = pick::workspace(dm)?;
    let surfaces = dm
        .descendants(world)
        .ok()?
        .filter(|id| !carried.contains(id))
        .filter_map(|id| pick::part_box(dm, id))
        .collect();
    Some(CarryDrag {
        grab,
        plane: slide_normal(ray.1),
        surfaces,
        carried: span(dm, targets)?,
    })
}

/// How far the carried box `b` moves to sit on the surface at `point` with outward `normal`, centred on the point;
/// along a surface lined up with an axis its place snaps to `step`.
fn set_down(b: &Bounds, point: Vec3, normal: Vec3, step: f32) -> Vec3 {
    let reach = normal.abs().dot(b.half);
    let mut center = point + normal * reach;
    if let Some(across) = (0..3).find(|&k| normal[k].abs() > 0.999) {
        for k in (0..3).filter(|&k| k != across) {
            center[k] = gizmo::snap(center[k], step);
        }
    }
    center - b.center
}

/// Carries the targets from where they started to where `ray` points, in steps of `step`.
pub(super) fn update(dm: &mut DataModel, drag: &CarryDrag, starts: &[Start], ray: Ray, step: f32) {
    let surface = drag
        .surfaces
        .iter()
        .filter_map(|bounds| bounds.hit_face(ray))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let delta = match surface {
        Some((t, normal)) => set_down(&drag.carried, ray.0 + ray.1 * t, normal, step),
        None => {
            let Some(at) = gizmo::ray_plane(ray, drag.grab, drag.plane, gizmo::MIN_PLANE_FACING * 0.6) else {
                return;
            };
            (at - drag.grab).map(|c| gizmo::snap(c, step))
        }
    };
    for start in starts {
        let target = CFrame::from_parts(start.pivot.position() + delta, start.pivot.rotation());
        place(dm, start.id, target, "carry");
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{Bench, look, position};
    use super::super::*;

    #[test]
    fn a_carried_part_sits_on_the_surface_under_the_pointer_or_slides_over_nothing() {
        let mut bench = Bench::new();
        let floor = bench.part(Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 1.0, 40.0));
        let crate_ = bench.part(Vec3::new(0.0, 1.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let wall = bench.part(Vec3::new(10.0, 4.0, 0.0), Vec3::new(2.0, 8.0, 8.0));
        bench.select(&[crate_]);
        let view = look(Vec3::new(0.0, 20.0, 30.0), Vec3::ZERO);
        // Grab the crate's top, carry it onto the floor at (5, 0, 5): it sits on it, centred there.
        let grab = view.project(Vec3::new(0.0, 2.0, 0.0)).unwrap();
        let mut drag = carry(
            &bench.dm,
            &crate::viewport::index::PartIndex::new(),
            &bench.selection,
            view.ray(grab).unwrap(),
        )
        .expect("a part under the pointer");
        let mut scene = bench.scene();
        let to = |at: Vec3| view.ray(view.project(at).unwrap()).unwrap();
        update_drag(
            &mut scene,
            &mut drag,
            to(Vec3::new(5.0, 0.0, 5.0)),
            Keys::default(),
            Snap::default(),
        );
        assert!(
            position(scene.dm, crate_).abs_diff_eq(Vec3::new(5.0, 1.0, 5.0), 1e-2),
            "{}",
            position(scene.dm, crate_)
        );
        // On the wall's top: on it.
        update_drag(
            &mut scene,
            &mut drag,
            to(Vec3::new(10.0, 8.0, 1.0)),
            Keys::default(),
            Snap::default(),
        );
        assert!((position(scene.dm, crate_).y - 9.0).abs() < 1e-2);
        // Over nothing (past the floor's edge): it slides level, where it was grabbed.
        update_drag(
            &mut scene,
            &mut drag,
            to(Vec3::new(-60.0, 2.0, 0.0)),
            Keys::default(),
            Snap::default(),
        );
        assert!((position(scene.dm, crate_).y - 1.0).abs() < 1e-2);
        assert!(position(scene.dm, crate_).x < -40.0);
        // The others never move; one undo step puts it back.
        let now = std::time::Instant::now();
        scene.commit(now, false);
        assert!(position(scene.dm, floor).abs_diff_eq(Vec3::new(0.0, -0.5, 0.0), 1e-4));
        assert!(position(scene.dm, wall).abs_diff_eq(Vec3::new(10.0, 4.0, 0.0), 1e-4));
        assert!(scene.undo(now));
        assert!(position(scene.dm, crate_).abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-4));
    }
}
