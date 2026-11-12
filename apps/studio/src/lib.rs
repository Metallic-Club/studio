//! Metallic Studio: creates and edits projects. Desktop only (Windows, Linux, macOS).
//!
//! A library plus a thin binary (ADR 0009, Layout of the code). The modules:
//!
//! | Module | What |
//! |---|---|
//! | [`app`] | the window's screens as a state machine ([`app::Studio`]), the splash and its startup steps, and the shell that runs them on Studio's own winit loop and the engine's render device |
//! | [`theme`] | the website's design tokens, dark and light modes, accents, the bundled fonts |
//! | [`settings`] | `StudioSettings`, `settings.toml` in the config folder, and the Settings window |
//! | [`keymap`] | the command registry, focus contexts and bindings (keys, mouse, wheel) every shortcut goes through |
//! | [`projects`] | the projects page, `projects.toml`, the templates, and the file dialogs behind a trait |
//! | [`editor`] | the open project, its game, and its panels' state; opening it off the UI thread |
//! | [`dock`] | the panel registry, the docked layout (egui_tiles) and `layout.json` |
//! | [`panels`] | the panels: Viewport, Explorer, Properties, Assets, Output, Settings |
//! | [`toolbar`] | the editor's toolbar: tabs of tools, spacers and separators, `toolbar.json`, and the tool registry plugins will add to |
//! | [`logging`] | the log the Output panel shows: the engine's `log` lines and the scripts' output |
//! | [`viewport`] | the editor camera, and the viewport's state between its panel and the shell that draws the game |
//! | [`emblem`] | the Metallic mark: the splash's emblem and the window icon |
//! | [`cli`] | the command line: a project to open straight into the editor, and registering `.mtproj` files |
//! | [`folders`] | Studio's folders: its own (`%APPDATA%\Metallic\Studio`, the icons in it) and the games folder (`Documents\Metallic`) |

// The one place Studio calls the operating system itself (`app::native`, Windows only): shaping the splash window
// and reading the system's animation switch need Win32 calls, which are unsafe. Each block says why it is sound.
#![cfg_attr(windows, allow(unsafe_code))]

pub mod app;
pub mod cli;
pub mod code;
pub mod dock;
pub mod editor;
pub mod emblem;
pub mod folders;
pub mod keymap;
pub mod logging;
pub mod panels;
pub mod projects;
pub mod settings;
pub mod theme;
pub mod toolbar;
pub mod viewport;

pub use app::{Screen, Studio, run};

/// Studio's version, as the splash shows it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Studio's id for its per-user folders (`metallic_platform`'s `AppId`).
pub const STUDIO_APP_ID: &str = "dev.metallic.Studio";

/// The `.mtproj` file type, as `--register-file-types` associates it with Studio for the current user.
pub const PROJECT_FILE_TYPE: metallic_engine::metallic_platform::file_types::FileType =
    metallic_engine::metallic_platform::file_types::FileType {
        extension: metallic_engine::metallic_project::PROJECT_EXTENSION,
        mime_type: "application/x-metallic-project",
        prog_id: "Metallic.Project",
        description: "Metallic project",
        app_id: STUDIO_APP_ID,
        app_name: "Metallic Studio",
    };

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
