//! Extruded 2D profiles (stairs, ramp, arch) with optional bevels on every edge.
//!
//! The profile is a simple counter-clockwise polygon in the `(u, v)` plane, pushed along `w`. With a bevel
//! of radius `r`:
//! 1. convex corners of the profile are rounded with radius `r` (the long edges along `w`);
//! 2. each end gets a ring of rounded edges: ring `j` is the rounded profile moved inward by
//!    `r·(1 - cos φ)` and outward along `w` by `r·sin φ`, for φ from 0 to 90°. Arc points move toward their
//!    arc centre (so arcs shrink exactly), other points along their miter;
//! 3. the innermost ring closes the end as a flat cap (ear-clipped).
//!
//! Concave corners stay sharp. Points are added near them (when baking AO) so the occlusion gradient in the
//! crease has vertices to live on.
use crate::builder::{Builder, Corner};
use crate::math::{Frame, edge_normal_2d};
use crate::shape::BevelStyle;
use crate::triangulate::triangulate;
use glam::{Vec2, Vec3};
use std::f32::consts::FRAC_PI_2;

/// One input profile point. `smooth` marks points on a sampled curve: the surface normal is continuous across
/// them and they are never rounded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProfilePoint {
    pub p: Vec2,
    pub smooth: bool,
}

impl ProfilePoint {
    pub(crate) fn sharp(x: f32, y: f32) -> ProfilePoint {
        ProfilePoint {
            p: Vec2::new(x, y),
            smooth: false,
        }
    }
    pub(crate) fn smooth(p: Vec2) -> ProfilePoint {
        ProfilePoint { p, smooth: true }
    }
}

/// Where the profile lives in 3D: `P = u·p.x + v·p.y + w·depth`, with `u × v = w`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Extrusion {
    pub u: Vec3,
    pub v: Vec3,
    pub w: Vec3,
    /// Half the extrusion length.
    pub half_w: f32,
    /// Half size of the whole part (for UV offsets).
    pub half: Vec3,
}

impl Extrusion {
    fn at(&self, p: Vec2, depth: f32) -> Vec3 {
        self.u * p.x + self.v * p.y + self.w * depth
    }
    fn dir(&self, n: Vec2) -> Vec3 {
        self.u * n.x + self.v * n.y
    }
}

#[derive(Clone, Copy, Debug)]
enum Inset {
    /// On a rounded arc: moves toward the arc centre.
    Radial { centre: Vec2, radius: f32 },
    /// Moves along the miter vector (inward is `-miter`).
    Miter(Vec2),
}

#[derive(Clone, Copy, Debug)]
struct Pt {
    p: Vec2,
    smooth: bool,
    inset: Inset,
}

impl Pt {
    fn inset(&self, d: f32) -> Vec2 {
        match self.inset {
            Inset::Radial { centre, radius } => {
                if radius <= 0.0 {
                    self.p
                } else {
                    self.p + (centre - self.p) * (d / radius).min(1.0)
                }
            }
            Inset::Miter(m) => self.p - m * d,
        }
    }
}

/// Left turn at each vertex in radians (positive = convex for a counter-clockwise polygon).
fn turns(pts: &[Vec2]) -> Vec<f32> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[i] - pts[(i + n - 1) % n];
            let b = pts[(i + 1) % n] - pts[i];
            a.perp_dot(b).atan2(a.dot(b))
        })
        .collect()
}

/// Largest bevel radius (up to `r`) that the profile and the extrusion length can take: along every edge the
/// two ends may use at most 90% of it, and each end of the extrusion at most 45% of its length (the rest stays
/// flat, so no sliver triangles appear).
pub(crate) fn clamp_radius(profile: &[ProfilePoint], r: f32, half_w: f32) -> f32 {
    if r <= 0.0 {
        return 0.0;
    }
    let pts: Vec<Vec2> = profile.iter().map(|q| q.p).collect();
    let t = turns(&pts);
    let n = pts.len();
    let mut r = r.min(0.9 * half_w);
    for i in 0..n {
        let j = (i + 1) % n;
        let use_i = (t[i] * 0.5).tan().abs();
        let use_j = (t[j] * 0.5).tan().abs();
        let len = pts[i].distance(pts[j]);
        if use_i + use_j > 1e-6 {
            r = r.min(0.9 * len / (use_i + use_j));
        }
    }
    r.max(0.0)
}

/// Rounds convex sharp corners with radius `r` (arc points carry their centre), returns the new polygon.
fn round_corners(profile: &[ProfilePoint], r: f32, segs: u32) -> Vec<Pt> {
    let pts: Vec<Vec2> = profile.iter().map(|q| q.p).collect();
    let t = turns(&pts);
    let n = pts.len();
    let mut out = Vec::new();
    for i in 0..n {
        let q = profile[i];
        if r > 0.0 && !q.smooth && t[i] > 1e-3 {
            let e_in = (pts[i] - pts[(i + n - 1) % n]).normalize_or(Vec2::X);
            let e_out = (pts[(i + 1) % n] - pts[i]).normalize_or(Vec2::X);
            let lt = r * (t[i] * 0.5).tan();
            let t1 = q.p - e_in * lt;
            let centre = t1 + e_in.perp() * r;
            let k = ((segs as f32 * t[i] / FRAC_PI_2).round() as u32).max(1);
            let start = t1 - centre;
            for s in 0..=k {
                let a = t[i] * s as f32 / k as f32;
                let p = if s == k {
                    q.p + e_out * lt
                } else {
                    centre + Vec2::from_angle(a).rotate(start)
                };
                out.push(Pt {
                    p,
                    smooth: true,
                    inset: Inset::Radial { centre, radius: r },
                });
            }
        } else {
            out.push(Pt {
                p: q.p,
                smooth: q.smooth,
                inset: Inset::Miter(Vec2::ZERO),
            });
        }
    }
    out
}

/// Adds collinear points on both edges next to each concave sharp corner, at 0.05, 0.15 and 0.35 studs.
fn refine_creases(pts: Vec<Pt>) -> Vec<Pt> {
    let pos: Vec<Vec2> = pts.iter().map(|q| q.p).collect();
    let t = turns(&pos);
    let n = pts.len();
    let concave = |i: usize| !pts[i].smooth && t[i] < -1e-3;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let j = (i + 1) % n;
        out.push(pts[i]);
        let (a, b) = (pos[i], pos[j]);
        let len = a.distance(b);
        let dir = (b - a).normalize_or(Vec2::X);
        let mut extra: Vec<f32> = Vec::new();
        for d in [0.05f32, 0.15, 0.35] {
            if d < 0.45 * len {
                if concave(i) {
                    extra.push(d);
                }
                if concave(j) {
                    extra.push(len - d);
                }
            }
        }
        extra.sort_by(f32::total_cmp);
        for d in extra {
            out.push(Pt {
                p: a + dir * d,
                smooth: true,
                inset: Inset::Miter(Vec2::ZERO),
            });
        }
    }
    out
}

/// `sin` and `cos` that are exactly 0 or 1 at 0 and 90°, so ring ends match walls and caps bit for bit.
fn exact_sin_cos(phi: f32) -> (f32, f32) {
    if phi <= 0.0 {
        (0.0, 1.0)
    } else if phi >= FRAC_PI_2 {
        (1.0, 0.0)
    } else {
        phi.sin_cos()
    }
}

/// Emits the extruded, optionally bevelled profile. `r` must already be clamped with [`clamp_radius`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit(
    b: &mut Builder,
    ex: &Extrusion,
    profile: &[ProfilePoint],
    r: f32,
    style: BevelStyle,
    segs: u32,
    refine: bool,
    tol: f32,
) {
    let chamfer = style == BevelStyle::Chamfer;
    let segs = if chamfer { 1 } else { segs.max(1) };
    let mut pts = round_corners(profile, r, segs);
    if refine {
        pts = refine_creases(pts);
    }
    let n = pts.len();
    let edge_n: Vec<Vec2> = (0..n)
        .map(|i| edge_normal_2d(pts[(i + 1) % n].p - pts[i].p))
        .collect();
    // Miter vectors for points that are not on a rounded arc.
    for i in 0..n {
        if let Inset::Miter(_) = pts[i].inset {
            let (na, nb) = (edge_n[(i + n - 1) % n], edge_n[i]);
            let m = (na + nb) / (1.0 + na.dot(nb)).max(0.05);
            pts[i].inset = Inset::Miter(m);
        }
    }
    let vert_n: Vec<Vec2> = (0..n)
        .map(|i| (edge_n[(i + n - 1) % n] + edge_n[i]).normalize_or(edge_n[i]))
        .collect();
    // Normal used by edge `e` at its endpoint `i`.
    let wall_n = |i: usize, e: usize| -> Vec2 { if pts[i].smooth { vert_n[i] } else { edge_n[e] } };

    // Wall charts: one per smooth run, u = arc length along the run.
    let start = (0..n).find(|&i| !pts[i].smooth).unwrap_or(0);
    let mut edge_chart = vec![0u32; n];
    let mut edge_u = vec![(0.0f32, 0.0f32); n];
    let mut chart = b.chart();
    let mut u = 0.0;
    for k in 0..n {
        let i = (start + k) % n;
        if k > 0 && !pts[i].smooth {
            chart = b.chart();
            u = 0.0;
        }
        let len = pts[i].p.distance(pts[(i + 1) % n].p);
        edge_chart[i] = chart;
        edge_u[i] = (u, u + len);
        u += len;
    }

    let hw = ex.half_w;
    let wall_end = hw - r;
    // Walls.
    for e in 0..n {
        let j = (e + 1) % n;
        let (u0, u1) = edge_u[e];
        let (ni, nj) = (ex.dir(wall_n(e, e)), ex.dir(wall_n(j, e)));
        let c = |p: Vec2, d: f32, nn: Vec3, uu: f32| {
            Corner::new(ex.at(p, d), nn, Vec2::new(uu, d + hw))
        };
        b.quad(
            edge_chart[e],
            c(pts[e].p, -wall_end, ni, u0),
            c(pts[j].p, -wall_end, nj, u1),
            c(pts[j].p, wall_end, nj, u1),
            c(pts[e].p, wall_end, ni, u0),
        );
    }

    // Ends.
    for s in [1.0f32, -1.0] {
        let cap_n = ex.w * s;
        let frame = Frame::for_normal(cap_n, ex.half);
        let cap_chart = b.chart();
        let rings = if r > 0.0 { segs } else { 0 };
        let ring = |j: u32| -> (Vec<Vec2>, f32, f32) {
            let phi = FRAC_PI_2 * j as f32 / rings.max(1) as f32;
            let (sin, cos) = exact_sin_cos(phi);
            let d = r * (1.0 - cos);
            let depth = s * (hw - r + r * sin);
            (pts.iter().map(|q| q.inset(d)).collect(), depth, phi)
        };
        let (mut prev, mut prev_depth, mut prev_phi) = ring(0);
        for j in 1..=rings {
            let (cur, depth, phi) = ring(j);
            let wall_side = 0.5 * (prev_phi + phi) < FRAC_PI_2 * 0.5;
            for e in 0..n {
                let k = (e + 1) % n;
                let quad = [
                    (prev[e], prev_depth, prev_phi, e),
                    (prev[k], prev_depth, prev_phi, k),
                    (cur[k], depth, phi, k),
                    (cur[e], depth, phi, e),
                ];
                let ch = if wall_side { edge_chart[e] } else { cap_chart };
                let corners = quad.map(|(p2, d, ph, i)| {
                    let p = ex.at(p2, d);
                    let (sin, cos) = exact_sin_cos(ph);
                    let nn = ex.dir(wall_n(i, e)) * cos + cap_n * sin;
                    let uv = if wall_side {
                        let uu = if i == e { edge_u[e].0 } else { edge_u[e].1 };
                        Vec2::new(uu, d + hw)
                    } else {
                        frame.uv(p)
                    };
                    Corner::new(p, nn, uv)
                });
                if chamfer {
                    let out = ex.dir(edge_n[e]) + cap_n;
                    b.flat_tri(ch, out, [corners[0], corners[1], corners[2]]);
                    b.flat_tri(ch, out, [corners[0], corners[2], corners[3]]);
                } else {
                    b.quad(ch, corners[0], corners[1], corners[2], corners[3]);
                }
            }
            prev = cur;
            prev_depth = depth;
            prev_phi = phi;
        }
        // Cap: drop repeated points (collapsed arcs), then ear-clip.
        let mut poly: Vec<Vec2> = Vec::with_capacity(prev.len());
        for p in &prev {
            if poly.last().is_none_or(|q: &Vec2| q.distance(*p) > tol) {
                poly.push(*p);
            }
        }
        while poly.len() > 1 && poly[0].distance(poly[poly.len() - 1]) <= tol {
            poly.pop();
        }
        for t in triangulate(&poly) {
            let c = t.map(|i| {
                let p = ex.at(poly[i], prev_depth);
                Corner::new(p, cap_n, frame.uv(p))
            });
            b.tri(cap_chart, c[0], c[1], c[2]);
        }
    }
}
