//! Easing curves: the Roblox-style `EasingStyle` x `EasingDirection` set.
//!
//! Every style is defined by its *In* form `f(t)` on `t` in `[0, 1]` with `f(0) = 0` and
//! `f(1) = 1`. The other directions are derived from it:
//!
//! - **Out**: `1 - f(1 - t)` (fast start, gentle stop; the usual default for UI motion).
//! - **InOut**: `f(2t) / 2` for the first half, `1 - f(2 - 2t) / 2` for the second, except
//!   `Back` and `Elastic`, which use Penner's own InOut forms (a 1.525 times larger overshoot,
//!   a 1.5 times longer period), as Roblox does.
//!
//! `Back` and `Elastic` overshoot (values leave `[0, 1]`); every other style stays inside.

use serde::{Deserialize, Serialize};

/// Shape of an easing curve. Names and order follow Roblox's `Enum.EasingStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EasingStyle {
    /// Constant speed.
    Linear,
    /// Quarter sine wave; the softest curve.
    Sine,
    /// Goes slightly past the end, then settles (overshoot 1.70158).
    Back,
    /// Square curve `t^2`. Roblox's default.
    #[default]
    Quad,
    /// Fourth power `t^4`.
    Quart,
    /// Fifth power `t^5`.
    Quint,
    /// Bounces off the end like a dropped ball.
    Bounce,
    /// Springs past the end with a decaying wobble.
    Elastic,
    /// Exponential `2^(10t - 10)`; very sharp.
    Exponential,
    /// Quarter circle.
    Circular,
    /// Cube curve `t^3`.
    Cubic,
}

impl EasingStyle {
    /// Every style, in Roblox enum order. Useful for menus.
    pub const ALL: [EasingStyle; 11] = [
        EasingStyle::Linear,
        EasingStyle::Sine,
        EasingStyle::Back,
        EasingStyle::Quad,
        EasingStyle::Quart,
        EasingStyle::Quint,
        EasingStyle::Bounce,
        EasingStyle::Elastic,
        EasingStyle::Exponential,
        EasingStyle::Circular,
        EasingStyle::Cubic,
    ];

    /// Short human name for menus ("Quad", "Back", ...).
    pub fn label(self) -> &'static str {
        match self {
            EasingStyle::Linear => "Linear",
            EasingStyle::Sine => "Sine",
            EasingStyle::Back => "Back",
            EasingStyle::Quad => "Quad",
            EasingStyle::Quart => "Quart",
            EasingStyle::Quint => "Quint",
            EasingStyle::Bounce => "Bounce",
            EasingStyle::Elastic => "Elastic",
            EasingStyle::Exponential => "Exponential",
            EasingStyle::Circular => "Circular",
            EasingStyle::Cubic => "Cubic",
        }
    }

    /// The *In* form of the curve for `t` in `[0, 1]`.
    fn ease_in(self, t: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            EasingStyle::Linear => t,
            EasingStyle::Sine => 1.0 - (t * PI / 2.0).cos(),
            EasingStyle::Back => {
                const S: f32 = 1.701_58;
                t * t * ((S + 1.0) * t - S)
            }
            EasingStyle::Quad => t * t,
            EasingStyle::Cubic => t * t * t,
            EasingStyle::Quart => t * t * t * t,
            EasingStyle::Quint => t * t * t * t * t,
            EasingStyle::Bounce => 1.0 - bounce_out(1.0 - t),
            EasingStyle::Elastic => {
                if t <= 0.0 {
                    0.0
                } else if t >= 1.0 {
                    1.0
                } else {
                    // Period 0.3, amplitude 1.
                    let c = 2.0 * PI / 3.0;
                    -(2.0f32.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * c).sin()
                }
            }
            EasingStyle::Exponential => {
                if t <= 0.0 {
                    0.0
                } else {
                    2.0f32.powf(10.0 * t - 10.0)
                }
            }
            EasingStyle::Circular => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        }
    }
}

/// Which end of the curve gets the easing. Names follow Roblox's `Enum.EasingDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EasingDirection {
    /// Eases at the start: slow, then fast.
    In,
    /// Eases at the end: fast, then slow. Roblox's default.
    #[default]
    Out,
    /// Eases at both ends.
    InOut,
}

impl EasingDirection {
    /// Every direction, for menus.
    pub const ALL: [EasingDirection; 3] = [
        EasingDirection::In,
        EasingDirection::Out,
        EasingDirection::InOut,
    ];

    /// Short human name ("In", "Out", "InOut").
    pub fn label(self) -> &'static str {
        match self {
            EasingDirection::In => "In",
            EasingDirection::Out => "Out",
            EasingDirection::InOut => "InOut",
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984_375
    }
}

/// Eased progress for linear progress `t`. `t` is clamped to `[0, 1]`; the result is exactly
/// `0` at `t = 0` and `1` at `t = 1` for every style and direction.
pub fn ease(style: EasingStyle, direction: EasingDirection, t: f32) -> f32 {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    match direction {
        EasingDirection::In => style.ease_in(t),
        EasingDirection::Out => 1.0 - style.ease_in(1.0 - t),
        EasingDirection::InOut => match style {
            EasingStyle::Back => back_in_out(t),
            EasingStyle::Elastic => elastic_in_out(t),
            _ if t < 0.5 => style.ease_in(2.0 * t) / 2.0,
            _ => 1.0 - style.ease_in(2.0 - 2.0 * t) / 2.0,
        },
    }
}

/// Penner's Back InOut: the overshoot is 1.525 times the In/Out one, so the curve is not the In
/// curve mirrored (Roblox matches this).
fn back_in_out(t: f32) -> f32 {
    const S: f32 = 1.701_58 * 1.525;
    let u = 2.0 * t;
    if t < 0.5 {
        u * u * ((S + 1.0) * u - S) / 2.0
    } else {
        let u = u - 2.0;
        (u * u * ((S + 1.0) * u + S) + 2.0) / 2.0
    }
}

/// Penner's Elastic InOut: period 0.45, 1.5 times the In/Out period of 0.3.
fn elastic_in_out(t: f32) -> f32 {
    use std::f32::consts::PI;
    let c = 2.0 * PI / 4.5;
    if t < 0.5 {
        -(2.0f32.powf(20.0 * t - 10.0) * ((20.0 * t - 11.125) * c).sin()) / 2.0
    } else {
        2.0f32.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * c).sin() / 2.0 + 1.0
    }
}
