//! Modelling operations. Each is a plain function over a [`crate::Mesh`] that uses only the journaled mutation
//! primitives, so running it inside [`crate::Mesh::record`] (as [`crate::MeshCommand`] does) makes it undoable.

pub mod bevel;
pub mod boolean;
pub mod delete;
pub mod dissolve;
pub mod extrude;
pub mod fill;
pub mod inset;
pub mod loops;
pub mod merge;
pub mod mirror;
pub mod normals;
pub mod subdivide;
pub mod transform;
