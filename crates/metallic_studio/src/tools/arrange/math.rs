//! Small f64 geometry for the arrange operations: vectors, rotation matrices, rigid poses and
//! boxes.
//!
//! Rotations follow the engine's `Orientation` property: Euler angles in degrees applied as Y, then
//! X, then Z (`R = Ry * Rx * Rz`), the same order `metallic_render` and `metallic_physics` use. A
//! [`Mat3`]'s columns are the part's local X, Y and Z axes in world space.

use metallic_core::Vec3;

/// Values closer than this are treated as equal by the geometry helpers.
pub const EPS: f64 = 1e-9;

/// Dot product.
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Cross product (right-handed).
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// Component-wise minimum.
pub fn vmin(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z))
}

/// Component-wise maximum.
pub fn vmax(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z))
}

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
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::new(1.0, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, 1.0, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, 1.0),
        }
    }

    /// This axis's component of `v`.
    pub fn of(self, v: Vec3) -> f64 {
        match self {
            Axis::X => v.x,
            Axis::Y => v.y,
            Axis::Z => v.z,
        }
    }
}

/// A 3x3 matrix stored as rows. Used for rotations and the mirror reflection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    /// Rows; `m[r][c]`.
    pub m: [[f64; 3]; 3],
}

impl Mat3 {
    /// The identity.
    pub const IDENTITY: Mat3 = Mat3 {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// A matrix whose columns are `x`, `y` and `z`.
    pub fn from_cols(x: Vec3, y: Vec3, z: Vec3) -> Mat3 {
        Mat3 {
            m: [[x.x, y.x, z.x], [x.y, y.y, z.y], [x.z, y.z, z.z]],
        }
    }

    /// Column `i` (a local axis in world space).
    pub fn col(&self, i: usize) -> Vec3 {
        Vec3::new(self.m[0][i], self.m[1][i], self.m[2][i])
    }

    /// The transpose (the inverse of a rotation).
    pub fn transpose(&self) -> Mat3 {
        let m = self.m;
        Mat3 {
            m: [
                [m[0][0], m[1][0], m[2][0]],
                [m[0][1], m[1][1], m[2][1]],
                [m[0][2], m[1][2], m[2][2]],
            ],
        }
    }

    /// Matrix product `self * o`.
    pub fn mul(&self, o: &Mat3) -> Mat3 {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.m[r][k] * o.m[k][c]).sum();
            }
        }
        Mat3 { m: out }
    }

    /// Matrix times vector.
    pub fn apply(&self, v: Vec3) -> Vec3 {
        let m = self.m;
        Vec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }

    /// The determinant (+1 for a rotation, -1 for a reflection).
    pub fn determinant(&self) -> f64 {
        let m = self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Rotation by `degrees` about the unit vector `axis` (right-hand rule).
    pub fn from_axis_angle(axis: Vec3, degrees: f64) -> Mat3 {
        let a = axis.unit();
        let (s, c) = degrees.to_radians().sin_cos();
        let t = 1.0 - c;
        Mat3 {
            m: [
                [
                    t * a.x * a.x + c,
                    t * a.x * a.y - s * a.z,
                    t * a.x * a.z + s * a.y,
                ],
                [
                    t * a.x * a.y + s * a.z,
                    t * a.y * a.y + c,
                    t * a.y * a.z - s * a.x,
                ],
                [
                    t * a.x * a.z - s * a.y,
                    t * a.y * a.z + s * a.x,
                    t * a.z * a.z + c,
                ],
            ],
        }
    }

    /// The reflection across the plane through the origin with unit normal `n`: `I - 2 n nᵀ`.
    pub fn reflection(n: Vec3) -> Mat3 {
        let n = n.unit();
        let v = [n.x, n.y, n.z];
        let mut out = Mat3::IDENTITY.m;
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell -= 2.0 * v[r] * v[c];
            }
        }
        Mat3 { m: out }
    }

    /// The rotation for an `Orientation` value (degrees, applied Y then X then Z).
    pub fn from_orientation(o: Vec3) -> Mat3 {
        let ry = Mat3::from_axis_angle(Axis::Y.unit(), o.y);
        let rx = Mat3::from_axis_angle(Axis::X.unit(), o.x);
        let rz = Mat3::from_axis_angle(Axis::Z.unit(), o.z);
        ry.mul(&rx).mul(&rz)
    }

    /// The `Orientation` value (degrees) for this rotation. X lies in -90..=90; at X = ±90 the Z
    /// angle is folded into Y. Tiny values are written as exactly 0.
    pub fn to_orientation(&self) -> Vec3 {
        // With R = Ry(b) Rx(a) Rz(c): m12 = -sin a, m02 = sin b cos a, m22 = cos b cos a,
        // m10 = cos a sin c, m11 = cos a cos c.
        let m = self.m;
        let sa = (-m[1][2]).clamp(-1.0, 1.0);
        let a = sa.asin();
        let (b, c) = if a.cos() > 1e-7 {
            (m[0][2].atan2(m[2][2]), m[1][0].atan2(m[1][1]))
        } else {
            ((-m[2][0]).atan2(m[0][0]), 0.0)
        };
        let clean = |r: f64| {
            let d = r.to_degrees();
            if d.abs() < 1e-9 { 0.0 } else { d }
        };
        Vec3::new(clean(a), clean(b), clean(c))
    }

    /// The largest absolute difference between entries.
    pub fn max_diff(&self, o: &Mat3) -> f64 {
        let mut worst: f64 = 0.0;
        for r in 0..3 {
            for c in 0..3 {
                worst = worst.max((self.m[r][c] - o.m[r][c]).abs());
            }
        }
        worst
    }
}

/// A rigid placement: rotation then translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Position (the part's centre).
    pub pos: Vec3,
    /// Rotation; columns are the local axes in world space.
    pub rot: Mat3,
}

impl Pose {
    /// No rotation, no translation.
    pub const IDENTITY: Pose = Pose {
        pos: Vec3::new(0.0, 0.0, 0.0),
        rot: Mat3::IDENTITY,
    };

    /// A pure translation.
    pub fn translation(d: Vec3) -> Pose {
        Pose {
            pos: d,
            rot: Mat3::IDENTITY,
        }
    }

    /// A rotation of `degrees` about the line through `centre` along `axis`.
    pub fn rotation_about(centre: Vec3, axis: Vec3, degrees: f64) -> Pose {
        let rot = Mat3::from_axis_angle(axis, degrees);
        Pose {
            pos: centre - rot.apply(centre),
            rot,
        }
    }

    /// `self * o`: first `o`, then `self`.
    pub fn compose(&self, o: &Pose) -> Pose {
        Pose {
            pos: self.rot.apply(o.pos) + self.pos,
            rot: self.rot.mul(&o.rot),
        }
    }

    /// The inverse placement.
    pub fn inverse(&self) -> Pose {
        let rt = self.rot.transpose();
        Pose {
            pos: rt.apply(self.pos) * -1.0,
            rot: rt,
        }
    }

    /// Maps a point.
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.rot.apply(p) + self.pos
    }
}

/// An axis-aligned box in some frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Smallest corner.
    pub min: Vec3,
    /// Largest corner.
    pub max: Vec3,
}

impl Aabb {
    /// A box holding only `p`.
    pub fn point(p: Vec3) -> Aabb {
        Aabb { min: p, max: p }
    }

    /// Grows the box to hold `p`.
    pub fn include(&mut self, p: Vec3) {
        self.min = vmin(self.min, p);
        self.max = vmax(self.max, p);
    }

    /// The smallest box holding both.
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: vmin(self.min, o.min),
            max: vmax(self.max, o.max),
        }
    }

    /// Centre point.
    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Extent per axis.
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

/// The eight corners of a box of `size` placed at `pose`.
pub fn box_corners(pose: &Pose, size: Vec3) -> [Vec3; 8] {
    let h = size * 0.5;
    let mut out = [Vec3::default(); 8];
    for (i, c) in out.iter_mut().enumerate() {
        let sx = if i & 1 == 0 { -h.x } else { h.x };
        let sy = if i & 2 == 0 { -h.y } else { h.y };
        let sz = if i & 4 == 0 { -h.z } else { h.z };
        *c = pose.point(Vec3::new(sx, sy, sz));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).magnitude() < 1e-9
    }

    #[test]
    fn orientation_round_trips() {
        for o in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(30.0, 45.0, -20.0),
            Vec3::new(-60.0, 170.0, 80.0),
            Vec3::new(10.0, -135.0, 179.0),
        ] {
            let back = Mat3::from_orientation(o).to_orientation();
            assert!(close(back, o), "{o:?} came back as {back:?}");
        }
    }

    #[test]
    fn orientation_at_gimbal_lock_keeps_the_rotation() {
        let r = Mat3::from_orientation(Vec3::new(90.0, 20.0, 30.0));
        let back = Mat3::from_orientation(r.to_orientation());
        assert!(back.max_diff(&r) < 1e-9);
    }

    #[test]
    fn orientation_order_is_y_then_x_then_z() {
        // Y 90 then X 90: local Z ends up pointing along world... compute by hand:
        // Ry(90) * Rx(90) applied to local Y (0,1,0): Rx gives (0,0,1), Ry gives (1,0,0).
        let r = Mat3::from_orientation(Vec3::new(90.0, 90.0, 0.0));
        assert!(close(r.col(1), Vec3::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn reflection_has_determinant_minus_one_and_flips_the_normal() {
        let n = Vec3::new(1.0, 2.0, -0.5).unit();
        let m = Mat3::reflection(n);
        assert!((m.determinant() + 1.0).abs() < 1e-12);
        assert!(close(m.apply(n), n * -1.0));
        let t = cross(n, Vec3::new(0.0, 0.0, 1.0));
        assert!(close(m.apply(t), t));
    }

    #[test]
    fn pose_inverse_and_compose() {
        let p = Pose {
            pos: Vec3::new(3.0, -2.0, 7.0),
            rot: Mat3::from_orientation(Vec3::new(20.0, 50.0, -10.0)),
        };
        let id = p.compose(&p.inverse());
        assert!(close(id.pos, Vec3::default()));
        assert!(id.rot.max_diff(&Mat3::IDENTITY) < 1e-12);
        let r = Pose::rotation_about(Vec3::new(5.0, 0.0, 0.0), Axis::Y.unit(), 90.0);
        assert!(close(
            r.point(Vec3::new(5.0, 0.0, 0.0)),
            Vec3::new(5.0, 0.0, 0.0)
        ));
        assert!(close(
            r.point(Vec3::new(6.0, 0.0, 0.0)),
            Vec3::new(5.0, 0.0, -1.0)
        ));
    }
}
