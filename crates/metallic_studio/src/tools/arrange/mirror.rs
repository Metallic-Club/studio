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

use metallic_core::{InstanceId, Vec3, World};

use super::math::{Axis, Mat3, Pose, dot};
use super::plan::{ArrangeError, Plan, describe};
use super::scene::{PartState, Shape, read_objects, selection_pivot};

/// Where the mirror plane passes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MirrorPlane {
    /// The selection's pivot: one object's pivot, or the centre of several objects' bounds.
    Pivot,
    /// The active (last selected) object's pivot.
    Active,
    /// A given point.
    Point(Vec3),
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
pub fn flip_axis(shape: Shape, rot: &Mat3, normal: Vec3) -> usize {
    match shape {
        Shape::Wedge => 0,
        Shape::Block | Shape::Ball | Shape::Cylinder => (0..3)
            .max_by(|&a, &b| {
                dot(rot.col(a), normal)
                    .abs()
                    .total_cmp(&dot(rot.col(b), normal).abs())
            })
            .unwrap_or(0),
    }
}

/// `part` reflected across the plane through `origin` with normal `normal`.
pub fn mirror_part(part: &PartState, origin: Vec3, normal: Vec3) -> PartState {
    let n = normal.unit();
    let m = Mat3::reflection(n);
    let pos = origin + m.apply(part.pose.pos - origin);
    let k = flip_axis(part.shape, &part.pose.rot, n);
    let mut flip = Mat3::IDENTITY;
    flip.m[k][k] = -1.0;
    let rot = m.mul(&part.pose.rot).mul(&flip);
    PartState {
        pose: Pose { pos, rot },
        ..*part
    }
}

/// Plans a mirror of the objects `ids`. Models mirror all their parts across the same plane.
///
/// Errors: [`ArrangeError::NeedMore`] with no objects.
pub fn mirror(
    world: &World,
    ids: &[InstanceId],
    opts: &MirrorOptions,
) -> Result<Plan, ArrangeError> {
    let objects = read_objects(world, ids)?;
    let Some(active) = objects.last() else {
        return Err(ArrangeError::NeedMore { needed: 1, got: 0 });
    };
    let origin = match opts.plane {
        MirrorPlane::Pivot => selection_pivot(&objects),
        MirrorPlane::Active => active.pivot().pos,
        MirrorPlane::Point(p) => {
            if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
                return Err(ArrangeError::Invalid(
                    "the mirror point must be a finite number".into(),
                ));
            }
            p
        }
    };
    let normal = opts.axis.unit();
    let verb = if opts.copy { "Mirror copy" } else { "Mirror" };
    let mut plan = Plan {
        label: format!("{verb} {}", describe(&objects)),
        ..Plan::default()
    };
    for o in &objects {
        let after: Vec<PartState> = o
            .parts
            .iter()
            .map(|p| mirror_part(p, origin, normal))
            .collect();
        if opts.copy {
            plan.push_copy(world, o, &after);
        } else {
            plan.push_object(o, &after);
        }
    }
    plan.check_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arrange::math::box_corners;
    use crate::tools::arrange::testing::*;
    use metallic_core::Value;

    /// The six corners of a wedge in world space.
    fn wedge_corners(p: &PartState) -> Vec<Vec3> {
        let h = p.size * 0.5;
        [
            v(-h.x, -h.y, -h.z),
            v(h.x, -h.y, -h.z),
            v(-h.x, -h.y, h.z),
            v(h.x, -h.y, h.z),
            v(-h.x, h.y, h.z),
            v(h.x, h.y, h.z),
        ]
        .iter()
        .map(|&c| p.pose.point(c))
        .collect()
    }

    fn same_point_sets(a: &[Vec3], b: &[Vec3]) -> bool {
        a.len() == b.len()
            && a.iter()
                .all(|p| b.iter().any(|q| (*p - *q).magnitude() < 1e-9))
    }

    fn reflect(p: Vec3, origin: Vec3, n: Vec3) -> Vec3 {
        p - n * (2.0 * dot(p - origin, n))
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
        let before = PartState::read(&w, id).unwrap();
        for axis in Axis::ALL {
            let origin = v(1.0, -2.0, 0.5);
            let n = axis.unit();
            let after = mirror_part(&before, origin, n);
            assert!((after.pose.rot.determinant() - 1.0).abs() < 1e-12);
            assert_eq!(after.size, before.size);
            let expected: Vec<Vec3> = wedge_corners(&before)
                .into_iter()
                .map(|c| reflect(c, origin, n))
                .collect();
            assert!(
                same_point_sets(&wedge_corners(&after), &expected),
                "wedge mirrored across {axis:?} does not fill the mirror image"
            );
            // The written Orientation reproduces the same rotation.
            let o = after.pose.rot.to_orientation();
            assert!(Mat3::from_orientation(o).max_diff(&after.pose.rot) < 1e-9);
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
        let p = PartState::read(&w, id).unwrap();
        let m = mirror_part(&p, v(0.0, 0.0, 0.0), Axis::Z.unit());
        assert_vec(m.pose.pos, v(0.0, 0.0, -4.0));
        // Local +Z (towards the tall edge) now points to world -Z.
        assert_vec(m.pose.rot.col(2), v(0.0, 0.0, -1.0));
        // Local X was flipped back: the rotation is 180 degrees about Y.
        assert_vec(m.pose.rot.col(0), v(-1.0, 0.0, 0.0));
        assert_vec(m.pose.rot.col(1), v(0.0, 1.0, 0.0));
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
        let p = PartState::read(&w, id).unwrap();
        let origin = v(0.0, 0.0, 0.0);
        let n = Axis::X.unit();
        let m = mirror_part(&p, origin, n);
        let expected: Vec<Vec3> = box_corners(&p.pose, p.size)
            .iter()
            .map(|&c| reflect(c, origin, n))
            .collect();
        assert!(same_point_sets(&box_corners(&m.pose, m.size), &expected));
        assert!((m.pose.rot.determinant() - 1.0).abs() < 1e-12);
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
        let plan = mirror(&w, &[a, b], &o).unwrap();
        assert_vec(pos_after(&plan, a), v(10.0, 0.0, 0.0));
        assert_vec(pos_after(&plan, b), v(0.0, 4.0, 0.0));
        assert_eq!(plan.label, "Mirror 2 parts");
        assert!(plan.copies.is_empty());

        let plan = mirror(&w, &[a, b], &MirrorOptions { copy: true, ..o }).unwrap();
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
        let plan = mirror(&w, &[m, hub], &o).unwrap();
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
            plane: MirrorPlane::Point(v(0.0, 0.0, 0.0)),
            copy: false,
        };
        let plan = mirror(&w, &[id], &o).unwrap();
        // Mirroring a yaw of 30 degrees across X gives a yaw of -30 degrees.
        match plan.after(id, "Orientation") {
            Some(Value::Vector3(o)) => {
                let r = Mat3::from_orientation(*o);
                let expected = Mat3::from_orientation(v(0.0, -30.0, 0.0));
                // Same box: equal up to flipping two axes, so compare the long axis direction.
                assert!((dot(r.col(0), expected.col(0)).abs() - 1.0).abs() < 1e-9);
                assert!((dot(r.col(2), expected.col(2)).abs() - 1.0).abs() < 1e-9);
            }
            other => panic!("expected an Orientation change, got {other:?}"),
        }
    }

    #[test]
    fn non_finite_mirror_points_are_rejected() {
        let mut w = world();
        let a = part(&mut w, v(1.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 0.0, 0.0));
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for copy in [false, true] {
                let o = MirrorOptions {
                    axis: Axis::X,
                    plane: MirrorPlane::Point(v(0.0, x, 0.0)),
                    copy,
                };
                assert!(mirror(&w, &[a], &o).is_err(), "point {x}");
            }
        }
        // A huge but finite point overflows the reflected position to infinity.
        let o = MirrorOptions {
            axis: Axis::X,
            plane: MirrorPlane::Point(v(f64::MAX, 0.0, 0.0)),
            copy: false,
        };
        assert!(mirror(&w, &[a], &o).is_err());
    }
}
