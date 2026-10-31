//! The small 2D maths types used at the crate boundary.
//!
//! The backend (Rapier) works in `f32`; callers get `f64` like the rest of the engine, so the
//! backend can be swapped without touching game code.
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// A 2D vector or point in studs (x to the right, y up).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f64,
    /// Vertical component (up is positive).
    pub y: f64,
}

impl Vec2 {
    /// The zero vector.
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    /// Unit vector pointing up (+y).
    pub const UP: Vec2 = Vec2 { x: 0.0, y: 1.0 };

    /// Creates a vector from its components.
    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    /// Length of the vector.
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// Dot product.
    pub fn dot(self, other: Vec2) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The vector scaled to length 1, or `None` when it is (nearly) zero or not finite.
    pub fn normalized(self) -> Option<Vec2> {
        let len = self.length();
        if len > 1e-12 && len.is_finite() {
            Some(self * (1.0 / len))
        } else {
            None
        }
    }

    /// True when both components are finite (not NaN or infinite).
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Rotates the vector counter-clockwise by `angle` radians.
    pub fn rotated(self, angle: f64) -> Vec2 {
        let (s, c) = angle.sin_cos();
        Vec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    /// Linear interpolation: `self` at `t = 0`, `other` at `t = 1`.
    pub fn lerp(self, other: Vec2, t: f64) -> Vec2 {
        self + (other - self) * t
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Position and rotation of a body: `angle` is counter-clockwise in radians.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose2 {
    /// World position of the body origin in studs.
    pub position: Vec2,
    /// Counter-clockwise rotation in radians.
    pub angle: f64,
}

impl Pose2 {
    /// Creates a pose.
    pub const fn new(position: Vec2, angle: f64) -> Self {
        Pose2 { position, angle }
    }

    /// Transforms a point from body space to world space.
    pub fn transform_point(self, local: Vec2) -> Vec2 {
        self.position + local.rotated(self.angle)
    }

    /// Interpolates position linearly and angle along the shortest arc.
    pub fn lerp(self, other: Pose2, t: f64) -> Pose2 {
        let mut d = (other.angle - self.angle) % std::f64::consts::TAU;
        if d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        } else if d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        Pose2::new(self.position.lerp(other.position, t), self.angle + d * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_quarter_turn() {
        let v = Vec2::new(1.0, 0.0).rotated(std::f64::consts::FRAC_PI_2);
        assert!((v.x).abs() < 1e-12 && (v.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn pose_lerp_takes_short_arc() {
        let a = Pose2::new(Vec2::ZERO, 3.0);
        let b = Pose2::new(Vec2::new(2.0, 0.0), -3.0);
        let m = a.lerp(b, 0.5);
        assert!((m.position.x - 1.0).abs() < 1e-12);
        // Halfway along the short arc through pi, not through 0.
        assert!(m.angle.abs() > 3.0);
    }

    #[test]
    fn normalized_rejects_zero_and_nan() {
        assert!(Vec2::ZERO.normalized().is_none());
        assert!(Vec2::new(f64::NAN, 1.0).normalized().is_none());
        assert!(Vec2::new(3.0, 4.0).normalized().is_some());
    }
}
