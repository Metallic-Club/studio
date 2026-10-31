//! The Collisions chip (building-tools spec 3.1): with it on, a drag stops where a moved part
//! would overlap a part that is not moving.
//!
//! Parts are oriented boxes; two boxes overlap when no separating axis exists (the 15-axis test
//! for boxes). Boxes that only touch do not count: each box is shrunk by [`TOUCH`] first, so a part
//! can still sit flush on another one.

use std::collections::HashSet;

use metallic_core::InstanceId;
use metallic_render::glam::{DMat3, DVec3};

use crate::picking::PickCache;
use crate::transform::PartTransform;

/// How far boxes may sink into each other (per side, studs) and still count as touching.
pub const TOUCH: f64 = 0.01;

/// Most parts tested per moved part; beyond this the drag is not held back (keeps a frame cheap).
pub const MAX_TESTS: usize = 20_000;

/// True when two oriented boxes overlap by more than [`TOUCH`].
pub fn boxes_overlap(a: &PartTransform, b: &PartTransform) -> bool {
    let ha = (a.size * 0.5 - DVec3::splat(TOUCH)).max(DVec3::ZERO);
    let hb = (b.size * 0.5 - DVec3::splat(TOUCH)).max(DVec3::ZERO);
    let ra = DMat3::from_quat(a.rotation);
    let rb = DMat3::from_quat(b.rotation);
    let axes_a = [ra.x_axis, ra.y_axis, ra.z_axis];
    let axes_b = [rb.x_axis, rb.y_axis, rb.z_axis];
    let d = b.position - a.position;
    let separated = |axis: DVec3| {
        let len = axis.length_squared();
        if len < 1e-12 {
            return false; // parallel edges: covered by the face axes
        }
        let pa: f64 = (0..3).map(|i| ha[i] * axes_a[i].dot(axis).abs()).sum();
        let pb: f64 = (0..3).map(|i| hb[i] * axes_b[i].dot(axis).abs()).sum();
        d.dot(axis).abs() >= pa + pb
    };
    for axis in axes_a.iter().chain(axes_b.iter()) {
        if separated(*axis) {
            return false;
        }
    }
    for x in &axes_a {
        for y in &axes_b {
            if separated(x.cross(*y)) {
                return false;
            }
        }
    }
    true
}

/// True when any of `moved` overlaps a part in `picks` that is not in `moving`.
pub fn blocked(picks: &PickCache, moved: &[PartTransform], moving: &HashSet<InstanceId>) -> bool {
    let mut tests = 0usize;
    for (id, other, _) in picks.iter() {
        if moving.contains(&id) {
            continue;
        }
        for m in moved {
            tests += 1;
            if tests > MAX_TESTS {
                return false;
            }
            // A cheap sphere test first: most parts are far away.
            let reach = (m.size.length() + other.size.length()) * 0.5;
            if m.position.distance_squared(other.position) > reach * reach {
                continue;
            }
            if boxes_overlap(m, other) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_render::glam::DQuat;

    fn cube(x: f64, y: f64, size: f64, turn: f64) -> PartTransform {
        PartTransform {
            position: DVec3::new(x, y, 0.0),
            rotation: DQuat::from_rotation_y(turn.to_radians()),
            size: DVec3::splat(size),
        }
    }

    #[test]
    fn touching_is_allowed_and_overlap_is_not() {
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(2.0, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(0.0, 2.0, 2.0, 0.0)
        ));
        assert!(boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(1.5, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(5.0, 0.0, 2.0, 0.0)
        ));
    }

    #[test]
    fn turned_boxes_use_their_real_shape() {
        // A cube turned 45 degrees reaches 1.414 along X: it hits a box 2.3 away, an
        // axis-aligned one would not.
        assert!(boxes_overlap(
            &cube(0.0, 0.0, 2.0, 45.0),
            &cube(2.3, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 0.0),
            &cube(2.3, 0.0, 2.0, 0.0)
        ));
        assert!(!boxes_overlap(
            &cube(0.0, 0.0, 2.0, 45.0),
            &cube(2.5, 0.0, 2.0, 0.0)
        ));
    }
}
