//! `CFrame`: a position and a rotation.

use std::ops::Mul;

use glam::{EulerRot, Mat3, Quat, Vec3, Vec4};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A coordinate frame: a position plus a rotation, with no scale. It places parts, cameras and lights in the world.
///
/// The rotation is always a unit quaternion; every constructor normalizes it. Axes follow the engine convention:
/// right is +X, up is +Y and forward ([`look_vector`](Self::look_vector)) is -Z, like Roblox.
///
/// # Serialized form
///
/// Seven numbers, `[px, py, pz, qx, qy, qz, qw]`: the position in metres, then the rotation quaternion. Scene files
/// rely on this form, so it must not change. Reading normalizes a quaternion that is not already unit length (one
/// that is, as every written one is, reads back bit for bit) and rejects non-finite numbers or a zero quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CFrame {
    position: Vec3,
    /// The unit quaternion as `[x, y, z, w]`. Stored as a plain array rather than a `Quat` because `Quat` is
    /// 16-byte aligned on SIMD targets, which would pad a `CFrame` from 28 to 32 bytes and every `Value` from 32 to
    /// 48. Loading it back into a `Quat` for the maths is cheap.
    rotation: [f32; 4],
}

impl CFrame {
    /// No translation and no rotation.
    pub const IDENTITY: Self = Self::new(Vec3::ZERO);

    /// A frame at `position` with no rotation.
    pub const fn new(position: Vec3) -> Self {
        Self {
            position,
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// A frame at the origin with the given rotation. A rotation that cannot be normalized becomes the identity.
    pub fn from_rotation(rotation: Quat) -> Self {
        Self::from_parts(Vec3::ZERO, rotation)
    }

    /// A frame at `position` with the given rotation. A rotation that cannot be normalized (zero length, NaN)
    /// becomes the identity, so a `CFrame` never holds an invalid rotation.
    pub fn from_parts(position: Vec3, rotation: Quat) -> Self {
        Self {
            position,
            rotation: normalized_or_identity(rotation).to_array(),
        }
    }

    /// A frame at `eye` whose forward (-Z) points at `target`, with +Y as close to `up` as possible.
    ///
    /// If `eye` and `target` are the same point the rotation is the identity. If the view direction is parallel to
    /// `up`, some perpendicular up direction is chosen.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let Some(forward) = (target - eye).try_normalize() else {
            return Self::new(eye);
        };
        let right = forward
            .cross(up)
            .try_normalize()
            .unwrap_or_else(|| forward.any_orthonormal_vector());
        let up = right.cross(forward);
        let basis = Mat3::from_cols(right, up, -forward);
        Self::from_parts(eye, Quat::from_mat3(&basis))
    }

    /// A rotation from Euler angles in degrees, applied as `Rx * Ry * Rz`: Z first, then Y, then X, about the
    /// frame's own axes. This is the order of the scripting API's `CFrame.Angles`.
    pub fn from_euler_xyz_deg(x: f32, y: f32, z: f32) -> Self {
        Self::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            x.to_radians(),
            y.to_radians(),
            z.to_radians(),
        ))
    }

    /// The Euler angles in degrees, `(x, y, z)`, in the order of [`from_euler_xyz_deg`](Self::from_euler_xyz_deg).
    /// Near gimbal lock (y = ±90°) the split between x and z is not unique.
    pub fn to_euler_xyz_deg(self) -> Vec3 {
        let (x, y, z) = self.quat().to_euler(EulerRot::XYZ);
        Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
    }

    /// A rotation from orientation angles in degrees, applied as `Ry * Rx * Rz` (YXZ order): roll about Z, then
    /// pitch about X, then yaw about Y. This is the order of the `Orientation` property.
    pub fn from_orientation_deg(x: f32, y: f32, z: f32) -> Self {
        Self::from_rotation(Quat::from_euler(
            EulerRot::YXZ,
            y.to_radians(),
            x.to_radians(),
            z.to_radians(),
        ))
    }

    /// The orientation angles in degrees, `(x, y, z)`, in the order of
    /// [`from_orientation_deg`](Self::from_orientation_deg). Near gimbal lock (x = ±90°) the split between y and z
    /// is not unique.
    pub fn to_orientation_deg(self) -> Vec3 {
        let (y, x, z) = self.quat().to_euler(EulerRot::YXZ);
        Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
    }

    /// The position, in metres.
    pub const fn position(self) -> Vec3 {
        self.position
    }

    /// The rotation, always a unit quaternion.
    pub fn rotation(self) -> Quat {
        self.quat()
    }

    /// The frame that undoes this one: `cf * cf.inverse()` is the identity.
    pub fn inverse(self) -> Self {
        let rotation = self.quat().conjugate();
        Self {
            position: -(rotation * self.position),
            rotation: rotation.to_array(),
        }
    }

    /// Moves a point from this frame's local space to the parent space (rotation and translation).
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.position + self.quat() * point
    }

    /// Rotates a direction from this frame's local space to the parent space (no translation).
    pub fn transform_vector(self, vector: Vec3) -> Vec3 {
        self.quat() * vector
    }

    /// The forward direction (-Z), a unit vector.
    pub fn look_vector(self) -> Vec3 {
        self.quat() * Vec3::NEG_Z
    }

    /// The right direction (+X), a unit vector.
    pub fn right_vector(self) -> Vec3 {
        self.quat() * Vec3::X
    }

    /// The up direction (+Y), a unit vector.
    pub fn up_vector(self) -> Vec3 {
        self.quat() * Vec3::Y
    }

    /// Interpolates towards `other`: the position linearly, the rotation along the shortest arc (slerp). `t = 0`
    /// gives `self` and `t = 1` gives `other`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::from_parts(
            self.position.lerp(other.position, t),
            self.quat().slerp(other.quat(), t),
        )
    }

    /// Whether two frames are equal within `max_abs_diff` per component. `q` and `-q` are the same rotation and
    /// compare equal.
    pub fn approx_eq(self, other: Self, max_abs_diff: f32) -> bool {
        self.position.abs_diff_eq(other.position, max_abs_diff)
            && (self.quat().abs_diff_eq(other.quat(), max_abs_diff)
                || self.quat().abs_diff_eq(-other.quat(), max_abs_diff))
    }

    fn quat(self) -> Quat {
        Quat::from_array(self.rotation)
    }
}

impl Default for CFrame {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Composition: `a * b` is `b` expressed in `a`'s space, so `(a * b).transform_point(p)` equals
/// `a.transform_point(b.transform_point(p))`.
impl Mul for CFrame {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        // Renormalizing keeps long chains of compositions from drifting away from a unit quaternion.
        Self::from_parts(self.transform_point(rhs.position), self.quat() * rhs.quat())
    }
}

/// `cf * point` is [`CFrame::transform_point`].
impl Mul<Vec3> for CFrame {
    type Output = Vec3;

    fn mul(self, rhs: Vec3) -> Vec3 {
        self.transform_point(rhs)
    }
}

impl Serialize for CFrame {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let p = self.position;
        let [qx, qy, qz, qw] = self.rotation;
        [p.x, p.y, p.z, qx, qy, qz, qw].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CFrame {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let [px, py, pz, qx, qy, qz, qw] = <[f32; 7]>::deserialize(deserializer)?;
        let position = Vec3::new(px, py, pz);
        if !position.is_finite() {
            return Err(D::Error::custom("CFrame position must be finite"));
        }
        let rotation = Vec4::new(qx, qy, qz, qw);
        // A quaternion that is already unit length, as every written one is, is kept bit for bit: normalizing it
        // again can move its last bits, and a scene saved after loading would then differ from the file.
        let unit = rotation.is_finite() && (rotation.length_squared() - 1.0).abs() <= UNIT_TOLERANCE;
        let rotation = if unit { Some(rotation) } else { rotation.try_normalize() }
            .ok_or_else(|| D::Error::custom("CFrame rotation must be a finite, non-zero quaternion"))?;
        Ok(Self {
            position,
            rotation: rotation.to_array(),
        })
    }
}

/// How far a quaternion's squared length may be from 1 for reading to keep it as it is: a few `f32` roundings, as
/// left by normalizing.
const UNIT_TOLERANCE: f32 = 4.0 * f32::EPSILON;

fn normalized_or_identity(rotation: Quat) -> Quat {
    Vec4::from(rotation)
        .try_normalize()
        .map_or(Quat::IDENTITY, Quat::from_vec4)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn is_seven_unpadded_floats() {
        // Per-instance memory budget: a CFrame must stay 28 bytes with 4-byte alignment so `Value` fits in 32.
        assert_eq!(size_of::<CFrame>(), 28);
        assert_eq!(align_of::<CFrame>(), 4);
    }

    fn sample() -> CFrame {
        CFrame::from_parts(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_euler(EulerRot::XYZ, 0.3, -0.7, 1.1),
        )
    }

    #[test]
    fn identity_is_default_and_neutral() {
        let cf = sample();
        assert_eq!(CFrame::default(), CFrame::IDENTITY);
        assert!((CFrame::IDENTITY * cf).approx_eq(cf, EPS));
        assert!((cf * CFrame::IDENTITY).approx_eq(cf, EPS));
    }

    #[test]
    fn compose_with_inverse_round_trips() {
        let a = sample();
        let b = CFrame::from_parts(Vec3::new(-4.0, 0.5, 9.0), Quat::from_rotation_y(2.0));
        assert!((a * a.inverse()).approx_eq(CFrame::IDENTITY, EPS));
        assert!((a.inverse() * a).approx_eq(CFrame::IDENTITY, EPS));
        assert!((a.inverse() * (a * b)).approx_eq(b, EPS));
        assert!(((a * b).inverse()).approx_eq(b.inverse() * a.inverse(), EPS));
    }

    #[test]
    fn compose_matches_nested_transforms() {
        let a = sample();
        let b = CFrame::from_parts(Vec3::new(5.0, -1.0, 0.0), Quat::from_rotation_z(0.4));
        let p = Vec3::new(0.25, -3.0, 7.0);
        assert!(
            (a * b)
                .transform_point(p)
                .abs_diff_eq(a.transform_point(b.transform_point(p)), EPS)
        );
        assert!((a * p).abs_diff_eq(a.transform_point(p), EPS));
    }

    #[test]
    fn transform_point_matches_hand_computed_values() {
        // 90° about Y maps +X to -Z and +Z to +X.
        let cf = CFrame::new(Vec3::new(10.0, 0.0, 0.0)) * CFrame::from_euler_xyz_deg(0.0, 90.0, 0.0);
        assert!(
            cf.transform_point(Vec3::new(1.0, 2.0, 0.0))
                .abs_diff_eq(Vec3::new(10.0, 2.0, -1.0), EPS)
        );
        assert!(
            cf.transform_point(Vec3::new(0.0, 0.0, 1.0))
                .abs_diff_eq(Vec3::new(11.0, 0.0, 0.0), EPS)
        );
        assert!(cf.transform_vector(Vec3::X).abs_diff_eq(Vec3::NEG_Z, EPS));
    }

    #[test]
    fn direction_vectors_follow_the_axes() {
        assert!(CFrame::IDENTITY.look_vector().abs_diff_eq(Vec3::NEG_Z, EPS));
        assert!(CFrame::IDENTITY.right_vector().abs_diff_eq(Vec3::X, EPS));
        assert!(CFrame::IDENTITY.up_vector().abs_diff_eq(Vec3::Y, EPS));
        // Yaw 90° left: forward turns from -Z to -X.
        let cf = CFrame::from_orientation_deg(0.0, 90.0, 0.0);
        assert!(cf.look_vector().abs_diff_eq(Vec3::NEG_X, EPS));
        assert!(cf.right_vector().abs_diff_eq(Vec3::NEG_Z, EPS));
    }

    #[test]
    fn look_at_points_forward_at_target() {
        let eye = Vec3::new(0.0, 5.0, 10.0);
        let target = Vec3::new(3.0, 1.0, -2.0);
        let cf = CFrame::look_at(eye, target, Vec3::Y);
        assert!(cf.position().abs_diff_eq(eye, EPS));
        assert!(cf.look_vector().abs_diff_eq((target - eye).normalize(), EPS));
        assert!(cf.right_vector().y.abs() < EPS, "no roll when up is +Y");
        assert!(cf.up_vector().y > 0.0);

        let straight = CFrame::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -5.0), Vec3::Y);
        assert!(straight.approx_eq(CFrame::IDENTITY, EPS));
    }

    #[test]
    fn look_at_degenerate_cases_stay_valid() {
        let same = CFrame::look_at(Vec3::ONE, Vec3::ONE, Vec3::Y);
        assert!(same.approx_eq(CFrame::new(Vec3::ONE), EPS));

        let down = CFrame::look_at(Vec3::ZERO, Vec3::new(0.0, -3.0, 0.0), Vec3::Y);
        assert!(down.rotation().is_normalized());
        assert!(down.look_vector().abs_diff_eq(Vec3::NEG_Y, EPS));
    }

    #[test]
    fn euler_xyz_round_trips() {
        for (x, y, z) in [
            (10.0, 20.0, 30.0),
            (-45.0, 60.0, 170.0),
            (89.0, -30.0, -120.0),
            (0.0, 0.0, 0.0),
        ] {
            let back = CFrame::from_euler_xyz_deg(x, y, z).to_euler_xyz_deg();
            assert!(
                back.abs_diff_eq(Vec3::new(x, y, z), 1e-2),
                "({x}, {y}, {z}) came back as {back}"
            );
        }
    }

    #[test]
    fn euler_xyz_is_x_then_y_then_z_matrix_order() {
        let expected = Quat::from_rotation_x(0.5) * Quat::from_rotation_y(0.25) * Quat::from_rotation_z(-1.0);
        let cf = CFrame::from_euler_xyz_deg(0.5f32.to_degrees(), 0.25f32.to_degrees(), (-1.0f32).to_degrees());
        assert!(cf.approx_eq(CFrame::from_rotation(expected), EPS));
    }

    #[test]
    fn orientation_round_trips() {
        for (x, y, z) in [
            (10.0, 20.0, 30.0),
            (-45.0, 160.0, 5.0),
            (80.0, -100.0, -170.0),
            (0.0, 0.0, 0.0),
        ] {
            let back = CFrame::from_orientation_deg(x, y, z).to_orientation_deg();
            assert!(
                back.abs_diff_eq(Vec3::new(x, y, z), 1e-2),
                "({x}, {y}, {z}) came back as {back}"
            );
        }
    }

    #[test]
    fn orientation_is_y_then_x_then_z_matrix_order() {
        let expected = Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.25) * Quat::from_rotation_z(-1.0);
        let cf = CFrame::from_orientation_deg(0.25f32.to_degrees(), 0.5f32.to_degrees(), (-1.0f32).to_degrees());
        assert!(cf.approx_eq(CFrame::from_rotation(expected), EPS));
    }

    #[test]
    fn lerp_interpolates_position_and_rotation() {
        let a = CFrame::new(Vec3::ZERO);
        let b = CFrame::new(Vec3::new(10.0, 0.0, 0.0)) * CFrame::from_euler_xyz_deg(0.0, 90.0, 0.0);
        assert!(a.lerp(b, 0.0).approx_eq(a, EPS));
        assert!(a.lerp(b, 1.0).approx_eq(b, EPS));
        let mid = a.lerp(b, 0.5);
        let expected = CFrame::new(Vec3::new(5.0, 0.0, 0.0)) * CFrame::from_euler_xyz_deg(0.0, 45.0, 0.0);
        assert!(mid.approx_eq(expected, EPS));
    }

    #[test]
    fn invalid_rotation_becomes_identity() {
        let cf = CFrame::from_parts(Vec3::X, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0));
        assert_eq!(cf.rotation(), Quat::IDENTITY);
        let cf = CFrame::from_rotation(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0));
        assert_eq!(cf.rotation(), Quat::IDENTITY);
        let scaled = CFrame::from_rotation(Quat::from_xyzw(0.0, 2.0, 0.0, 0.0));
        assert_eq!(scaled.rotation(), Quat::from_xyzw(0.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn approx_eq_treats_q_and_minus_q_as_equal() {
        let q = Quat::from_rotation_y(1.0);
        assert!(CFrame::from_rotation(q).approx_eq(CFrame::from_rotation(-q), EPS));
        assert!(!CFrame::from_rotation(q).approx_eq(CFrame::IDENTITY, EPS));
    }

    #[test]
    fn serde_round_trips_as_seven_numbers() {
        let cf = CFrame::from_parts(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        let json = serde_json::to_string(&cf).expect("serialize");
        let numbers: Vec<f32> = serde_json::from_str(&json).expect("plain array");
        assert_eq!(numbers.len(), 7);
        assert_eq!(&numbers[..3], &[1.0, 2.0, 3.0]);
        let back: CFrame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, cf);
    }

    #[test]
    fn serde_normalizes_and_rejects_bad_rotations() {
        let cf: CFrame = serde_json::from_str("[0, 0, 0, 0, 0, 0, 2]").expect("non-unit quaternion is accepted");
        assert_eq!(cf, CFrame::IDENTITY);
        assert!(serde_json::from_str::<CFrame>("[0, 0, 0, 0, 0, 0, 0]").is_err());
        assert!(serde_json::from_str::<CFrame>("[0, 0, 0, 0, 0, 1]").is_err());
    }

    #[test]
    fn serde_keeps_unit_rotations_bit_for_bit() {
        // A rotation whose normalized form is not a fixed point of normalizing (from a stress scene, #186).
        let json = "[-191.19336, 1.199839, 27.382965, 0.0, 0.36817282, 0.0, 0.9297575]";
        let cf: CFrame = serde_json::from_str(json).expect("deserialize");
        assert_eq!(serde_json::to_string(&cf).expect("serialize"), json.replace(' ', ""));
        for angle in 0..64 {
            let cf = CFrame::from_rotation(Quat::from_euler(
                EulerRot::YXZ,
                angle as f32 * 0.37,
                angle as f32 * 0.11,
                0.2,
            ));
            let back: CFrame = serde_json::from_str(&serde_json::to_string(&cf).expect("serialize")).expect("read");
            assert_eq!(back, cf, "angle {angle}");
        }
    }
}
