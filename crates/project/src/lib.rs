//! Everything about files: projects, scenes, assets and packages.
//!
//! Owns: `metallic.toml`, scene save and load, the asset index and asset decoding, safe file operations and the
//! `game.pak` format.
//!
//! Must not depend on: the render, physics, scripting or platform crates. See `docs/adr/0006-project-format.md`.
