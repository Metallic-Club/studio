//! egui widgets for editing [`metallic_anim`] clips, ready to embed in Metallic Studio.
//!
//! | Widget | What it does |
//! |---|---|
//! | [`DopeSheet`] | Track rows with key glyphs: select, box-select, drag, scale, snap, scrub, zoom and pan. |
//! | [`CurveEditor`] | Value curves of the active track with Bezier handles, tangent modes and frame-all. |
//! | [`TransportBar`] | Play/pause, previous/next key, start/end, loop, snapping, speed, time readout. |
//! | [`KeyInspector`] | Clip settings, the selected key's time and value, interpolation and an easing gallery. |
//! | [`preview`] | Painter-only previews: a 2D bone rig and a blocky 3D figure. |
//!
//! All widgets edit one [`EditorState`]; its methods ([`EditorState::move_selected`],
//! [`EditorState::scale_selected`], ...) are the only way keys change, so Studio can wrap them
//! in undoable commands. [`DemoApp`] puts everything together; run it with
//! `cargo run -p metallic_anim_editor --example anim_editor_demo`.
#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod curve_editor;
mod demo;
mod dope_sheet;
mod inspector;
pub mod paint;
pub mod preview;
pub mod scalar;
mod state;
pub mod theme;
mod transport;

pub use curve_editor::CurveEditor;
pub use demo::{
    DemoApp, EditorTab, bone_rig_2d, figure_parts, figure_skeleton, walk_clip, wave_clip,
};
pub use dope_sheet::{
    BAND_HEIGHT, DopeSheet, DopeSheetLayout, LABEL_WIDTH, ROW_HEIGHT, RULER_HEIGHT, track_labels,
};
pub use inspector::KeyInspector;
pub use state::{DragOp, EditorState, KeyRef, TimeView, ValueView};
pub use theme::Theme;
pub use transport::TransportBar;
