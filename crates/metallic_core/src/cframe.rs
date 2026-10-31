//! [`CFrame`]: a rigid transform (position plus rotation), with Roblox's conventions.
//!
//! The rotation is a row-major 3x3 matrix. Its columns are the right (X), up (Y) and back (Z)
//! axes in world space, so the look direction is minus the third column. Euler angles are in
//! radians here; `BasePart.Orientation` stores degrees applied as Y, then X, then Z, which is
//! [`CFrame::from_euler_yxz`].
//!
//! A stored CFrame always has a right-handed orthonormal rotation (`inverse` relies on it):
//! `World::set` and the scene loader pass every CFrame through [`CFrame::try_orthonormalize`],
//! which keeps the look vector, and refuse the ones it cannot fix.
use crate::value::Vec3;
use serde::{Deserialize, Serialize};
use std::ops::Mul;

/// Order in which three Euler angles are applied (Roblox `Enum.RotationOrder`).
///
/// `Xyz` means `R = Rx * Ry * Rz`: the Z rotation is applied to a vector first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationOrder {
    /// `Rx * Ry * Rz` (`CFrame.Angles`).
    Xyz,
    /// `Rx * Rz * Ry`.
    Xzy,
    /// `Ry * Rz * Rx`.
    Yzx,
    /// `Ry * Rx * Rz` (`CFrame.fromOrientation`, `BasePart.Orientation`).
    Yxz,
    /// `Rz * Rx * Ry`.
    Zxy,
    /// `Rz * Ry * Rx`.
    Zyx,
}

impl RotationOrder {
    /// Every order, in the item order of Roblox's `Enum.RotationOrder`.
    pub const ALL: [RotationOrder; 6] = [
        RotationOrder::Xyz,
        RotationOrder::Xzy,
        RotationOrder::Yzx,
        RotationOrder::Yxz,
        RotationOrder::Zxy,
        RotationOrder::Zyx,
    ];

    /// The `Enum.RotationOrder` item name.
    pub const fn name(self) -> &'static str {
        match self {
            RotationOrder::Xyz => "XYZ",
            RotationOrder::Xzy => "XZY",
            RotationOrder::Yzx => "YZX",
            RotationOrder::Yxz => "YXZ",
            RotationOrder::Zxy => "ZXY",
            RotationOrder::Zyx => "ZYX",
        }
    }
}

/// A position and an orientation in 3D (Roblox `CFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CFrame {
    /// Translation (studs).
    pub position: Vec3,
    /// Rotation matrix, row-major: `rotation[row][column]`.
    pub rotation: [[f64; 3]; 3],
}

impl Default for CFrame {
    fn default() -> Self {
        CFrame::IDENTITY
    }
}

const IDENTITY_MATRIX: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

fn transpose(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

fn mat_vec(m: &[[f64; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}

fn rot_x(a: f64) -> [[f64; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(a: f64) -> [[f64; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(a: f64) -> [[f64; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Cross product.
pub(crate) fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// Dot product.
pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Rotations within this of orthonormal (largest entry of `R^T R - I`) and right-handed are kept
/// bit for bit by [`CFrame::try_orthonormalize`]. Float drift from composing CFrames or an `f32`
/// round trip (about 1e-7) stays under it.
pub const ORTHONORMAL_TOLERANCE: f64 = 1e-6;

/// An axis column shorter than this counts as zero: [`CFrame::try_orthonormalize`] refuses it.
pub const MIN_AXIS_LENGTH: f64 = 1e-9;

/// [`CFrame::try_orthonormalize`] refuses a rotation whose columns are this close to parallel or
/// coplanar: `|det R| / (|right| |up| |back|)`, which is scale-free (1 for an orthonormal basis,
/// 0 for a flat one).
pub const DEGENERATE_EPSILON: f64 = 1e-6;

/// Below this `|sin|` of the middle angle the Euler decomposition treats the rotation as
/// gimbal-locked and puts the whole remaining turn in the first angle.
const GIMBAL_EPSILON: f64 = 1e-9;

impl CFrame {
    /// No translation, no rotation.
    pub const IDENTITY: CFrame = CFrame {
        position: Vec3::new(0.0, 0.0, 0.0),
        rotation: IDENTITY_MATRIX,
    };

    /// A pure translation.
    pub const fn from_position(position: Vec3) -> CFrame {
        CFrame {
            position,
            rotation: IDENTITY_MATRIX,
        }
    }

    /// A position and a row-major rotation matrix, taken as given.
    pub const fn from_parts(position: Vec3, rotation: [[f64; 3]; 3]) -> CFrame {
        CFrame { position, rotation }
    }

    /// From the position and the three axis columns (right, up, back), taken as given.
    pub fn from_axes(position: Vec3, right: Vec3, up: Vec3, back: Vec3) -> CFrame {
        CFrame {
            position,
            rotation: [
                [right.x, up.x, back.x],
                [right.y, up.y, back.y],
                [right.z, up.z, back.z],
            ],
        }
    }

    /// Euler angles (radians) applied in `order`; see [`RotationOrder`].
    pub fn from_euler(rx: f64, ry: f64, rz: f64, order: RotationOrder) -> CFrame {
        let (x, y, z) = (rot_x(rx), rot_y(ry), rot_z(rz));
        let (a, b, c) = match order {
            RotationOrder::Xyz => (x, y, z),
            RotationOrder::Xzy => (x, z, y),
            RotationOrder::Yzx => (y, z, x),
            RotationOrder::Yxz => (y, x, z),
            RotationOrder::Zxy => (z, x, y),
            RotationOrder::Zyx => (z, y, x),
        };
        CFrame {
            position: Vec3::default(),
            rotation: mat_mul(&mat_mul(&a, &b), &c),
        }
    }

    /// `Rx * Ry * Rz` (`CFrame.Angles`, `CFrame.fromEulerAnglesXYZ`).
    pub fn from_euler_xyz(rx: f64, ry: f64, rz: f64) -> CFrame {
        CFrame::from_euler(rx, ry, rz, RotationOrder::Xyz)
    }

    /// `Ry * Rx * Rz` (`CFrame.fromOrientation`, `CFrame.fromEulerAnglesYXZ`).
    pub fn from_euler_yxz(rx: f64, ry: f64, rz: f64) -> CFrame {
        CFrame::from_euler(rx, ry, rz, RotationOrder::Yxz)
    }

    /// A rotation of `angle` radians about `axis` (right-handed). A zero axis gives identity.
    pub fn from_axis_angle(axis: Vec3, angle: f64) -> CFrame {
        let m = axis.magnitude();
        if m == 0.0 {
            return CFrame::IDENTITY;
        }
        let (x, y, z) = (axis.x / m, axis.y / m, axis.z / m);
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        CFrame {
            position: Vec3::default(),
            rotation: [
                [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
                [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
                [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
            ],
        }
    }

    /// At `at`, looking towards `target`, with `up` as the preferred up direction
    /// (`CFrame.lookAt`). When the look direction is parallel to `up`, a world axis is used.
    pub fn look_at(at: Vec3, target: Vec3, up: Vec3) -> CFrame {
        let look = (target - at).unit();
        if look.magnitude() == 0.0 {
            return CFrame::from_position(at);
        }
        let mut right = cross(look, up);
        if right.magnitude() < 1e-9 {
            // Looking straight along `up`: pick any right vector orthogonal to the look.
            right = cross(look, Vec3::new(0.0, 0.0, 1.0));
            if right.magnitude() < 1e-9 {
                right = cross(look, Vec3::new(1.0, 0.0, 0.0));
            }
        }
        let right = right.unit();
        let up = cross(right, look).unit();
        CFrame::from_axes(at, right, up, look * -1.0)
    }

    /// The frame of a part from its `Position` and `Orientation` (degrees, Y then X then Z).
    pub fn from_position_orientation(position: Vec3, orientation_degrees: Vec3) -> CFrame {
        let o = orientation_degrees;
        let mut cf = CFrame::from_euler_yxz(o.x.to_radians(), o.y.to_radians(), o.z.to_radians());
        cf.position = position;
        cf
    }

    /// `Orientation` in degrees (Y then X then Z), as a part stores its rotation.
    pub fn orientation_degrees(self) -> Vec3 {
        let (x, y, z) = self.to_euler(RotationOrder::Yxz);
        Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
    }

    /// The rotation part only (position zero).
    pub fn rotation_only(self) -> CFrame {
        CFrame {
            position: Vec3::default(),
            rotation: self.rotation,
        }
    }

    /// Right axis (first column; `RightVector`, `XVector`).
    pub fn right_vector(self) -> Vec3 {
        let r = self.rotation;
        Vec3::new(r[0][0], r[1][0], r[2][0])
    }

    /// Up axis (second column; `UpVector`, `YVector`).
    pub fn up_vector(self) -> Vec3 {
        let r = self.rotation;
        Vec3::new(r[0][1], r[1][1], r[2][1])
    }

    /// Back axis (third column; `ZVector`).
    pub fn back_vector(self) -> Vec3 {
        let r = self.rotation;
        Vec3::new(r[0][2], r[1][2], r[2][2])
    }

    /// Look direction: minus the back axis (`LookVector`).
    pub fn look_vector(self) -> Vec3 {
        self.back_vector() * -1.0
    }

    /// The inverse transform. Assumes an orthonormal rotation (its transpose is its inverse).
    pub fn inverse(self) -> CFrame {
        let rotation = transpose(&self.rotation);
        CFrame {
            position: mat_vec(&rotation, self.position) * -1.0,
            rotation,
        }
    }

    /// A point in this frame's local space, in world space.
    pub fn point_to_world(self, v: Vec3) -> Vec3 {
        mat_vec(&self.rotation, v) + self.position
    }

    /// A world-space point, in this frame's local space.
    pub fn point_to_object(self, v: Vec3) -> Vec3 {
        mat_vec(&transpose(&self.rotation), v - self.position)
    }

    /// A direction in local space, in world space (rotation only).
    pub fn vector_to_world(self, v: Vec3) -> Vec3 {
        mat_vec(&self.rotation, v)
    }

    /// A world direction, in local space (rotation only).
    pub fn vector_to_object(self, v: Vec3) -> Vec3 {
        mat_vec(&transpose(&self.rotation), v)
    }

    /// `self * other`: `other` given relative to this frame, in world space.
    pub fn to_world(self, other: CFrame) -> CFrame {
        self * other
    }

    /// `other` relative to this frame: `self:Inverse() * other`.
    pub fn to_object(self, other: CFrame) -> CFrame {
        self.inverse() * other
    }

    /// The same frame moved by `offset` in world space.
    pub fn translated(self, offset: Vec3) -> CFrame {
        CFrame {
            position: self.position + offset,
            rotation: self.rotation,
        }
    }

    /// `x, y, z, R00, R01, R02, R10, R11, R12, R20, R21, R22` (`GetComponents`).
    pub fn components(self) -> [f64; 12] {
        let p = self.position;
        let r = self.rotation;
        [
            p.x, p.y, p.z, r[0][0], r[0][1], r[0][2], r[1][0], r[1][1], r[1][2], r[2][0], r[2][1],
            r[2][2],
        ]
    }

    /// Euler angles (radians) that rebuild this rotation with `order`
    /// (`ToEulerAngles`, `ToEulerAnglesXYZ`, `ToEulerAnglesYXZ`, `ToOrientation`).
    ///
    /// The middle angle is in `[-pi/2, pi/2]`, the other two in `(-pi, pi]`. In gimbal lock
    /// the last angle is zero.
    pub fn to_euler(self, order: RotationOrder) -> (f64, f64, f64) {
        let m = self.rotation;
        // For an order (a, b, c) the matrix is Ra * Rb * Rc. With i, j, k the axis indices of
        // a, b, c and the sign of the permutation, the entry m[i][k] is sign * sin(b).
        let (i, j, k) = match order {
            RotationOrder::Xyz => (0, 1, 2),
            RotationOrder::Xzy => (0, 2, 1),
            RotationOrder::Yzx => (1, 2, 0),
            RotationOrder::Yxz => (1, 0, 2),
            RotationOrder::Zxy => (2, 0, 1),
            RotationOrder::Zyx => (2, 1, 0),
        };
        // +1 for cyclic orders (XYZ, YZX, ZXY), -1 for the others.
        let sign = if (j + 3 - i) % 3 == 1 { 1.0 } else { -1.0 };
        let sb = (sign * m[i][k]).clamp(-1.0, 1.0);
        let b = sb.asin();
        let (a, c) = if (1.0 - sb.abs()) > GIMBAL_EPSILON {
            (
                (-sign * m[j][k]).atan2(m[k][k]),
                (-sign * m[i][j]).atan2(m[i][i]),
            )
        } else {
            // cos(b) is 0: only a + c (or a - c) is defined; take c = 0.
            ((sign * m[k][j]).atan2(m[j][j]), 0.0)
        };
        let mut angles = [0.0; 3];
        angles[i] = a;
        angles[j] = b;
        angles[k] = c;
        (angles[0], angles[1], angles[2])
    }

    /// The rotation as a unit quaternion `[x, y, z, w]`.
    pub fn quaternion(self) -> [f64; 4] {
        let m = self.rotation;
        let trace = m[0][0] + m[1][1] + m[2][2];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [
                (m[2][1] - m[1][2]) / s,
                (m[0][2] - m[2][0]) / s,
                (m[1][0] - m[0][1]) / s,
                0.25 * s,
            ]
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            [
                0.25 * s,
                (m[0][1] + m[1][0]) / s,
                (m[0][2] + m[2][0]) / s,
                (m[2][1] - m[1][2]) / s,
            ]
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            [
                (m[0][1] + m[1][0]) / s,
                0.25 * s,
                (m[1][2] + m[2][1]) / s,
                (m[0][2] - m[2][0]) / s,
            ]
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            [
                (m[0][2] + m[2][0]) / s,
                (m[1][2] + m[2][1]) / s,
                0.25 * s,
                (m[1][0] - m[0][1]) / s,
            ]
        };
        let n = q.iter().map(|c| c * c).sum::<f64>().sqrt();
        if n == 0.0 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            q.map(|c| c / n)
        }
    }

    /// The rotation of a quaternion `[x, y, z, w]` (normalised first) at `position`.
    pub fn from_quaternion(position: Vec3, q: [f64; 4]) -> CFrame {
        let n = q.iter().map(|c| c * c).sum::<f64>().sqrt();
        let [x, y, z, w] = if n == 0.0 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            q.map(|c| c / n)
        };
        CFrame {
            position,
            rotation: [
                [
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y - z * w),
                    2.0 * (x * z + y * w),
                ],
                [
                    2.0 * (x * y + z * w),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z - x * w),
                ],
                [
                    2.0 * (x * z - y * w),
                    2.0 * (y * z + x * w),
                    1.0 - 2.0 * (x * x + y * y),
                ],
            ],
        }
    }

    /// The rotation as a unit axis and an angle in radians (`ToAxisAngle`).
    /// Identity gives the X axis and 0.
    pub fn to_axis_angle(self) -> (Vec3, f64) {
        let [x, y, z, w] = self.quaternion();
        let s = (x * x + y * y + z * z).sqrt();
        if s < 1e-12 {
            return (Vec3::new(1.0, 0.0, 0.0), 0.0);
        }
        (Vec3::new(x / s, y / s, z / s), 2.0 * s.atan2(w))
    }

    /// Interpolates position linearly and rotation along the shortest arc (`Lerp`).
    /// `alpha` 0 gives `self`, 1 gives `goal`; values outside extrapolate.
    pub fn lerp(self, goal: CFrame, alpha: f64) -> CFrame {
        let a = self.quaternion();
        let mut b = goal.quaternion();
        let mut d: f64 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        if d < 0.0 {
            b = b.map(|c| -c);
            d = -d;
        }
        let q = if d > 0.9995 {
            // Nearly the same rotation: linear blend, normalised by from_quaternion.
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * alpha)
        } else {
            let theta = d.clamp(-1.0, 1.0).acos();
            let s = theta.sin();
            let wa = ((1.0 - alpha) * theta).sin() / s;
            let wb = (alpha * theta).sin() / s;
            std::array::from_fn(|i| a[i] * wa + b[i] * wb)
        };
        let position = self.position + (goal.position - self.position) * alpha;
        CFrame::from_quaternion(position, q)
    }

    /// The rotation as a right-handed orthonormal basis, by the rule every stored CFrame follows
    /// (`World::set`, the scene loader, Luau `CFrame.new` with 12 numbers).
    ///
    /// - A rotation that is already orthonormal and right-handed to within
    ///   [`ORTHONORMAL_TOLERANCE`] comes back bit for bit, so round trips and saved scenes do not
    ///   drift.
    /// - Otherwise the look direction is kept, the up axis loses its part along the look and is
    ///   normalised (Gram-Schmidt), and right is `look x up`: the same basis as
    ///   [`CFrame::look_at`]. A scale on the axes is dropped; a mirrored (left-handed) matrix gets
    ///   its right axis flipped.
    /// - `None` when a component is not finite, an axis column is shorter than
    ///   [`MIN_AXIS_LENGTH`], or the columns are (nearly) parallel or coplanar:
    ///   `|det R| / (|right| |up| |back|)` below [`DEGENERATE_EPSILON`].
    ///
    /// The position is never changed.
    pub fn try_orthonormalize(self) -> Option<CFrame> {
        if !self.is_finite() {
            return None;
        }
        let (right, up, back) = (self.right_vector(), self.up_vector(), self.back_vector());
        let lengths = [right.magnitude(), up.magnitude(), back.magnitude()];
        if lengths.iter().any(|l| *l < MIN_AXIS_LENGTH) {
            return None;
        }
        let det = dot(right, cross(up, back));
        if det.abs() / (lengths[0] * lengths[1] * lengths[2]) < DEGENERATE_EPSILON {
            return None;
        }
        if det > 0.0 && self.is_orthonormal() {
            return Some(self);
        }
        let look = back * (-1.0 / lengths[2]);
        let right = cross(look, up).unit();
        let up = cross(right, look);
        let out = CFrame::from_axes(self.position, right, up, look * -1.0);
        out.is_finite().then_some(out)
    }

    /// True when every entry of `R^T R` is within [`ORTHONORMAL_TOLERANCE`] of the identity
    /// (unit, perpendicular columns; a mirrored matrix also passes).
    fn is_orthonormal(self) -> bool {
        let r = self.rotation;
        let rtr = mat_mul(&transpose(&r), &r);
        (0..3).all(|i| {
            (0..3).all(|j| {
                let id = if i == j { 1.0 } else { 0.0 };
                (rtr[i][j] - id).abs() <= ORTHONORMAL_TOLERANCE
            })
        })
    }

    /// The rotation re-orthonormalised by the stored-CFrame rule ([`CFrame::try_orthonormalize`],
    /// which keeps the look vector) (`Orthonormalize`). A rotation that cannot be orthonormalised
    /// (degenerate or not finite) comes back unchanged, so writing it to a property still fails.
    pub fn orthonormalize(self) -> CFrame {
        self.try_orthonormalize().unwrap_or(self)
    }

    /// True when every component is finite.
    pub fn is_finite(self) -> bool {
        self.components().iter().all(|c| c.is_finite())
    }
}

impl Mul for CFrame {
    type Output = CFrame;
    fn mul(self, o: CFrame) -> CFrame {
        CFrame {
            position: self.point_to_world(o.position),
            rotation: mat_mul(&self.rotation, &o.rotation),
        }
    }
}

impl Mul<Vec3> for CFrame {
    type Output = Vec3;
    fn mul(self, v: Vec3) -> Vec3 {
        self.point_to_world(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).magnitude() < 1e-9
    }

    #[test]
    fn angles_turn_axes_like_roblox() {
        // A quarter turn about Y takes the look vector (-Z) to -X.
        let cf = CFrame::from_euler_xyz(0.0, FRAC_PI_2, 0.0);
        assert!(close(cf.look_vector(), Vec3::new(-1.0, 0.0, 0.0)));
        assert!(close(cf.right_vector(), Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn euler_round_trips_in_every_order() {
        for order in RotationOrder::ALL {
            let (x, y, z) = (0.3, -0.7, 1.1);
            let cf = CFrame::from_euler(x, y, z, order);
            let (a, b, c) = cf.to_euler(order);
            let back = CFrame::from_euler(a, b, c, order);
            for (u, v) in cf.components().iter().zip(back.components()) {
                assert!((u - v).abs() < 1e-12, "{order:?}");
            }
            assert!((a - x).abs() < 1e-12 && (b - y).abs() < 1e-12 && (c - z).abs() < 1e-12);
        }
    }

    #[test]
    fn euler_in_gimbal_lock_rebuilds_the_rotation() {
        for order in RotationOrder::ALL {
            let mut angles = [0.4, 0.4, 0.4];
            // The middle angle of each order at +90 degrees.
            let middle = match order {
                RotationOrder::Yxz | RotationOrder::Zxy => 0,
                RotationOrder::Xyz | RotationOrder::Zyx => 1,
                RotationOrder::Xzy | RotationOrder::Yzx => 2,
            };
            angles[middle] = FRAC_PI_2;
            let cf = CFrame::from_euler(angles[0], angles[1], angles[2], order);
            let (a, b, c) = cf.to_euler(order);
            let back = CFrame::from_euler(a, b, c, order);
            for (u, v) in cf.components().iter().zip(back.components()) {
                assert!((u - v).abs() < 1e-9, "{order:?}");
            }
        }
    }

    #[test]
    fn inverse_undoes_the_transform() {
        let cf = CFrame::from_position(Vec3::new(1.0, 2.0, 3.0))
            * CFrame::from_euler_xyz(0.2, 0.5, -0.9);
        let p = Vec3::new(4.0, -5.0, 6.0);
        assert!(close(cf.inverse() * (cf * p), p));
        assert!(close(cf.point_to_object(cf.point_to_world(p)), p));
    }

    #[test]
    fn lerp_halfway_is_half_the_turn() {
        let a = CFrame::IDENTITY;
        let b = CFrame::from_position(Vec3::new(10.0, 0.0, 0.0))
            * CFrame::from_euler_xyz(0.0, FRAC_PI_2, 0.0);
        let m = a.lerp(b, 0.5);
        assert!(close(m.position, Vec3::new(5.0, 0.0, 0.0)));
        let (_, y, _) = m.to_euler(RotationOrder::Xyz);
        assert!((y - FRAC_PI_2 / 2.0).abs() < 1e-12);
    }
}
