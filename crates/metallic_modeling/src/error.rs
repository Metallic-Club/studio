//! Typed errors for every fallible modelling call.

use crate::ids::{EdgeId, FaceId, VertId};

/// Everything that can go wrong while editing, converting or exporting a mesh.
///
/// A failed command leaves the mesh exactly as it was (the change journal is rolled back).
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ModelError {
    /// A vertex id does not refer to a live vertex.
    #[error("vertex {0:?} does not exist")]
    MissingVert(VertId),
    /// An edge id does not refer to a live edge.
    #[error("edge {0:?} does not exist")]
    MissingEdge(EdgeId),
    /// A face id does not refer to a live face.
    #[error("face {0:?} does not exist")]
    MissingFace(FaceId),
    /// A face needs at least three distinct live vertices.
    #[error("invalid face: {0}")]
    InvalidFace(String),
    /// An edge needs two distinct live vertices.
    #[error("invalid edge between {0:?} and {1:?}")]
    InvalidEdge(VertId, VertId),
    /// The operation needs a selection of the given kind and found none.
    #[error("nothing selected: {0}")]
    NothingSelected(&'static str),
    /// The operation needs manifold geometry (edges with at most two faces, disc-like vertex fans) here.
    #[error("non-manifold geometry: {0}")]
    NonManifold(String),
    /// The input is valid but this operation does not handle it (yet); the message says why.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// A numeric parameter is out of range.
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
    /// The mesh broke an internal invariant (reported by [`crate::Mesh::validate`]).
    #[error("invariant broken: {0}")]
    Invariant(String),
    /// A boolean backend failed or none is available.
    #[error("boolean failed: {0}")]
    Boolean(String),
    /// Building an export failed.
    #[error("export failed: {0}")]
    Export(String),
}

/// Result alias used across the crate.
pub type ModelResult<T> = Result<T, ModelError>;
