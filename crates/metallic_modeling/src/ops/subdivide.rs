//! Subdivision: simple split of selected faces, and Catmull-Clark smoothing of the whole mesh.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// Most Catmull-Clark levels in one command (each level multiplies the face count by about four).
pub const MAX_SMOOTH_LEVELS: u32 = 4;

/// Shared splitting step: every face in `faces` becomes one quad per corner around a centre point; the centre
/// positions come from `face_point`, edge midpoints from `edge_point`. Neighbouring faces outside the set get the new
/// edge points inserted so no T-junctions appear. Original edges are split in two (halves keep the marks).
fn split_faces(
    mesh: &mut Mesh,
    faces: &BTreeSet<FaceId>,
    face_point: &BTreeMap<FaceId, Vec3>,
    edge_point: &BTreeMap<EdgeId, Vec3>,
) -> ModelResult<Vec<FaceId>> {
    let mut ep: BTreeMap<EdgeId, VertId> = BTreeMap::new();
    for (&e, &p) in edge_point {
        ep.insert(e, mesh.add_vert(p));
    }
    let mut out = Vec::new();
    let split_edges: Vec<EdgeId> = ep.keys().copied().collect();
    // Marks of split edges, re-applied to both halves.
    let marks: Vec<(VertId, VertId, VertId, crate::mesh::EdgeFlags)> = split_edges
        .iter()
        .filter_map(|&e| {
            mesh.edge(e)
                .map(|x| (x.verts[0], x.verts[1], ep[&e], x.flags))
        })
        .collect();
    let mut neighbours: BTreeSet<FaceId> = BTreeSet::new();
    for &e in &split_edges {
        neighbours.extend(
            mesh.edge_faces(e)
                .iter()
                .copied()
                .filter(|f| !faces.contains(f)),
        );
    }
    for &f in faces {
        let face = mesh.face_ref(f)?.clone();
        let n = face.verts.len();
        let c = mesh.add_vert(
            face_point
                .get(&f)
                .copied()
                .unwrap_or_else(|| mesh.face_center(f)),
        );
        let uv_c = face.uvs.iter().copied().sum::<Vec2>() / n as f32;
        let mid = |mesh: &Mesh, a: VertId, b: VertId| -> ModelResult<VertId> {
            let e = mesh
                .find_edge(a, b)
                .ok_or(ModelError::Invariant("face side without edge".into()))?;
            ep.get(&e)
                .copied()
                .ok_or(ModelError::Invariant("edge point missing".into()))
        };
        for i in 0..n {
            let prev = (i + n - 1) % n;
            let next = (i + 1) % n;
            let (v, vp, vn) = (face.verts[i], face.verts[prev], face.verts[next]);
            let quad = [v, mid(mesh, v, vn)?, c, mid(mesh, vp, v)?];
            let uvs = [
                face.uvs[i],
                (face.uvs[i] + face.uvs[next]) * 0.5,
                uv_c,
                (face.uvs[prev] + face.uvs[i]) * 0.5,
            ];
            if i == 0 {
                mesh.set_face(f, &quad, &uvs)?;
                out.push(f);
            } else {
                out.push(mesh.add_face_with_uvs(&quad, &uvs)?);
            }
        }
    }
    for f in neighbours {
        let face = mesh.face_ref(f)?.clone();
        let m = face.verts.len();
        let mut vs = Vec::new();
        let mut uvs = Vec::new();
        for i in 0..m {
            let (a, b) = (face.verts[i], face.verts[(i + 1) % m]);
            vs.push(a);
            uvs.push(face.uvs[i]);
            if let Some(&p) = mesh.find_edge(a, b).and_then(|e| ep.get(&e)) {
                vs.push(p);
                uvs.push((face.uvs[i] + face.uvs[(i + 1) % m]) * 0.5);
            }
        }
        mesh.set_face(f, &vs, &uvs)?;
    }
    mesh.remove_loose(split_edges, std::iter::empty());
    for (a, b, p, flags) in marks {
        for (x, y) in [(a, p), (p, b)] {
            if let Some(e) = mesh.find_edge(x, y) {
                mesh.set_edge_flags(e, flags);
            }
        }
    }
    Ok(out)
}

/// Splits each selected face into one quad per corner (a centre point plus edge midpoints); positions are not
/// smoothed. Repeats `levels` times. Returns the new faces selected.
///
/// # Errors
/// [`ModelError::NothingSelected`] or a bad level count.
pub fn subdivide(mesh: &mut Mesh, faces: &BTreeSet<FaceId>, levels: u32) -> ModelResult<Selection> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to subdivide"));
    }
    if levels == 0 || levels > MAX_SMOOTH_LEVELS {
        return Err(ModelError::InvalidParameter(format!(
            "subdivide levels {levels}"
        )));
    }
    let mut current: BTreeSet<FaceId> = faces.clone();
    for _ in 0..levels {
        let fp: BTreeMap<FaceId, Vec3> =
            current.iter().map(|&f| (f, mesh.face_center(f))).collect();
        let ep: BTreeMap<EdgeId, Vec3> = current
            .iter()
            .flat_map(|&f| mesh.face_edges(f))
            .map(|e| (e, mesh.edge_midpoint(e)))
            .collect();
        current = split_faces(mesh, &current, &fp, &ep)?.into_iter().collect();
    }
    Ok(Selection::from_faces(mesh, current))
}

/// Catmull-Clark subdivision of the whole mesh, `levels` times. Boundary edges and edges marked sharp act as creases:
/// their edge points are midpoints, vertices on two of them follow the crease rule `(a + 6p + b) / 8`, and vertices
/// on three or more stay put. Vertex ids of the original corners survive (they move), and every original face keeps
/// its id as the first of its new quads.
///
/// # Errors
/// [`ModelError::InvalidParameter`] for zero or too many levels, [`ModelError::NothingSelected`] for an empty mesh.
pub fn catmull_clark(mesh: &mut Mesh, levels: u32) -> ModelResult<Selection> {
    if levels == 0 || levels > MAX_SMOOTH_LEVELS {
        return Err(ModelError::InvalidParameter(format!(
            "smooth levels {levels}"
        )));
    }
    if mesh.face_count() == 0 {
        return Err(ModelError::NothingSelected("faces to smooth"));
    }
    for _ in 0..levels {
        catmull_clark_once(mesh)?;
    }
    Ok(Selection::from_faces(mesh, mesh.face_ids()))
}

fn catmull_clark_once(mesh: &mut Mesh) -> ModelResult<()> {
    let faces: BTreeSet<FaceId> = mesh.face_ids().into_iter().collect();
    let fp: BTreeMap<FaceId, Vec3> = faces.iter().map(|&f| (f, mesh.face_center(f))).collect();
    let crease = |mesh: &Mesh, e: EdgeId| {
        mesh.edge_faces(e).len() != 2 || mesh.edge(e).is_some_and(|x| x.flags.sharp)
    };
    let mut ep: BTreeMap<EdgeId, Vec3> = BTreeMap::new();
    for (e, edge) in mesh.edges() {
        let around = mesh.edge_faces(e);
        if around.is_empty() {
            continue;
        }
        let mid =
            (mesh.position_or_zero(edge.verts[0]) + mesh.position_or_zero(edge.verts[1])) * 0.5;
        let p = if crease(mesh, e) {
            mid
        } else {
            let f_avg = (fp.get(&around[0]).copied().unwrap_or(mid)
                + fp.get(&around[1]).copied().unwrap_or(mid))
                * 0.5;
            (mid + f_avg) * 0.5
        };
        ep.insert(e, p);
    }
    let mut new_pos: BTreeMap<VertId, Vec3> = BTreeMap::new();
    for (v, vert) in mesh.verts() {
        let p = vert.position;
        let edges = mesh.vert_edges(v);
        let vfaces = mesh.vert_faces(v);
        if vfaces.is_empty() {
            continue;
        }
        let creases: Vec<EdgeId> = edges.iter().copied().filter(|&e| crease(mesh, e)).collect();
        let q = match creases.len() {
            0 | 1 => {
                let n = edges.len() as f32;
                if edges.len() != vfaces.len() {
                    // Mixed wire edges or odd fans: keep the vertex where it is.
                    p
                } else {
                    let f = geom::centroid(
                        &vfaces
                            .iter()
                            .filter_map(|f| fp.get(f).copied())
                            .collect::<Vec<_>>(),
                    );
                    let r = geom::centroid(
                        &edges
                            .iter()
                            .map(|&e| mesh.edge_midpoint(e))
                            .collect::<Vec<_>>(),
                    );
                    (f + r * 2.0 + p * (n - 3.0)) / n
                }
            }
            2 => {
                let a = mesh.edge_ref(creases[0])?.other(v);
                let b = mesh.edge_ref(creases[1])?.other(v);
                (mesh.position(a)? + p * 6.0 + mesh.position(b)?) / 8.0
            }
            _ => p,
        };
        new_pos.insert(v, q);
    }
    split_faces(mesh, &faces, &fp, &ep)?;
    for (v, p) in new_pos {
        mesh.set_position(v, p)?;
    }
    Ok(())
}
