//! Boolean (CSG) interface: union, difference and intersection between the selected part and the rest.
//!
//! No backend ships yet (see `docs/agents/136-modeling.md`): the robust pure-Rust option (`manifold-rust`,
//! Apache-2.0) pulls in `clipper2-rust`, which is BSL-1.0 and not on the licence allow-list; the C++ bindings
//! (`manifold-csg`, MIT/Apache, wrapping manifold3d Apache-2.0) need CMake and a C++ toolchain at build time. Both
//! are owner decisions. The trait below is what either backend plugs into.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{FaceId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// Which boolean to compute (`A` is the unselected part, `B` the selected part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BooleanOp {
    /// `A ∪ B`.
    Union,
    /// `A − B`.
    Difference,
    /// `A ∩ B`.
    Intersection,
}

/// A closed triangle mesh handed to and returned from a backend (counter-clockwise triangles seen from outside).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    /// Vertex positions.
    pub positions: Vec<Vec3>,
    /// Triangles as indices into `positions`.
    pub triangles: Vec<[u32; 3]>,
}

/// A boolean engine. Implementations must accept closed, consistently wound triangle meshes and return one.
pub trait BooleanBackend {
    /// Short name for messages.
    fn name(&self) -> &str;

    /// Computes `a op b`.
    ///
    /// # Errors
    /// A human-readable reason when the inputs are not closed or the engine fails.
    fn apply(&self, a: &TriMesh, b: &TriMesh, op: BooleanOp) -> Result<TriMesh, String>;
}

/// The placeholder backend: always reports that booleans are not available yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBackend;

impl BooleanBackend for NoBackend {
    fn name(&self) -> &str {
        "none"
    }

    fn apply(&self, _a: &TriMesh, _b: &TriMesh, _op: BooleanOp) -> Result<TriMesh, String> {
        Err("no boolean backend is available in this build yet".into())
    }
}

/// Triangulates a set of faces into a [`TriMesh`] (shared vertices stay shared).
#[must_use]
pub fn to_trimesh(mesh: &Mesh, faces: &BTreeSet<FaceId>) -> TriMesh {
    let mut out = TriMesh::default();
    let mut index: BTreeMap<VertId, u32> = BTreeMap::new();
    for &f in faces {
        let Some(face) = mesh.face(f) else { continue };
        let pts = mesh.face_positions(f);
        let ids: Vec<u32> = face
            .verts
            .iter()
            .map(|&v| {
                *index.entry(v).or_insert_with(|| {
                    out.positions.push(mesh.position_or_zero(v));
                    (out.positions.len() - 1) as u32
                })
            })
            .collect();
        for t in geom::triangulate_polygon(&pts) {
            out.triangles.push([ids[t[0]], ids[t[1]], ids[t[2]]]);
        }
    }
    out
}

/// Replaces the selected faces (B) and the other faces (A) by the result of `A op B` from `backend`. The result
/// arrives as triangles; dissolve coplanar edges afterwards to get n-gons back. Returns the result faces selected.
///
/// # Errors
/// [`ModelError::NothingSelected`] when either part is empty, [`ModelError::NonManifold`] when a part is not closed,
/// [`ModelError::Boolean`] when the backend fails.
pub fn boolean(mesh: &mut Mesh, selected: &BTreeSet<FaceId>, op: BooleanOp, backend: &dyn BooleanBackend) -> ModelResult<Selection> {
    let b_faces: BTreeSet<FaceId> = selected.iter().copied().filter(|&f| mesh.face(f).is_some()).collect();
    let a_faces: BTreeSet<FaceId> = mesh.face_ids().into_iter().filter(|f| !b_faces.contains(f)).collect();
    if a_faces.is_empty() || b_faces.is_empty() {
        return Err(ModelError::NothingSelected("a selected part and an unselected part"));
    }
    for part in [&a_faces, &b_faces] {
        let closed = part
            .iter()
            .flat_map(|&f| mesh.face_edges(f))
            .all(|e| mesh.edge_faces(e).iter().filter(|g| part.contains(g)).count() == 2);
        if !closed {
            return Err(ModelError::NonManifold("both parts must be closed for a boolean".into()));
        }
    }
    let a = to_trimesh(mesh, &a_faces);
    let b = to_trimesh(mesh, &b_faces);
    let result = backend.apply(&a, &b, op).map_err(|e| ModelError::Boolean(format!("{}: {e}", backend.name())))?;
    let all: Vec<FaceId> = mesh.face_ids();
    let verts: Vec<VertId> = mesh.vert_ids();
    for f in all {
        mesh.remove_face(f)?;
    }
    for v in verts {
        mesh.remove_vert(v)?;
    }
    let ids: Vec<VertId> = result.positions.iter().map(|&p| mesh.add_vert(p)).collect();
    let mut faces = Vec::new();
    for t in &result.triangles {
        let tri = t.map(|i| ids.get(i as usize).copied());
        let [Some(x), Some(y), Some(z)] = tri else {
            return Err(ModelError::Boolean("backend returned an index out of range".into()));
        };
        faces.push(mesh.add_face(&[x, y, z])?);
    }
    Ok(Selection::from_faces(mesh, faces))
}
