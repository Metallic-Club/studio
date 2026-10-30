//! CPU mesh generation for every Metallic Part shape.
//!
//! `metallic_meshgen` turns a [`MeshDesc`] (shape, size in studs, bevel, level of detail, shading) into a
//! [`MeshData`] that a renderer can upload as is, and a [`ColliderHint`] that physics can use. It has no GPU,
//! window or engine dependency; it only needs `glam`.
//!
//! # Conventions
//! - Units are studs. The mesh is built **at its real size**, centred on the origin, filling the box
//!   `-size/2 ..= size/2`. The instance transform must carry only rotation and translation, never the size:
//!   bevel widths and UV density are measured in studs, so they would stretch if the mesh were scaled.
//! - Right-handed, +Y up (as `metallic_core` and `metallic_render`). Triangles are counter-clockwise seen from
//!   outside (wgpu `FrontFace::Ccw`, back faces culled).
//! - Orientation of each shape (matches Roblox where Roblox has the shape):
//!   - `Wedge`, `Ramp`, `Stairs` rise toward **+Z** (the tall side is at +Z, the low edge at -Z).
//!   - `CornerWedge` has its peak above the (+X, +Z) corner.
//!   - Round extrusions run along **X**: `Cylinder`, `Pipe`, `Capsule`, `HexPrism`, `Torus` (axis X),
//!     `TriangularPrism` (ridge along X).
//!   - Pointed or domed shapes point **+Y**: `Cone`, `Pyramid`, `Hemisphere`.
//!   - `Arch` spans X (legs at ±X); you walk through it along Z.
//!
//! # Mesh layout
//! [`MeshData`] stores separate attribute arrays (one entry per vertex) plus a `u32` triangle list:
//!
//! | attribute   | type       | meaning |
//! |-------------|------------|---------|
//! | `positions` | `[f32; 3]` | studs, mesh space |
//! | `normals`   | `[f32; 3]` | unit length, outward |
//! | `tangents`  | `[f32; 4]` | MikkTSpace: unit `xyz` along +U of `uv0`, `w` = ±1 bitangent sign (`B = w·cross(N, T)`) |
//! | `uv0`       | `[f32; 2]` | world-scale: **1.0 = 1 stud**, the same density at every size |
//! | `uv1`       | `[f32; 2]` | 0..1 across each UV chart (each flat face, each curved side) |
//! | `ao`        | `f32`      | baked ambient occlusion, 1 = open, lower in concave creases |
//!
//! [`MeshData::interleaved`] packs them into [`Vertex`] (60 bytes, documented offsets) for one vertex buffer.
//!
//! # Sharing buffers
//! [`MeshDesc::key`] quantises the request into a [`MeshKey`] (size to 1/4096 stud, irrelevant parameters
//! zeroed), so identical Parts produce equal keys. [`MeshCache`] keeps one `Arc<MeshData>` per key, and
//! [`MeshKey::stable_hash`] is a run-to-run stable 64-bit hash for on-disk caches.
//!
//! # Example
//! ```
//! use metallic_meshgen::{generate, Bevel, MeshDesc, Shape};
//!
//! let desc = MeshDesc::new(Shape::Wedge, [4.0, 1.0, 2.0]).with_bevel(Bevel::round(0.1));
//! let mesh = generate(&desc)?;
//! assert_eq!(mesh.indices.len() % 3, 0);
//! assert!(mesh.bounds.max[1] <= 0.5 + 1e-4);
//! # Ok::<(), metallic_meshgen::MeshGenError>(())
//! ```
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod ao;
mod builder;
mod cache;
mod collider;
mod error;
mod extrude;
mod lathe;
mod math;
mod mesh;
mod polytope;
mod shape;
mod shapes;
mod tangent;
mod tess;
mod triangulate;

pub use cache::{MeshCache, MeshKey};
pub use collider::{Axis, ColliderHint, ColliderPart, collider_hint};
pub use error::MeshGenError;
pub use mesh::{Aabb, MeshData, Vertex};
pub use shape::{
    Bevel, BevelStyle, Lod, MAX_SIZE, MIN_SIZE, MeshDesc, Shading, Shape, ShapeParams,
};

/// Version of the generation algorithm. It is mixed into [`MeshKey::stable_hash`], so on-disk caches made by an
/// older version are not reused after the geometry changes.
pub const MESHGEN_VERSION: u32 = 1;

/// Builds the mesh described by `desc`.
///
/// The size is validated and quantised first (see [`MeshDesc::key`]), so two descriptions with the same key
/// always produce bit-identical meshes.
pub fn generate(desc: &MeshDesc) -> Result<MeshData, MeshGenError> {
    Ok(generate_key(&desc.key()?))
}

/// Builds the mesh for an already validated key. Deterministic: the same key gives the same bytes.
pub fn generate_key(key: &MeshKey) -> MeshData {
    let desc = key.desc();
    let built = shapes::build(&desc);
    builder::finish(built, &desc)
}
