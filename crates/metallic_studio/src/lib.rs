//! Metallic Studio: the editor application.
//!
//! This crate holds the app shell (window, theme, splash), the start screen and the template
//! handling. The 3D viewport, Explorer and the rest of the editor arrive in later tasks and plug
//! into [`app::StudioApp`].
//!
//! Library code returns [`StudioError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod app;
pub mod error;
pub mod png_image;
pub mod recent;
pub mod screenshot;
pub mod start_screen;
pub mod templates;
pub mod theme;

pub use error::StudioError;
