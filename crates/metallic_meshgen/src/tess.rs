//! Segment counts for curves: a fixed chord error in studs per [`Lod`], clamped to a range per level.
use crate::shape::Lod;
use std::f32::consts::{FRAC_PI_2, TAU};

struct Budget {
    /// Largest allowed gap between the true curve and its chord, in studs.
    chord_error: f32,
    /// Segment range for a full circle.
    circle: (u32, u32),
    /// Segment range for a 90° bevel.
    bevel: (u32, u32),
}

fn budget(lod: Lod) -> Budget {
    match lod {
        Lod::High => Budget {
            chord_error: 0.003,
            circle: (24, 128),
            bevel: (3, 8),
        },
        Lod::Medium => Budget {
            chord_error: 0.012,
            circle: (16, 64),
            bevel: (2, 5),
        },
        Lod::Low => Budget {
            chord_error: 0.05,
            circle: (8, 32),
            bevel: (1, 2),
        },
    }
}

/// Segments needed to follow an arc of `angle` radians and `radius` studs within the chord error.
fn raw_segments(radius: f32, angle: f32, chord_error: f32) -> f32 {
    if radius <= chord_error {
        return 1.0;
    }
    let step = 2.0 * (1.0 - chord_error / radius).clamp(-1.0, 1.0).acos();
    (angle / step.max(1e-4)).ceil()
}

/// Segments for a full circle of `radius` studs; always a multiple of 4 so the four extreme points (and so the
/// exact bounding box) are on the mesh.
pub(crate) fn circle_segments(radius: f32, lod: Lod) -> u32 {
    let b = budget(lod);
    let n = raw_segments(radius, TAU, b.chord_error) as u32;
    let n = n.clamp(b.circle.0, b.circle.1);
    n.div_ceil(4) * 4
}

/// Segments for an arc of `angle` radians (at least 1, at least the circle minimum scaled to the angle).
pub(crate) fn arc_segments(radius: f32, angle: f32, lod: Lod) -> u32 {
    let b = budget(lod);
    let frac = (angle / TAU).clamp(0.0, 1.0);
    let lo = ((b.circle.0 as f32 * frac).ceil() as u32).max(1);
    let hi = ((b.circle.1 as f32 * frac).ceil() as u32).max(lo);
    (raw_segments(radius, angle, b.chord_error) as u32).clamp(lo, hi)
}

/// Segments for a 90° round bevel of `radius` studs.
pub(crate) fn bevel_segments(radius: f32, lod: Lod) -> u32 {
    let b = budget(lod);
    (raw_segments(radius, FRAC_PI_2, b.chord_error) as u32).clamp(b.bevel.0, b.bevel.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circles_grow_with_size_and_lod() {
        assert!(circle_segments(10.0, Lod::High) > circle_segments(0.5, Lod::High));
        assert!(circle_segments(1.0, Lod::High) > circle_segments(1.0, Lod::Low));
        for lod in Lod::ALL {
            for r in [0.001, 0.1, 1.0, 1000.0] {
                assert_eq!(circle_segments(r, lod) % 4, 0);
            }
        }
    }
}
