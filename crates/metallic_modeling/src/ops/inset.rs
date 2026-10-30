//! Inset faces: shrink a face (or a region of faces) inwards and surround it with a ring of quads.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// Largest miter stretch at sharp corners, so thin spikes stay bounded.
const MAX_MITER: f32 = 4.0;

/// Insets faces by `thickness` (measured perpendicular to each side, in the face plane) and pushes the inner part
/// `depth` along the normal (negative depth pushes inwards).
///
/// - Region (`individual = false`): connected faces are inset as one; only the region boundary gets a ring.
/// - Individual: each face gets its own ring.
///
/// The selected faces keep their ids and stay selected.
///
/// # Errors
/// [`ModelError::NothingSelected`], [`ModelError::MissingFace`] or a negative/non-finite thickness.
pub fn inset(
    mesh: &mut Mesh,
    faces: &BTreeSet<FaceId>,
    thickness: f32,
    depth: f32,
    individual: bool,
) -> ModelResult<Selection> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to inset"));
    }
    if !(thickness.is_finite() && thickness >= 0.0 && depth.is_finite()) {
        return Err(ModelError::InvalidParameter(format!(
            "inset thickness {thickness}, depth {depth}"
        )));
    }
    for &f in faces {
        mesh.face_ref(f)?;
    }
    if individual {
        for &f in faces {
            inset_region(mesh, &BTreeSet::from([f]), thickness, depth)?;
        }
    } else {
        inset_region(mesh, faces, thickness, depth)?;
    }
    Ok(Selection::from_faces(mesh, faces.iter().copied()))
}

fn inset_region(
    mesh: &mut Mesh,
    region: &BTreeSet<FaceId>,
    thickness: f32,
    depth: f32,
) -> ModelResult<()> {
    let mut boundary: Vec<(VertId, VertId, FaceId)> = Vec::new();
    let mut region_edges: BTreeSet<EdgeId> = BTreeSet::new();
    let mut region_verts: BTreeSet<VertId> = BTreeSet::new();
    for &f in region {
        let face = mesh.face_ref(f)?.clone();
        region_verts.extend(face.verts.iter().copied());
        for (a, b) in face.directed_edges() {
            let e = mesh
                .find_edge(a, b)
                .ok_or(ModelError::Invariant("face side without edge".into()))?;
            region_edges.insert(e);
            if mesh
                .edge_faces(e)
                .iter()
                .filter(|g| region.contains(g))
                .count()
                == 1
            {
                boundary.push((a, b, f));
            }
        }
    }
    if boundary.is_empty() {
        return Err(ModelError::Unsupported(
            "inset needs a region with a boundary (not a closed shell)".into(),
        ));
    }
    let region_normal_at = |mesh: &Mesh, v: VertId| -> Vec3 {
        mesh.vert_faces(v)
            .iter()
            .filter(|f| region.contains(f))
            .map(|&f| geom::newell(&mesh.face_positions(f)))
            .sum::<Vec3>()
            .normalize_or_zero()
    };
    let mut incoming: BTreeMap<VertId, Vec<VertId>> = BTreeMap::new();
    let mut outgoing: BTreeMap<VertId, Vec<VertId>> = BTreeMap::new();
    for &(a, b, _) in &boundary {
        outgoing.entry(a).or_default().push(b);
        incoming.entry(b).or_default().push(a);
    }
    let mut inner: BTreeMap<VertId, VertId> = BTreeMap::new();
    for (&v, outs) in &outgoing {
        let p = mesh.position(v)?;
        let n = region_normal_at(mesh, v);
        let ins = incoming.get(&v).cloned().unwrap_or_default();
        let perps: Vec<Vec3> = ins
            .iter()
            .map(|&u| n.cross((p - mesh.position_or_zero(u)).normalize_or_zero()))
            .chain(
                outs.iter()
                    .map(|&w| n.cross((mesh.position_or_zero(w) - p).normalize_or_zero())),
            )
            .collect();
        let dir = if ins.len() == 1 && outs.len() == 1 {
            let m = (perps[0] + perps[1]).normalize_or_zero();
            let c = m.dot(perps[0]).max(1.0 / MAX_MITER);
            m / c
        } else {
            perps.iter().copied().sum::<Vec3>().normalize_or_zero()
        };
        let q = p + dir * thickness + n * depth;
        inner.insert(v, mesh.add_vert(q));
    }
    if depth != 0.0 {
        for &v in region_verts.iter().filter(|v| !inner.contains_key(v)) {
            let p = mesh.position(v)? + region_normal_at(mesh, v) * depth;
            mesh.set_position(v, p)?;
        }
    }
    let mapped = |v: &VertId| inner.get(v).copied().unwrap_or(*v);
    for &f in region {
        let face = mesh.face_ref(f)?.clone();
        let verts: Vec<VertId> = face.verts.iter().map(mapped).collect();
        mesh.set_face(f, &verts, &face.uvs)?;
    }
    for &(a, b, f) in &boundary {
        let face = mesh.face_ref(f)?;
        let uv = |v: VertId| {
            face.position_of(mapped(&v))
                .map_or(glam::Vec2::ZERO, |i| face.uvs[i])
        };
        let (ua, ub) = (uv(a), uv(b));
        mesh.add_face_with_uvs(&[a, b, mapped(&b), mapped(&a)], &[ua, ub, ub, ua])?;
    }
    mesh.remove_loose(region_edges, region_verts);
    Ok(())
}
