//! Metallic Studio: the editor application.
//!
//! This crate holds the app shell (window, theme, splash), the start screen, the template
//! handling, the editor, and the installer side: [`updater`] (update check, download, apply) and
//! [`github_auth`] (GitHub device sign-in and token storage).
//!
//! # The editor
//!
//! - [`editor::EditorState`]: the open game's world, [`selection::Selection`],
//!   [`commands::History`] (every edit is a [`commands::Command`]), tool settings, Output sink and
//!   [`play::PlayController`]. No UI.
//! - [`editor_ui::EditorUi`]: the window layout around the panels: [`viewport`] (3D view, picking,
//!   [`gizmo`]s, [`camera`]), [`explorer`], [`properties`], [`output`] and [`script_editor`].
//! - [`actions`] and [`keymap`]: toolbar, menu and keyboard commands as data.
//!
//! Seams for later work, documented in their modules: actions as data ([`actions`]), the tagged
//! non-blocking Output sink ([`output`]), play modes with start/stop/status hooks ([`play`]),
//! and the undo, selection and gizmo APIs the building tools use ([`commands`], [`selection`],
//! [`gizmo`]). Panels take colours, font sizes and metrics only from [`tokens::ui_tokens`].
//!
//! Library code returns [`StudioError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod actions;
pub mod app;
pub mod camera;
pub mod commands;
pub mod editor;
pub mod editor_ui;
pub mod error;
pub mod explorer;
pub mod expr;
pub mod github_auth;
pub mod gizmo;
pub mod icons;
pub mod keymap;
pub mod output;
pub mod picking;
pub mod play;
pub mod png_image;
pub mod properties;
pub mod recent;
pub mod screenshot;
pub mod script_editor;
pub mod selection;
pub mod start_screen;
pub mod templates;
pub mod theme;
pub mod tokens;
pub mod transform;
pub mod updater;
pub mod viewport;

pub use error::StudioError;
