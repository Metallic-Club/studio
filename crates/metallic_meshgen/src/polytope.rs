//! Convex shapes (block, wedges, pyramid, prisms) with optional rounded or chamfered edges.
//!
//! A convex solid is given as planes. A bevel of radius `r` is the solid shrunk by `r` (every plane moved in
//! by `r`), then grown back by a ball of radius `r`: flat faces return to their place, edges become cylinder
//! strips and corners become sphere patches. That is exact for any convex solid and keeps the bevel width the
//! same at every size. The shrunk solid is rebuilt by half-space intersection, so corners where four planes
//! meet (pyramid tips) stay correct even when the shrink splits them into a short ridge.
use crate::builder::{Builder, Corner};
use crate::math::{Frame, slerp};
use crate::shape::BevelStyle;
use glam::Vec3;

/// A half-space `n · p <= d` with unit `n` pointing out of the solid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plane {
    pub n: Vec3,
    pub d: f32,
}

/// Planes of the convex hull faces `faces` (vertex index lists) of `verts`, oriented away from the centroid.
pub(crate) fn planes_from_faces(verts: &[Vec3], faces: &[&[usize]]) -> Vec<Plane> {
    let centroid = verts.iter().copied().sum::<Vec3>() / verts.len().max(1) as f32;
    faces
        .iter()
        .map(|f| {
            let (a, b, c) = (verts[f[0]], verts[f[1]], verts[f[2]]);
            let mut n = (b - a).cross(c - a).normalize_or(Vec3::Y);
            if n.dot(a - centroid) < 0.0 {
                n = -n;
            }
            Plane { n, d: n.dot(a) }
        })
        .collect()
}

/// Vertices and faces of a convex solid built from planes.
struct Solid {
    verts: Vec<Vec3>,
    /// Planes each vertex lies on.
    incident: Vec<Vec<usize>>,
    /// Per plane, its vertices counter-clockwise seen from outside (empty if the plane does not touch).
    faces: Vec<Vec<usize>>,
}

fn intersect3(a: &Plane, b: &Plane, c: &Plane) -> Option<Vec3> {
    let bc = b.n.cross(c.n);
    let det = a.n.dot(bc);
    if det.abs() < 1e-7 {
        return None;
    }
    Some((bc * a.d + c.n.cross(a.n) * b.d + a.n.cross(b.n) * c.d) / det)
}

/// Half-space intersection by brute force (shapes have at most 8 planes). Corners closer than `merge` are
/// one corner (a pyramid tip that the shrink splits into a hair-thin ridge stays a tip, instead of a nest
/// of slivers); a corner lies on every plane that any of its merged points lies on within `tol`.
fn solid(planes: &[Plane], tol: f32, merge: f32) -> Solid {
    let np = planes.len();
    // (position sum, point count, incident planes) per corner.
    let mut groups: Vec<(Vec3, f32, Vec<usize>)> = Vec::new();
    for i in 0..np {
        for j in i + 1..np {
            for k in j + 1..np {
                let Some(p) = intersect3(&planes[i], &planes[j], &planes[k]) else {
                    continue;
                };
                if planes.iter().any(|pl| pl.n.dot(p) > pl.d + tol) {
                    continue;
                }
                let on: Vec<usize> = (0..np)
                    .filter(|&f| (planes[f].n.dot(p) - planes[f].d).abs() <= tol)
                    .collect();
                match groups.iter_mut().find(|g| (g.0 / g.1).distance(p) <= merge) {
                    Some(g) => {
                        g.0 += p;
                        g.1 += 1.0;
                        for f in on {
                            if !g.2.contains(&f) {
                                g.2.push(f);
                            }
                        }
                    }
                    None => groups.push((p, 1.0, on)),
                }
            }
        }
    }
    let verts: Vec<Vec3> = groups.iter().map(|g| g.0 / g.1).collect();
    let incident: Vec<Vec<usize>> = groups
        .into_iter()
        .map(|mut g| {
            g.2.sort_unstable();
            g.2
        })
        .collect();
    let faces = (0..np)
        .map(|f| {
            let mut on: Vec<usize> = (0..verts.len())
                .filter(|&v| incident[v].contains(&f))
                .collect();
            if on.len() < 3 {
                return Vec::new();
            }
            let c = on.iter().map(|&v| verts[v]).sum::<Vec3>() / on.len() as f32;
            let fr = Frame::for_normal(planes[f].n, Vec3::ZERO);
            on.sort_by(|&a, &b| {
                let da = verts[a] - c;
                let db = verts[b] - c;
                let aa = da.dot(fr.v).atan2(da.dot(fr.u));
                let ab = db.dot(fr.v).atan2(db.dot(fr.u));
                aa.total_cmp(&ab)
            });
            on
        })
        .collect();
    Solid {
        verts,
        incident,
        faces,
    }
}

fn face_area(s: &Solid, f: usize) -> f32 {
    let face = &s.faces[f];
    if face.len() < 3 {
        return 0.0;
    }
    let a = s.verts[face[0]];
    let mut sum = Vec3::ZERO;
    for w in face[1..].windows(2) {
        sum += (s.verts[w[0]] - a).cross(s.verts[w[1]] - a);
    }
    sum.length() * 0.5
}

fn shrink(planes: &[Plane], r: f32) -> Vec<Plane> {
    planes
        .iter()
        .map(|p| Plane { n: p.n, d: p.d - r })
        .collect()
}

/// True when shrinking by `r` keeps every face with a real area.
fn shrink_ok(planes: &[Plane], r: f32, tol: f32) -> bool {
    let s = solid(&shrink(planes, r), tol, tol);
    (0..planes.len()).all(|f| face_area(&s, f) > tol * tol * 100.0)
}

/// The radius to use for a requested `r`: at most 80% of the largest radius that keeps every face, so the
/// shrunk core never gets thinner than its own rounding (no slivers at extreme proportions).
pub(crate) fn clamp_radius(planes: &[Plane], r: f32, tol: f32) -> f32 {
    if r <= 0.0 {
        return 0.0;
    }
    let limit = 1.25 * r;
    if shrink_ok(planes, limit, tol) {
        return r;
    }
    let (mut lo, mut hi) = (0.0f32, limit);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if shrink_ok(planes, mid, tol) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    r.min(lo * 0.8)
}

/// Steps across the round between faces with normals `a` and `b`: `segs` per 90° of turn, at least one
/// (two when the turn is over 120°, so a smooth strip never spans a near-half-turn in one step).
fn arc_steps(a: Vec3, b: Vec3, segs: u32, chamfer: bool) -> u32 {
    if chamfer {
        return 1;
    }
    let angle = a.dot(b).clamp(-1.0, 1.0).acos();
    let k = (segs as f32 * angle / std::f32::consts::FRAC_PI_2).round() as u32;
    let min = if angle > 2.0 * std::f32::consts::FRAC_PI_3 {
        2
    } else {
        1
    };
    k.max(min)
}

/// The plane sharing the edge `a b` with plane `f`.
fn neighbour(s: &Solid, f: usize, a: usize, b: usize) -> Option<usize> {
    s.incident[a]
        .iter()
        .copied()
        .find(|g| *g != f && s.incident[b].contains(g))
}

/// Emits the solid bounded by `planes`, with edges rounded or chamfered by `r` (already clamped).
/// `segments` is the number of steps across a 90° round. Returns the vertices of the shrunk solid
/// (useful as convex-hull collider points).
pub(crate) fn emit(
    b: &mut Builder,
    planes: &[Plane],
    half: Vec3,
    r: f32,
    style: BevelStyle,
    segments: u32,
    tol: f32,
) -> Vec<Vec3> {
    let inner = shrunk(planes, r, tol);
    let frames: Vec<Frame> = planes
        .iter()
        .map(|p| Frame::for_normal(p.n, half))
        .collect();
    let charts: Vec<u32> = planes.iter().map(|_| b.chart()).collect();
    let corner = |f: usize, p: Vec3, n: Vec3| Corner::new(p, n, frames[f].uv(p));

    // Flat faces.
    for (f, face) in inner.faces.iter().enumerate() {
        if face.len() < 3 {
            continue;
        }
        let n = planes[f].n;
        let pts: Vec<Vec3> = face.iter().map(|&v| inner.verts[v] + n * r).collect();
        for i in 1..pts.len() - 1 {
            b.tri(
                charts[f],
                corner(f, pts[0], n),
                corner(f, pts[i], n),
                corner(f, pts[i + 1], n),
            );
        }
    }
    if r <= 0.0 {
        return inner.verts;
    }
    let chamfer = style == BevelStyle::Chamfer;
    let segs = if chamfer { 1 } else { segments.max(1) };
    // Rounds narrower than this would be welded shut anyway: skip them (the faces on both sides weld).
    let negligible = 0.5 * crate::builder::weld_tolerance(half);
    let tiny = |a: Vec3, c: Vec3| r * a.dot(c).clamp(-1.0, 1.0).acos() < negligible;

    // Edge strips.
    for (f, face) in inner.faces.iter().enumerate() {
        let m = face.len();
        for i in 0..m {
            let (va, vb) = (face[i], face[(i + 1) % m]);
            let Some(g) = neighbour(&inner, f, va, vb) else {
                continue;
            };
            if g < f {
                continue; // emitted from the other face
            }
            let (a, bb) = (inner.verts[va], inner.verts[vb]);
            let (nf, ng) = (planes[f].n, planes[g].n);
            if tiny(nf, ng) {
                continue;
            }
            let k = arc_steps(nf, ng, segs, chamfer);
            for j in 0..k {
                let t0 = j as f32 / k as f32;
                let t1 = (j + 1) as f32 / k as f32;
                let (m0, m1) = (slerp(nf, ng, t0), slerp(nf, ng, t1));
                let owner = if 0.5 * (t0 + t1) <= 0.5 { f } else { g };
                let quad = [a + m0 * r, bb + m0 * r, bb + m1 * r, a + m1 * r];
                let ns = if chamfer {
                    let n = (nf + ng).normalize_or(nf);
                    [n; 4]
                } else {
                    [m0, m0, m1, m1]
                };
                b.quad(
                    charts[owner],
                    corner(owner, quad[0], ns[0]),
                    corner(owner, quad[1], ns[1]),
                    corner(owner, quad[2], ns[2]),
                    corner(owner, quad[3], ns[3]),
                );
            }
        }
    }

    // Corner patches.
    for (v, inc) in inner.incident.iter().enumerate() {
        if inc.len() < 3 {
            continue;
        }
        let centre = inner.verts[v];
        let plain = inc
            .iter()
            .map(|&f| planes[f].n)
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let fr = Frame::for_normal(plain, Vec3::ZERO);
        let mut ring: Vec<usize> = inc.clone();
        ring.sort_by(|&x, &y| {
            let (nx, ny) = (planes[x].n, planes[y].n);
            let ax = nx.dot(fr.v).atan2(nx.dot(fr.u));
            let ay = ny.dot(fr.v).atan2(ny.dot(fr.u));
            ax.total_cmp(&ay)
        });
        // Centre of the corner's normal cone: arc midpoints weighted by arc length. Unlike the plain average of
        // the face normals, this stays inside the cone at knife-edge corners (very flat wedges).
        let k = ring.len();
        let avg = (0..k)
            .map(|i| {
                let (a, c) = (planes[ring[i]].n, planes[ring[(i + 1) % k]].n);
                (a + c).normalize_or(plain) * a.dot(c).clamp(-1.0, 1.0).acos()
            })
            .sum::<Vec3>()
            .normalize_or(plain);
        let owner_of = |dir: Vec3| -> usize {
            let mut best = ring[0];
            for &f in &ring {
                if planes[f].n.dot(dir) > planes[best].n.dot(dir) {
                    best = f;
                }
            }
            best
        };
        if chamfer {
            let pts: Vec<Vec3> = ring.iter().map(|&f| centre + planes[f].n * r).collect();
            let owner = owner_of(avg);
            for i in 1..pts.len() - 1 {
                let c = [pts[0], pts[i], pts[i + 1]].map(|p| corner(owner, p, avg));
                b.flat_tri(charts[owner], avg, c);
            }
            continue;
        }
        // Rows from the centre out: enough that no step turns much more than the edge strips do.
        let rows = ring
            .iter()
            .map(|&f| arc_steps(avg, planes[f].n, segs, false))
            .max()
            .unwrap_or(segs)
            .max(segs);
        for i in 0..k {
            let (na, nb) = (planes[ring[i]].n, planes[ring[(i + 1) % k]].n);
            // Skip sectors that are thinner than the weld tolerance: a negligible arc, or a sector lying
            // along one great circle (a corner where two faces are almost coplanar, like a flat roof ridge).
            if tiny(na, nb) {
                continue;
            }
            let arc_plane = na.cross(slerp(na, nb, 0.5)).normalize_or(Vec3::ZERO);
            if r * avg.dot(arc_plane).abs() < negligible {
                continue;
            }
            // Grid over the spherical triangle (avg, na, nb): rows from the centre out, columns along the arc
            // (as many as the edge strip that ends here).
            let cols = arc_steps(na, nb, segs, false);
            let dir = |row: u32, col: u32| -> Vec3 {
                let edge = slerp(na, nb, col as f32 / cols as f32);
                slerp(avg, edge, row as f32 / rows as f32)
            };
            for row in 0..rows {
                for col in 0..cols {
                    let d = [
                        dir(row, col),
                        dir(row, col + 1),
                        dir(row + 1, col + 1),
                        dir(row + 1, col),
                    ];
                    let owner = owner_of((d[0] + d[1] + d[2] + d[3]).normalize_or(avg));
                    let c = d.map(|n| corner(owner, centre + n * r, n));
                    b.quad(charts[owner], c[0], c[1], c[2], c[3]);
                }
            }
        }
    }
    inner.verts
}

/// Vertices of the solid shrunk by `r` (the core that a rounded convex collider grows back by `r`).
pub(crate) fn inner_vertices(planes: &[Plane], r: f32, tol: f32) -> Vec<Vec3> {
    shrunk(planes, r, tol).verts
}

/// The solid shrunk by `r`. Corners closer than a tenth of `r` (far below what the bevel shows) are merged,
/// unless that would take a face below three corners; then only float noise is merged.
fn shrunk(planes: &[Plane], r: f32, tol: f32) -> Solid {
    let inset = shrink(planes, r);
    let exact = solid(&inset, tol, tol);
    let merge = 0.1 * r;
    if merge <= tol {
        return exact;
    }
    let merged = solid(&inset, tol, merge);
    let faces = |s: &Solid| s.faces.iter().filter(|f| f.len() >= 3).count();
    if faces(&merged) == faces(&exact) {
        merged
    } else {
        exact
    }
}
