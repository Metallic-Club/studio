//! Edge loops and rings: loop select, ring select, loop cut and bridge edge loops.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// The edge loop through `e`: across four-edge interior vertices on a quad mesh, or along the boundary when `e` is a
/// boundary edge. Stops at poles (vertices with other valences), wire edges and non-manifold spots.
#[must_use]
pub fn edge_loop(mesh: &Mesh, e: EdgeId) -> Vec<EdgeId> {
    let Some(edge) = mesh.edge(e) else { return Vec::new() };
    let boundary = mesh.is_boundary_edge(e);
    let mut out = vec![e];
    let mut seen = BTreeSet::from([e]);
    for start in edge.verts {
        let mut cur = e;
        let mut v = start;
        loop {
            let next = if boundary { next_boundary_edge(mesh, cur, v) } else { next_loop_edge(mesh, cur, v) };
            let Some(n) = next else { break };
            if !seen.insert(n) {
                break;
            }
            out.push(n);
            cur = n;
            v = mesh.edge(n).map_or(v, |x| x.other(v));
        }
    }
    out.sort_unstable();
    out
}

fn next_loop_edge(mesh: &Mesh, cur: EdgeId, v: VertId) -> Option<EdgeId> {
    let fan = mesh.vert_fan(v)?;
    if !fan.closed || fan.edges.len() != 4 {
        return None;
    }
    let i = fan.edges.iter().position(|&x| x == cur)?;
    Some(fan.edges[(i + 2) % 4])
}

fn next_boundary_edge(mesh: &Mesh, cur: EdgeId, v: VertId) -> Option<EdgeId> {
    let candidates: Vec<EdgeId> = mesh.vert_edges(v).iter().copied().filter(|&x| x != cur && mesh.is_boundary_edge(x)).collect();
    if candidates.len() == 1 { Some(candidates[0]) } else { None }
}

/// The edge ring through `e`: edges facing each other across quads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ring {
    /// Ring edges in walking order.
    pub edges: Vec<EdgeId>,
    /// The quads between consecutive ring edges.
    pub faces: Vec<FaceId>,
    /// Whether the ring goes all the way round.
    pub closed: bool,
}

fn opposite_edge(mesh: &Mesh, quad: FaceId, e: EdgeId) -> Option<EdgeId> {
    let edges = mesh.face_edges(quad);
    if edges.len() != 4 {
        return None;
    }
    let i = edges.iter().position(|&x| x == e)?;
    Some(edges[(i + 2) % 4])
}

/// Walks the ring through `e` in both directions.
#[must_use]
pub fn edge_ring(mesh: &Mesh, e: EdgeId) -> Ring {
    let mut ring = Ring { edges: vec![e], faces: Vec::new(), closed: false };
    if mesh.edge(e).is_none() {
        return ring;
    }
    let start_faces: Vec<FaceId> = mesh.edge_faces(e).to_vec();
    if start_faces.is_empty() || start_faces.len() > 2 {
        return ring;
    }
    let walk = |first: FaceId, edges: &mut Vec<EdgeId>, faces: &mut Vec<FaceId>| -> bool {
        let mut cur_e = e;
        let mut cur_f = first;
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > mesh.face_count() + 2 {
                return false;
            }
            let Some(o) = opposite_edge(mesh, cur_f, cur_e) else { return false };
            faces.push(cur_f);
            if o == e {
                return true;
            }
            if edges.contains(&o) {
                return false;
            }
            edges.push(o);
            let around = mesh.edge_faces(o);
            if around.len() != 2 {
                return false;
            }
            let next = if around[0] == cur_f { around[1] } else { around[0] };
            cur_e = o;
            cur_f = next;
        }
    };
    let mut fwd_edges = Vec::new();
    let mut fwd_faces = Vec::new();
    let closed = walk(start_faces[0], &mut fwd_edges, &mut fwd_faces);
    if closed {
        ring.edges.extend(fwd_edges);
        ring.faces = fwd_faces;
        ring.closed = true;
        return ring;
    }
    let mut back_edges = Vec::new();
    let mut back_faces = Vec::new();
    if start_faces.len() == 2 {
        walk(start_faces[1], &mut back_edges, &mut back_faces);
    }
    let mut edges: Vec<EdgeId> = back_edges.into_iter().rev().collect();
    edges.push(e);
    edges.extend(fwd_edges);
    let mut faces: Vec<FaceId> = back_faces.into_iter().rev().collect();
    faces.extend(fwd_faces);
    // A ring that hit the same edge from both sides would repeat it; keep the first occurrence.
    let mut seen = BTreeSet::new();
    edges.retain(|x| seen.insert(*x));
    let mut seen_f = BTreeSet::new();
    faces.retain(|x| seen_f.insert(*x));
    ring.edges = edges;
    ring.faces = faces;
    ring
}

/// Cuts `cuts` evenly spaced edge loops across the ring through `e`. Faces at the ends of an open ring (n-gons,
/// triangles) receive the new vertices so no holes or T-junctions appear. Returns the new loop edges selected.
///
/// # Errors
/// [`ModelError::MissingEdge`], [`ModelError::InvalidParameter`] for zero cuts, or [`ModelError::Unsupported`] when no
/// quad touches `e`.
pub fn loop_cut(mesh: &mut Mesh, e: EdgeId, cuts: u32) -> ModelResult<Selection> {
    mesh.edge_ref(e)?;
    if cuts == 0 || cuts > 256 {
        return Err(ModelError::InvalidParameter(format!("loop cut count {cuts}")));
    }
    let ring = edge_ring(mesh, e);
    if ring.faces.is_empty() {
        return Err(ModelError::Unsupported("loop cut needs a quad next to the edge".into()));
    }
    let ring_set: BTreeSet<EdgeId> = ring.edges.iter().copied().collect();
    let n = cuts as usize;
    // Split points along each ring edge, from verts[0] to verts[1].
    let mut splits: BTreeMap<EdgeId, Vec<VertId>> = BTreeMap::new();
    for &r in &ring.edges {
        let edge = *mesh.edge_ref(r)?;
        let (p0, p1) = (mesh.position(edge.verts[0])?, mesh.position(edge.verts[1])?);
        let pts = (1..=n).map(|k| mesh.add_vert(p0.lerp(p1, k as f32 / (n + 1) as f32))).collect();
        splits.insert(r, pts);
    }
    let along = |mesh: &Mesh, a: VertId, b: VertId| -> ModelResult<Vec<VertId>> {
        let r = mesh.find_edge(a, b).ok_or(ModelError::Invariant("ring edge missing".into()))?;
        let pts = splits.get(&r).ok_or(ModelError::Invariant("split missing".into()))?;
        let forward = mesh.edge_ref(r)?.verts[0] == a;
        let mut seq = vec![a];
        if forward {
            seq.extend(pts.iter().copied());
        } else {
            seq.extend(pts.iter().rev().copied());
        }
        seq.push(b);
        Ok(seq)
    };
    let ring_faces: BTreeSet<FaceId> = ring.faces.iter().copied().collect();
    let mut loops: Vec<Vec<(VertId, VertId)>> = vec![Vec::new(); n];
    for &f in &ring.faces {
        let face = mesh.face_ref(f)?.clone();
        let vs = &face.verts;
        let side_in_ring = |i: usize| mesh.find_edge(vs[i], vs[(i + 1) % 4]).is_some_and(|x| ring_set.contains(&x));
        let rot = (0..4)
            .find(|&i| side_in_ring(i) && side_in_ring((i + 2) % 4))
            .ok_or(ModelError::Invariant("ring quad without opposite ring sides".into()))?;
        if side_in_ring((rot + 1) % 4) && side_in_ring((rot + 3) % 4) {
            return Err(ModelError::Unsupported("the ring crosses the same quad twice".into()));
        }
        let idx = |k: usize| (rot + k) % 4;
        let (a, b, c, d) = (vs[idx(0)], vs[idx(1)], vs[idx(2)], vs[idx(3)]);
        let (ua, ub, uc, ud) = (face.uvs[idx(0)], face.uvs[idx(1)], face.uvs[idx(2)], face.uvs[idx(3)]);
        let p = along(mesh, a, b)?;
        let q = along(mesh, d, c)?;
        for k in 0..=n {
            let t0 = k as f32 / (n + 1) as f32;
            let t1 = (k + 1) as f32 / (n + 1) as f32;
            let quad = [p[k], p[k + 1], q[k + 1], q[k]];
            let uvs = [ua.lerp(ub, t0), ua.lerp(ub, t1), ud.lerp(uc, t1), ud.lerp(uc, t0)];
            if k == 0 {
                mesh.set_face(f, &quad, &uvs)?;
            } else {
                mesh.add_face_with_uvs(&quad, &uvs)?;
            }
        }
        for k in 1..=n {
            loops[k - 1].push((p[k], q[k]));
        }
    }
    // Faces outside the ring that use a ring edge get the split points inserted.
    let mut others: BTreeSet<FaceId> = BTreeSet::new();
    for &r in &ring.edges {
        others.extend(mesh.edge_faces(r).iter().copied().filter(|f| !ring_faces.contains(f)));
    }
    for f in others {
        let face = mesh.face_ref(f)?.clone();
        let mut verts = Vec::new();
        let mut uvs = Vec::new();
        let m = face.verts.len();
        for i in 0..m {
            let (a, b) = (face.verts[i], face.verts[(i + 1) % m]);
            verts.push(a);
            uvs.push(face.uvs[i]);
            if mesh.find_edge(a, b).is_some_and(|x| ring_set.contains(&x)) {
                let seq = along(mesh, a, b)?;
                let (ua, ub) = (face.uvs[i], face.uvs[(i + 1) % m]);
                for (k, &v) in seq[1..seq.len() - 1].iter().enumerate() {
                    verts.push(v);
                    uvs.push(ua.lerp(ub, (k + 1) as f32 / (n + 1) as f32));
                }
            }
        }
        mesh.set_face(f, &verts, &uvs)?;
    }
    mesh.remove_loose(ring.edges.iter().copied(), std::iter::empty());
    let new_edges: Vec<EdgeId> = loops.iter().flatten().filter_map(|&(a, b)| mesh.find_edge(a, b)).collect();
    Ok(Selection::from_edges(mesh, new_edges))
}

/// Splits a set of edges into connected chains and orders each chain's vertices. Returns `(vertices, closed)`.
///
/// # Errors
/// [`ModelError::Unsupported`] when a chain branches (a vertex with three or more selected edges).
pub fn edge_chains(mesh: &Mesh, edges: &BTreeSet<EdgeId>) -> ModelResult<Vec<(Vec<VertId>, bool)>> {
    let mut adj: BTreeMap<VertId, Vec<VertId>> = BTreeMap::new();
    for &e in edges {
        let edge = mesh.edge_ref(e)?;
        adj.entry(edge.verts[0]).or_default().push(edge.verts[1]);
        adj.entry(edge.verts[1]).or_default().push(edge.verts[0]);
    }
    if adj.values().any(|n| n.len() > 2) {
        return Err(ModelError::Unsupported("selected edges branch; select simple loops".into()));
    }
    let mut used: BTreeSet<VertId> = BTreeSet::new();
    let mut chains = Vec::new();
    // Open chains first (start at an end), then closed loops.
    let starts: Vec<VertId> = adj.iter().filter(|(_, n)| n.len() == 1).map(|(&v, _)| v).chain(adj.keys().copied()).collect();
    for s in starts {
        if used.contains(&s) {
            continue;
        }
        let mut chain = vec![s];
        used.insert(s);
        let mut prev: Option<VertId> = None;
        let mut cur = s;
        let mut closed = false;
        loop {
            let next = adj[&cur].iter().copied().find(|&n| Some(n) != prev && !used.contains(&n));
            match next {
                Some(n) => {
                    used.insert(n);
                    chain.push(n);
                    prev = Some(cur);
                    cur = n;
                }
                None => {
                    if chain.len() > 2 && adj[&cur].contains(&s) {
                        closed = true;
                    }
                    break;
                }
            }
        }
        chains.push((chain, closed));
    }
    Ok(chains)
}

/// Connects two selected edge loops (or two open chains) with a band of quads. Loops must have the same vertex
/// count. The pairing that keeps winding consistent with the faces next to the loops wins; among those, the twist
/// with the shortest total distance. Returns the new faces selected.
///
/// # Errors
/// [`ModelError::Unsupported`] unless the selection is exactly two simple loops/chains of equal length.
pub fn bridge_edge_loops(mesh: &mut Mesh, edges: &BTreeSet<EdgeId>) -> ModelResult<Selection> {
    if edges.is_empty() {
        return Err(ModelError::NothingSelected("two edge loops to bridge"));
    }
    let chains = edge_chains(mesh, edges)?;
    if chains.len() != 2 {
        return Err(ModelError::Unsupported(format!("bridge needs exactly two edge loops, found {}", chains.len())));
    }
    let (l, l_closed) = chains[0].clone();
    let (m, m_closed) = chains[1].clone();
    if l_closed != m_closed || l.len() != m.len() || l.len() < 2 {
        return Err(ModelError::Unsupported(format!(
            "bridge needs two loops of the same kind and length ({} vs {} vertices)",
            l.len(),
            m.len()
        )));
    }
    let closed = l_closed;
    let n = l.len();
    let segs = if closed { n } else { n - 1 };
    let pos = |v: VertId| mesh.position_or_zero(v);
    let mut best: Option<(i64, f32, Vec<VertId>, bool)> = None;
    for reversed in [false, true] {
        let mut mm = m.clone();
        if reversed {
            mm.reverse();
        }
        let rotations: Vec<usize> = if closed { (0..n).collect() } else { vec![0] };
        for flip in [false, true] {
            // Winding consistency with existing faces.
            let mut score = 0i64;
            for i in 0..segs {
                let (a, b) = (l[i], l[(i + 1) % n]);
                score += consistency(mesh, a, b, flip);
            }
            for &r in &rotations {
                let mr: Vec<VertId> = (0..n).map(|i| mm[(i + r) % n]).collect();
                let mut s = score;
                let mut dist = 0.0;
                for i in 0..segs {
                    let (c, d) = (mr[(i + 1) % n], mr[i]);
                    s += consistency(mesh, c, d, flip);
                }
                for i in 0..n {
                    dist += pos(l[i]).distance_squared(pos(mr[i]));
                }
                let better = match &best {
                    None => true,
                    Some((bs, bd, _, _)) => s > *bs || (s == *bs && dist < *bd - 1e-6),
                };
                if better {
                    best = Some((s, dist, mr, flip));
                }
            }
        }
    }
    let Some((_, _, mr, flip)) = best else {
        return Err(ModelError::Invariant("bridge found no pairing".into()));
    };
    let mut faces = Vec::new();
    for i in 0..segs {
        let j = (i + 1) % n;
        let mut quad = vec![l[i], l[j], mr[j], mr[i]];
        if flip {
            quad.reverse();
        }
        quad.dedup();
        if quad.len() >= 3 {
            let uvs = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
            faces.push(mesh.add_face_with_uvs(&quad, &uvs[..quad.len()])?);
        }
    }
    Ok(Selection::from_faces(mesh, faces))
}

/// +1 when a new face walking `a -> b` (or `b -> a` if `flip`) agrees with the existing face on that edge, -1 when it
/// disagrees, 0 when the edge has no face.
fn consistency(mesh: &Mesh, a: VertId, b: VertId, flip: bool) -> i64 {
    let (a, b) = if flip { (b, a) } else { (a, b) };
    let Some(e) = mesh.find_edge(a, b) else { return 0 };
    let mut s = 0;
    for &f in mesh.edge_faces(e) {
        if let Some(face) = mesh.face(f) {
            if face.has_directed_edge(b, a) {
                s += 1;
            } else if face.has_directed_edge(a, b) {
                s -= 1;
            }
        }
    }
    s
}

/// Average position of a loop (used by callers for previews).
#[must_use]
pub fn loop_center(mesh: &Mesh, verts: &[VertId]) -> Vec3 {
    crate::geom::centroid(&verts.iter().map(|&v| mesh.position_or_zero(v)).collect::<Vec<_>>())
}
