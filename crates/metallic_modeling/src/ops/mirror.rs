//! Mirror geometry across an axis plane, welding the seam.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{ModelError, ModelResult};
use crate::ids::{FaceId, VertId};
use crate::mesh::Mesh;
use crate::ops::transform::Axis;
use crate::selection::Selection;

/// Adds a mirrored copy of `faces` (and any selected loose vertices/wire edges via `verts`) across the plane through
/// the origin perpendicular to `axis`. Vertices within `merge_distance` of the plane are snapped onto it and shared by
/// both halves, so a half model becomes one closed piece. Mirrored faces are wound so normals still point outwards.
/// Returns the original and mirrored faces selected.
///
/// # Errors
/// [`ModelError::NothingSelected`] when there is nothing to mirror, [`ModelError::InvalidParameter`] for a bad
/// distance.
pub fn mirror(
    mesh: &mut Mesh,
    faces: &BTreeSet<FaceId>,
    verts: &BTreeSet<VertId>,
    axis: Axis,
    merge_distance: f32,
) -> ModelResult<Selection> {
    if faces.is_empty() && verts.is_empty() {
        return Err(ModelError::NothingSelected("geometry to mirror"));
    }
    if !(merge_distance.is_finite() && merge_distance >= 0.0) {
        return Err(ModelError::InvalidParameter(format!(
            "mirror merge distance {merge_distance}"
        )));
    }
    let mut all_verts: BTreeSet<VertId> = verts.clone();
    for &f in faces {
        all_verts.extend(mesh.face_ref(f)?.verts.iter().copied());
    }
    let k = axis.index();
    let mut map: BTreeMap<VertId, VertId> = BTreeMap::new();
    for &v in &all_verts {
        let p = mesh.position(v)?;
        if p.to_array()[k].abs() <= merge_distance {
            let mut a = p.to_array();
            a[k] = 0.0;
            mesh.set_position(v, glam::Vec3::from_array(a))?;
            map.insert(v, v);
        } else {
            map.insert(v, mesh.add_vert(axis.mirror(p)));
        }
    }
    let to = |v: &VertId| map.get(v).copied().unwrap_or(*v);
    // Wire edges between selected vertices.
    let wires: Vec<(VertId, VertId)> = mesh
        .edges()
        .filter(|(e, edge)| {
            mesh.edge_faces(*e).is_empty() && edge.verts.iter().all(|v| all_verts.contains(v))
        })
        .map(|(_, edge)| (edge.verts[0], edge.verts[1]))
        .collect();
    for (a, b) in wires {
        let (ma, mb) = (to(&a), to(&b));
        if ma != mb {
            mesh.add_edge(ma, mb)?;
        }
    }
    let mut out: Vec<FaceId> = faces.iter().copied().collect();
    for &f in faces {
        let face = mesh.face_ref(f)?.clone();
        let mut vs: Vec<VertId> = face.verts.iter().map(to).collect();
        let mut uvs = face.uvs.clone();
        vs.reverse();
        uvs.reverse();
        let distinct: BTreeSet<VertId> = vs.iter().copied().collect();
        if distinct.len() == vs.len() && distinct.iter().any(|v| !face.verts.contains(v)) {
            out.push(mesh.add_face_with_uvs(&vs, &uvs)?);
        }
    }
    // Copy edge marks onto the mirrored edges.
    for &f in faces {
        for e in mesh.face_edges(f) {
            let edge = *mesh.edge_ref(e)?;
            if let Some(me) = mesh.find_edge(to(&edge.verts[0]), to(&edge.verts[1])) {
                mesh.set_edge_flags(me, edge.flags);
            }
        }
    }
    Ok(Selection::from_faces(mesh, out))
}
