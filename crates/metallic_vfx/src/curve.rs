//! Value types for effect properties: [`Range`] (Roblox `NumberRange`), [`Curve`] (Roblox
//! `NumberSequence`, plus smooth and stepped keys) and [`Gradient`] (Roblox `ColorSequence`).
//!
//! These are deliberately small. When `metallic_anim` (#40) lands, `Curve` should become a thin wrapper
//! over its curve type so the VFX editor and the animation timeline share one curve widget and one file
//! shape; the sampling rules below are written to make that merge mechanical.

use crate::color::Rgb;
use serde::{Deserialize, Serialize};

/// A closed interval `[min, max]` sampled uniformly per particle (Roblox `NumberRange`). Written `(min, max)`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "(f32, f32)", into = "(f32, f32)")]
pub struct Range {
    /// Lower bound.
    pub min: f32,
    /// Upper bound.
    pub max: f32,
}

impl From<(f32, f32)> for Range {
    fn from(v: (f32, f32)) -> Self {
        Self { min: v.0, max: v.1 }
    }
}
impl From<Range> for (f32, f32) {
    fn from(r: Range) -> Self {
        (r.min, r.max)
    }
}

impl Range {
    /// A range from `min` to `max`.
    #[must_use]
    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// A single value.
    #[must_use]
    pub const fn constant(v: f32) -> Self {
        Self { min: v, max: v }
    }

    /// Value at `t` in `0..=1` (`t` usually comes from the particle's random stream).
    #[must_use]
    pub fn at(&self, t: f32) -> f32 {
        self.min + (self.max - self.min) * t
    }

    /// True when both ends are finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }
}

/// How a [`Curve`] or [`Gradient`] moves between keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Interp {
    /// Straight lines between keys (Roblox sequences).
    #[default]
    Linear,
    /// Monotone cubic: smooth, and never overshoots the keys (sizes never go negative by accident).
    Smooth,
    /// Holds each key's value until the next key: the toon and pixel "on twos" look.
    Step,
}

fn is_linear(i: &Interp) -> bool {
    *i == Interp::Linear
}

/// One key of a [`Curve`]. Written `(t, v)`, or `(t, v, env)` when it has an envelope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    /// Normalised time, `0..=1` (fraction of the particle's life, or of a trail's length).
    pub t: f32,
    /// Value at `t`.
    pub v: f32,
    /// Random variation `±env` per particle (Roblox keypoint envelope).
    pub env: f32,
}

impl Serialize for Key {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let n = if self.env == 0.0 { 2 } else { 3 };
        let mut t = s.serialize_tuple(n)?;
        t.serialize_element(&self.t)?;
        t.serialize_element(&self.v)?;
        if n == 3 {
            t.serialize_element(&self.env)?;
        }
        t.end()
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Key;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a key `(t, v)` or `(t, v, envelope)`")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Key, A::Error> {
                let t = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let v = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let env = seq.next_element()?.unwrap_or(0.0);
                Ok(Key { t, v, env })
            }
        }
        d.deserialize_tuple(3, V)
    }
}

/// A scalar function of normalised time, built from keys. Like Roblox `NumberSequence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// Keys sorted by `t`. Empty means a constant 0; one key means a constant.
    pub keys: Vec<Key>,
    /// Interpolation between keys.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub interp: Interp,
}

impl Default for Curve {
    fn default() -> Self {
        Self::constant(1.0)
    }
}

impl Curve {
    /// A constant curve.
    #[must_use]
    pub fn constant(v: f32) -> Self {
        Self {
            keys: vec![Key { t: 0.0, v, env: 0.0 }],
            interp: Interp::Linear,
        }
    }

    /// A curve through `(t, v)` points with linear interpolation.
    #[must_use]
    pub fn linear(points: &[(f32, f32)]) -> Self {
        Self::from_points(points, Interp::Linear)
    }

    /// A curve through `(t, v)` points with monotone smooth interpolation.
    #[must_use]
    pub fn smooth(points: &[(f32, f32)]) -> Self {
        Self::from_points(points, Interp::Smooth)
    }

    /// A curve through `(t, v)` points that holds values (stepped).
    #[must_use]
    pub fn stepped(points: &[(f32, f32)]) -> Self {
        Self::from_points(points, Interp::Step)
    }

    /// A curve through `(t, v)` points with the given interpolation. Points are sorted by `t`.
    #[must_use]
    pub fn from_points(points: &[(f32, f32)], interp: Interp) -> Self {
        let mut keys: Vec<Key> = points
            .iter()
            .map(|&(t, v)| Key { t, v, env: 0.0 })
            .collect();
        keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self { keys, interp }
    }

    /// Sets the same envelope on every key (Roblox keypoint envelope).
    #[must_use]
    pub fn with_envelope(mut self, env: f32) -> Self {
        for k in &mut self.keys {
            k.env = env;
        }
        self
    }

    /// Sorts keys by time; call after editing keys by hand.
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    }

    /// Value at normalised time `t` (clamped to `0..=1`), without envelope.
    #[must_use]
    pub fn sample(&self, t: f32) -> f32 {
        self.sample_with(t, 0.0)
    }

    /// Value at `t`, offset by `noise * envelope` where `noise` is the particle's random value in `-1..=1`.
    #[must_use]
    pub fn sample_with(&self, t: f32, noise: f32) -> f32 {
        let keys = &self.keys;
        let Some(first) = keys.first() else {
            return 0.0;
        };
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        if keys.len() == 1 || t <= first.t {
            return first.v + first.env * noise;
        }
        let last = keys[keys.len() - 1];
        if t >= last.t {
            return last.v + last.env * noise;
        }
        // Few keys in practice: a linear scan beats a binary search here.
        let mut i = 0;
        while i + 2 < keys.len() && keys[i + 1].t <= t {
            i += 1;
        }
        let (a, b) = (keys[i], keys[i + 1]);
        let span = b.t - a.t;
        let u = if span > 0.0 { (t - a.t) / span } else { 1.0 };
        let env = a.env + (b.env - a.env) * u;
        let v = match self.interp {
            Interp::Linear => a.v + (b.v - a.v) * u,
            Interp::Step => {
                if u >= 1.0 {
                    b.v
                } else {
                    a.v
                }
            }
            Interp::Smooth => {
                let m0 = self.tangent(i);
                let m1 = self.tangent(i + 1);
                hermite(a.v, b.v, m0 * span, m1 * span, u)
            }
        };
        v + env * noise
    }

    /// Fritsch-Carlson monotone tangent at key `i` (slope per unit `t`).
    fn tangent(&self, i: usize) -> f32 {
        let k = &self.keys;
        let slope = |a: usize, b: usize| {
            let dt = k[b].t - k[a].t;
            if dt > 0.0 { (k[b].v - k[a].v) / dt } else { 0.0 }
        };
        if i == 0 {
            return slope(0, 1);
        }
        if i == k.len() - 1 {
            return slope(i - 1, i);
        }
        let d0 = slope(i - 1, i);
        let d1 = slope(i, i + 1);
        if d0 * d1 <= 0.0 {
            0.0
        } else {
            // Harmonic mean keeps the cubic inside the key values.
            2.0 * d0 * d1 / (d0 + d1)
        }
    }

    /// Largest value the curve can reach, including envelopes (used for bounds).
    #[must_use]
    pub fn max_value(&self) -> f32 {
        if self.keys.is_empty() {
            return 0.0;
        }
        self.keys
            .iter()
            .map(|k| k.v + k.env.abs())
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// True when all keys are finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.keys
            .iter()
            .all(|k| k.t.is_finite() && k.v.is_finite() && k.env.is_finite())
    }
}

fn hermite(p0: f32, p1: f32, m0: f32, m1: f32, u: f32) -> f32 {
    let u2 = u * u;
    let u3 = u2 * u;
    (2.0 * u3 - 3.0 * u2 + 1.0) * p0
        + (u3 - 2.0 * u2 + u) * m0
        + (-2.0 * u3 + 3.0 * u2) * p1
        + (u3 - u2) * m1
}

/// Colour space used to blend [`Gradient`] keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendSpace {
    /// Plain sRGB lerp (Roblox `ColorSequence`).
    #[default]
    Srgb,
    /// Oklab: perceptually even, keeps fire gradients from turning muddy.
    Oklab,
}

fn is_srgb(b: &BlendSpace) -> bool {
    *b == BlendSpace::Srgb
}

/// One key of a [`Gradient`]. Written `(t, "#RRGGBB")`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "(f32, Rgb)", into = "(f32, Rgb)")]
pub struct ColorKey {
    /// Normalised time, `0..=1`.
    pub t: f32,
    /// Colour at `t`.
    pub c: Rgb,
}

/// A colour function of normalised time (Roblox `ColorSequence`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    /// Keys sorted by `t`.
    pub keys: Vec<ColorKey>,
    /// How keys blend.
    #[serde(default, skip_serializing_if = "is_srgb")]
    pub space: BlendSpace,
    /// Linear blending or hard bands (toon colour steps).
    #[serde(default, skip_serializing_if = "is_linear")]
    pub interp: Interp,
}

impl From<(f32, Rgb)> for ColorKey {
    fn from(v: (f32, Rgb)) -> Self {
        Self { t: v.0, c: v.1 }
    }
}
impl From<ColorKey> for (f32, Rgb) {
    fn from(k: ColorKey) -> Self {
        (k.t, k.c)
    }
}

impl Default for Gradient {
    fn default() -> Self {
        Self::constant(Rgb::WHITE)
    }
}

impl Gradient {
    /// A single colour.
    #[must_use]
    pub fn constant(c: Rgb) -> Self {
        Self {
            keys: vec![ColorKey { t: 0.0, c }],
            space: BlendSpace::Srgb,
            interp: Interp::Linear,
        }
    }

    /// Keys from `(t, 0xRRGGBB)` pairs, blended in Oklab.
    #[must_use]
    pub fn hex(points: &[(f32, u32)]) -> Self {
        let mut keys: Vec<ColorKey> = points
            .iter()
            .map(|&(t, c)| ColorKey { t, c: Rgb::hex(c) })
            .collect();
        keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self {
            keys,
            space: BlendSpace::Oklab,
            interp: Interp::Linear,
        }
    }

    /// Same gradient with stepped (banded) interpolation.
    #[must_use]
    pub fn stepped(mut self) -> Self {
        self.interp = Interp::Step;
        self
    }

    /// Sorts keys by time.
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    }

    /// Colour at normalised time `t` (clamped to `0..=1`).
    #[must_use]
    pub fn sample(&self, t: f32) -> Rgb {
        let keys = &self.keys;
        let Some(first) = keys.first() else {
            return Rgb::WHITE;
        };
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        if keys.len() == 1 || t <= first.t {
            return first.c;
        }
        let last = keys[keys.len() - 1];
        if t >= last.t {
            return last.c;
        }
        let mut i = 0;
        while i + 2 < keys.len() && keys[i + 1].t <= t {
            i += 1;
        }
        let (a, b) = (keys[i], keys[i + 1]);
        let span = b.t - a.t;
        let u = if span > 0.0 { (t - a.t) / span } else { 1.0 };
        match (self.interp, self.space) {
            (Interp::Step, _) => {
                if u >= 1.0 {
                    b.c
                } else {
                    a.c
                }
            }
            (_, BlendSpace::Srgb) => a.c.lerp(b.c, u),
            (_, BlendSpace::Oklab) => a.c.lerp_oklab(b.c, u),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_curve_samples_between_keys() {
        let c = Curve::linear(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]);
        assert_eq!(c.sample(0.0), 0.0);
        assert!((c.sample(0.25) - 0.5).abs() < 1e-6);
        assert_eq!(c.sample(0.5), 1.0);
        assert!((c.sample(0.75) - 0.5).abs() < 1e-6);
        assert_eq!(c.sample(2.0), 0.0);
        assert_eq!(c.sample(-1.0), 0.0);
        assert_eq!(c.sample(f32::NAN), 0.0);
    }

    #[test]
    fn smooth_curve_never_overshoots() {
        let c = Curve::smooth(&[(0.0, 0.0), (0.2, 1.0), (0.4, 1.0), (1.0, 0.0)]);
        for i in 0..=1000 {
            let v = c.sample(i as f32 / 1000.0);
            assert!((-1e-5..=1.0 + 1e-5).contains(&v), "overshoot {v}");
        }
        assert!((c.sample(0.2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn step_curve_holds_values() {
        let c = Curve::stepped(&[(0.0, 1.0), (0.5, 2.0), (1.0, 3.0)]);
        assert_eq!(c.sample(0.49), 1.0);
        assert_eq!(c.sample(0.5), 2.0);
        assert_eq!(c.sample(0.99), 2.0);
        assert_eq!(c.sample(1.0), 3.0);
    }

    #[test]
    fn envelope_offsets_values() {
        let c = Curve::constant(2.0).with_envelope(0.5);
        assert_eq!(c.sample_with(0.3, 1.0), 2.5);
        assert_eq!(c.sample_with(0.3, -1.0), 1.5);
        assert_eq!(c.max_value(), 2.5);
    }

    #[test]
    fn empty_curve_and_gradient_have_safe_defaults() {
        let c = Curve {
            keys: vec![],
            interp: Interp::Linear,
        };
        assert_eq!(c.sample(0.5), 0.0);
        let g = Gradient {
            keys: vec![],
            space: BlendSpace::Srgb,
            interp: Interp::Linear,
        };
        assert_eq!(g.sample(0.5), Rgb::WHITE);
    }

    #[test]
    fn gradient_blends_and_steps() {
        let g = Gradient {
            keys: vec![
                ColorKey {
                    t: 0.0,
                    c: Rgb::BLACK,
                },
                ColorKey {
                    t: 1.0,
                    c: Rgb::WHITE,
                },
            ],
            space: BlendSpace::Srgb,
            interp: Interp::Linear,
        };
        assert!((g.sample(0.5).r - 0.5).abs() < 1e-6);
        assert_eq!(g.clone().stepped().sample(0.5), Rgb::BLACK);
    }
}
