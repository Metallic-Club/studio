//! Everything about files: projects, scenes, assets and packages.
//!
//! Owns: `.mtproj` projects ([`Project`]), `metallic.toml`, scene save and load, the asset index ([`AssetIndex`])
//! and asset decoding, safe file operations and the `game.pak` format ([`Pak`]). A game is read the same from its
//! project or its pak, through [`GameFiles`].
//!
//! Must not depend on: the render, physics, scripting or platform crates. See `docs/adr/0006-project-format.md`.

pub mod asset_index;
pub mod assets;
mod game_files;
pub mod manifest;
pub mod pak;
pub mod project;
pub mod scene;

pub use asset_index::{
    Asset, AssetId, AssetIndex, AssetIndexError, AssetMove, EDITOR_DATA, Fingerprint, INDEX_FILE, IndexReport,
    ParseAssetIdError, is_editor_data,
};
pub use assets::{AssetError, AssetReader, DecodeError, ImageAsset, MeshAsset, decode_image, decode_mesh};
pub use game_files::GameFiles;
pub use manifest::{LoadedManifest, Manifest, ManifestError, ManifestWarning, ProjectKind, ProjectMode};
pub use pak::{PAK_FILE, PAK_FORMAT, PAK_MAGIC, Pak, PakError};
pub use project::{
    MemberMove, MoveReport, PROJECT_EXTENSION, Project, ProjectError, References, Template, check_member_path,
};
pub use scene::{
    LoadedScene, PATH_PROPERTIES, SCENE_FORMAT, SavedScene, SceneError, SceneIds, SceneReference, SceneWarning,
    load_scene, load_scene_bytes, save_scene,
};
