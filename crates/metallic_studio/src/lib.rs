//! Metallic Studio: the editor application.
//!
//! This crate holds the app shell (window, theme, splash), the start screen, the template
//! handling, and the installer side: [`updater`] (update check, download, apply) and
//! [`github_auth`] (GitHub device sign-in and token storage). The 3D viewport, Explorer and the
//! rest of the editor arrive in later tasks and plug into [`app::StudioApp`].
//!
//! Library code returns [`StudioError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod app;
pub mod error;
pub mod github_auth;
pub mod input_actions;
pub mod png_image;
pub mod recent;
pub mod screenshot;
pub mod start_screen;
pub mod templates;
pub mod theme;
pub mod updater;

pub use error::StudioError;
