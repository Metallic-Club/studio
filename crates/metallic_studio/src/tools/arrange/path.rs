//! The curve for "array along path": a centripetal Catmull-Rom spline through picked points,
//! measured by arc length so copies can sit at equal distances along it.
//!
//! Centripetal parametrisation (knot spacing = square root of the chord length) cannot form
//! cusps or self-loops inside a segment, which keeps sharp corners tidy (spec 8.2). Each segment is
//! evaluated with the Barry-Goldman pyramid; the two ends get mirrored phantom points so the curve
//! starts and ends exactly on the first and last picked points.

use glam::DVec3;

use super::math::EPS;
use super::plan::ArrangeError;

/// Arc-length table samples per segment. With 128 samples the table's error on gentle curves is
/// far under the 1% spacing tolerance.
const SAMPLES_PER_SEGMENT: usize = 128;

/// The most points a path may have. Each segment keeps a fixed-size arc-length table, so this
/// bounds the table at about 130,000 samples.
pub const MAX_PATH_POINTS: usize = 1024;

/// A centripetal Catmull-Rom curve with an arc-length table.
#[derive(Debug, Clone)]
pub struct PathCurve {
    /// Control points with one phantom point at each end.
    ctrl: Vec<DVec3>,
    /// `(arc length, segment, u)` at each sample, arc length increasing.
    table: Vec<(f64, usize, f64)>,
}

impl PathCurve {
    /// Builds the curve through `points`. Repeated consecutive points are dropped.
    ///
    /// Errors: [`ArrangeError::Invalid`] with fewer than two distinct points or more than
    /// [`MAX_PATH_POINTS`] points.
    pub fn new(points: &[DVec3]) -> Result<PathCurve, ArrangeError> {
        if points.len() > MAX_PATH_POINTS {
            return Err(ArrangeError::Invalid(format!(
                "a path has at most {MAX_PATH_POINTS} points (got {})",
                points.len()
            )));
        }
        if !points.iter().all(|p| p.is_finite()) {
            return Err(ArrangeError::Invalid(
                "path points must be finite numbers".into(),
            ));
        }
        let mut pts: Vec<DVec3> = Vec::with_capacity(points.len());
        for &p in points {
            if pts.last().is_none_or(|q: &DVec3| (p - *q).length() > EPS) {
                pts.push(p);
            }
        }
        if pts.len() < 2 {
            return Err(ArrangeError::Invalid(
                "a path needs two or more different points".into(),
            ));
        }
        let n = pts.len();
        let mut ctrl = Vec::with_capacity(n + 2);
        ctrl.push(pts[0] * 2.0 - pts[1]);
        ctrl.extend_from_slice(&pts);
        ctrl.push(pts[n - 1] * 2.0 - pts[n - 2]);
        let mut curve = PathCurve {
            ctrl,
            table: Vec::new(),
        };
        let mut s = 0.0;
        let mut prev = curve.eval(0, 0.0);
        curve.table.push((0.0, 0, 0.0));
        for seg in 0..curve.segments() {
            for i in 1..=SAMPLES_PER_SEGMENT {
                let u = i as f64 / SAMPLES_PER_SEGMENT as f64;
                let p = curve.eval(seg, u);
                s += (p - prev).length();
                prev = p;
                curve.table.push((s, seg, u));
            }
        }
        if !curve.length().is_finite() {
            return Err(ArrangeError::Invalid("the path is too long".into()));
        }
        Ok(curve)
    }

    /// Number of segments (picked points minus one).
    pub fn segments(&self) -> usize {
        self.ctrl.len() - 3
    }

    /// The point on segment `seg` at `u` in 0..=1 (u = 0 is picked point `seg`).
    pub fn eval(&self, seg: usize, u: f64) -> DVec3 {
        let [p0, p1, p2, p3] = [
            self.ctrl[seg],
            self.ctrl[seg + 1],
            self.ctrl[seg + 2],
            self.ctrl[seg + 3],
        ];
        let knot = |a: DVec3, b: DVec3| (b - a).length().sqrt().max(EPS);
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        let t = t1 + (t2 - t1) * u;
        let lerp = |a: DVec3, b: DVec3, ta: f64, tb: f64| {
            a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
        };
        let a1 = lerp(p0, p1, t0, t1);
        let a2 = lerp(p1, p2, t1, t2);
        let a3 = lerp(p2, p3, t2, t3);
        let b1 = lerp(a1, a2, t0, t2);
        let b2 = lerp(a2, a3, t1, t3);
        lerp(b1, b2, t1, t2)
    }

    /// Total arc length.
    pub fn length(&self) -> f64 {
        self.table.last().map_or(0.0, |e| e.0)
    }

    /// The segment and `u` at arc length `s` (clamped to the curve).
    pub fn locate(&self, s: f64) -> (usize, f64) {
        let s = s.clamp(0.0, self.length());
        let i = self.table.partition_point(|e| e.0 < s);
        if i == 0 {
            return (0, 0.0);
        }
        let (s1, seg1, u1) = self.table[i];
        let (s0, seg0, u0) = self.table[i - 1];
        // A sample at u = 1 ends a segment; the next segment starts at u = 0.
        let u0 = if seg0 == seg1 { u0 } else { 0.0 };
        let f = if s1 > s0 { (s - s0) / (s1 - s0) } else { 0.0 };
        (seg1, u0 + (u1 - u0) * f)
    }

    /// The point at arc length `s`.
    pub fn point_at(&self, s: f64) -> DVec3 {
        let (seg, u) = self.locate(s);
        self.eval(seg, u)
    }

    /// The unit tangent at arc length `s`.
    pub fn tangent_at(&self, s: f64) -> DVec3 {
        let (seg, u) = self.locate(s);
        let h = 1e-4;
        let (a, b) = (
            self.eval(seg, (u - h).max(0.0)),
            self.eval(seg, (u + h).min(1.0)),
        );
        (b - a).normalize_or_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arrange::testing::d as v;

    #[test]
    fn passes_through_every_point() {
        let pts = [
            v(0.0, 0.0, 0.0),
            v(10.0, 0.0, 0.0),
            v(10.0, 0.0, 10.0),
            v(0.0, 3.0, 12.0),
        ];
        let c = PathCurve::new(&pts).unwrap();
        for (i, p) in pts.iter().enumerate().take(3) {
            assert!((c.eval(i, 0.0) - *p).length() < 1e-9);
        }
        assert!((c.eval(2, 1.0) - pts[3]).length() < 1e-9);
        assert!((c.point_at(c.length()) - pts[3]).length() < 1e-6);
    }

    #[test]
    fn two_points_make_a_straight_line() {
        let c = PathCurve::new(&[v(0.0, 0.0, 0.0), v(0.0, 0.0, 8.0)]).unwrap();
        assert!((c.length() - 8.0).abs() < 1e-9);
        assert!((c.point_at(2.0) - v(0.0, 0.0, 2.0)).length() < 1e-9);
        assert!((c.tangent_at(4.0) - v(0.0, 0.0, 1.0)).length() < 1e-6);
    }

    #[test]
    fn needs_two_distinct_points() {
        assert!(PathCurve::new(&[v(1.0, 1.0, 1.0), v(1.0, 1.0, 1.0)]).is_err());
    }

    #[test]
    fn sharp_corner_does_not_overshoot_into_a_loop() {
        // A hairpin: centripetal Catmull-Rom stays within the points' hull here.
        let c = PathCurve::new(&[v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0), v(0.0, 0.0, 1.0)]).unwrap();
        for i in 0..=200 {
            let p = c.point_at(c.length() * i as f64 / 200.0);
            assert!(p.x < 10.5 && p.x > -1.5, "overshoot at {p:?}");
        }
    }
}
