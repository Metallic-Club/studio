//! The Metallic Engine runtime: one running game.
//!
//! A [`Game`] ties the pieces of the engine together for one project: the Instance tree
//! ([`metallic_core::World`]), the physics ([`metallic_physics::PhysicsWorld`]) and Humanoid
//! characters, the Luau scripts ([`metallic_script::ScriptHost`]), the third-person
//! [`CameraController`] and the in-game GUI ([`gui`]). The local character is drawn as the
//! skinned default avatar (`art/avatar/out/default_avatar.glb`), animated by a small built-in
//! locomotion controller (idle, walk, run, jump, fall, land) until the animation system (#40)
//! replaces it. The player app and Studio's Play mode both
//! drive a `Game`: [`Game::load`], [`Game::start`], then [`Game::frame`] once per displayed
//! frame, [`Game::render`] and [`gui::paint`], and finally [`Game::stop`].
//!
//! Simulation runs at a fixed [`TICK_RATE`] (60 Hz) whatever the display rate, so a headless run
//! of N frames of 1/60 s always does the same thing. See [`Game::frame`] for the order of work
//! inside a tick.
#![warn(missing_docs)]

mod avatar;
mod camera_controller;
mod game;
pub mod gui;
mod input;
mod locomotion;
mod project;

pub use camera_controller::CameraController;
pub use game::{FrameOutput, Game, MAX_FRAME_DT, RUN_MULTIPLIER, StdoutSink, TICK, TICK_RATE};
pub use input::{Action, FrameInput, InputState};
pub use project::ProjectConfig;

use std::path::PathBuf;

/// Why a game could not be loaded or started.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// `metallic.toml` is missing, unreadable or invalid.
    #[error("project file {path}: {reason}")]
    Config {
        /// The `metallic.toml` path.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The scene file named by the project (or `--scene`) does not exist.
    #[error("scene file not found: {}", .0.display())]
    MissingScene(PathBuf),
    /// The scene file exists but could not be loaded.
    #[error("cannot load scene {}: {source}", .path.display())]
    Scene {
        /// The scene path.
        path: PathBuf,
        /// The loader's error.
        source: metallic_core::CoreError,
    },
    /// The scripts or the local player could not be started.
    #[error("cannot start the game: {0}")]
    Start(String),
}
