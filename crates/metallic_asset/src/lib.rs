//! Imported assets for the Metallic Engine: skinned characters from glTF 2.0 binary files (`.glb`).
//!
//! [`SkinnedModel::from_glb`] reads one skeleton ([`Skeleton`]), its skinned meshes grouped into
//! levels of detail ([`SkinnedMesh`], LOD0 first), the material slots ([`MaterialSlot`]) and every
//! animation clip ([`Clip`]). [`Clip::sample`] evaluates a clip into a [`Pose`] (local joint
//! transforms), [`Pose::blend`] mixes two poses and [`Pose::skin_matrices`] turns a pose into the
//! joint matrices the renderer's linear-blend skinning uses.
//!
//! The importer treats every file as untrusted: a bad or hostile `.glb` returns an
//! [`AssetError`], never panics, and never makes the importer allocate more than the limits below
//! allow ([`MAX_JOINTS`], [`MAX_VERTICES`], [`MAX_INDICES`], [`MAX_CLIPS`], [`MAX_KEYFRAMES`],
//! [`MAX_MATERIALS`], [`MAX_GLB_BYTES`]). Only the file's own BIN chunk is read: external or
//! data-URI buffers, images and textures are ignored or refused, so an asset can never make the
//! engine open another file.
//!
//! Units and axes are glTF's (metres, +Y up, a character faces +Z); the caller scales and turns the
//! model into the world.
#![warn(missing_docs)]

mod accessor;
mod anim;
mod import;
mod skeleton;

pub use anim::{Channel, Clip, Interpolation, Pose, Track, Transform};
pub use skeleton::Skeleton;

use glam::Vec3;

/// Largest `.glb` accepted, in bytes (64 MiB).
pub const MAX_GLB_BYTES: usize = 64 << 20;
/// Most joints in a skeleton.
pub const MAX_JOINTS: usize = 256;
/// Most vertices over all levels of detail together.
pub const MAX_VERTICES: usize = 200_000;
/// Most triangle indices over all levels of detail together.
pub const MAX_INDICES: usize = 1_200_000;
/// Most animation clips.
pub const MAX_CLIPS: usize = 256;
/// Most keyframes over all clips and tracks together.
pub const MAX_KEYFRAMES: usize = 1_000_000;
/// Most material slots (the renderer reserves this many per skinned instance).
pub const MAX_MATERIALS: usize = 8;
/// Most joints that influence one vertex. Extra influences are dropped (smallest first) and the
/// rest renormalised to sum to 1.
pub const MAX_INFLUENCES: usize = 4;

/// Why an asset could not be imported.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AssetError {
    /// The bytes are not a readable `.glb` (bad header, truncated, broken JSON, failed glTF
    /// validation).
    #[error("cannot read the glTF file: {0}")]
    Parse(String),
    /// Valid glTF, but a feature this importer does not support (external buffers, sparse
    /// accessors, non-triangle primitives, several skins, ...).
    #[error("unsupported glTF content: {0}")]
    Unsupported(String),
    /// The content breaks a rule the engine relies on (an index out of range, a NaN weight,
    /// a non-increasing key time, ...).
    #[error("invalid glTF content: {0}")]
    Invalid(String),
    /// Something is larger than the importer's limits.
    #[error("{what}: {count} is more than the limit of {limit}")]
    TooLarge {
        /// What was counted.
        what: String,
        /// How many the file has.
        count: usize,
        /// The limit.
        limit: usize,
    },
}

/// One vertex of a skinned mesh, in the mesh's bind pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinnedVertex {
    /// Position in metres (glTF mesh space).
    pub position: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
    /// First texture coordinate set; zero when the file has none.
    pub uv: [f32; 2],
    /// Up to [`MAX_INFLUENCES`] joint indices into [`Skeleton`] (parents-first order).
    pub joints: [u16; 4],
    /// Weight of each joint in `joints`; non-negative and summing to 1 (unused entries are 0).
    pub weights: [f32; 4],
    /// Index into [`SkinnedModel::materials`].
    pub material: u8,
}

/// One level of detail: every skinned primitive of that level merged into one triangle list.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedMesh {
    /// Name of the first glTF node of this level (for messages).
    pub name: String,
    /// Vertices of all primitives.
    pub vertices: Vec<SkinnedVertex>,
    /// Triangle list, counter-clockwise seen from outside; every index is below
    /// `vertices.len()`.
    pub indices: Vec<u32>,
    /// Smallest corner of the bind-pose bounding box (metres).
    pub min: Vec3,
    /// Largest corner of the bind-pose bounding box (metres).
    pub max: Vec3,
}

/// Shading of one material slot, from the glTF material's metallic-roughness factors.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSlot {
    /// glTF material name (`M_Head`, ...), or `default` for primitives without a material.
    pub name: String,
    /// Linear RGBA base colour.
    pub base_color: [f32; 4],
    /// Roughness, 0 to 1.
    pub roughness: f32,
    /// Metalness, 0 to 1.
    pub metallic: f32,
}

/// A skinned character: skeleton, meshes per level of detail, material slots and clips.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedModel {
    /// The joint hierarchy, sorted so that parents come before their children.
    pub skeleton: Skeleton,
    /// Levels of detail, most detailed first. Nodes named `..._LOD<n>` form level `n`; skinned
    /// nodes without such a suffix belong to level 0. Levels are kept in increasing `n`.
    pub lods: Vec<SkinnedMesh>,
    /// Material slots; [`SkinnedVertex::material`] indexes this list.
    pub materials: Vec<MaterialSlot>,
    /// Animation clips in file order.
    pub clips: Vec<Clip>,
}

impl SkinnedModel {
    /// Imports a skinned model from the bytes of a `.glb` file. See the crate documentation for
    /// what is read and the limits.
    ///
    /// Errors: [`AssetError::Parse`] for unreadable bytes, [`AssetError::Unsupported`] for glTF
    /// features outside this importer, [`AssetError::Invalid`] for broken references or values
    /// and [`AssetError::TooLarge`] past a limit.
    pub fn from_glb(bytes: &[u8]) -> Result<SkinnedModel, AssetError> {
        import::from_glb(bytes)
    }

    /// The clip called `name`.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }

    /// Height of the LOD0 bind pose (top minus bottom of its bounding box), in metres.
    pub fn height(&self) -> f32 {
        self.lods.first().map_or(0.0, |l| l.max.y - l.min.y)
    }
}
