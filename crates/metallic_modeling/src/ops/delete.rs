//! Delete vertices, edges or faces.

use std::collections::BTreeSet;

use crate::error::{ModelError, ModelResult};
use crate::ids::{EdgeId, VertId};
use crate::mesh::Mesh;
use crate::selection::Selection;

/// What to delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeleteKind {
    /// Selected vertices with every edge and face using them.
    Verts,
    /// Selected edges with their faces; vertices left without edges go too.
    Edges,
    /// Selected faces; edges and vertices only they used go too.
    Faces,
    /// Selected faces only; their edges and vertices stay (an open hole).
    OnlyFaces,
}

/// Deletes the selected elements of the given kind. Returns an empty selection (same mode).
///
/// # Errors
/// [`ModelError::NothingSelected`] when the relevant set is empty.
pub fn delete(mesh: &mut Mesh, sel: &Selection, kind: DeleteKind) -> ModelResult<Selection> {
    match kind {
        DeleteKind::Verts => {
            if sel.verts.is_empty() {
                return Err(ModelError::NothingSelected("vertices to delete"));
            }
            for &v in &sel.verts {
                if mesh.vert(v).is_some() {
                    mesh.remove_vert(v)?;
                }
            }
        }
        DeleteKind::Edges => {
            if sel.edges.is_empty() {
                return Err(ModelError::NothingSelected("edges to delete"));
            }
            let mut ends: BTreeSet<VertId> = BTreeSet::new();
            for &e in &sel.edges {
                if let Some(edge) = mesh.edge(e) {
                    ends.extend(edge.verts);
                    mesh.remove_edge(e)?;
                }
            }
            mesh.remove_loose(std::iter::empty(), ends);
        }
        DeleteKind::Faces | DeleteKind::OnlyFaces => {
            if sel.faces.is_empty() {
                return Err(ModelError::NothingSelected("faces to delete"));
            }
            let mut edges: BTreeSet<EdgeId> = BTreeSet::new();
            let mut verts: BTreeSet<VertId> = BTreeSet::new();
            for &f in &sel.faces {
                if let Some(face) = mesh.face(f) {
                    verts.extend(face.verts.iter().copied());
                    edges.extend(mesh.face_edges(f));
                    mesh.remove_face(f)?;
                }
            }
            if kind == DeleteKind::Faces {
                mesh.remove_loose(edges, verts);
            }
        }
    }
    Ok(Selection::new(sel.mode))
}
