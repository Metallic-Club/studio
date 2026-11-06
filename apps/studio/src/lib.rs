//! Metallic Studio: creates and edits projects. Desktop only (Windows, Linux, macOS).
//!
//! A library plus a thin binary (ADR 0009, Layout of the code). The modules:
//!
//! | Module | What |
//! |---|---|
//! | [`app`] | the window's screens as a state machine ([`app::Studio`]), the splash and its startup steps, and the shell that runs them on Studio's own winit loop and the engine's render device |
//! | [`theme`] | the website's design tokens, dark and light modes, accents, the bundled fonts |
//! | [`settings`] | `StudioSettings`, `settings.toml` in the config folder, and the Settings window |
//! | [`emblem`] | the Metallic mark: the splash's emblem and the window icon |
//!
//! Later issues add `keymap`, `dock`, `panels`, `viewport` and `projects` beside them.

// The one place Studio calls the operating system itself (`app::native`, Windows only): shaping the splash window
// and reading the system's animation switch need Win32 calls, which are unsafe. Each block says why it is sound.
#![cfg_attr(windows, allow(unsafe_code))]

pub mod app;
pub mod emblem;
pub mod settings;
pub mod theme;

pub use app::{Screen, Studio, run};

/// Studio's version, as the splash shows it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Studio's id for its per-user folders (`metallic_platform`'s `AppId`).
pub const STUDIO_APP_ID: &str = "dev.metallic.Studio";

/// Why Studio could not start or stopped.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// The event loop could not start or failed.
    #[error("the event loop failed: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    /// The window could not be opened.
    #[error("could not open the window: {0}")]
    Window(#[from] winit::error::OsError),
    /// No render device could draw into the window.
    #[error(transparent)]
    Render(#[from] metallic_engine::metallic_render::RenderError),
    /// The startup thread could not be started.
    #[error("could not start the startup thread: {0}")]
    Thread(#[from] std::io::Error),
}
