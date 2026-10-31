//! Values that can be animated: the [`Animatable`] trait and the [`Color3`] type.

use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt::Debug;

/// An RGB colour with components in `[0, 1]` (values outside are allowed while animating,
/// for example with `Back` easing). Mirrors Roblox's `Color3`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Color3 {
    /// Red, `0..=1`.
    pub r: f32,
    /// Green, `0..=1`.
    pub g: f32,
    /// Blue, `0..=1`.
    pub b: f32,
}

impl Color3 {
    /// A colour from components in `[0, 1]`.
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// A colour from 8-bit components (Roblox's `Color3.fromRGB`).
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        )
    }

    /// Components clamped to `[0, 1]` and converted to 8-bit.
    pub fn to_rgb(self) -> [u8; 3] {
        let q = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b)]
    }

    fn map2(a: Self, b: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        Self::new(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b))
    }
}

/// A value type that keyframes, tweens and blends can work with.
///
/// The three operations define a small vector-space-like structure:
/// [`interpolate`](Animatable::interpolate) moves between two values, [`delta`](Animatable::delta)
/// takes the difference `a - from`, and [`offset`](Animatable::offset) applies `base + delta * scale`.
/// Bezier handles, auto tangents and additive blending are all built from these.
///
/// Rotations ([`Quat`]) use the rotation group instead: the delta is the relative rotation,
/// scaling a delta scales its angle, and interpolation is a shortest-path slerp.
/// Discrete values (`bool`) step: they hold the first value until `t` reaches `1`.
pub trait Animatable: Copy + PartialEq + Debug + Serialize + DeserializeOwned + 'static {
    /// Blend from `a` (`t = 0`) to `b` (`t = 1`). `t` may leave `[0, 1]` (overshoot).
    fn interpolate(a: Self, b: Self, t: f32) -> Self;
    /// The change that takes `from` to `a` (`a - from`).
    fn delta(a: Self, from: Self) -> Self;
    /// `base + delta * scale`.
    fn offset(base: Self, delta: Self, scale: f32) -> Self;
    /// The "no change" delta (zero vector, identity rotation).
    fn zero_delta() -> Self;
    /// The cubic Bezier through control points `p0..p3` at parameter `s`. The default is de
    /// Casteljau with [`interpolate`](Animatable::interpolate) (exact for vectors); rotations
    /// override it with a weighted sum in quaternion space and one normalise, which is far
    /// cheaper than six slerps and indistinguishable for the short arcs between keys.
    fn bezier(p0: Self, p1: Self, p2: Self, p3: Self, s: f32) -> Self {
        let q0 = Self::interpolate(p0, p1, s);
        let q1 = Self::interpolate(p1, p2, s);
        let q2 = Self::interpolate(p2, p3, s);
        let r0 = Self::interpolate(q0, q1, s);
        let r1 = Self::interpolate(q1, q2, s);
        Self::interpolate(r0, r1, s)
    }
    /// True for step-only values such as `bool`. Editors draw these as steps and hide curves.
    fn is_discrete() -> bool {
        false
    }
    /// For auto-clamped tangents: true when `cur` is a local extreme between `prev` and `next`,
    /// so its tangent should be flat to avoid overshoot. Only scalars can tell; others say no.
    fn is_extreme(_prev: Self, _cur: Self, _next: Self) -> bool {
        false
    }
}

impl Animatable for f32 {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }
    fn delta(a: Self, from: Self) -> Self {
        a - from
    }
    fn offset(base: Self, delta: Self, scale: f32) -> Self {
        base + delta * scale
    }
    fn zero_delta() -> Self {
        0.0
    }
    fn is_extreme(prev: Self, cur: Self, next: Self) -> bool {
        (cur >= prev && cur >= next) || (cur <= prev && cur <= next)
    }
}

impl Animatable for Vec2 {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        a.lerp(b, t)
    }
    fn delta(a: Self, from: Self) -> Self {
        a - from
    }
    fn offset(base: Self, delta: Self, scale: f32) -> Self {
        base + delta * scale
    }
    fn zero_delta() -> Self {
        Vec2::ZERO
    }
}

impl Animatable for Vec3 {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        a.lerp(b, t)
    }
    fn delta(a: Self, from: Self) -> Self {
        a - from
    }
    fn offset(base: Self, delta: Self, scale: f32) -> Self {
        base + delta * scale
    }
    fn zero_delta() -> Self {
        Vec3::ZERO
    }
}

impl Animatable for Color3 {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        Color3::map2(a, b, |x, y| x + (y - x) * t)
    }
    fn delta(a: Self, from: Self) -> Self {
        Color3::map2(a, from, |x, y| x - y)
    }
    fn offset(base: Self, delta: Self, scale: f32) -> Self {
        Color3::map2(base, delta, |x, d| x + d * scale)
    }
    fn zero_delta() -> Self {
        Color3::default()
    }
}

impl Animatable for Quat {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        // glam's slerp takes the shortest arc and extrapolates correctly for t outside [0, 1].
        a.slerp(b, t).normalize()
    }
    fn delta(a: Self, from: Self) -> Self {
        (from.inverse() * a).normalize()
    }
    fn offset(base: Self, delta: Self, scale: f32) -> Self {
        // Fast paths for Bezier handle points (scale +1 / -1): no trigonometry.
        if scale == 1.0 {
            return (base * delta).normalize();
        }
        if scale == -1.0 {
            return (base * delta.conjugate()).normalize();
        }
        (base * quat_pow(delta, scale)).normalize()
    }
    fn bezier(p0: Self, p1: Self, p2: Self, p3: Self, s: f32) -> Self {
        // Keep every control point in the same hemisphere as its predecessor.
        let p1 = if p0.dot(p1) < 0.0 { -p1 } else { p1 };
        let p2 = if p1.dot(p2) < 0.0 { -p2 } else { p2 };
        let p3 = if p2.dot(p3) < 0.0 { -p3 } else { p3 };
        let u = 1.0 - s;
        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s);
        (p0 * b0 + p1 * b1 + p2 * b2 + p3 * b3).normalize()
    }
    fn zero_delta() -> Self {
        Quat::IDENTITY
    }
}

impl Animatable for bool {
    fn interpolate(a: Self, b: Self, t: f32) -> Self {
        if t >= 1.0 { b } else { a }
    }
    fn delta(a: Self, _from: Self) -> Self {
        a
    }
    fn offset(base: Self, _delta: Self, _scale: f32) -> Self {
        base
    }
    fn zero_delta() -> Self {
        false
    }
    fn is_discrete() -> bool {
        true
    }
}

/// `q` raised to a real power: the same axis, `scale` times the angle. Takes the short way round.
pub fn quat_pow(q: Quat, scale: f32) -> Quat {
    let q = if q.w < 0.0 { -q } else { q };
    let (axis, angle) = q.to_axis_angle();
    if angle.abs() < 1e-7 || !axis.is_finite() {
        return Quat::IDENTITY;
    }
    Quat::from_axis_angle(axis, angle * scale)
}
