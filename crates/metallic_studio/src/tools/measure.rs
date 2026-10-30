//! Measure tool and distance readouts (spec 10.2, issue #220).
//!
//! - [`MeasureTool`]: Shift+M; click A, click B. Each [`Measurement`] reports the distance in
//!   studs, the X, Y, Z deltas and the angle to the ground. Up to [`MAX_MEASUREMENTS`] stay
//!   on screen; a ninth drops the oldest. Esc clears them. They are never saved.
//! - [`snap_to_vertex`] and [`box_corners`]: the vertex part of snapping for measure clicks.
//!   The full snapping rules (spec section 5) belong to the snapping work (#215).
//! - [`Aabb`] and [`axis_gaps`]: the Alt-hover readouts of the gaps between the selection's
//!   bounding box and the hovered part's on each axis.

use std::collections::VecDeque;

use glam::{DQuat, DVec3, EulerRot};

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

/// Rotation from a Roblox-style `Orientation` in degrees (applied Y, then X, then Z).
pub fn rotation_from_orientation_degrees(orientation: DVec3) -> DQuat {
    DQuat::from_euler(
        EulerRot::YXZ,
        orientation.y.to_radians(),
        orientation.x.to_radians(),
        orientation.z.to_radians(),
    )
}

/// The eight corners of a box with this centre, rotation and size.
pub fn box_corners(centre: DVec3, rotation: DQuat, size: DVec3) -> [DVec3; 8] {
    let h = size * 0.5;
    let mut out = [DVec3::ZERO; 8];
    for (i, c) in out.iter_mut().enumerate() {
        let sx = if i & 1 == 0 { -h.x } else { h.x };
        let sy = if i & 2 == 0 { -h.y } else { h.y };
        let sz = if i & 4 == 0 { -h.z } else { h.z };
        *c = centre + rotation * DVec3::new(sx, sy, sz);
    }
    out
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

/// An axis-aligned bounding box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Smallest corner.
    pub min: DVec3,
    /// Largest corner.
    pub max: DVec3,
}

impl Aabb {
    /// The box around a set of points; `None` when there are none.
    pub fn from_points(points: impl IntoIterator<Item = DVec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(
            Self {
                min: first,
                max: first,
            },
            |b, p| Self {
                min: b.min.min(p),
                max: b.max.max(p),
            },
        ))
    }

    /// The world box around a rotated part.
    pub fn from_oriented_box(centre: DVec3, rotation: DQuat, size: DVec3) -> Self {
        let r = glam::DMat3::from_quat(rotation);
        let h = size.abs() * 0.5;
        // Half extent on each world axis: sum of |r_ij| * h_j.
        let ext = DVec3::new(
            r.x_axis.x.abs() * h.x + r.y_axis.x.abs() * h.y + r.z_axis.x.abs() * h.z,
            r.x_axis.y.abs() * h.x + r.y_axis.y.abs() * h.y + r.z_axis.y.abs() * h.z,
            r.x_axis.z.abs() * h.x + r.y_axis.z.abs() * h.y + r.z_axis.z.abs() * h.z,
        );
        Self {
            min: centre - ext,
            max: centre + ext,
        }
    }

    /// The box around both.
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Size per axis.
    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }
}

/// Gap between two boxes on each axis: positive is empty space between them, zero is
/// touching, negative is how far they overlap on that axis.
pub fn axis_gaps(a: &Aabb, b: &Aabb) -> DVec3 {
    (b.min - a.max).max(a.min - b.max)
}

/// Shortest distance between two boxes; 0 when they touch or overlap.
pub fn gap_distance(a: &Aabb, b: &Aabb) -> f64 {
    axis_gaps(a, b).max(DVec3::ZERO).length()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn box_corners_are_a_box() {
        let rot = rotation_from_orientation_degrees(DVec3::new(30.0, 45.0, 0.0));
        let c = box_corners(DVec3::ONE, rot, DVec3::new(4.0, 1.0, 2.0));
        let diag = DVec3::new(4.0, 1.0, 2.0).length();
        // Opposite corners (i and 7 - i) are a full diagonal apart.
        for i in 0..4 {
            assert!((c[i].distance(c[7 - i]) - diag).abs() < 1e-12);
        }
        // Edge along local X.
        assert!((c[0].distance(c[1]) - 4.0).abs() < 1e-12);
        let centre = c.iter().copied().sum::<DVec3>() / 8.0;
        assert!((centre - DVec3::ONE).length() < 1e-12);
    }

    #[test]
    fn oriented_aabb_matches_its_corners() {
        let rot = rotation_from_orientation_degrees(DVec3::new(12.0, 70.0, -33.0));
        let centre = DVec3::new(2.0, -1.0, 5.0);
        let size = DVec3::new(3.0, 0.5, 6.0);
        let from_corners = Aabb::from_points(box_corners(centre, rot, size)).unwrap();
        let direct = Aabb::from_oriented_box(centre, rot, size);
        assert!((from_corners.min - direct.min).length() < 1e-12);
        assert!((from_corners.max - direct.max).length() < 1e-12);
        assert_eq!(Aabb::from_points([]), None);
    }

    #[test]
    fn axis_gap_readouts() {
        let a = Aabb {
            min: DVec3::ZERO,
            max: DVec3::new(2.0, 2.0, 2.0),
        };
        // 3 studs to the right on X, overlapping on Y by 1, touching on Z.
        let b = Aabb {
            min: DVec3::new(5.0, 1.0, 2.0),
            max: DVec3::new(6.0, 4.0, 3.0),
        };
        let g = axis_gaps(&a, &b);
        assert_eq!(g, DVec3::new(3.0, -1.0, 0.0));
        assert_eq!(axis_gaps(&b, &a), g, "symmetric");
        assert_eq!(gap_distance(&a, &b), 3.0);
        // Diagonal gap.
        let c = Aabb {
            min: DVec3::new(5.0, 6.0, 0.0),
            max: DVec3::new(6.0, 7.0, 1.0),
        };
        assert_eq!(gap_distance(&a, &c), 5.0);
        // A box inside another: negative on every axis, distance 0.
        let inner = Aabb {
            min: DVec3::splat(0.5),
            max: DVec3::splat(1.0),
        };
        assert!(axis_gaps(&a, &inner).cmplt(DVec3::ZERO).all());
        assert_eq!(gap_distance(&a, &inner), 0.0);
        // The selection's box is the union of its parts.
        let sel = a.union(&inner).union(&c);
        assert_eq!(sel.size(), DVec3::new(6.0, 7.0, 2.0));
    }
}
