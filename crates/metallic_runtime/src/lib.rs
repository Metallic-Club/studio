//! The Metallic Engine runtime: one running game.
//!
//! A [`Game`] ties the pieces of the engine together for one project: the Instance tree
//! ([`metallic_core::World`]), the physics ([`metallic_physics::PhysicsWorld`]) and Humanoid
//! characters, the Luau scripts ([`metallic_script::ScriptHost`]), the third-person
//! [`CameraController`] and the in-game GUI ([`gui`]). The local character is drawn as the
//! skinned default avatar (`art/avatar/out/default_avatar.glb`), animated by a `metallic_anim`
//! state machine (idle, walk, run, jump, fall, land; see `locomotion`). The player app and
//! Studio's Play mode both drive a `Game`: [`Game::load`], [`Game::start`], then
//! [`Game::frame`] once per displayed frame, [`Game::render`] and [`gui::paint`], and finally
//! [`Game::stop`].
//!
//! Input is read as actions ([`input`]): the app feeds raw events (keys, mouse, gamepad, scripted
//! events) to an [`InputRuntime`], which evaluates the scene's `InputContext`s once per frame and
//! hands the character's actions to [`Game::frame`] as a [`FrameInput`].
//!
//! Simulation runs at a fixed [`TICK_RATE`] (60 Hz) whatever the display rate, so a headless run
//! of N frames of 1/60 s always does the same thing. See [`Game::frame`] for the order of work
//! inside a tick.
//!
//! The runtime caches physical part signatures and Humanoid-to-Model membership dependencies.
//! Journaled topology changes update affected cache entries; ordinary Humanoid properties and
//! part poses do not trigger physics synchronization. Lost history rebuilds the runtime cache.
#![warn(missing_docs)]

#[cfg(test)]
mod alloc_count;
mod avatar;
mod body_pose;
mod camera_controller;
mod debug_draw;
mod game;
pub mod gui;
pub mod gui_input;
pub mod input;
mod local_input;
mod locomotion;
mod physics_sync;
mod project;

pub use camera_controller::CameraController;
pub use game::{FrameOutput, Game, MAX_FRAME_DT, RUN_MULTIPLIER, StdoutSink, TICK, TICK_RATE};
pub use input::{FrameInput, InputRuntime};
pub use local_input::{LocalInput, move_direction};
pub use metallic_physics::{DebugDrawOptions, HumanoidState};
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
