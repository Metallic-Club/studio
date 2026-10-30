//! Small geometry helpers shared by the operations.

use glam::{Vec2, Vec3};

/// Newell's method: an area-weighted normal of a (possibly non-planar) polygon. Its length is twice the area.
#[must_use]
pub fn newell(points: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
    }
    n
}

/// Unit normal of a polygon, or zero for a degenerate one.
#[must_use]
pub fn polygon_normal(points: &[Vec3]) -> Vec3 {
    newell(points).normalize_or_zero()
}

/// Area of a polygon.
#[must_use]
pub fn polygon_area(points: &[Vec3]) -> f32 {
    newell(points).length() * 0.5
}

/// Arithmetic mean of points (zero for an empty slice).
#[must_use]
pub fn centroid(points: &[Vec3]) -> Vec3 {
    if points.is_empty() {
        return Vec3::ZERO;
    }
    points.iter().copied().sum::<Vec3>() / points.len() as f32
}

/// Two unit vectors spanning the plane with normal `n`, right-handed (`u × v = n`).
#[must_use]
pub fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let n = n.normalize_or(Vec3::Y);
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let v = n.cross(helper).normalize_or(Vec3::Z);
    let u = v.cross(n);
    (u, v)
}

/// Signed area of a 2D polygon (positive when counter-clockwise).
#[must_use]
pub fn signed_area_2d(points: &[Vec2]) -> f32 {
    let mut a = 0.0;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

/// Ear-clipping triangulation of a simple polygon given in 3D (projected onto its own plane).
///
/// Returns index triples into `points`, counter-clockwise around `newell(points)`. Falls back to a fan when the
/// polygon is degenerate or self-intersecting, so the output always has `n - 2` triangles.
#[must_use]
pub fn triangulate_polygon(points: &[Vec3]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let normal = newell(points);
    let (u, v) = plane_basis(normal);
    let p2: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.dot(u), p.dot(v))).collect();
    if n == 4 {
        // Split a quad along the diagonal that keeps both triangles counter-clockwise; prefer the shorter one.
        let ok = |a: usize, b: usize, c: usize| cross2(p2[a], p2[b], p2[c]) > 0.0;
        let d02 = p2[0].distance_squared(p2[2]);
        let d13 = p2[1].distance_squared(p2[3]);
        let first = ok(0, 1, 2) && ok(0, 2, 3);
        let second = ok(1, 2, 3) && ok(1, 3, 0);
        return if second && (!first || d13 < d02) {
            vec![[1, 2, 3], [1, 3, 0]]
        } else {
            vec![[0, 1, 2], [0, 2, 3]]
        };
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            if cross2(p2[a], p2[b], p2[c]) <= 1e-12 {
                continue;
            }
            let blocked = idx
                .iter()
                .any(|&k| k != a && k != b && k != c && point_in_triangle(p2[k], p2[a], p2[b], p2[c]));
            if blocked {
                continue;
            }
            out.push([a, b, c]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    // Whatever is left (a triangle, or a degenerate remainder) is fanned so the count stays n - 2.
    for i in 1..idx.len().saturating_sub(1) {
        out.push([idx[0], idx[i], idx[i + 1]]);
    }
    out
}

fn cross2(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    cross2(a, b, p) >= 0.0 && cross2(b, c, p) >= 0.0 && cross2(c, a, p) >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newell_of_unit_square_points_up() {
        let pts = [Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::X];
        let n = polygon_normal(&pts);
        assert!((n - Vec3::Y).length() < 1e-6, "{n}");
        assert!((polygon_area(&pts) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn plane_basis_is_right_handed() {
        for n in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, -3.0).normalize()] {
            let (u, v) = plane_basis(n);
            assert!((u.cross(v) - n).length() < 1e-5, "{n}");
        }
    }

    #[test]
    fn ear_clipping_handles_concave_l_shape() {
        let pts: Vec<Vec3> = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)]
            .iter()
            .map(|&(x, y)| Vec3::new(x, y, 0.0))
            .collect();
        let tris = triangulate_polygon(&pts);
        assert_eq!(tris.len(), 4);
        let area: f32 = tris.iter().map(|t| polygon_area(&[pts[t[0]], pts[t[1]], pts[t[2]]])).sum();
        assert!((area - 3.0).abs() < 1e-5, "{area}");
        for t in &tris {
            let n = newell(&[pts[t[0]], pts[t[1]], pts[t[2]]]);
            assert!(n.z > 0.0, "triangle {t:?} flipped");
        }
    }
}
