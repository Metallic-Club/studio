//! Spatial audio math: distance roll-off, stereo panning, Doppler, occlusion and the rear cue.
//!
//! All functions are pure so they can be tested against closed formulas. The mixer calls
//! [`spatialize`] once per sound per mix block (64 frames) and ramps gains across the block.
//!
//! Panning is equal-power stereo (assistant's call, see `docs/agents/81-audio.md`): cheap,
//! predictable on speakers and headphones, and the same code path later serves proximity voice.
//! An HRTF renderer (for example Steam Audio, Apache-2.0) can replace [`spatialize`] behind the
//! same inputs.

use crate::types::{Emitter, Listener, RollOff, RollOffMode, SpatialSettings};
use glam::Vec3;

/// Speed of sound in metres per second (dry air, 20 degrees C).
pub const SPEED_OF_SOUND_MPS: f32 = 343.0;
/// Doppler pitch factors are clamped to this range so extreme velocities cannot alias.
pub const DOPPLER_RANGE: (f32, f32) = (0.25, 4.0);
/// Cutoff (Hz) treated as "no filtering".
pub const OPEN_CUTOFF_HZ: f32 = 20_000.0;

/// Gain (0..=1) of a sound `distance` studs away for a roll-off curve.
///
/// Every mode returns 1 for `distance <= min` and 0 for `distance >= max`. Between:
/// - Inverse: `min / distance`
/// - Linear: `(max - distance) / (max - min)`
/// - LinearSquare: Linear squared
/// - InverseTapered: `min(Inverse, LinearSquare)`
///
/// Negative distances and a negative `min` are treated as 0; `max < min` is raised to `min`.
/// A non-finite distance returns 0.
pub fn roll_off_gain(
    mode: RollOffMode,
    distance: f32,
    min_distance: f32,
    max_distance: f32,
) -> f32 {
    if !distance.is_finite() {
        return 0.0;
    }
    let distance = distance.max(0.0);
    let min = if min_distance.is_finite() {
        min_distance.max(0.0)
    } else {
        0.0
    };
    let max = if max_distance.is_finite() {
        max_distance.max(min)
    } else {
        f32::MAX
    };
    if distance <= min {
        return 1.0;
    }
    if distance >= max {
        return 0.0;
    }
    // Here min < distance < max, so max - min > 0 and distance > 0.
    let inverse = min / distance;
    let linear = (max - distance) / (max - min);
    match mode {
        RollOffMode::Inverse => inverse,
        RollOffMode::Linear => linear,
        RollOffMode::LinearSquare => linear * linear,
        RollOffMode::InverseTapered => inverse.min(linear * linear),
    }
}

/// Equal-power pan law: `pan` -1 is hard left, 0 centre, 1 hard right. Returns `(left, right)`
/// gains with `left^2 + right^2 == 1`; the centre is `sqrt(1/2)` (-3 dB) on each side.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// `world` expressed in the listener's frame: x = right, y = up, z = forward.
pub fn listener_local(listener: &Listener, world: Vec3) -> Vec3 {
    let forward = listener.forward.try_normalize().unwrap_or(Vec3::NEG_Z);
    // Gram-Schmidt so an `up` that is not quite orthogonal still gives a clean basis.
    let up = (listener.up - forward * listener.up.dot(forward))
        .try_normalize()
        .unwrap_or_else(|| {
            // `up` parallel to `forward`: pick any perpendicular.
            forward.any_orthonormal_vector()
        });
    let right = forward.cross(up);
    let d = world - listener.position;
    Vec3::new(d.dot(right), d.dot(up), d.dot(forward))
}

/// Doppler pitch factor (> 1 when source and listener approach each other).
///
/// `f' / f = (c - v_l) / (c - v_s)` with `u` the unit vector from source to listener, `v_s` and
/// `v_l` the source and listener velocities projected on `u` and scaled by `doppler_scale`, and
/// `c` the speed of sound in studs per second. Clamped to [`DOPPLER_RANGE`].
pub fn doppler_factor(listener: &Listener, emitter: &Emitter, settings: &SpatialSettings) -> f32 {
    let scale = settings.doppler_scale;
    if scale <= 0.0 || !scale.is_finite() {
        return 1.0;
    }
    let Some(u) = (listener.position - emitter.position).try_normalize() else {
        return 1.0;
    };
    let c = SPEED_OF_SOUND_MPS * settings.distance_factor.max(1e-3);
    let v_s = emitter.velocity.dot(u) * scale;
    let v_l = listener.velocity.dot(u) * scale;
    let denom = (c - v_s).max(c * 0.05);
    let factor = (c - v_l) / denom;
    if factor.is_finite() {
        factor.clamp(DOPPLER_RANGE.0, DOPPLER_RANGE.1)
    } else {
        1.0
    }
}

/// Result of spatializing one sound for one mix block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatialized {
    /// Distance roll-off times occlusion gain (0..=1), before panning.
    pub gain: f32,
    /// Final left gain: `gain * pan_left`.
    pub left: f32,
    /// Final right gain: `gain * pan_right`.
    pub right: f32,
    /// Pitch factor from Doppler.
    pub doppler: f32,
    /// Low-pass cutoff in Hz from occlusion and the rear cue ([`OPEN_CUTOFF_HZ`] = none).
    pub cutoff_hz: f32,
}

/// Everything about one positional sound relative to the listener.
///
/// `occlusion` is 0 (clear line of sight) to 1 (fully blocked), as answered by the runtime's
/// occlusion hook. Occlusion scales the gain towards `settings.occlusion_gain` and lowers the
/// cutoff towards `settings.occlusion_cutoff_hz`; a sound behind the listener gets a cutoff
/// towards `settings.rear_cutoff_hz`. Cutoffs interpolate on a log-frequency scale.
pub fn spatialize(
    listener: &Listener,
    emitter: &Emitter,
    roll_off: RollOff,
    settings: &SpatialSettings,
    occlusion: f32,
) -> Spatialized {
    let local = listener_local(listener, emitter.position);
    let distance = local.length();
    let scale = if settings.rolloff_scale.is_finite() {
        settings.rolloff_scale.max(0.0)
    } else {
        1.0
    };
    let occlusion = if occlusion.is_finite() {
        occlusion.clamp(0.0, 1.0)
    } else {
        0.0
    };

    let distance_gain = roll_off_gain(
        roll_off.mode,
        distance * scale,
        roll_off.min_distance,
        roll_off.max_distance,
    );
    let occlusion_gain = 1.0 - occlusion * (1.0 - settings.occlusion_gain.clamp(0.0, 1.0));
    let gain = distance_gain * occlusion_gain;

    let (pan, behind) = if distance > 1e-4 {
        (local.x / distance, (-local.z / distance).max(0.0))
    } else {
        (0.0, 0.0)
    };
    let (pan_left, pan_right) = pan_gains(pan);

    let rear = log_lerp(OPEN_CUTOFF_HZ, settings.rear_cutoff_hz, behind);
    let occluded = log_lerp(OPEN_CUTOFF_HZ, settings.occlusion_cutoff_hz, occlusion);

    Spatialized {
        gain,
        left: gain * pan_left,
        right: gain * pan_right,
        doppler: doppler_factor(listener, emitter, settings),
        cutoff_hz: rear.min(occluded),
    }
}

/// Interpolate between two frequencies on a log scale (`t` 0 gives `from`, 1 gives `to`).
fn log_lerp(from: f32, to: f32, t: f32) -> f32 {
    let to = if to.is_finite() {
        to.clamp(20.0, OPEN_CUTOFF_HZ)
    } else {
        OPEN_CUTOFF_HZ
    };
    if t <= 0.0 || t.is_nan() {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    (from.ln() + (to.ln() - from.ln()) * t).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lerp_endpoints() {
        assert!((log_lerp(20_000.0, 1_000.0, 0.0) - 20_000.0).abs() < 1.0);
        assert!((log_lerp(20_000.0, 1_000.0, 1.0) - 1_000.0).abs() < 0.1);
        let mid = log_lerp(20_000.0, 2_000.0, 0.5);
        assert!((mid - (20_000.0f32 * 2_000.0).sqrt()).abs() < 1.0);
    }
}
