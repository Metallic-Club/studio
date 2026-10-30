//! # metallic_modeling
//!
//! The core of Metallic's built-in modeller (epic #19): an editable polygon mesh with stable element ids, vertex /
//! edge / face selections, and modelling operations as reversible commands, plus triangulation to GPU-ready
//! [`MeshData`], UV mapping and glTF/OBJ export. It has no dependency on the renderer or on Studio; Studio wraps
//! [`MeshCommand`] in its own `History`.
//!
//! ```
//! use metallic_modeling::{EditMesh, History, ModelError, NormalMode, Op, Primitive, Selection, triangulate};
//! use glam::Vec3;
//!
//! let mesh = Primitive::Cube { size: 2.0 }.build(Vec3::ZERO)?;
//! let mut edit = EditMesh::new(mesh);
//! let mut history = History::new();
//! // Select the top face and pull it up one stud.
//! let top = edit.mesh.faces().map(|(id, _)| id).find(|&f| edit.mesh.face_normal(f).y > 0.9)
//!     .ok_or(ModelError::NothingSelected("top face"))?;
//! let select = Op::Select(Selection::from_faces(&edit.mesh, [top]));
//! history.apply(&mut edit, &select)?;
//! history.apply(&mut edit, &Op::ExtrudeFaces { distance: 1.0, individual: false })?;
//! assert_eq!(edit.mesh.face_count(), 10);
//! history.undo(&mut edit);
//! assert_eq!(edit.mesh.face_count(), 6);
//! let gpu = triangulate(&edit.mesh, NormalMode::default());
//! assert_eq!(gpu.data.triangle_count(), 12);
//! # Ok::<(), metallic_modeling::ModelError>(())
//! ```
//!
//! Layout:
//! - [`mesh`]: the data structure (BMesh-style radial edges in id arenas) and its journal.
//! - [`selection`]: selection sets and flushing between vertex, edge and face modes.
//! - [`ops`]: the operations (extrude, inset, bevel, loop cut, bridge, merge, dissolve, delete, fill, mirror,
//!   subdivide, Catmull-Clark, normals, marks, booleans interface).
//! - [`command`]: [`Op`], [`MeshCommand`] and [`History`].
//! - [`triangulate`], [`uv`], [`export`], [`primitives`].

pub mod command;
pub mod error;
pub mod export;
pub mod geom;
pub mod ids;
pub mod mesh;
pub mod ops;
pub mod primitives;
pub mod selection;
pub mod triangulate;
pub mod uv;

pub use command::{EditMesh, History, MeshCommand, Op, contextual_extrude};
pub use error::{ModelError, ModelResult};
pub use ids::{EdgeId, FaceId, VertId};
pub use mesh::{ChangeSet, Edge, EdgeFlags, Face, Fan, Mesh, Vertex};
pub use ops::delete::DeleteKind;
pub use ops::transform::{Axis, Symmetry, Transform};
pub use primitives::Primitive;
pub use selection::{SelectMode, Selection};
pub use triangulate::{MeshData, NormalMode, Triangulated, triangulate};
