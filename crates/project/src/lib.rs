//! Everything about files: projects, scenes, assets and packages.
//!
//! Owns: `.mtproj` projects ([`Project`]), `metallic.toml`, scene save and load, the asset index ([`AssetIndex`])
//! and asset decoding, safe file operations and the `game.pak` format.
//!
//! Must not depend on: the render, physics, scripting or platform crates. See `docs/adr/0006-project-format.md`.

pub mod asset_index;
pub mod assets;
pub mod manifest;
pub mod project;
pub mod scene;

pub use asset_index::{
    Asset, AssetId, AssetIndex, AssetIndexError, AssetMove, Fingerprint, INDEX_FILE, IndexReport, ParseAssetIdError,
};
pub use assets::{AssetError, AssetReader, DecodeError, ImageAsset, MeshAsset, decode_image, decode_mesh};
pub use manifest::{LoadedManifest, Manifest, ManifestError, ManifestWarning, ProjectKind, ProjectMode};
pub use project::{
    MemberMove, MoveReport, PROJECT_EXTENSION, Project, ProjectError, References, Template, check_member_path,
};
pub use scene::{
    LoadedScene, PATH_PROPERTIES, SCENE_FORMAT, SavedScene, SceneError, SceneIds, SceneReference, SceneWarning,
    load_scene, load_scene_bytes, save_scene,
};
