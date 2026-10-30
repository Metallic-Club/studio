//! Measure tool and distance readouts (spec 10.2, issue #220).
//!
//! - [`MeasureTool`]: Shift+M; click A, click B. Each [`Measurement`] reports the distance in
//!   studs, the X, Y, Z deltas and the angle to the ground. Up to [`MAX_MEASUREMENTS`] stay
//!   on screen; a ninth drops the oldest. Esc clears them. They are never saved.
//! - [`snap_to_vertex`]: the vertex part of snapping for measure clicks, over corners from
//!   [`geom::box_corners`]. The full snapping rules (spec section 5) belong to #215.
//! - The Alt-hover gap readouts use [`geom::axis_gaps`] and [`geom::gap_distance`] on the
//!   selection's and the hovered part's [`geom::Aabb`].
//!
//! [`geom::box_corners`]: super::geom::box_corners
//! [`geom::axis_gaps`]: super::geom::axis_gaps
//! [`geom::gap_distance`]: super::geom::gap_distance
//! [`geom::Aabb`]: super::geom::Aabb

use std::collections::VecDeque;

use glam::DVec3;

/// Most measurements shown at once.
pub const MAX_MEASUREMENTS: usize = 8;

/// A measurement from `a` to `b` in world space (studs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// First point.
    pub a: DVec3,
    /// Second point.
    pub b: DVec3,
}

impl Measurement {
    /// Straight-line distance.
    pub fn distance(&self) -> f64 {
        self.a.distance(self.b)
    }

    /// `b - a` per axis (shown in the axis colours).
    pub fn delta(&self) -> DVec3 {
        self.b - self.a
    }

    /// Distance in the ground (XZ) plane.
    pub fn horizontal(&self) -> f64 {
        let d = self.delta();
        d.x.hypot(d.z)
    }

    /// Angle between the line and the ground plane in degrees, 0 to 90. Zero-length lines
    /// give 0.
    pub fn angle_to_ground_deg(&self) -> f64 {
        let d = self.delta();
        if d.length_squared() == 0.0 {
            return 0.0;
        }
        d.y.abs().atan2(self.horizontal()).to_degrees()
    }
}

/// State of the Measure tool.
#[derive(Debug, Clone, Default)]
pub struct MeasureTool {
    pending: Option<DVec3>,
    list: VecDeque<Measurement>,
}

impl MeasureTool {
    /// A tool with no measurements.
    pub fn new() -> Self {
        Self::default()
    }

    /// A click at a (snapped) world point. The first click sets A; the second completes the
    /// measurement and returns it.
    pub fn click(&mut self, point: DVec3) -> Option<Measurement> {
        match self.pending.take() {
            None => {
                self.pending = Some(point);
                None
            }
            Some(a) => {
                let m = Measurement { a, b: point };
                if self.list.len() == MAX_MEASUREMENTS {
                    self.list.pop_front();
                }
                self.list.push_back(m);
                Some(m)
            }
        }
    }

    /// Point A of a measurement in progress (the viewport draws a rubber band to the cursor).
    pub fn pending(&self) -> Option<DVec3> {
        self.pending
    }

    /// Measurements on screen, oldest first.
    pub fn measurements(&self) -> impl ExactSizeIterator<Item = &Measurement> {
        self.list.iter()
    }

    /// Esc or a tool change: drop everything.
    pub fn clear(&mut self) {
        self.pending = None;
        self.list.clear();
    }
}

/// The vertex nearest to `point` within `max_distance`, if any. Ties keep the first.
pub fn snap_to_vertex(
    point: DVec3,
    vertices: impl IntoIterator<Item = DVec3>,
    max_distance: f64,
) -> Option<DVec3> {
    let limit = max_distance * max_distance;
    vertices
        .into_iter()
        .map(|v| (v, v.distance_squared(point)))
        .filter(|(_, d)| *d <= limit)
        .fold(None, |best: Option<(DVec3, f64)>, (v, d)| match best {
            Some((_, bd)) if bd <= d => best,
            _ => Some((v, d)),
        })
        .map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::geom::{box_corners, rotation_from_orientation_degrees};

    const EPS: f64 = 1e-4;

    #[test]
    fn measurement_values() {
        let m = Measurement {
            a: DVec3::new(1.0, 2.0, 3.0),
            b: DVec3::new(4.0, 6.0, 3.0),
        };
        assert!((m.distance() - 5.0).abs() < 1e-12);
        assert_eq!(m.delta(), DVec3::new(3.0, 4.0, 0.0));
        assert!((m.horizontal() - 3.0).abs() < 1e-12);
        assert!((m.angle_to_ground_deg() - 4f64.atan2(3.0).to_degrees()).abs() < 1e-12);
        let flat = Measurement {
            a: DVec3::ZERO,
            b: DVec3::new(0.0, 0.0, -7.0),
        };
        assert_eq!(flat.angle_to_ground_deg(), 0.0);
        let down = Measurement {
            a: DVec3::new(0.0, 5.0, 0.0),
            b: DVec3::ZERO,
        };
        assert!((down.angle_to_ground_deg() - 90.0).abs() < 1e-12);
        let zero = Measurement {
            a: DVec3::ONE,
            b: DVec3::ONE,
        };
        assert_eq!((zero.distance(), zero.angle_to_ground_deg()), (0.0, 0.0));
    }

    #[test]
    fn tool_pairs_clicks_and_keeps_eight() {
        let mut tool = MeasureTool::new();
        assert_eq!(tool.click(DVec3::ZERO), None);
        assert_eq!(tool.pending(), Some(DVec3::ZERO));
        let m = tool.click(DVec3::X).unwrap();
        assert_eq!(m.distance(), 1.0);
        assert_eq!(tool.pending(), None);
        for i in 0..10 {
            tool.click(DVec3::ZERO);
            tool.click(DVec3::Y * f64::from(i + 2));
        }
        assert_eq!(tool.measurements().len(), MAX_MEASUREMENTS);
        // The oldest were dropped: the first kept one is from i = 2 (length 4).
        assert_eq!(tool.measurements().next().unwrap().distance(), 4.0);
        tool.click(DVec3::ONE);
        tool.clear();
        assert_eq!(tool.measurements().len(), 0);
        assert_eq!(tool.pending(), None);
    }

    /// Issue #220 "done when": a measure between two snapped vertices equals the true
    /// distance within 1e-4.
    #[test]
    fn measure_between_snapped_vertices() {
        // Two rotated parts.
        let rot_a = rotation_from_orientation_degrees(DVec3::new(10.0, 35.0, -20.0));
        let rot_b = rotation_from_orientation_degrees(DVec3::new(0.0, -60.0, 45.0));
        let ca = box_corners(DVec3::new(3.0, 1.0, -2.0), rot_a, DVec3::new(4.0, 1.0, 2.0));
        let cb = box_corners(
            DVec3::new(-15.5, 7.25, 9.0),
            rot_b,
            DVec3::new(2.0, 6.0, 1.5),
        );
        let true_a = ca[5];
        let true_b = cb[2];
        let expected = true_a.distance(true_b);

        // Clicks land a little off the corners, as a cursor does.
        let click_a = true_a + DVec3::new(0.08, -0.05, 0.03);
        let click_b = true_b + DVec3::new(-0.04, 0.1, 0.02);
        let all: Vec<DVec3> = ca.iter().chain(cb.iter()).copied().collect();
        let a = snap_to_vertex(click_a, all.iter().copied(), 0.25).unwrap();
        let b = snap_to_vertex(click_b, all.iter().copied(), 0.25).unwrap();
        assert_eq!((a, b), (true_a, true_b));

        let mut tool = MeasureTool::new();
        tool.click(a);
        let m = tool.click(b).unwrap();
        assert!((m.distance() - expected).abs() < EPS);
        assert!((m.delta() - (true_b - true_a)).length() < EPS);
    }

    #[test]
    fn snap_misses_outside_radius_and_picks_nearest() {
        let verts = [DVec3::ZERO, DVec3::X, DVec3::new(0.6, 0.0, 0.0)];
        assert_eq!(snap_to_vertex(DVec3::new(5.0, 0.0, 0.0), verts, 1.0), None);
        assert_eq!(
            snap_to_vertex(DVec3::new(0.7, 0.0, 0.0), verts, 1.0),
            Some(DVec3::new(0.6, 0.0, 0.0))
        );
        assert_eq!(snap_to_vertex(DVec3::ZERO, [], 1.0), None);
    }
}
