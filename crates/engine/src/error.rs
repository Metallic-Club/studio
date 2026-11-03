//! The engine's error type.

use metallic_classes::GameError;
use metallic_script::HostError;
use thiserror::Error;

/// Why the engine could not be made, started or run.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The settings cannot run (a zero tick rate, ...).
    #[error("invalid engine settings: {0}")]
    Config(&'static str),
    /// The game's data model could not be made, or lacks something the engine needs (`RunService`).
    #[error(transparent)]
    Game(#[from] GameError),
    /// The script host refused (a game is already running, the registry lacks the script classes, ...).
    #[error(transparent)]
    Script(#[from] HostError),
    /// The Luau backend could not be created.
    #[cfg(feature = "luau")]
    #[error(transparent)]
    Luau(#[from] metallic_luau::LuauError),
    /// A stress scene could not be built ([`crate::stress`]).
    #[error(transparent)]
    Stress(#[from] crate::stress::StressError),
    /// [`Engine::start`](crate::Engine::start) was called while the game runs.
    #[error("the game is already running")]
    AlreadyRunning,
    /// A windowed engine was asked for in a build without the `render` feature, or a headless engine was handed to
    /// the windowed driver.
    #[error("{0}")]
    Mode(&'static str),
    /// The platform layer failed (the event loop, a window).
    #[cfg(feature = "render")]
    #[error(transparent)]
    Platform(#[from] metallic_platform::PlatformError),
    /// The render hook could not attach to the window.
    #[cfg(feature = "render")]
    #[error("the renderer could not attach to the window: {0}")]
    Render(Box<dyn std::error::Error + Send + Sync>),
}
