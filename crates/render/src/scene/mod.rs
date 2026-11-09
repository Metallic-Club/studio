//! The GPU-resident scene (ADR 0008 §2): every renderable instance keeps a slot in a GPU instance buffer from the
//! moment it enters the world until it leaves, and only what changed is uploaded.
//!
//! - [`InstanceScene`] is the CPU side, with no GPU in it: it drains a change consumer on the data model
//!   (`TRANSFORM | VISUAL | CREATED | STRUCTURE`, plus removals) and repacks only the instances named there, in
//!   their slots. Nothing scans the scene or diffs a copy of it; [`InstanceScene::rebuild_all`] is the one full
//!   pass, for attaching and recovery. [`InstanceScene::coalesce`] turns the dirty slots into a few ranges.
//! - [`GpuScene`] owns the instance buffers and uploads those ranges.
//! - The **draw lists** say which slots each draw covers: one list of entries (slot ids, [`entry`]) per mesh kind,
//!   plus one of partly see-through instances, updated in [`InstanceScene::coalesce`] from the same dirty slots
//!   ([`InstanceScene::draw_list`]). [`GpuScene`] uploads their changed entries into one buffer with a segment
//!   per kind ([`GpuScene::list_buffer`]), which the renderer draws one instanced draw per kind and level of detail
//!   from ([`crate::parts`]).
//! - **Culling** ([`crate::cull`]): every listed instance's bounding sphere is also in a loose grid, kept current
//!   from the scene's changes (list changes in [`InstanceScene::coalesce`], moves queued for the next cull).
//!   [`InstanceScene::cull`] walks it for a view and
//!   writes the visible entries in the lists' format, each kind's split by level of detail ([`crate::lod`]);
//!   [`GpuScene::upload_view`] uploads those into the same segments, level after level, instead of every live entry.
//!
//! # Slots and regions
//!
//! Each renderable instance has a [`SlotRef`]: a [`Region`] and an index in it. Freed slots are reused through a
//! free list; nothing is compacted, so one removal uploads one zeroed slot. There are two regions, each with its
//! own buffer, so moving parts never touch the static data and their writes stay contiguous:
//!
//! - **Static**, where every instance starts.
//! - **Dynamic**, for instances that keep moving. An instance is promoted when its transform changes in
//!   [`PROMOTE_MOVES_UNANCHORED`] frames (or [`PROMOTE_MOVES_ANCHORED`] frames when `Anchored`) within a window of
//!   [`PROMOTE_WINDOW`] frames, and demoted after [`DEMOTE_REST`] frames without a transform change. `Anchored` is
//!   the hint: unanchored parts are what physics moves every step, so they are promoted after two moving frames,
//!   while an anchored part needs a script moving it steadily. Resting parts are found by checking a bounded number
//!   of dynamic slots per frame ([`REST_CHECKS_PER_FRAME`], round-robin), never by scanning the region. Changing
//!   region is the one time an instance's slot changes.
//!
//! # Camera-relative translations
//!
//! Translations are stored relative to the scene's origin ([`InstanceScene::origin`]), not the world origin, so
//! the f32 maths on the GPU stays precise far from the world origin (ADR 0008 §11). The origin snaps to a grid of
//! [`ORIGIN_GRID`] metres and only moves when the camera gets more than one grid step from it; the shader adds the
//! per-frame offset `camera − origin` from a uniform, so a moving camera costs no instance uploads. When the
//! origin does move, every live instance's translation is recomputed from its `CFrame` (exact, not accumulated) and
//! uploaded once: a rare full upload instead of a per-frame one.
//!
//! # Shadow-caster changes
//!
//! Cached shadow cascades ([`crate::shadow`]) hold the static region's casters and are redrawn only when one of them
//! changes inside the cascade. The sync therefore notes, for every static-region slot whose shadow could change (it
//! moved, was resized or changed shape, started or stopped casting, entered or left the region), the bounding
//! spheres it had and has ([`InstanceScene::shadow_changes`]). A recolour notes nothing, and neither does any change
//! in the dynamic region, whose casters are redrawn every frame anyway. The notes are origin-relative like the
//! instances; when the origin moves, or the scene is rebuilt, or more than [`MAX_SHADOW_CHANGES`] pile up, the scene
//! bumps [`InstanceScene::shadow_epoch`] instead, which redraws every cascade.
//!
//! # Mesh assets
//!
//! A class with the `AssetMesh` component (`MeshPart`) is drawn as the mesh asset its `MeshId` names, textured with
//! its `TextureId`: each pair is a mesh kind of its own, numbered as the scene meets it ([`MeshAssetKey`],
//! [`InstanceScene::mesh_assets`]), so its parts are listed, culled, sorted and drawn like any kind's. Reading the ids
//! is kept out of the parts' path: a primitive part's change pays one test for it.
//!
//! # Lights
//!
//! `PointLight`s and `SpotLight`s in the world are kept as dense records ([`SceneLight`], [`InstanceScene::lights`])
//! from the same change consumer, with their world placement resolved (a light in a part follows the part). They
//! have no slot: the renderer selects, clusters and uploads the ones it draws every frame ([`crate::lights`]).

mod assets;
mod components;
mod gpu;
mod grid;
mod instance;
mod lights;
mod lists;
mod slots;
mod sync;
#[cfg(test)]
mod tests;

pub use assets::{MAX_MESH_ASSET_KINDS, MeshAssetKey};
pub use components::{LightComponent, MeshComponent, MeshFn, MeshInput, PartProps, RenderComponents};
pub(crate) use gpu::TransparentEntries;
pub use gpu::{GpuScene, InstanceBinding, ListSegment, SCATTER_MIN_RANGES, UploadStats};
pub use grid::CELL_SIZE;
pub(crate) use grid::bounding_sphere;
pub use instance::{GpuInstance, MeshKind, affine_rows, pack_color};
pub use lights::{LightKind, SceneLight};
pub(crate) use lists::TRANSPARENT_LIST;
pub use lists::{DYNAMIC_ENTRY, entry, entry_slot};
pub use sync::InstanceScene;

/// Frames in which a static instance's transform must change, within [`PROMOTE_WINDOW`], to move it to the
/// dynamic region when it is not `Anchored`.
pub const PROMOTE_MOVES_UNANCHORED: u16 = 2;
/// Like [`PROMOTE_MOVES_UNANCHORED`], for an `Anchored` instance (moved only by scripts).
pub const PROMOTE_MOVES_ANCHORED: u16 = 4;
/// The window, in frames, the promotion counts are taken over.
pub const PROMOTE_WINDOW: u32 = 30;
/// Frames without a transform change after which a dynamic instance goes back to the static region.
pub const DEMOTE_REST: u32 = 120;
/// Dynamic slots checked for rest per frame.
pub const REST_CHECKS_PER_FRAME: u32 = 256;
/// The grid, in metres, the scene origin snaps to.
pub const ORIGIN_GRID: f32 = 1024.0;
/// Clean slots between two dirty runs that are uploaded anyway to merge the runs into one range.
pub const MAX_UPLOAD_GAP: u32 = 4;

/// The most static shadow-caster changes [`InstanceScene::shadow_changes`] keeps between two takes; past it the
/// scene notes that everything changed instead ([`InstanceScene::shadow_epoch`]), so a burst of edits (a level
/// loading) costs one redraw of every cascade and no memory beyond this.
pub const MAX_SHADOW_CHANGES: usize = 4096;

/// The flags that decide whether an instance casts a sun shadow.
const SHADOW_FLAGS: u32 = GpuInstance::LIVE | GpuInstance::CAST_SHADOW | GpuInstance::HIDDEN | GpuInstance::TRANSLUCENT;

/// Whether a packed instance casts a sun shadow: live, `CastShadow` on, and solid. See-through parts cast none
/// yet (their shadows would need the transparent list sorted per cascade).
#[inline]
pub fn is_shadow_caster(instance: &GpuInstance) -> bool {
    instance.flags & SHADOW_FLAGS == GpuInstance::LIVE | GpuInstance::CAST_SHADOW
}

/// One of the scene's two instance regions, each with its own buffer. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    /// Instances that rarely move: every instance starts here.
    Static,
    /// Instances that moved repeatedly in a short time.
    Dynamic,
}

impl Region {
    /// Both regions.
    pub const ALL: [Self; 2] = [Self::Static, Self::Dynamic];

    /// 0 for static, 1 for dynamic.
    pub const fn index(self) -> usize {
        match self {
            Self::Static => 0,
            Self::Dynamic => 1,
        }
    }
}

/// Where an instance is drawn from: its region and its slot (instance index) in that region's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotRef {
    /// The region.
    pub region: Region,
    /// The index in the region's instance buffer.
    pub slot: u32,
}

/// A run of slots to upload: `count` instances from slot `first` of `region`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UploadRange {
    /// The region whose buffer is written.
    pub region: Region,
    /// The first slot.
    pub first: u32,
    /// How many slots.
    pub count: u32,
}

/// What one [`InstanceScene::sync`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Instances that got a slot (entered the world, or every one on a rebuild).
    pub added: u32,
    /// Instances that left (destroyed, or moved out of the world) and freed their slot.
    pub removed: u32,
    /// Instances repacked in place because a property or their place in the tree changed.
    pub updated: u32,
    /// Instances moved to the dynamic region.
    pub promoted: u32,
    /// Instances moved back to the static region.
    pub demoted: u32,
    /// Watched instances ([`InstanceScene::watch`], such as `Lighting`) that changed: read them again.
    pub watched: u32,
    /// Lights (local or directional) that appeared, changed, left, or were placed again because the part they
    /// follow moved.
    pub lights: u32,
}
