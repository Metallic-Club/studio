//! UV mapping: box and planar projection, and seam-based unwrapping with Least Squares Conformal Maps (LSCM).
//!
//! LSCM (Lévy, Petitjean, Ray, Maillot 2002) is implemented from the paper's formulation: for each triangle the
//! Cauchy-Riemann residual `Σ W_j U_j / sqrt(area)` is minimised in least squares, with two vertices pinned. The
//! normal equations are solved with Jacobi-preconditioned conjugate gradients in `f64`. Charts are the face groups
//! separated by seam edges; vertices are split along seams inside a chart too. A chart without any boundary (a closed
//! shell with no seams) cannot be flattened and falls back to box projection; the report counts those.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DVec2, DVec3, Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{FaceId, VertId};
use crate::mesh::{FxBuild, Mesh};

/// Box projection: each face is projected along the axis its normal is closest to, `scale` UV units per stud.
///
/// # Errors
/// [`ModelError::MissingFace`].
pub fn box_project(mesh: &mut Mesh, faces: &BTreeSet<FaceId>, scale: f32) -> ModelResult<()> {
    for &f in faces {
        let n = mesh.face_normal(f);
        let pts = mesh.face_positions(f);
        let a = n.abs();
        let uv = |p: Vec3| -> Vec2 {
            if a.x >= a.y && a.x >= a.z {
                if n.x >= 0.0 { Vec2::new(-p.z, p.y) } else { Vec2::new(p.z, p.y) }
            } else if a.y >= a.z {
                if n.y >= 0.0 { Vec2::new(p.x, -p.z) } else { Vec2::new(p.x, p.z) }
            } else if n.z >= 0.0 {
                Vec2::new(p.x, p.y)
            } else {
                Vec2::new(-p.x, p.y)
            }
        };
        let uvs: Vec<Vec2> = pts.iter().map(|&p| uv(p) * scale).collect();
        mesh.set_face_uvs(f, &uvs)?;
    }
    Ok(())
}

/// Planar projection along `normal` (as if a slide projector shone along it), `scale` UV units per stud.
///
/// # Errors
/// [`ModelError::MissingFace`].
pub fn planar_project(mesh: &mut Mesh, faces: &BTreeSet<FaceId>, normal: Vec3, scale: f32) -> ModelResult<()> {
    let (u, v) = geom::plane_basis(normal);
    for &f in faces {
        let uvs: Vec<Vec2> = mesh.face_positions(f).iter().map(|&p| Vec2::new(p.dot(u), p.dot(v)) * scale).collect();
        mesh.set_face_uvs(f, &uvs)?;
    }
    Ok(())
}

/// Options for [`unwrap`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnwrapOptions {
    /// Gap between charts in the final 0..1 layout.
    pub margin: f32,
    /// Conjugate-gradient iteration cap per chart.
    pub max_iterations: usize,
}

impl Default for UnwrapOptions {
    fn default() -> Self {
        Self { margin: 0.02, max_iterations: 4000 }
    }
}

/// What [`unwrap`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnwrapReport {
    /// Charts laid out.
    pub charts: usize,
    /// Charts that had no boundary (closed shells) and were box-projected instead.
    pub fallback_charts: usize,
    /// Largest number of solver iterations any chart needed.
    pub max_iterations_used: usize,
}

/// One flattened chart: faces, per-corner chart vertex index, and solved UVs.
struct Chart {
    faces: Vec<FaceId>,
    corners: Vec<Vec<usize>>,
    uv: Vec<DVec2>,
    area3d: f64,
}

/// Unwraps `faces` into non-overlapping charts packed into the unit square: charts are cut along edges marked seam
/// (and along edges shared with unselected faces), each chart is flattened with LSCM, scaled to keep relative
/// texel density, and shelf-packed.
///
/// # Errors
/// [`ModelError::NothingSelected`] for an empty face set, [`ModelError::MissingFace`].
pub fn unwrap(mesh: &mut Mesh, faces: &BTreeSet<FaceId>, options: UnwrapOptions) -> ModelResult<UnwrapReport> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to unwrap"));
    }
    for &f in faces {
        mesh.face_ref(f)?;
    }
    let linked = |e| {
        let around = mesh.edge_faces(e);
        around.len() == 2
            && around.iter().all(|f| faces.contains(f))
            && !mesh.edge(e).is_some_and(|x| x.flags.seam)
    };
    let mut report = UnwrapReport::default();
    let mut charts: Vec<Chart> = Vec::new();
    let mut fallback: BTreeSet<FaceId> = BTreeSet::new();
    let mut seen: BTreeSet<FaceId> = BTreeSet::new();
    for &seed in faces {
        if !seen.insert(seed) {
            continue;
        }
        let mut comp = vec![seed];
        let mut stack = vec![seed];
        while let Some(f) = stack.pop() {
            for e in mesh.face_edges(f) {
                if !linked(e) {
                    continue;
                }
                for &g in mesh.edge_faces(e) {
                    if seen.insert(g) {
                        comp.push(g);
                        stack.push(g);
                    }
                }
            }
        }
        comp.sort_unstable();
        // Chart vertices: corners of the same vertex joined through linked edges share one chart vertex.
        let comp_set: BTreeSet<FaceId> = comp.iter().copied().collect();
        let mut wedge: HashMap<(VertId, FaceId), usize, FxBuild> = HashMap::default();
        let mut positions: Vec<DVec3> = Vec::new();
        let chart_verts: BTreeSet<VertId> =
            comp.iter().filter_map(|&f| mesh.face(f)).flat_map(|f| f.verts.iter().copied()).collect();
        for &v in &chart_verts {
            let vf: Vec<FaceId> = mesh.vert_faces(v).into_iter().filter(|f| comp_set.contains(f)).collect();
            let mut parent: Vec<usize> = (0..vf.len()).collect();
            for &e in mesh.vert_edges(v) {
                if !linked(e) {
                    continue;
                }
                let around = mesh.edge_faces(e);
                if let (Ok(a), Ok(b)) = (vf.binary_search(&around[0]), vf.binary_search(&around[1])) {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
            let mut ids: BTreeMap<usize, usize> = BTreeMap::new();
            for (i, &f) in vf.iter().enumerate() {
                let r = root(&mut parent, i);
                let idx = *ids.entry(r).or_insert_with(|| {
                    positions.push(mesh.position_or_zero(v).as_dvec3());
                    positions.len() - 1
                });
                wedge.insert((v, f), idx);
            }
        }
        let corners: Vec<Vec<usize>> = comp
            .iter()
            .map(|&f| mesh.face(f).map_or_else(Vec::new, |face| face.verts.iter().map(|&v| wedge[&(v, f)]).collect()))
            .collect();
        // Triangles in chart indices.
        let mut tris: Vec<[usize; 3]> = Vec::new();
        for (k, &f) in comp.iter().enumerate() {
            for t in geom::triangulate_polygon(&mesh.face_positions(f)) {
                tris.push([corners[k][t[0]], corners[k][t[1]], corners[k][t[2]]]);
            }
        }
        // Boundary check: some chart edge must be used by exactly one chart triangle side... use polygon sides.
        let mut side_count: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for c in &corners {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                *side_count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let has_boundary = side_count.values().any(|&n| n == 1);
        let area3d: f64 = comp.iter().map(|&f| f64::from(mesh.face_area(f))).sum();
        if !has_boundary || positions.len() < 3 {
            fallback.extend(comp.iter().copied());
            report.fallback_charts += 1;
            continue;
        }
        let (uv, iters) = lscm(&positions, &tris, options.max_iterations);
        report.max_iterations_used = report.max_iterations_used.max(iters);
        charts.push(Chart { faces: comp, corners, uv, area3d });
    }
    // Box-project the closed charts in place, then treat each of them as one chart for packing.
    if !fallback.is_empty() {
        box_project(mesh, &fallback, 1.0)?;
        let mut seen_fb: BTreeSet<FaceId> = BTreeSet::new();
        for &f in &fallback {
            if !seen_fb.insert(f) {
                continue;
            }
            let face = mesh.face_ref(f)?;
            let uv: Vec<DVec2> = face.uvs.iter().map(|u| u.as_dvec2()).collect();
            let n = uv.len();
            charts.push(Chart { faces: vec![f], corners: vec![(0..n).collect()], uv, area3d: f64::from(mesh.face_area(f)) });
        }
    }
    report.charts = charts.len();
    pack(&mut charts, f64::from(options.margin));
    for chart in &charts {
        for (k, &f) in chart.faces.iter().enumerate() {
            let uvs: Vec<Vec2> = chart.corners[k].iter().map(|&i| chart.uv[i].as_vec2()).collect();
            mesh.set_face_uvs(f, &uvs)?;
        }
    }
    Ok(report)
}

fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Solves LSCM for one chart; returns per-vertex UVs and the iteration count.
fn lscm(pos: &[DVec3], tris: &[[usize; 3]], max_iterations: usize) -> (Vec<DVec2>, usize) {
    let n = pos.len();
    // Pins: the two vertices farthest apart along the longest bounding-box axis.
    let (lo, hi) = pos.iter().fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(l, h), &p| (l.min(p), h.max(p)));
    let ext = hi - lo;
    let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
    let key = |i: usize| pos[i].to_array()[axis];
    let p0 = (0..n).min_by(|&a, &b| key(a).total_cmp(&key(b))).unwrap_or(0);
    let p1 = (0..n).max_by(|&a, &b| key(a).total_cmp(&key(b))).unwrap_or(n - 1);
    let pin_len = pos[p0].distance(pos[p1]).max(1e-9);
    let pinned: [(usize, DVec2); 2] = [(p0, DVec2::ZERO), (p1, DVec2::new(pin_len, 0.0))];
    // Unknown numbering: u_i -> 2i, v_i -> 2i + 1; pinned ones are removed from the system.
    let mut col = vec![usize::MAX; 2 * n];
    let mut free = 0;
    for i in 0..n {
        if pinned.iter().any(|p| p.0 == i) {
            continue;
        }
        col[2 * i] = free;
        col[2 * i + 1] = free + 1;
        free += 2;
    }
    let pinned_value = |var: usize| -> f64 {
        let i = var / 2;
        pinned.iter().find(|p| p.0 == i).map_or(0.0, |p| if var % 2 == 0 { p.1.x } else { p.1.y })
    };
    // Accumulate the normal equations AᵀA x = Aᵀb row by row.
    let mut ata: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); free];
    let mut atb = vec![0.0; free];
    for t in tris {
        let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
        let x = (b - a).normalize_or_zero();
        let nrm = (b - a).cross(c - a);
        let area = nrm.length() * 0.5;
        if area < 1e-14 || x == DVec3::ZERO {
            continue;
        }
        let y = nrm.normalize().cross(x);
        let z = [DVec2::ZERO, DVec2::new((b - a).length(), 0.0), DVec2::new((c - a).dot(x), (c - a).dot(y))];
        let s = 1.0 / area.sqrt();
        // W_j = z_{j+2} - z_{j+1}.
        let w: [DVec2; 3] = [(z[2] - z[1]) * s, (z[0] - z[2]) * s, (z[1] - z[0]) * s];
        // Real row: Σ a_j u_j - b_j v_j ; imaginary row: Σ b_j u_j + a_j v_j.
        for row in 0..2 {
            let mut entries: Vec<(usize, f64)> = Vec::with_capacity(6);
            for j in 0..3 {
                let (wa, wb) = (w[j].x, w[j].y);
                let (cu, cv) = if row == 0 { (wa, -wb) } else { (wb, wa) };
                entries.push((2 * t[j], cu));
                entries.push((2 * t[j] + 1, cv));
            }
            let mut rhs = 0.0;
            for &(var, coef) in &entries {
                if col[var] == usize::MAX {
                    rhs -= coef * pinned_value(var);
                }
            }
            for &(vi, ci) in &entries {
                let r = col[vi];
                if r == usize::MAX {
                    continue;
                }
                atb[r] += ci * rhs;
                for &(vj, cj) in &entries {
                    let c2 = col[vj];
                    if c2 != usize::MAX {
                        *ata[r].entry(c2).or_insert(0.0) += ci * cj;
                    }
                }
            }
        }
    }
    let csr: Vec<Vec<(usize, f64)>> = ata.into_iter().map(|m| m.into_iter().collect()).collect();
    let (x, iters) = conjugate_gradient(&csr, &atb, max_iterations);
    let mut uv = vec![DVec2::ZERO; n];
    for (i, item) in uv.iter_mut().enumerate() {
        *item = if let Some(p) = pinned.iter().find(|p| p.0 == i) {
            p.1
        } else {
            DVec2::new(x[col[2 * i]], x[col[2 * i + 1]])
        };
    }
    // Keep the layout front-facing (counter-clockwise triangles in UV space).
    let signed: f64 = tris
        .iter()
        .map(|t| (uv[t[1]] - uv[t[0]]).perp_dot(uv[t[2]] - uv[t[0]]))
        .sum();
    if signed < 0.0 {
        for p in &mut uv {
            p.x = -p.x;
        }
    }
    (uv, iters)
}

/// Jacobi-preconditioned conjugate gradients for a symmetric positive (semi-)definite sparse matrix.
fn conjugate_gradient(a: &[Vec<(usize, f64)>], b: &[f64], max_iterations: usize) -> (Vec<f64>, usize) {
    let n = b.len();
    let mut x = vec![0.0; n];
    if n == 0 {
        return (x, 0);
    }
    let mul = |v: &[f64], out: &mut [f64]| {
        for (i, row) in a.iter().enumerate() {
            out[i] = row.iter().map(|&(j, c)| c * v[j]).sum();
        }
    };
    let diag: Vec<f64> = a
        .iter()
        .enumerate()
        .map(|(i, row)| row.iter().find(|e| e.0 == i).map_or(1.0, |e| if e.1.abs() > 1e-30 { e.1 } else { 1.0 }))
        .collect();
    let mut r = b.to_vec();
    let mut z: Vec<f64> = r.iter().zip(&diag).map(|(r, d)| r / d).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut ap = vec![0.0; n];
    let mut iters = 0;
    for k in 0..max_iterations.max(1) {
        iters = k + 1;
        mul(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        if r_norm / b_norm < 1e-10 {
            break;
        }
        for i in 0..n {
            z[i] = r[i] / diag[i];
        }
        let rz_new: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = rz_new / rz;
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    (x, iters)
}

/// Scales every chart to its 3D area, then shelf-packs all charts into the unit square with `margin` between them.
fn pack(charts: &mut [Chart], margin: f64) {
    if charts.is_empty() {
        return;
    }
    // Normalise each chart: origin at its min corner, scaled so UV area matches 3D area.
    let mut sizes: Vec<DVec2> = Vec::with_capacity(charts.len());
    for chart in charts.iter_mut() {
        let (lo, hi) = chart.uv.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(l, h), &p| (l.min(p), h.max(p)));
        let mut uv_area = 0.0;
        for c in &chart.corners {
            let poly: Vec<Vec2> = c.iter().map(|&i| chart.uv[i].as_vec2()).collect();
            uv_area += f64::from(geom::signed_area_2d(&poly).abs());
        }
        let s = if uv_area > 1e-20 { (chart.area3d / uv_area).sqrt() } else { 1.0 };
        for p in &mut chart.uv {
            *p = (*p - lo) * s;
        }
        sizes.push((hi - lo) * s);
    }
    // The margin is a fraction of the final square; express it in chart units.
    let margin = margin * sizes.iter().map(|s| s.x * s.y).sum::<f64>().sqrt().max(1e-9);
    let total: f64 = sizes.iter().map(|s| (s.x + margin) * (s.y + margin)).sum();
    let widest = sizes.iter().map(|s| s.x).fold(0.0, f64::max);
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| sizes[b].y.total_cmp(&sizes[a].y).then(a.cmp(&b)));
    let mut side = total.sqrt().max(widest + margin);
    // Grow the square until the shelves fit.
    let offsets = loop {
        let mut offsets = vec![DVec2::ZERO; charts.len()];
        let (mut x, mut y, mut shelf) = (margin, margin, 0.0f64);
        for &i in &order {
            let s = sizes[i];
            if x + s.x + margin > side && x > margin {
                x = margin;
                y += shelf + margin;
                shelf = 0.0;
            }
            offsets[i] = DVec2::new(x, y);
            x += s.x + margin;
            shelf = shelf.max(s.y);
        }
        if y + shelf + margin <= side {
            break offsets;
        }
        side *= 1.1;
    };
    for (i, chart) in charts.iter_mut().enumerate() {
        for p in &mut chart.uv {
            *p = (*p + offsets[i]) / side;
        }
    }
}
