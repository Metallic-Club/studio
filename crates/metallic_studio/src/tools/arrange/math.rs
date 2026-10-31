//! Rigid placements and the reflection used by mirror, on top of the shared
//! [`crate::tools::geom`] types (glam `DVec3`, `DMat3`, `DQuat`).
//!
//! Rotations follow the engine's `Orientation` property (degrees, applied Y, then X, then Z).
//! Conversions to and from `Orientation` go through `metallic_core::CFrame` so the tools, the
//! renderer and Luau agree. A `DMat3`'s columns are the part's local X, Y and Z axes in world space.

use glam::{DMat3, DQuat, DVec3};
use metallic_core::{CFrame, Vec3};

/// Values closer than this are treated as equal by the geometry helpers.
pub const EPS: f64 = 1e-9;

/// A world or local axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// X.
    X,
    /// Y.
    Y,
    /// Z.
    Z,
}

impl Axis {
    /// All three axes in order.
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// 0, 1 or 2.
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// The unit vector along this axis.
    pub fn unit(self) -> DVec3 {
        match self {
            Axis::X => DVec3::X,
            Axis::Y => DVec3::Y,
            Axis::Z => DVec3::Z,
        }
    }

    /// This axis's component of `v`.
    pub fn of(self, v: DVec3) -> f64 {
        v[self.index()]
    }
}

/// Converts a property vector to glam.
pub fn to_glam(v: Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

/// Converts a glam vector to a property vector.
pub fn to_core(v: DVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Whether every component is finite.
pub fn finite(v: DVec3) -> bool {
    v.is_finite()
}

/// The reflection across the plane through the origin with unit normal `n`: `I - 2 n nᵀ`.
pub fn reflection(n: DVec3) -> DMat3 {
    let n = n.normalize();
    DMat3::IDENTITY - DMat3::from_cols(n * n.x, n * n.y, n * n.z) * 2.0
}

/// The rotation for an `Orientation` value (degrees).
pub fn rotation_from_orientation(o: Vec3) -> DMat3 {
    let [x, y, z, w] = CFrame::from_position_orientation(Vec3::default(), o).quaternion();
    DMat3::from_quat(DQuat::from_xyzw(x, y, z, w).normalize())
}

/// The `Orientation` value (degrees) for a proper rotation.
pub fn orientation_of(rot: &DMat3) -> Vec3 {
    let cf = CFrame::from_axes(
        Vec3::default(),
        to_core(rot.x_axis),
        to_core(rot.y_axis),
        to_core(rot.z_axis),
    );
    let o = cf.orientation_degrees();
    let clean = |d: f64| if d.abs() < 1e-9 { 0.0 } else { d };
    Vec3::new(clean(o.x), clean(o.y), clean(o.z))
}

/// The largest absolute difference between entries (NaN if any entry is NaN).
pub fn max_diff(a: &DMat3, b: &DMat3) -> f64 {
    let d = (*a - *b).to_cols_array();
    if d.iter().any(|x| x.is_nan()) {
        return f64::NAN;
    }
    d.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// A rigid placement: rotation then translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Position (the part's centre).
    pub pos: DVec3,
    /// Rotation; columns are the local axes in world space.
    pub rot: DMat3,
}

impl Pose {
    /// A pure translation.
    pub fn translation(d: DVec3) -> Pose {
        Pose {
            pos: d,
            rot: DMat3::IDENTITY,
        }
    }

    /// A rotation of `degrees` about the line through `centre` along `axis`.
    pub fn rotation_about(centre: DVec3, axis: DVec3, degrees: f64) -> Pose {
        let rot = DMat3::from_axis_angle(axis.normalize(), degrees.to_radians());
        Pose {
            pos: centre - rot * centre,
            rot,
        }
    }

    /// `self * o`: first `o`, then `self`.
    pub fn compose(&self, o: &Pose) -> Pose {
        Pose {
            pos: self.rot * o.pos + self.pos,
            rot: self.rot * o.rot,
        }
    }

    /// Maps a point.
    pub fn point(&self, p: DVec3) -> DVec3 {
        self.rot * p + self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn orientation_round_trips() {
        for o in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(30.0, 45.0, -20.0),
            Vec3::new(-60.0, 170.0, 80.0),
            Vec3::new(10.0, -135.0, 179.0),
        ] {
            let back = orientation_of(&rotation_from_orientation(o));
            assert!(
                close(to_glam(back), to_glam(o)),
                "{o:?} came back as {back:?}"
            );
        }
    }

    #[test]
    fn orientation_at_gimbal_lock_keeps_the_rotation() {
        let r = rotation_from_orientation(Vec3::new(90.0, 20.0, 30.0));
        let back = rotation_from_orientation(orientation_of(&r));
        assert!(max_diff(&back, &r) < 1e-9);
    }

    #[test]
    fn orientation_order_is_y_then_x_then_z() {
        // Ry(90) * Rx(90) applied to local Y: Rx gives +Z, Ry turns +Z into +X.
        let r = rotation_from_orientation(Vec3::new(90.0, 90.0, 0.0));
        assert!(close(r.y_axis, DVec3::X));
        // Same as the shared geom helper.
        let q = crate::tools::geom::rotation_from_orientation_degrees(DVec3::new(90.0, 90.0, 0.0));
        assert!(max_diff(&DMat3::from_quat(q), &r) < 1e-12);
    }

    #[test]
    fn reflection_has_determinant_minus_one_and_flips_the_normal() {
        let n = DVec3::new(1.0, 2.0, -0.5).normalize();
        let m = reflection(n);
        assert!((m.determinant() + 1.0).abs() < 1e-12);
        assert!(close(m * n, -n));
        let t = n.cross(DVec3::Z);
        assert!(close(m * t, t));
    }

    #[test]
    fn pose_compose_and_rotation_about() {
        let r = Pose::rotation_about(DVec3::new(5.0, 0.0, 0.0), DVec3::Y, 90.0);
        assert!(close(
            r.point(DVec3::new(5.0, 0.0, 0.0)),
            DVec3::new(5.0, 0.0, 0.0)
        ));
        assert!(close(
            r.point(DVec3::new(6.0, 0.0, 0.0)),
            DVec3::new(5.0, 0.0, -1.0)
        ));
        let t = Pose::translation(DVec3::new(1.0, 2.0, 3.0));
        assert!(close(
            t.compose(&r).point(DVec3::new(6.0, 0.0, 0.0)),
            DVec3::new(6.0, 2.0, 2.0)
        ));
        assert!(max_diff(&DMat3::IDENTITY, &DMat3::IDENTITY) == 0.0);
        assert!(max_diff(&DMat3::IDENTITY, &(DMat3::IDENTITY * f64::NAN)).is_nan());
    }
}
