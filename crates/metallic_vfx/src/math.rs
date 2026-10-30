//! Small vector maths used by the simulator.
//!
//! The crate keeps its own tiny `Vec3` instead of pulling in a maths crate, so the effect file format
//! stays independent of any dependency's serialisation (a vector is always written as `(x, y, z)`).

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

/// A 3D vector in studs (or studs per second). Written as `(x, y, z)` in effect files.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "(f32, f32, f32)", into = "(f32, f32, f32)")]
pub struct Vec3 {
    /// X component (right).
    pub x: f32,
    /// Y component (up).
    pub y: f32,
    /// Z component (towards the viewer in the default preview camera).
    pub z: f32,
}

impl From<(f32, f32, f32)> for Vec3 {
    fn from(v: (f32, f32, f32)) -> Self {
        Self::new(v.0, v.1, v.2)
    }
}

impl From<Vec3> for (f32, f32, f32) {
    fn from(v: Vec3) -> Self {
        (v.x, v.y, v.z)
    }
}

impl Vec3 {
    /// All zeros.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// All ones.
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0);
    /// Unit X.
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    /// Unit Y (world up).
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    /// Unit Z.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    /// Builds a vector.
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Same value in every component.
    #[must_use]
    pub const fn splat(v: f32) -> Self {
        Self::new(v, v, v)
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Cross product.
    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Squared length.
    #[must_use]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Unit vector in the same direction, or `fallback` when the length is (almost) zero.
    #[must_use]
    pub fn normalize_or(self, fallback: Self) -> Self {
        let len = self.length();
        if len > 1e-6 && len.is_finite() {
            self / len
        } else {
            fallback
        }
    }

    /// Component-wise product.
    #[must_use]
    pub fn mul_elem(self, o: Self) -> Self {
        Self::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }

    /// Component-wise minimum.
    #[must_use]
    pub fn min(self, o: Self) -> Self {
        Self::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    /// Component-wise maximum.
    #[must_use]
    pub fn max(self, o: Self) -> Self {
        Self::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    /// Linear interpolation from `self` (t = 0) to `o` (t = 1).
    #[must_use]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }

    /// True when every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Two unit vectors perpendicular to `self` (assumed unit length) and to each other.
    #[must_use]
    pub fn basis(self) -> (Self, Self) {
        let up = if self.y.abs() > 0.99 { Self::X } else { Self::Y };
        let t = up.cross(self).normalize_or(Self::X);
        let b = self.cross(t);
        (t, b)
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}
impl MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}
impl Div<f32> for Vec3 {
    type Output = Self;
    fn div(self, s: f32) -> Self {
        Self::new(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// A rigid transform: position plus an orthonormal rotation basis (right, up, back), like a Roblox CFrame
/// without the scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// World position.
    pub position: Vec3,
    /// Local +X in world space (Roblox `RightVector`).
    pub right: Vec3,
    /// Local +Y in world space (Roblox `UpVector`).
    pub up: Vec3,
    /// Local +Z in world space (Roblox `-LookVector`).
    pub back: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The identity transform at the origin.
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        right: Vec3::X,
        up: Vec3::Y,
        back: Vec3::Z,
    };

    /// Identity rotation at `position`.
    #[must_use]
    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            ..Self::IDENTITY
        }
    }

    /// Rotation of `angle` radians about the world Y axis, at `position`.
    #[must_use]
    pub fn from_position_yaw(position: Vec3, angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            position,
            right: Vec3::new(c, 0.0, -s),
            up: Vec3::Y,
            back: Vec3::new(s, 0.0, c),
        }
    }

    /// Rotates a local direction into world space.
    #[must_use]
    pub fn rotate(&self, v: Vec3) -> Vec3 {
        self.right * v.x + self.up * v.y + self.back * v.z
    }

    /// Maps a local point into world space.
    #[must_use]
    pub fn point(&self, v: Vec3) -> Vec3 {
        self.position + self.rotate(v)
    }

    /// Interpolates position linearly and the basis by normalised lerp (good enough between two frames).
    #[must_use]
    pub fn lerp(&self, o: &Self, t: f32) -> Self {
        let right = self.right.lerp(o.right, t).normalize_or(Vec3::X);
        let up0 = self.up.lerp(o.up, t);
        let back = right.cross(up0).normalize_or(Vec3::Z);
        let up = back.cross(right);
        Self {
            position: self.position.lerp(o.position, t),
            right,
            up,
            back,
        }
    }
}

/// Smooth Hermite step between `e0` and `e1`.
#[must_use]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear interpolation of scalars.
#[must_use]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_is_orthonormal() {
        for d in [Vec3::X, Vec3::Y, -Vec3::Y, Vec3::new(1.0, 2.0, 3.0).normalize_or(Vec3::Y)] {
            let (t, b) = d.basis();
            assert!(t.dot(d).abs() < 1e-5);
            assert!(b.dot(d).abs() < 1e-5);
            assert!(t.dot(b).abs() < 1e-5);
            assert!((t.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn yaw_transform_rotates_right_vector() {
        let t = Transform::from_position_yaw(Vec3::ZERO, std::f32::consts::FRAC_PI_2);
        let r = t.rotate(Vec3::X);
        assert!((r - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
    }
}
