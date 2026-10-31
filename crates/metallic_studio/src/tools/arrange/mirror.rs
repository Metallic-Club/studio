//! Mirror in place and mirror copy (building-tools spec 7).
//!
//! A reflection is not a rotation, so a part cannot simply take the reflected matrix. For a part
//! with rotation `R`, reflection `M` and a local-axis flip `F` (one diagonal entry -1), `M R F` is a
//! proper rotation again. Because each supported shape is symmetric across the plane normal to the
//! flipped local axis, the part then fills exactly the mirror image of its old volume, with the
//! same `Size`:
//!
//! - Block, Ball, Cylinder: symmetric across every local axis; the axis most aligned with the
//!   plane normal is flipped, which keeps the angles tidy.
//! - Wedge: its profile lies in local Y-Z, so it is symmetric only across local X; X is flipped.

use glam::{DMat3, DVec3};

use super::math::{Axis, Pose, reflection};
use super::plan::{ArrangeError, Plan, describe};
use super::scene::{Object, PartState, Shape, selection_pivot};

/// Where the mirror plane passes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MirrorPlane {
    /// The selection's pivot: one object's pivot, or the centre of several objects' bounds.
    Pivot,
    /// The active (last selected) object's pivot.
    Active,
    /// A given point.
    Point(DVec3),
}

/// Mirror options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MirrorOptions {
    /// The plane's normal (world axis).
    pub axis: Axis,
    /// Where the plane passes.
    pub plane: MirrorPlane,
    /// `false` mirrors in place (Ctrl+M); `true` makes mirrored copies (Ctrl+Shift+M).
    pub copy: bool,
}

/// The local axis to flip back after reflecting a part with rotation `rot` across a plane with
/// unit normal `normal`.
pub fn flip_axis(shape: Shape, rot: &DMat3, normal: DVec3) -> usize {
    match shape {
        Shape::Wedge => 0,
        Shape::Block | Shape::Ball | Shape::Cylinder => (0..3)
            .max_by(|&a, &b| {
                rot.col(a)
                    .dot(normal)
                    .abs()
                    .total_cmp(&rot.col(b).dot(normal).abs())
            })
            .unwrap_or(0),
    }
}

/// `part` reflected across the plane through `origin` with normal `normal`.
pub fn mirror_part(part: &PartState, origin: DVec3, normal: DVec3) -> PartState {
    let n = normal.normalize();
    let m = reflection(n);
    let pose = part.pose();
    let pos = origin + m * (pose.pos - origin);
    let k = flip_axis(part.shape, &pose.rot, n);
    let mut flip = DVec3::ONE;
    flip[k] = -1.0;
    let rot = m * pose.rot * DMat3::from_diagonal(flip);
    part.with_pose(&Pose { pos, rot })
}

/// Plans a mirror of `objects`. Models mirror all their parts across the same plane.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects; [`ArrangeError::Invalid`] for a point that
/// is not finite or a result that overflows.
pub fn mirror(objects: &[Object], opts: &MirrorOptions) -> Result<Plan, ArrangeError> {
    let Some(active) = objects.last() else {
        return Err(ArrangeError::NeedMore { needed: 1, got: 0 });
    };
    let origin = match opts.plane {
        MirrorPlane::Pivot => selection_pivot(objects),
        MirrorPlane::Active => active.pivot().pos,
        MirrorPlane::Point(p) if p.is_finite() => p,
        MirrorPlane::Point(_) => {
            return Err(ArrangeError::Invalid(
                "the mirror point must be a finite number".into(),
            ));
        }
    };
    let normal = opts.axis.unit();
    let verb = if opts.copy { "Mirror copy" } else { "Mirror" };
    let mut plan = Plan {
        label: format!("{verb} {}", describe(objects)),
        ..Plan::default()
    };
    for o in objects {
        let after: Vec<PartState> = o
            .parts
            .iter()
            .map(|p| mirror_part(p, origin, normal))
            .collect();
        if opts.copy {
            plan.push_copy(o, &after);
        } else {
            plan.push_object(o, &after);
        }
    }
    plan.check_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arrange::math::{max_diff, rotation_from_orientation};
    use crate::tools::arrange::testing::*;
    use crate::tools::geom::box_corners;
    use metallic_core::Value;

    fn the_part(w: &metallic_core::World, id: metallic_core::InstanceId) -> PartState {
        objs(w, &[id])[0].parts[0]
    }

    /// The six corners of a wedge in world space.
    fn wedge_corners(p: &PartState) -> Vec<DVec3> {
        let h = p.snap.size * 0.5;
        let pose = p.pose();
        [
            d(-h.x, -h.y, -h.z),
            d(h.x, -h.y, -h.z),
            d(-h.x, -h.y, h.z),
            d(h.x, -h.y, h.z),
            d(-h.x, h.y, h.z),
            d(h.x, h.y, h.z),
        ]
        .iter()
        .map(|&c| pose.point(c))
        .collect()
    }

    fn same_point_sets(a: &[DVec3], b: &[DVec3]) -> bool {
        a.len() == b.len()
            && a.iter()
                .all(|p| b.iter().any(|q| (*p - *q).length() < 1e-9))
    }

    fn reflect(p: DVec3, origin: DVec3, n: DVec3) -> DVec3 {
        p - n * (2.0 * (p - origin).dot(n))
    }

    #[test]
    fn rotated_wedge_mirrors_to_its_exact_mirror_image() {
        let mut w = world();
        let ws = w.root("Workspace").unwrap();
        let id = part_under(
            &mut w,
            ws,
            v(3.0, 2.0, -1.0),
            v(2.0, 3.0, 5.0),
            v(20.0, 35.0, -50.0),
            "Wedge",
        );
        let before = the_part(&w, id);
        assert_eq!(before.shape, Shape::Wedge);
        for axis in Axis::ALL {
            let origin = d(1.0, -2.0, 0.5);
            let n = axis.unit();
            let after = mirror_part(&before, origin, n);
            assert!((after.pose().rot.determinant() - 1.0).abs() < 1e-12);
            assert_eq!(after.snap.size, before.snap.size);
            let expected: Vec<DVec3> = wedge_corners(&before)
                .into_iter()
                .map(|c| reflect(c, origin, n))
                .collect();
            assert!(
                same_point_sets(&wedge_corners(&after), &expected),
                "wedge mirrored across {axis:?} does not fill the mirror image"
            );
            // The written Orientation reproduces the same rotation.
            let o = crate::tools::arrange::math::orientation_of(&after.pose().rot);
            assert!(max_diff(&rotation_from_orientation(o), &after.pose().rot) < 1e-9);
        }
    }

    #[test]
    fn wedge_flips_local_x_and_slope_faces_the_other_way() {
        // Unrotated wedge, tall edge at +Z, mirrored across Z = 0: tall edge must end at -Z.
        let mut w = world();
        let ws = w.root("Workspace").unwrap();
        let id = part_under(
            &mut w,
            ws,
            v(0.0, 0.0, 4.0),
            v(2.0, 2.0, 2.0),
            v(0.0, 0.0, 0.0),
            "Wedge",
        );
        let m = mirror_part(&the_part(&w, id), DVec3::ZERO, DVec3::Z).pose();
        assert_dvec(m.pos, d(0.0, 0.0, -4.0));
        // Local +Z (towards the tall edge) now points to world -Z.
        assert_dvec(m.rot.z_axis, d(0.0, 0.0, -1.0));
        // Local X was flipped back: the rotation is 180 degrees about Y.
        assert_dvec(m.rot.x_axis, d(-1.0, 0.0, 0.0));
        assert_dvec(m.rot.y_axis, d(0.0, 1.0, 0.0));
    }

    #[test]
    fn rotated_block_mirrors_its_box() {
        let mut w = world();
        let id = part(
            &mut w,
            v(5.0, 1.0, 2.0),
            v(4.0, 1.0, 2.0),
            v(10.0, 30.0, 70.0),
        );
        let p = the_part(&w, id);
        let n = DVec3::X;
        let m = mirror_part(&p, DVec3::ZERO, n);
        let expected: Vec<DVec3> = p
            .snap
            .corners()
            .iter()
            .map(|&c| reflect(c, DVec3::ZERO, n))
            .collect();
        let got = box_corners(m.snap.position, m.snap.rotation, m.snap.size);
        assert!(same_point_sets(&got, &expected));
        assert!((m.pose().rot.determinant() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mirror_in_place_and_copy_across_the_selection_pivot() {
        let mut w = world();
        let a = part(&mut w, v(0.0, 0.0, 0.0), v(2.0, 2.0, 2.0), v(0.0, 0.0, 0.0));
        let b = part(
            &mut w,
            v(10.0, 4.0, 0.0),
            v(2.0, 2.0, 2.0),
            v(0.0, 0.0, 0.0),
        );
        // Selection bounds X -1..11, centre 5.
        let o = MirrorOptions {
            axis: Axis::X,
            plane: MirrorPlane::Pivot,
            copy: false,
        };
        let plan = mirror(&objs(&w, &[a, b]), &o).unwrap();
        assert_vec(pos_after(&plan, a), v(10.0, 0.0, 0.0));
        assert_vec(pos_after(&plan, b), v(0.0, 4.0, 0.0));
        assert_eq!(plan.label, "Mirror 2 parts");
        assert!(plan.copies.is_empty());

        let plan = mirror(&objs(&w, &[a, b]), &MirrorOptions { copy: true, ..o }).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.copies.len(), 2);
        assert_eq!(plan.copies[0].source, a);
        assert_vec(plan.copies[0].parts[0].position, v(10.0, 0.0, 0.0));
        assert_eq!(plan.copies[0].parent, w.root("Workspace"));
        assert_eq!(plan.label, "Mirror copy 2 parts");
    }

    #[test]
    fn mirror_a_model_about_the_active_pivot() {
        let mut w = world();
        let (m, kids) = model(
            &mut w,
            &[
                (v(2.0, 0.0, 0.0), v(1.0, 1.0, 1.0)),
                (v(4.0, 0.0, 3.0), v(1.0, 1.0, 1.0)),
            ],
        );
        let hub = part(&mut w, v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let o = MirrorOptions {
            axis: Axis::Z,
            plane: MirrorPlane::Active,
            copy: false,
        };
        let plan = mirror(&objs(&w, &[m, hub]), &o).unwrap();
        assert!(plan.after(kids[0], "Position").is_none());
        assert_vec(pos_after(&plan, kids[1]), v(4.0, 0.0, -3.0));
        // Unrotated blocks need no Orientation change: flipping the Z axis twice is identity.
        assert!(plan.after(kids[1], "Orientation").is_none());
    }

    #[test]
    fn rotated_part_gets_a_proper_orientation_value() {
        let mut w = world();
        let id = part(
            &mut w,
            v(0.0, 0.0, 0.0),
            v(4.0, 1.0, 2.0),
            v(0.0, 30.0, 0.0),
        );
        let o = MirrorOptions {
            axis: Axis::X,
            plane: MirrorPlane::Point(DVec3::ZERO),
            copy: false,
        };
        let plan = mirror(&objs(&w, &[id]), &o).unwrap();
        // Mirroring a yaw of 30 degrees across X gives a yaw of -30 degrees.
        match plan.after(id, "Orientation") {
            Some(Value::Vector3(o)) => {
                let r = rotation_from_orientation(*o);
                let expected = rotation_from_orientation(metallic_core::Vec3::new(0.0, -30.0, 0.0));
                // Same box: equal up to flipping two axes, so compare axis directions.
                assert!((r.x_axis.dot(expected.x_axis).abs() - 1.0).abs() < 1e-9);
                assert!((r.z_axis.dot(expected.z_axis).abs() - 1.0).abs() < 1e-9);
            }
            other => panic!("expected an Orientation change, got {other:?}"),
        }
    }

    #[test]
    fn non_finite_mirror_points_are_rejected() {
        let mut w = world();
        let a = part(&mut w, v(1.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        let objects = objs(&w, &[a]);
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for copy in [false, true] {
                let o = MirrorOptions {
                    axis: Axis::X,
                    plane: MirrorPlane::Point(d(0.0, x, 0.0)),
                    copy,
                };
                assert!(mirror(&objects, &o).is_err(), "point {x}");
            }
        }
        // A huge but finite point overflows the reflected position to infinity.
        let o = MirrorOptions {
            axis: Axis::X,
            plane: MirrorPlane::Point(d(f64::MAX, 0.0, 0.0)),
            copy: false,
        };
        assert!(mirror(&objects, &o).is_err());
        assert!(matches!(
            mirror(&[], &o),
            Err(ArrangeError::NeedMore { needed: 1, got: 0 })
        ));
    }
}
