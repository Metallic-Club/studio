//! The Input Actions panel (issue #253, spec `docs/design/input-actions.md` section 10).
//!
//! Self-contained: it edits the `InputContext` > `InputAction` > `InputBinding` instances of a
//! [`World`](metallic_core::World) and needs nothing else from the editor. It docks into the
//! editor shell once #10 exists; until then [`InputActionsPanel::show`] can be put in any `Ui`.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`commands`] | Undo-ready edits: [`StepBuilder`], [`EditStep`], [`History`], [`CommandSink`] |
//! | [`model`] | The tree as the panel reads it; device columns, labels, instance snapshots |
//! | [`capture`] | Listen: "press a key, button or move a stick" |
//! | [`checks`] | The Checks strip rules (spec 10.1) |
//! | [`presets`] | Role presets: add an action from a role, one-handed and default bindings |
//! | [`touch`] | Device presets, the automatic touch layout, custom positions, editing rules |
//! | [`preview`] | "Preview as", the touch simulation, the virtual gamepad, Play key routing, keymap data |
//! | `view`, `view_touch`, `style` | The egui view |
//!
//! Every change goes through one [`EditStep`] handed to a [`CommandSink`]: one click, one drag
//! of a touch control or one preset is one undo step.

pub mod capture;
pub mod checks;
pub mod commands;
pub mod model;
pub mod presets;
pub mod preview;
mod style;
pub mod touch;
mod view;
mod view_touch;

pub use commands::{CommandSink, EditStep, History, StepBuilder};
pub use view::{InputActionsPanel, Tab};
