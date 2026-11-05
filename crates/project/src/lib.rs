//! Everything about files: projects, scenes, assets and packages.
//!
//! Owns: `.mtproj` projects ([`Project`]), `metallic.toml`, scene save and load, the asset index and asset decoding,
//! safe file operations and the `game.pak` format.
//!
//! Must not depend on: the render, physics, scripting or platform crates. See `docs/adr/0006-project-format.md`.

pub mod manifest;
pub mod project;
pub mod scene;

pub use manifest::{LoadedManifest, Manifest, ManifestError, ManifestWarning, ProjectKind, ProjectMode};
pub use project::{PROJECT_EXTENSION, Project, ProjectError, Template, check_member_path};
pub use scene::{
    LoadedScene, SCENE_FORMAT, SavedScene, SceneError, SceneIds, SceneWarning, load_scene, load_scene_bytes, save_scene,
};
