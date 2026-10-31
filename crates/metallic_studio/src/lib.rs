//! Metallic Studio: the editor application.
//!
//! This crate holds the app shell (window, theme, splash), the start screen, the template
//! handling, and the installer side: [`updater`] (update check, download, apply) and
//! [`github_auth`] (GitHub device sign-in and token storage). The 3D viewport, Explorer and the
//! rest of the editor arrive in later tasks and plug into [`app::StudioApp`]. [`colour`] holds the
//! building colour popover, its colour maths and palette files; until the editor exists it runs
//! as a demo screen (`--scene colour`). [`tools`] holds the pure cores of the building tools
//! (shared geometry, scatter, selection, measure, arrange) that the viewport will drive. [`widgets`]
//! holds Studio's own buttons, plates and the cut.
//!
//! Library code returns [`StudioError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod app;
pub mod colour;
pub mod data_dirs;
pub mod error;
pub mod github_auth;
pub mod png_image;
pub mod recent;
pub mod screenshot;
pub mod splash;
pub mod start_screen;
pub mod templates;
pub mod theme;
pub mod tools;
pub mod updater;
pub mod widgets;

pub use error::StudioError;
