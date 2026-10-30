//! Extrude faces (as one region or one by one), edges and vertices.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::{SelectMode, Selection};

/// Extrudes faces and moves the new cap `distance` along the normal.
///
/// - Region (`individual = false`): connected faces move together along the region's average normal; side walls are
///   built only along the region's boundary. A closed region (no boundary, e.g. a whole cube) is duplicated instead.
/// - Individual: every face gets its own walls and moves along its own normal.
///
/// The extruded faces keep their ids (they become the cap); the returned selection is the cap in face mode.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingFace`].
pub fn extrude_faces(
    mesh: &mut Mesh,
    faces: &BTreeSet<FaceId>,
    distance: f32,
    individual: bool,
) -> ModelResult<Selection> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to extrude"));
    }
    if !distance.is_finite() {
        return Err(ModelError::InvalidParameter("extrude distance".into()));
    }
    for &f in faces {
        mesh.face_ref(f)?;
    }
    if individual {
        for &f in faces {
            extrude_region(mesh, &BTreeSet::from([f]), distance)?;
        }
    } else {
        extrude_region(mesh, faces, distance)?;
    }
    Ok(Selection::from_faces(mesh, faces.iter().copied()))
}

fn extrude_region(mesh: &mut Mesh, region: &BTreeSet<FaceId>, distance: f32) -> ModelResult<()> {
    // Boundary sides, directed as their region face walks them.
    let mut boundary: Vec<(VertId, VertId, FaceId)> = Vec::new();
    let mut region_edges: BTreeSet<EdgeId> = BTreeSet::new();
    let mut region_verts: BTreeSet<VertId> = BTreeSet::new();
    let mut region_normal = Vec3::ZERO;
    for &f in region {
        let face = mesh.face_ref(f)?.clone();
        region_verts.extend(face.verts.iter().copied());
        region_normal += geom::newell(&mesh.face_positions(f));
        for (a, b) in face.directed_edges() {
            let e = mesh
                .find_edge(a, b)
                .ok_or(ModelError::Invariant("face side without edge".into()))?;
            region_edges.insert(e);
            let inside = mesh
                .edge_faces(e)
                .iter()
                .filter(|g| region.contains(g))
                .count();
            if inside == 1 {
                boundary.push((a, b, f));
            }
        }
    }
    let region_dir = region_normal.normalize_or_zero();
    let offset_of = |mesh: &Mesh, v: VertId| -> Vec3 {
        if region.len() == 1 || region_dir != Vec3::ZERO && !boundary.is_empty() {
            region_dir * distance
        } else {
            // Closed or cancelling regions: move each vertex along its own normal within the region.
            let n: Vec3 = mesh
                .vert_faces(v)
                .iter()
                .filter(|f| region.contains(f))
                .map(|&f| geom::newell(&mesh.face_positions(f)))
                .sum();
            n.normalize_or_zero() * distance
        }
    };
    let mut map: BTreeMap<VertId, VertId> = BTreeMap::new();
    for &v in &region_verts {
        let p = mesh.position(v)? + offset_of(mesh, v);
        map.insert(v, mesh.add_vert(p));
    }
    let mapped = |v: &VertId| map.get(v).copied().unwrap_or(*v);
    if boundary.is_empty() {
        // Closed region: keep the original shell and add a moved copy.
        for &f in region {
            let face = mesh.face_ref(f)?.clone();
            let verts: Vec<VertId> = face.verts.iter().map(mapped).collect();
            mesh.add_face_with_uvs(&verts, &face.uvs)?;
        }
        return Ok(());
    }
    let old_flags: Vec<(VertId, VertId, crate::mesh::EdgeFlags)> = region_edges
        .iter()
        .filter_map(|&e| mesh.edge(e))
        .map(|e| (e.verts[0], e.verts[1], e.flags))
        .collect();
    for &f in region {
        let face = mesh.face_ref(f)?.clone();
        let verts: Vec<VertId> = face.verts.iter().map(mapped).collect();
        mesh.set_face(f, &verts, &face.uvs)?;
    }
    for (a, b, f) in &boundary {
        let face = mesh.face_ref(*f)?;
        let uv_of = |v: VertId| {
            face.position_of(map.get(&v).copied().unwrap_or(v))
                .map_or(Vec2::ZERO, |i| face.uvs[i])
        };
        let (ua, ub) = (uv_of(*a), uv_of(*b));
        mesh.add_face_with_uvs(&[*a, *b, mapped(b), mapped(a)], &[ua, ub, ub, ua])?;
    }
    for (a, b, flags) in old_flags {
        if let Some(e) = mesh.find_edge(mapped(&a), mapped(&b)) {
            mesh.set_edge_flags(e, flags);
        }
    }
    mesh.remove_loose(region_edges, region_verts);
    Ok(())
}

/// Extrudes edges into quads, moving the new edges by `offset`. Edges sharing a vertex stay connected. The new quad
/// is wound against the edge's single face (if it has one) so normals stay consistent.
///
/// Returns the new edges, selected in edge mode.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingEdge`].
pub fn extrude_edges(
    mesh: &mut Mesh,
    edges: &BTreeSet<EdgeId>,
    offset: Vec3,
) -> ModelResult<Selection> {
    if edges.is_empty() {
        return Err(ModelError::NothingSelected("edges to extrude"));
    }
    let mut map: BTreeMap<VertId, VertId> = BTreeMap::new();
    let mut new_edges = Vec::new();
    for &e in edges {
        let edge = *mesh.edge_ref(e)?;
        for v in edge.verts {
            if let std::collections::btree_map::Entry::Vacant(slot) = map.entry(v) {
                let p = mesh.position(v)? + offset;
                slot.insert(mesh.add_vert(p));
            }
        }
    }
    for &e in edges {
        let edge = *mesh.edge_ref(e)?;
        let [a, b] = edge.verts;
        let (na, nb) = (map[&a], map[&b]);
        let faces = mesh.edge_faces(e).to_vec();
        let forward = faces.len() == 1
            && mesh
                .face(faces[0])
                .is_some_and(|f| f.has_directed_edge(a, b));
        let quad = if forward {
            [b, a, na, nb]
        } else {
            [a, b, nb, na]
        };
        mesh.add_face(&quad)?;
        if let Some(ne) = mesh.find_edge(na, nb) {
            new_edges.push(ne);
        }
    }
    Ok(Selection::from_edges(mesh, new_edges))
}

/// Extrudes vertices into wire edges, moving the new vertices by `offset`. Returns the new vertices selected.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingVert`].
pub fn extrude_verts(
    mesh: &mut Mesh,
    verts: &BTreeSet<VertId>,
    offset: Vec3,
) -> ModelResult<Selection> {
    if verts.is_empty() {
        return Err(ModelError::NothingSelected("vertices to extrude"));
    }
    let mut new = Vec::new();
    for &v in verts {
        let p = mesh.position(v)? + offset;
        let n = mesh.add_vert(p);
        mesh.add_edge(v, n)?;
        new.push(n);
    }
    let mut sel = Selection::new(SelectMode::Vertex);
    sel.verts = new.into_iter().collect();
    sel.flush(mesh);
    Ok(sel)
}
