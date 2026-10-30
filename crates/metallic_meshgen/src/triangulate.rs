//! Ear-clipping triangulation of simple 2D polygons (cap faces of stairs, arches and ramps).
use glam::Vec2;

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

/// Signed distance of `p` from the line through `a` and `b` (positive on the left).
fn side(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    cross(a, b, p) / (b - a).length().max(f32::MIN_POSITIVE)
}

/// True when `p` lies inside triangle `abc` (counter-clockwise) or within `eps` of its border.
fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2, eps: f32) -> bool {
    side(a, b, p) >= -eps && side(b, c, p) >= -eps && side(c, a, p) >= -eps
}

/// Height of triangle `abc` over its longest side (negative when clockwise).
fn height(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let longest = (b - a).length().max((c - b).length()).max((a - c).length());
    cross(a, b, c) / longest.max(f32::MIN_POSITIVE)
}

/// Triangulates a simple counter-clockwise polygon. Returns index triples into `pts`, counter-clockwise.
/// Every vertex is used (collinear ones too), so the result shares its border exactly with neighbouring
/// strips and stays watertight.
pub(crate) fn triangulate(pts: &[Vec2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    let scale = pts
        .iter()
        .fold(0.0f32, |m, p| m.max(p.x.abs()).max(p.y.abs()))
        .max(1e-6);
    // Distances below this are float noise (about 1e-7 of the coordinates).
    let eps = scale * 2e-7;
    let mut idx: Vec<usize> = (0..n).collect();
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            // Convex by more than float noise (sine of the turn, so long thin ears still count).
            if cross(a, b, c) <= 1e-6 * (b - a).length() * (c - b).length() {
                continue; // reflex or collinear: not an ear
            }
            let blocked = idx.iter().any(|&j| {
                j != ia
                    && j != ib
                    && j != ic
                    && pts[j] != a
                    && pts[j] != b
                    && pts[j] != c
                    && in_triangle(pts[j], a, b, c, eps)
            });
            if !blocked {
                out.push([ia, ib, ic]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Numerically stuck (should not happen for simple polygons): clip the most convex corner.
            let m = idx.len();
            let best = (0..m)
                .max_by(|&x, &y| {
                    let cx = height(
                        pts[idx[(x + m - 1) % m]],
                        pts[idx[x]],
                        pts[idx[(x + 1) % m]],
                    );
                    let cy = height(
                        pts[idx[(y + m - 1) % m]],
                        pts[idx[y]],
                        pts[idx[(y + 1) % m]],
                    );
                    cx.total_cmp(&cy)
                })
                .unwrap_or(0);
            out.push([idx[(best + m - 1) % m], idx[best], idx[(best + 1) % m]]);
            idx.remove(best);
        }
    }
    out.push([idx[0], idx[1], idx[2]]);
    split_flat_triangles(pts, &mut out, eps);
    out
}

/// True when the three points are collinear within float noise: the sine of the angle at the middle point
/// is below 1e-6, or the points are closer than `eps`.
fn is_flat(a: Vec2, b: Vec2, c: Vec2, eps: f32) -> bool {
    let mut l = [(b - a).length(), (c - b).length(), (a - c).length()];
    l.sort_by(f32::total_cmp);
    l[0] <= eps || cross(a, b, c).abs() <= 1e-6 * l[0] * l[1]
}

/// Collinear vertices can end up as a zero-area triangle `(a, p, b)` with `p` on the diagonal `a b`. Replace
/// it and the triangle on the other side of `a b`, `(b, a, x)`, by `(a, p, x)` and `(p, b, x)`.
fn split_flat_triangles(pts: &[Vec2], tris: &mut Vec<[usize; 3]>, eps: f32) {
    for _ in 0..tris.len() {
        let Some(flat) = tris
            .iter()
            .position(|t| is_flat(pts[t[0]], pts[t[1]], pts[t[2]], eps))
        else {
            return;
        };
        let t = tris[flat];
        // The middle point is the one opposite the longest side.
        let len = |i: usize, j: usize| pts[t[i]].distance(pts[t[j]]);
        let k = [len(1, 2), len(2, 0), len(0, 1)]
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.total_cmp(y.1))
            .map_or(0, |m| m.0);
        let (p, a, b) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
        let other = tris
            .iter()
            .position(|u| (0..3).any(|i| u[i] == b && u[(i + 1) % 3] == a));
        let Some(o) = other else {
            tris.remove(flat);
            continue;
        };
        let u = tris[o];
        let x = u.iter().copied().find(|v| *v != a && *v != b).unwrap_or(a);
        let (first, second) = ([b, p, x], [p, a, x]);
        let (hi, lo) = if flat > o { (flat, o) } else { (o, flat) };
        tris.remove(hi);
        tris.remove(lo);
        tris.push(first);
        tris.push(second);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_shape_with_collinear_points_uses_every_vertex() {
        let pts = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), pts.len() - 2);
        let area: f32 = tris
            .iter()
            .map(|t| cross(pts[t[0]], pts[t[1]], pts[t[2]]) * 0.5)
            .sum();
        assert!((area - 3.0).abs() < 1e-5);
        assert!(
            tris.iter()
                .all(|t| !is_flat(pts[t[0]], pts[t[1]], pts[t[2]], 1e-6))
        );
        assert!(
            tris.iter()
                .all(|t| cross(pts[t[0]], pts[t[1]], pts[t[2]]) > 0.0)
        );
    }

    #[test]
    fn long_thin_cap_with_collinear_crease_points_has_no_flat_triangles() {
        // The side cap of a 2048-deep, 1-tall staircase with extra points next to the inner corner.
        let v = Vec2::new;
        let pts = [
            v(-1023.95, -0.45),
            v(1023.95, -0.45),
            v(1023.95, 0.45),
            v(0.05, 0.45),
            v(0.05, 0.15),
            v(0.05, 0.05),
            v(0.05, -0.05),
            v(-0.05, -0.05),
            v(-0.15, -0.05),
            v(-0.35, -0.05),
            v(-1023.95, -0.05),
        ];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), pts.len() - 2);
        for t in &tris {
            assert!(!is_flat(pts[t[0]], pts[t[1]], pts[t[2]], 1e-4), "{t:?}");
            assert!(cross(pts[t[0]], pts[t[1]], pts[t[2]]) > 0.0, "{t:?}");
        }
    }
}
