//! Edge bevel with any number of segments (a rounded profile).
//!
//! Construction (own design; Blender's bevel was only read about in its user docs):
//! 1. Every beveled edge is replaced by a strip whose two rails run `width` away from the edge inside each of its two
//!    faces. At a vertex, a rail ends where it meets the next edge of that face: on a non-beveled edge the meeting
//!    point slides along that edge ("slide point"); between two beveled edges it is the point `width` away from both
//!    ("corner point").
//! 2. Every face around a beveled vertex swaps that corner for its slide/corner points.
//! 3. With more than one segment the strip follows a quarter-ellipse profile from rail to rail that is tangent to
//!    both faces (a quarter circle for right angles).
//! 4. What is left open around a vertex (the ring of new points) becomes one "vertex polygon", except the common case
//!    of a single beveled edge ending at a vertex: there the faces that close the corner share the profile points
//!    (with more than one such face, the edges between them move onto the profile).

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::{EdgeFlags, Fan, Mesh};
use crate::selection::Selection;

/// Most segments a bevel may have.
pub const MAX_BEVEL_SEGMENTS: u32 = 64;

struct Ctx {
    beveled: BTreeSet<EdgeId>,
    /// Slide point of a non-beveled edge at a vertex (absent: the original vertex is reused).
    slide: BTreeMap<(VertId, EdgeId), VertId>,
    /// Corner point of a face whose two edges at a vertex are both beveled.
    corner: BTreeMap<(VertId, FaceId), VertId>,
}

impl Ctx {
    /// The point that replaces vertex `v` on edge `e` inside face `f`.
    fn point(&self, mesh: &Mesh, v: VertId, e: EdgeId, f: FaceId) -> ModelResult<VertId> {
        if !self.beveled.contains(&e) {
            return Ok(self.slide.get(&(v, e)).copied().unwrap_or(v));
        }
        let other = other_edge_at(mesh, f, v, e)?;
        if self.beveled.contains(&other) {
            self.corner.get(&(v, f)).copied().ok_or_else(|| ModelError::Invariant("bevel corner point missing".into()))
        } else {
            Ok(self.slide.get(&(v, other)).copied().unwrap_or(v))
        }
    }
}

/// The incoming and outgoing edges of face `f` at corner `v`.
fn corner_edges(mesh: &Mesh, f: FaceId, v: VertId) -> ModelResult<(EdgeId, EdgeId)> {
    let face = mesh.face_ref(f)?;
    let (prev, next) = face.neighbours(v).ok_or(ModelError::Invariant("vertex not in face".into()))?;
    let e_in = mesh.find_edge(prev, v).ok_or(ModelError::Invariant("missing edge".into()))?;
    let e_out = mesh.find_edge(v, next).ok_or(ModelError::Invariant("missing edge".into()))?;
    Ok((e_in, e_out))
}

fn other_edge_at(mesh: &Mesh, f: FaceId, v: VertId, e: EdgeId) -> ModelResult<EdgeId> {
    let (e_in, e_out) = corner_edges(mesh, f, v)?;
    Ok(if e_in == e { e_out } else { e_in })
}

fn edge_dir(mesh: &Mesh, e: EdgeId, from: VertId) -> ModelResult<Vec3> {
    let edge = mesh.edge_ref(e)?;
    Ok((mesh.position(edge.other(from))? - mesh.position(from)?).normalize_or_zero())
}

/// Bevels `edges` with the given `width` (distance from the edge to each rail) and `segments` (1 = flat chamfer).
///
/// Each edge needs exactly two consistently wound faces, and every end vertex a manifold fan. Returns the new strip
/// faces selected.
///
/// # Errors
/// [`ModelError::NothingSelected`], [`ModelError::InvalidParameter`], [`ModelError::NonManifold`], or
/// [`ModelError::Unsupported`] when the width is so large that new points overlap.
pub fn bevel_edges(mesh: &mut Mesh, edges: &BTreeSet<EdgeId>, width: f32, segments: u32) -> ModelResult<Selection> {
    if edges.is_empty() {
        return Err(ModelError::NothingSelected("edges to bevel"));
    }
    if !(width.is_finite() && width > 0.0) || segments == 0 || segments > MAX_BEVEL_SEGMENTS {
        return Err(ModelError::InvalidParameter(format!("bevel width {width}, segments {segments}")));
    }
    // Validate and gather.
    let mut verts: BTreeSet<VertId> = BTreeSet::new();
    let mut sides: BTreeMap<EdgeId, (FaceId, FaceId)> = BTreeMap::new();
    for &e in edges {
        let edge = *mesh.edge_ref(e)?;
        let faces = mesh.edge_faces(e);
        if faces.len() != 2 {
            return Err(ModelError::NonManifold(format!("bevel needs two faces on edge {e:?}, found {}", faces.len())));
        }
        let [a, b] = edge.verts;
        let f1 = faces.iter().copied().find(|&f| mesh.face(f).is_some_and(|x| x.has_directed_edge(a, b)));
        let f2 = faces.iter().copied().find(|&f| mesh.face(f).is_some_and(|x| x.has_directed_edge(b, a)));
        let (Some(f1), Some(f2)) = (f1, f2) else {
            return Err(ModelError::NonManifold(format!("faces around edge {e:?} disagree on winding")));
        };
        sides.insert(e, (f1, f2));
        verts.extend(edge.verts);
    }
    let mut fans: BTreeMap<VertId, Fan> = BTreeMap::new();
    for &v in &verts {
        let fan = mesh.vert_fan(v).ok_or_else(|| ModelError::NonManifold(format!("vertex {v:?} is not a manifold fan")))?;
        fans.insert(v, fan);
    }
    let mut ctx = Ctx { beveled: edges.clone(), slide: BTreeMap::new(), corner: BTreeMap::new() };

    // 1. Slide points on non-beveled edges, corner points between two beveled edges.
    for (&v, fan) in &fans {
        let p = mesh.position(v)?;
        for &e in &fan.edges {
            if ctx.beveled.contains(&e) {
                continue;
            }
            let d = edge_dir(mesh, e, v)?;
            let far = mesh.edge_ref(e)?.other(v);
            let len = mesh.position(far)?.distance(p);
            // Both ends sliding along the same edge must not cross.
            let limit = if verts.contains(&far) { 0.499 } else { 0.999 };
            let mut slides = Vec::new();
            for &f in mesh.edge_faces(e) {
                let other = other_edge_at(mesh, f, v, e)?;
                if ctx.beveled.contains(&other) {
                    let sin = d.cross(edge_dir(mesh, other, v)?).length();
                    if sin > 1e-4 {
                        slides.push(width / sin);
                    }
                }
            }
            if !slides.is_empty() {
                let s = slides.iter().sum::<f32>() / slides.len() as f32;
                if s >= len * limit {
                    return Err(ModelError::Unsupported(format!("bevel width {width} is wider than a neighbouring edge")));
                }
                let id = mesh.add_vert(p + d * s);
                ctx.slide.insert((v, e), id);
            }
        }
        for &f in &fan.faces {
            let (e_in, e_out) = corner_edges(mesh, f, v)?;
            if ctx.beveled.contains(&e_in) && ctx.beveled.contains(&e_out) {
                let d1 = edge_dir(mesh, e_in, v)?;
                let d2 = edge_dir(mesh, e_out, v)?;
                let sin = d1.cross(d2).length();
                let q = if sin > 1e-4 {
                    p + (d1 + d2) * (width / sin)
                } else {
                    let n = mesh.face_normal(f);
                    let mut dir = n.cross(d1).normalize_or_zero();
                    if dir.dot(mesh.face_center(f) - p) < 0.0 {
                        dir = -dir;
                    }
                    p + dir * width
                };
                let id = mesh.add_vert(q);
                ctx.corner.insert((v, f), id);
            }
        }
    }

    // 2. Profiles: for each beveled edge end, points from the f1 rail to the f2 rail.
    let mut profiles: BTreeMap<(EdgeId, VertId), Vec<VertId>> = BTreeMap::new();
    for (&e, &(f1, f2)) in &sides {
        let edge = *mesh.edge_ref(e)?;
        for x in edge.verts {
            let a = ctx.point(mesh, x, e, f1)?;
            let b = ctx.point(mesh, x, e, f2)?;
            let mut pts = vec![a];
            if segments > 1 {
                let (pa, pb, px) = (mesh.position(a)?, mesh.position(b)?, mesh.position(x)?);
                let d = edge_dir(mesh, e, x)?;
                let along = ((pa - px).dot(d) + (pb - px).dot(d)) * 0.5;
                let corner = px + d * along;
                let center = pa + pb - corner;
                for k in 1..segments {
                    let phi = std::f32::consts::FRAC_PI_2 * k as f32 / segments as f32;
                    let q = center + (pa - center) * phi.cos() + (pb - center) * phi.sin();
                    pts.push(mesh.add_vert(q));
                }
            }
            pts.push(b);
            profiles.insert((e, x), pts);
        }
    }

    // 3. New corner lists for every face around a beveled vertex.
    let mut touched_faces: BTreeSet<FaceId> = BTreeSet::new();
    for fan in fans.values() {
        touched_faces.extend(fan.faces.iter().copied());
    }
    let mut new_loops: BTreeMap<FaceId, Vec<(VertId, glam::Vec2, Option<VertId>)>> = BTreeMap::new();
    for &f in &touched_faces {
        let face = mesh.face_ref(f)?.clone();
        let mut out: Vec<(VertId, glam::Vec2, Option<VertId>)> = Vec::new();
        for (i, &u) in face.verts.iter().enumerate() {
            if !verts.contains(&u) {
                out.push((u, face.uvs[i], None));
                continue;
            }
            let (e_in, e_out) = corner_edges(mesh, f, u)?;
            out.push((ctx.point(mesh, u, e_in, f)?, face.uvs[i], Some(u)));
            out.push((ctx.point(mesh, u, e_out, f)?, face.uvs[i], Some(u)));
        }
        new_loops.insert(f, out);
    }

    // 4. Holes around each vertex: merge into a face or emit a vertex polygon.
    let mut vertex_polys: Vec<Vec<VertId>> = Vec::new();
    // Non-beveled edges that moved from a vertex to a profile point: (vertex, edge, new end).
    let mut moved: Vec<(VertId, EdgeId, VertId)> = Vec::new();
    for (&v, fan) in &fans {
        if !fan.closed {
            continue;
        }
        let mut ring: Vec<VertId> = Vec::new();
        let mut bevel_count = 0;
        let m = fan.faces.len();
        for i in 0..m {
            let f = fan.faces[i];
            let (e_in, e_out) = corner_edges(mesh, f, v)?;
            let p = ctx.point(mesh, v, e_in, f)?;
            let q = ctx.point(mesh, v, e_out, f)?;
            ring.push(p);
            ring.push(q);
            if ctx.beveled.contains(&e_out) {
                bevel_count += 1;
                let pts = profiles.get(&(e_out, v)).ok_or(ModelError::Invariant("missing profile".into()))?;
                let (f1, _) = sides[&e_out];
                let mids = &pts[1..pts.len() - 1];
                if f == f1 {
                    ring.extend(mids.iter().copied());
                } else {
                    ring.extend(mids.iter().rev().copied());
                }
            }
        }
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            continue;
        }
        if bevel_count == 1 && m >= 3 {
            // A single beveled edge ends here. The faces not touching it form a chain that closes the corner; they
            // share the profile between them (each chain edge moves to a profile point), so nothing stays behind at
            // the old corner.
            let mut at = None;
            for (i, &f) in fan.faces.iter().enumerate() {
                let (_, e_out) = corner_edges(mesh, f, v)?;
                if ctx.beveled.contains(&e_out) {
                    at = Some((i, e_out));
                }
            }
            let (i, e) = at.ok_or(ModelError::Invariant("bevel: beveled edge not in fan".into()))?;
            let pts = profiles.get(&(e, v)).ok_or(ModelError::Invariant("missing profile".into()))?;
            // From the rail in the face before the edge to the rail in the face after it.
            let path: Vec<VertId> =
                if fan.faces[i] == sides[&e].0 { pts.clone() } else { pts.iter().rev().copied().collect() };
            let chain: Vec<FaceId> = (2..m).map(|k| fan.faces[(i + k) % m]).collect();
            let (n, last) = (chain.len(), path.len() - 1);
            // Chain position j (0 = after the edge, n = before it) to a profile index, walking the path backwards.
            let index = |j: usize| ((last as f32) * (1.0 - j as f32 / n as f32)).round() as usize;
            for (j, &f) in chain.iter().enumerate() {
                let (hi, lo) = (index(j), index(j + 1));
                let lp = new_loops.get_mut(&f).ok_or(ModelError::Invariant("bevel: chain face".into()))?;
                let pos = lp.iter().position(|c| c.2 == Some(v)).ok_or(ModelError::Invariant("bevel: corner".into()))?;
                let uv = lp[pos].1;
                lp.splice(pos..pos + 2, (lo..=hi).rev().map(|k| (path[k], uv, None)));
                if j + 1 < n {
                    let (_, e_out) = corner_edges(mesh, f, v)?;
                    moved.push((v, e_out, path[lo]));
                }
            }
        } else {
            ring.reverse();
            vertex_polys.push(ring);
        }
    }

    // 5. Write faces back.
    for (f, lp) in new_loops {
        let mut verts_out: Vec<VertId> = Vec::new();
        let mut uvs_out = Vec::new();
        for (vtx, uv, _) in lp {
            if verts_out.last() != Some(&vtx) {
                verts_out.push(vtx);
                uvs_out.push(uv);
            }
        }
        while verts_out.len() > 1 && verts_out.first() == verts_out.last() {
            verts_out.pop();
            uvs_out.pop();
        }
        mesh.set_face(f, &verts_out, &uvs_out)
            .map_err(|_| ModelError::Unsupported("bevel width too large for this geometry".into()))?;
    }
    let mut strip_faces = Vec::new();
    for (&e, _) in &sides {
        let edge = *mesh.edge_ref(e)?;
        let [a, b] = edge.verts;
        let (pa, pb) = (&profiles[&(e, a)], &profiles[&(e, b)]);
        for k in 0..segments as usize {
            let quad = [pb[k], pa[k], pa[k + 1], pb[k + 1]];
            let f = mesh.add_face(&quad).map_err(|_| ModelError::Unsupported("bevel produced a degenerate strip".into()))?;
            strip_faces.push(f);
        }
    }
    for poly in vertex_polys {
        let f = mesh.add_face(&poly).map_err(|_| ModelError::Unsupported("bevel produced a degenerate corner".into()))?;
        strip_faces.push(f);
    }
    // 6. Carry edge marks to the shortened edges, then drop what got disconnected.
    let carried = ctx.slide.iter().map(|(&(v, e), &s)| (v, e, s)).chain(moved);
    for (v, e, s) in carried {
        let edge = *mesh.edge_ref(e)?;
        let flags: EdgeFlags = edge.flags;
        if let Some(ne) = mesh.find_edge(s, edge.other(v)) {
            mesh.set_edge_flags(ne, flags);
        }
    }
    let old_edges: BTreeSet<EdgeId> = verts.iter().flat_map(|&v| mesh.vert_edges(v).to_vec()).chain(edges.iter().copied()).collect();
    mesh.remove_loose(old_edges, verts);
    Ok(Selection::from_faces(mesh, strip_faces))
}
