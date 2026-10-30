//! Dissolve vertices, edges or faces: remove them while keeping the surface closed by merging neighbouring faces.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec2;

use crate::error::{ModelError, ModelResult};
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// The boundary of a set of faces as closed corner loops `(vertex, uv)`, walking in the faces' winding direction.
/// Sides for which `interior(edge)` is true are skipped.
///
/// # Errors
/// [`ModelError::Unsupported`] when the boundary pinches (a vertex with two outgoing boundary sides).
pub fn region_boundary(
    mesh: &Mesh,
    region: &BTreeSet<FaceId>,
    interior: impl Fn(EdgeId) -> bool,
) -> ModelResult<Vec<Vec<(VertId, Vec2)>>> {
    let mut next: BTreeMap<VertId, (VertId, Vec2)> = BTreeMap::new();
    for &f in region {
        let face = mesh.face_ref(f)?;
        for (i, (a, b)) in face.directed_edges().enumerate() {
            let e = mesh.find_edge(a, b).ok_or(ModelError::Invariant("face side without edge".into()))?;
            if interior(e) {
                continue;
            }
            if next.insert(a, (b, face.uvs[i])).is_some() {
                return Err(ModelError::Unsupported("the region boundary touches itself at a vertex".into()));
            }
        }
    }
    let mut loops = Vec::new();
    let mut used: BTreeSet<VertId> = BTreeSet::new();
    for &start in next.keys() {
        if used.contains(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = start;
        loop {
            if !used.insert(cur) {
                break;
            }
            let Some(&(n, uv)) = next.get(&cur) else {
                return Err(ModelError::Unsupported("the region boundary is not closed".into()));
            };
            lp.push((cur, uv));
            cur = n;
        }
        if cur != start {
            return Err(ModelError::Unsupported("the region boundary is not a simple loop".into()));
        }
        loops.push(lp);
    }
    Ok(loops)
}

/// Replaces a connected group of faces by one n-gon along its single boundary loop. The lowest face id survives.
fn merge_region(mesh: &mut Mesh, region: &BTreeSet<FaceId>, interior: &dyn Fn(EdgeId) -> bool) -> ModelResult<FaceId> {
    let loops = region_boundary(mesh, region, interior)?;
    if loops.len() != 1 {
        return Err(ModelError::Unsupported(format!("the region has {} boundary loops (holes are not dissolved)", loops.len())));
    }
    let lp = &loops[0];
    let keep = *region.iter().next().ok_or(ModelError::NothingSelected("faces"))?;
    let mut inner_edges: BTreeSet<EdgeId> = BTreeSet::new();
    let mut inner_verts: BTreeSet<VertId> = BTreeSet::new();
    for &f in region {
        inner_edges.extend(mesh.face_edges(f));
        inner_verts.extend(mesh.face_ref(f)?.verts.iter().copied());
    }
    let verts: Vec<VertId> = lp.iter().map(|x| x.0).collect();
    let uvs: Vec<Vec2> = lp.iter().map(|x| x.1).collect();
    for &f in region.iter().skip(1) {
        mesh.remove_face(f)?;
    }
    mesh.set_face(keep, &verts, &uvs)?;
    mesh.remove_loose(inner_edges, inner_verts);
    Ok(keep)
}

/// Groups faces into components connected through edges for which `link` is true.
fn components(mesh: &Mesh, faces: &BTreeSet<FaceId>, link: &dyn Fn(EdgeId) -> bool) -> Vec<BTreeSet<FaceId>> {
    let mut seen: BTreeSet<FaceId> = BTreeSet::new();
    let mut out = Vec::new();
    for &f in faces {
        if !seen.insert(f) {
            continue;
        }
        let mut comp = BTreeSet::from([f]);
        let mut stack = vec![f];
        while let Some(g) = stack.pop() {
            for e in mesh.face_edges(g) {
                if !link(e) {
                    continue;
                }
                for &h in mesh.edge_faces(e) {
                    if faces.contains(&h) && seen.insert(h) {
                        comp.insert(h);
                        stack.push(h);
                    }
                }
            }
        }
        out.push(comp);
    }
    out
}

/// Dissolves faces: each connected group of selected faces becomes one n-gon (inner edges and vertices go away).
///
/// # Errors
/// [`ModelError::NothingSelected`], or [`ModelError::Unsupported`] for a group with holes or a pinched boundary.
pub fn dissolve_faces(mesh: &mut Mesh, faces: &BTreeSet<FaceId>) -> ModelResult<Selection> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to dissolve"));
    }
    let snapshot = mesh.clone();
    let interior = |e: EdgeId| {
        let around = snapshot.edge_faces(e);
        around.len() == 2 && around.iter().all(|f| faces.contains(f))
    };
    let mut kept = Vec::new();
    for comp in components(&snapshot, faces, &interior) {
        if comp.len() < 2 {
            kept.extend(comp);
            continue;
        }
        kept.push(merge_region(mesh, &comp, &interior)?);
    }
    Ok(Selection::from_faces(mesh, kept))
}

/// Dissolves edges: the two faces on each selected edge merge (connected groups merge into one face). Edges without
/// exactly two faces are skipped; wire edges are removed.
///
/// # Errors
/// [`ModelError::NothingSelected`], or [`ModelError::Unsupported`] when a merged group would have holes.
pub fn dissolve_edges(mesh: &mut Mesh, edges: &BTreeSet<EdgeId>) -> ModelResult<Selection> {
    if edges.is_empty() {
        return Err(ModelError::NothingSelected("edges to dissolve"));
    }
    for &e in edges {
        mesh.edge_ref(e)?;
    }
    let snapshot = mesh.clone();
    let dissolvable = |e: EdgeId| edges.contains(&e) && snapshot.edge_faces(e).len() == 2;
    let faces: BTreeSet<FaceId> =
        edges.iter().filter(|&&e| dissolvable(e)).flat_map(|&e| snapshot.edge_faces(e).iter().copied()).collect();
    let mut kept = Vec::new();
    for comp in components(&snapshot, &faces, &dissolvable) {
        if comp.len() < 2 {
            continue;
        }
        kept.push(merge_region(mesh, &comp, &dissolvable)?);
    }
    let wires: Vec<EdgeId> = edges.iter().copied().filter(|&e| mesh.edge(e).is_some() && mesh.edge_faces(e).is_empty()).collect();
    mesh.remove_loose(wires, std::iter::empty());
    Ok(Selection::from_faces(mesh, kept))
}

/// Dissolves vertices: a vertex inside a surface is removed and its faces merge into one; a vertex in the middle of
/// a chain (two edges) is removed and its neighbours joined.
///
/// # Errors
/// [`ModelError::NothingSelected`], [`ModelError::NonManifold`] for a vertex whose fan is not a disc or half-disc,
/// or [`ModelError::Unsupported`] when a face would drop below three corners.
pub fn dissolve_verts(mesh: &mut Mesh, verts: &BTreeSet<VertId>) -> ModelResult<Selection> {
    if verts.is_empty() {
        return Err(ModelError::NothingSelected("vertices to dissolve"));
    }
    let mut kept = Vec::new();
    for &v in verts {
        if mesh.vert(v).is_none() {
            continue;
        }
        let faces = mesh.vert_faces(v);
        let edges = mesh.vert_edges(v).to_vec();
        if faces.is_empty() {
            if edges.len() == 2 {
                let a = mesh.edge_ref(edges[0])?.other(v);
                let b = mesh.edge_ref(edges[1])?.other(v);
                mesh.remove_vert(v)?;
                if a != b {
                    mesh.add_edge(a, b)?;
                }
            } else {
                mesh.remove_vert(v)?;
            }
            continue;
        }
        if edges.len() == 2 {
            // A corner in the middle of a side: drop it from every face.
            for f in faces {
                let face = mesh.face_ref(f)?.clone();
                if face.verts.len() <= 3 {
                    return Err(ModelError::Unsupported("dissolving this vertex would collapse a triangle".into()));
                }
                let i = face.position_of(v).ok_or(ModelError::Invariant("vertex not in face".into()))?;
                let mut vs = face.verts.clone();
                let mut uvs = face.uvs.clone();
                vs.remove(i);
                uvs.remove(i);
                mesh.set_face(f, &vs, &uvs)?;
                kept.push(f);
            }
            mesh.remove_vert(v)?;
            continue;
        }
        let fan = mesh.vert_fan(v).ok_or_else(|| ModelError::NonManifold(format!("vertex {v:?} is not a manifold fan")))?;
        let region: BTreeSet<FaceId> = fan.faces.iter().copied().collect();
        let around: BTreeSet<EdgeId> = fan.edges.iter().copied().collect();
        let open = !fan.closed;
        // Interior: the spokes, except the two boundary spokes of an open fan.
        let interior = |e: EdgeId| around.contains(&e) && mesh.edge_faces(e).len() == 2;
        let mut loops = region_boundary(mesh, &region, interior)?;
        if loops.len() != 1 {
            return Err(ModelError::Unsupported("vertex fan has a hole".into()));
        }
        let mut lp = loops.remove(0);
        if open {
            lp.retain(|x| x.0 != v);
        }
        if lp.len() < 3 {
            return Err(ModelError::Unsupported("dissolving this vertex leaves fewer than three corners".into()));
        }
        let keep = fan.faces.iter().copied().min().ok_or(ModelError::Invariant("empty fan".into()))?;
        for &f in &region {
            if f != keep {
                mesh.remove_face(f)?;
            }
        }
        let vs: Vec<VertId> = lp.iter().map(|x| x.0).collect();
        let uvs: Vec<Vec2> = lp.iter().map(|x| x.1).collect();
        mesh.set_face(keep, &vs, &uvs)?;
        mesh.remove_vert(v)?;
        kept.push(keep);
    }
    Ok(Selection::from_faces(mesh, kept))
}
