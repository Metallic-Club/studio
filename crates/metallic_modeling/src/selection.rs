//! Vertex, edge and face selection with Blender-like "flushing" between the three element kinds.
//!
//! The current [`SelectMode`] decides which set is primary; [`Selection::flush`] derives the other two:
//! - vertex mode: an edge is selected when both ends are, a face when all its corners are;
//! - edge mode: the ends of selected edges are selected, a face when all its edges are;
//! - face mode: the corners and sides of selected faces are selected.

use std::collections::BTreeSet;

use glam::Vec3;

use crate::ids::{EdgeId, FaceId, VertId};
use crate::mesh::Mesh;

/// Which element kind the user picks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SelectMode {
    /// Pick vertices.
    Vertex,
    /// Pick edges.
    Edge,
    /// Pick faces.
    #[default]
    Face,
}

/// A selection of vertices, edges and faces. Sets are ordered, so operations iterate deterministically.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The element kind the user is picking.
    pub mode: SelectMode,
    /// Selected vertices.
    pub verts: BTreeSet<VertId>,
    /// Selected edges.
    pub edges: BTreeSet<EdgeId>,
    /// Selected faces.
    pub faces: BTreeSet<FaceId>,
}

impl Selection {
    /// An empty selection in `mode`.
    #[must_use]
    pub fn new(mode: SelectMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// A face-mode selection of `faces`, flushed.
    #[must_use]
    pub fn from_faces(mesh: &Mesh, faces: impl IntoIterator<Item = FaceId>) -> Self {
        let mut s = Self::new(SelectMode::Face);
        s.faces = faces
            .into_iter()
            .filter(|&f| mesh.face(f).is_some())
            .collect();
        s.flush(mesh);
        s
    }

    /// An edge-mode selection of `edges`, flushed.
    #[must_use]
    pub fn from_edges(mesh: &Mesh, edges: impl IntoIterator<Item = EdgeId>) -> Self {
        let mut s = Self::new(SelectMode::Edge);
        s.edges = edges
            .into_iter()
            .filter(|&e| mesh.edge(e).is_some())
            .collect();
        s.flush(mesh);
        s
    }

    /// A vertex-mode selection of `verts`, flushed.
    #[must_use]
    pub fn from_verts(mesh: &Mesh, verts: impl IntoIterator<Item = VertId>) -> Self {
        let mut s = Self::new(SelectMode::Vertex);
        s.verts = verts
            .into_iter()
            .filter(|&v| mesh.vert(v).is_some())
            .collect();
        s.flush(mesh);
        s
    }

    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }

    /// Deselects everything (the mode stays).
    pub fn clear(&mut self) {
        self.verts.clear();
        self.edges.clear();
        self.faces.clear();
    }

    /// Drops ids that are no longer live, then flushes.
    pub fn prune(&mut self, mesh: &Mesh) {
        self.verts.retain(|&v| mesh.vert(v).is_some());
        self.edges.retain(|&e| mesh.edge(e).is_some());
        self.faces.retain(|&f| mesh.face(f).is_some());
        self.flush(mesh);
    }

    /// Derives the non-primary sets from the primary one (see the module docs).
    pub fn flush(&mut self, mesh: &Mesh) {
        match self.mode {
            SelectMode::Vertex => {
                self.edges = self
                    .verts
                    .iter()
                    .flat_map(|&v| mesh.vert_edges(v).iter().copied())
                    .filter(|&e| {
                        mesh.edge(e)
                            .is_some_and(|e| e.verts.iter().all(|v| self.verts.contains(v)))
                    })
                    .collect();
                self.faces = self
                    .edges
                    .iter()
                    .flat_map(|&e| mesh.edge_faces(e).iter().copied())
                    .filter(|&f| {
                        mesh.face(f)
                            .is_some_and(|f| f.verts.iter().all(|v| self.verts.contains(v)))
                    })
                    .collect();
            }
            SelectMode::Edge => {
                self.verts = self
                    .edges
                    .iter()
                    .filter_map(|&e| mesh.edge(e))
                    .flat_map(|e| e.verts)
                    .collect();
                self.faces = self
                    .edges
                    .iter()
                    .flat_map(|&e| mesh.edge_faces(e).iter().copied())
                    .filter(|&f| mesh.face_edges(f).iter().all(|e| self.edges.contains(e)))
                    .collect();
            }
            SelectMode::Face => {
                self.edges = self
                    .faces
                    .iter()
                    .flat_map(|&f| mesh.face_edges(f))
                    .collect();
                self.verts = self
                    .faces
                    .iter()
                    .filter_map(|&f| mesh.face(f))
                    .flat_map(|f| f.verts.iter().copied())
                    .collect();
            }
        }
    }

    /// Switches mode, keeping what is selected as far as the new mode can express it, then flushes.
    pub fn set_mode(&mut self, mode: SelectMode, mesh: &Mesh) {
        self.flush(mesh);
        self.mode = mode;
        self.flush(mesh);
    }

    /// Selects every element.
    pub fn select_all(&mut self, mesh: &Mesh) {
        self.verts = mesh.verts().map(|(id, _)| id).collect();
        self.edges = mesh.edges().map(|(id, _)| id).collect();
        self.faces = mesh.faces().map(|(id, _)| id).collect();
    }

    /// Inverts the primary set, then flushes.
    pub fn invert(&mut self, mesh: &Mesh) {
        match self.mode {
            SelectMode::Vertex => {
                self.verts = mesh
                    .verts()
                    .map(|(id, _)| id)
                    .filter(|v| !self.verts.contains(v))
                    .collect()
            }
            SelectMode::Edge => {
                self.edges = mesh
                    .edges()
                    .map(|(id, _)| id)
                    .filter(|e| !self.edges.contains(e))
                    .collect()
            }
            SelectMode::Face => {
                self.faces = mesh
                    .faces()
                    .map(|(id, _)| id)
                    .filter(|f| !self.faces.contains(f))
                    .collect()
            }
        }
        self.flush(mesh);
    }

    /// Grows the selection to every element connected to it (through edges).
    pub fn select_linked(&mut self, mesh: &Mesh) {
        let mut stack: Vec<VertId> = self.verts.iter().copied().collect();
        let mut seen: BTreeSet<VertId> = self.verts.clone();
        while let Some(v) = stack.pop() {
            for n in mesh.vert_neighbours(v) {
                if seen.insert(n) {
                    stack.push(n);
                }
            }
        }
        let mode = self.mode;
        self.mode = SelectMode::Vertex;
        self.verts = seen;
        self.flush(mesh);
        self.mode = mode;
    }

    /// Adds or removes a vertex (vertex mode semantics), then flushes.
    pub fn toggle_vert(&mut self, mesh: &Mesh, v: VertId) {
        if !self.verts.remove(&v) {
            self.verts.insert(v);
        }
        self.flush(mesh);
    }

    /// Adds or removes an edge (edge mode semantics), then flushes.
    pub fn toggle_edge(&mut self, mesh: &Mesh, e: EdgeId) {
        if !self.edges.remove(&e) {
            self.edges.insert(e);
        }
        self.flush(mesh);
    }

    /// Adds or removes a face (face mode semantics), then flushes.
    pub fn toggle_face(&mut self, mesh: &Mesh, f: FaceId) {
        if !self.faces.remove(&f) {
            self.faces.insert(f);
        }
        self.flush(mesh);
    }

    /// Centre of the selected vertices, the usual pivot for transforms.
    #[must_use]
    pub fn center(&self, mesh: &Mesh) -> Option<Vec3> {
        if self.verts.is_empty() {
            return None;
        }
        let sum: Vec3 = self.verts.iter().map(|&v| mesh.position_or_zero(v)).sum();
        Some(sum / self.verts.len() as f32)
    }
}
