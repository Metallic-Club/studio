//! Fill holes: one n-gon from a loop ("F"), or a quad grid from a loop ("grid fill").

use std::collections::BTreeSet;

use glam::{Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::VertId;
use crate::mesh::Mesh;
use crate::ops::loops::edge_chains;
use crate::selection::Selection;

/// Sum of +1 (agrees) / -1 (disagrees) over the loop's sides for a new face walking the loop in order.
fn loop_consistency(mesh: &Mesh, lp: &[VertId]) -> i64 {
    let n = lp.len();
    let mut s = 0;
    for i in 0..n {
        let (a, b) = (lp[i], lp[(i + 1) % n]);
        let Some(e) = mesh.find_edge(a, b) else { continue };
        for &f in mesh.edge_faces(e) {
            if let Some(face) = mesh.face(f) {
                if face.has_directed_edge(b, a) {
                    s += 1;
                } else if face.has_directed_edge(a, b) {
                    s -= 1;
                }
            }
        }
    }
    s
}

/// Orders loose vertices into a loop by angle around their centre in their best-fit plane.
fn order_by_angle(mesh: &Mesh, verts: &BTreeSet<VertId>) -> Vec<VertId> {
    let pts: Vec<(VertId, Vec3)> = verts.iter().map(|&v| (v, mesh.position_or_zero(v))).collect();
    let c = geom::centroid(&pts.iter().map(|x| x.1).collect::<Vec<_>>());
    // Best-fit normal: largest cross product of spokes.
    let mut n = Vec3::ZERO;
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let cr = (pts[i].1 - c).cross(pts[j].1 - c);
            n += if cr.dot(n) < 0.0 { -cr } else { cr };
        }
    }
    let (u, w) = geom::plane_basis(n.normalize_or(Vec3::Y));
    let mut sorted = pts;
    sorted.sort_by(|a, b| {
        let aa = (a.1 - c).dot(w).atan2((a.1 - c).dot(u));
        let bb = (b.1 - c).dot(w).atan2((b.1 - c).dot(u));
        aa.total_cmp(&bb)
    });
    sorted.into_iter().map(|x| x.0).collect()
}

/// Makes one face from the selection: from a closed (or open, which gets closed) edge loop, or from three or more
/// vertices with no selected edges (ordered around their centre). Two vertices get an edge instead.
/// The new face is wound to agree with neighbouring faces.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::Unsupported`] for several loops.
pub fn fill(mesh: &mut Mesh, sel: &Selection) -> ModelResult<Selection> {
    let mut lp: Vec<VertId> = if !sel.edges.is_empty() {
        let chains = edge_chains(mesh, &sel.edges)?;
        if chains.len() != 1 {
            return Err(ModelError::Unsupported(format!("fill needs one edge loop, found {}", chains.len())));
        }
        chains[0].0.clone()
    } else if sel.verts.len() == 2 {
        let mut it = sel.verts.iter().copied();
        if let (Some(a), Some(b)) = (it.next(), it.next()) {
            mesh.add_edge(a, b)?;
            return Ok(Selection::from_verts(mesh, [a, b]));
        }
        return Err(ModelError::NothingSelected("vertices"));
    } else if sel.verts.len() >= 3 {
        order_by_angle(mesh, &sel.verts)
    } else {
        return Err(ModelError::NothingSelected("an edge loop or three vertices to fill"));
    };
    if lp.len() < 3 {
        return Err(ModelError::Unsupported("fill needs at least three vertices".into()));
    }
    if loop_consistency(mesh, &lp) < 0 {
        lp.reverse();
    } else if loop_consistency(mesh, &lp) == 0 {
        // No neighbours: face the camera-agnostic default, normal pointing away from the mesh centre.
        let n = geom::newell(&lp.iter().map(|&v| mesh.position_or_zero(v)).collect::<Vec<_>>());
        let c = geom::centroid(&lp.iter().map(|&v| mesh.position_or_zero(v)).collect::<Vec<_>>());
        let mesh_c = mesh.bounds().map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) * 0.5);
        if n.dot(c - mesh_c) < 0.0 {
            lp.reverse();
        }
    }
    let f = mesh.add_face(&lp)?;
    Ok(Selection::from_faces(mesh, [f]))
}

/// Fills a closed edge loop with a grid of quads (Coons patch through the loop). The loop needs an even number of
/// vertices (at least 4). `span` is the number of quads along the first side (default: a quarter of the loop) and
/// `offset` rotates where the first side starts.
///
/// # Errors
/// [`ModelError::Unsupported`] for anything but one closed loop with an even vertex count, or a bad span.
pub fn grid_fill(mesh: &mut Mesh, sel: &Selection, span: Option<u32>, offset: u32) -> ModelResult<Selection> {
    let chains = edge_chains(mesh, &sel.edges)?;
    if chains.len() != 1 || !chains[0].1 {
        return Err(ModelError::Unsupported("grid fill needs one closed edge loop".into()));
    }
    let mut lp = chains[0].0.clone();
    let l = lp.len();
    if l < 4 || l % 2 != 0 {
        return Err(ModelError::Unsupported(format!("grid fill needs an even loop of 4+ vertices, found {l}")));
    }
    if loop_consistency(mesh, &lp) < 0 {
        lp.reverse();
    }
    lp.rotate_left(offset as usize % l);
    let half = l / 2;
    let w = span.map_or((l / 4).max(1), |s| s as usize);
    if w == 0 || w >= half {
        return Err(ModelError::InvalidParameter(format!("grid fill span {w} for a loop of {l}")));
    }
    let h = half - w;
    let lpos: Vec<Vec3> = lp.iter().map(|&v| mesh.position_or_zero(v)).collect();
    // Boundary lookup in grid coordinates: index into the loop.
    let boundary = |i: usize, j: usize| -> Option<usize> {
        if j == 0 {
            Some(i)
        } else if i == w {
            Some(w + j)
        } else if j == h {
            Some((w + h + (w - i)) % l)
        } else if i == 0 {
            Some((2 * w + h + (h - j)) % l)
        } else {
            None
        }
    };
    let bpos = |i: usize, j: usize| boundary(i, j).map_or(Vec3::ZERO, |k| lpos[k]);
    let p = |k: usize| lpos[k % l];
    let (p00, p10, p11, p01) = (p(0), p(w), p(w + h), p(2 * w + h));
    let mut grid: Vec<Vec<VertId>> = vec![Vec::new(); w + 1];
    for (i, col) in grid.iter_mut().enumerate() {
        for j in 0..=h {
            let id = if let Some(k) = boundary(i, j) {
                lp[k]
            } else {
                let (u, v) = (i as f32 / w as f32, j as f32 / h as f32);
                let ruled_u = bpos(i, 0) * (1.0 - v) + bpos(i, h) * v;
                let ruled_v = bpos(0, j) * (1.0 - u) + bpos(w, j) * u;
                let bilinear = p00 * (1.0 - u) * (1.0 - v) + p10 * u * (1.0 - v) + p01 * (1.0 - u) * v + p11 * u * v;
                mesh.add_vert(ruled_u + ruled_v - bilinear)
            };
            col.push(id);
        }
    }
    let mut faces = Vec::new();
    for i in 0..w {
        for j in 0..h {
            let quad = [grid[i][j], grid[i + 1][j], grid[i + 1][j + 1], grid[i][j + 1]];
            let uvs = [
                Vec2::new(i as f32 / w as f32, j as f32 / h as f32),
                Vec2::new((i + 1) as f32 / w as f32, j as f32 / h as f32),
                Vec2::new((i + 1) as f32 / w as f32, (j + 1) as f32 / h as f32),
                Vec2::new(i as f32 / w as f32, (j + 1) as f32 / h as f32),
            ];
            faces.push(mesh.add_face_with_uvs(&quad, &uvs)?);
        }
    }
    Ok(Selection::from_faces(mesh, faces))
}
