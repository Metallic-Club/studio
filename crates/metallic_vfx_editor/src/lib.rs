//! The Metallic VFX editor: a Niagara-style module stack, an optional node graph, curve and gradient
//! editors, a timeline and a live preview that paints the `metallic_vfx` CPU simulation with egui.
//!
//! It is a library so Studio (M1 Task 9) can dock [`VfxEditor`] as a panel; the runnable demo is
//! `cargo run -p metallic_vfx_editor --example vfx_editor_demo`. Only `egui` is a normal dependency;
//! `eframe` is used by the examples.
//!
//! Theme values come from the Studio UI design doc (`docs/research/2026-10-30-studio-ui-design.md`,
//! section 5.13): graphite surfaces, silver icons, one restrained violet accent, no glow in the UI.

#![warn(missing_docs)]

pub mod app;
pub mod inspector;
pub mod preview;
pub mod theme;
pub mod widgets;

pub use app::{Selection, VfxEditor, View};
pub use inspector::Stage;
