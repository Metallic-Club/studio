//! The Input Actions panel (#253) docked in the editor: a resizable side panel opened from
//! View > Input actions and the toolbar.
//!
//! The panel runs with [`InputActionsPanel::show_with_sink`]: each finished [`EditStep`] becomes
//! one [`Command::Step`] in the editor's [`History`](crate::commands::History), so Ctrl+Z in the
//! editor undoes panel edits and the panel shows no undo buttons of its own. Steps that carry a
//! merge key (a value dragged over many frames) fold into the newest step until the panel seals
//! the gesture, as the panel's own history would.
//!
//! Undo of a panel delete rebuilds the instances with new ids. The history rewrites its own
//! commands and queues the id map; [`InputActionsDock::follow_history`] forwards every queued map
//! to [`InputActionsPanel::follow_remap`] before the panel draws, so its selection, touch view and
//! rename baseline follow the new ids.
use std::cell::Cell;
use std::rc::Rc;

use eframe::egui;
use metallic_core::World;

use crate::actions::{Action, ActionRegistry, ToolbarGroup};
use crate::commands::Command;
use crate::editor::EditorState;
use crate::icons::Icon;
use crate::input_actions::commands::Edit;
use crate::input_actions::{CommandSink, EditStep, InputActionsPanel};

/// Action id of View > Input actions (toolbar toggle; no single-key shortcut, spec 10).
pub const TOGGLE_ID: &str = "view.input_actions";

/// What the panel handed over during one frame, in order.
enum SinkEvent {
    Record(EditStep),
    Seal,
}

/// Collects the panel's steps while it borrows the world; they reach the history right after.
#[derive(Default)]
struct Buffer {
    events: Vec<SinkEvent>,
}

impl CommandSink for Buffer {
    fn record(&mut self, step: EditStep) {
        self.events.push(SinkEvent::Record(step));
    }

    fn seal(&mut self) {
        self.events.push(SinkEvent::Seal);
    }
}

/// The docked panel and its link to the editor's history.
pub struct InputActionsDock {
    panel: InputActionsPanel,
    open: Rc<Cell<bool>>,
    /// History revision right after the panel's newest step; a merge needs it unchanged.
    merge_revision: Option<u64>,
    sealed: bool,
    was_dirty: bool,
}

impl InputActionsDock {
    /// A closed dock for `world`; registers the View toggle in `actions`.
    pub fn new(world: &World, actions: &mut ActionRegistry) -> InputActionsDock {
        let open = Rc::new(Cell::new(false));
        let (toggle, shown) = (open.clone(), open.clone());
        actions.register(
            Action::new(
                TOGGLE_ID,
                "Input actions",
                Icon::Text("IA"),
                ToolbarGroup::View,
                move |_| toggle.set(!toggle.get()),
            )
            .checked(move |_| shown.get()),
        );
        InputActionsDock {
            panel: InputActionsPanel::new(world),
            open,
            merge_revision: None,
            sealed: true,
            was_dirty: false,
        }
    }

    /// True while the panel shows.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Shows or hides the panel.
    pub fn set_open(&self, open: bool) {
        self.open.set(open);
    }

    /// The panel (for tests and the Play menu's Preview as).
    pub fn panel(&self) -> &InputActionsPanel {
        &self.panel
    }

    /// Mutable panel.
    pub fn panel_mut(&mut self) -> &mut InputActionsPanel {
        &mut self.panel
    }

    /// Forwards every id map the history queued (undo or redo of a panel delete) to the panel,
    /// and resets the rename baseline after a save. Runs every frame, open or not.
    pub fn follow_history(&mut self, state: &mut EditorState) {
        for map in state.history.take_remaps() {
            self.panel.follow_remap(&map);
        }
        let dirty = state.is_dirty();
        if self.was_dirty && !dirty {
            self.panel.mark_saved(&state.world);
        }
        self.was_dirty = dirty;
    }

    /// Draws the panel into `ui` (the editor puts it in a side panel) and records its edits.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        self.follow_history(state);
        let playing = state.play.is_playing();
        let mut buffer = Buffer::default();
        ui.add_enabled_ui(!playing, |ui| {
            if playing {
                ui.label("Stop the game to edit input actions.");
            }
            self.panel.show_with_sink(ui, &mut state.world, &mut buffer);
        });
        self.flush(state, buffer);
        // A focused panel row keeps the editor's keymap quiet; undo keys over the panel still
        // reach the editor's history (text fields keep their own undo).
        let rect = ui.min_rect();
        if !playing && ui.rect_contains_pointer(rect) && !ui.ctx().text_edit_focused() {
            let (undo, redo) = ui.input_mut(|i| {
                let undo = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
                let redo = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || i.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    );
                (undo, redo)
            });
            if undo {
                state.undo();
            }
            if redo {
                state.redo();
            }
        }
    }

    fn flush(&mut self, state: &mut EditorState, buffer: Buffer) {
        for event in buffer.events {
            match event {
                SinkEvent::Seal => self.sealed = true,
                SinkEvent::Record(step) => {
                    if !self.merge_into_last(state, &step) {
                        state.record(Command::Step(step));
                    }
                    self.merge_revision = Some(state.history.revision());
                    self.sealed = false;
                }
            }
        }
    }

    /// Folds `step` into the newest history step when both carry the same merge key, the
    /// gesture is not sealed and nothing else was recorded or undone since.
    fn merge_into_last(&mut self, state: &mut EditorState, step: &EditStep) -> bool {
        if self.sealed
            || step.merge_key.is_none()
            || self.merge_revision != Some(state.history.revision())
            || !state.is_dirty()
        {
            return false;
        }
        let Some(Command::Step(top)) = state.history.last_mut() else {
            return false;
        };
        if top.merge_key != step.merge_key {
            return false;
        }
        for edit in &step.edits {
            // Keep the first old value of a property and the last new one.
            if let Edit::Set { id, prop, new, .. } = edit
                && let Some(Edit::Set { new: top_new, .. }) = top
                    .edits
                    .iter_mut()
                    .find(|e| matches!(e, Edit::Set { id: i, prop: p, .. } if i == id && p == prop))
            {
                *top_new = new.clone();
                continue;
            }
            top.edits.push(edit.clone());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_actions::StepBuilder;
    use crate::output::OutputLog;
    use metallic_core::Value;
    use std::path::Path;

    fn state() -> EditorState {
        let log = OutputLog::new(16);
        EditorState::new(
            World::new_with_services(),
            Path::new("."),
            Path::new("main.mscene"),
            "Test",
            log.sink(),
        )
    }

    #[test]
    fn merged_steps_are_one_undo_step_and_seal_ends_the_gesture() {
        let mut s = state();
        let mut actions = ActionRegistry::new();
        let mut dock = InputActionsDock::new(&s.world, &mut actions);
        let rs = s.world.root("ReplicatedStorage").unwrap();
        let folder = s.world.create("Folder", Some(rs)).unwrap();
        let drag = |world: &mut World, v: &str| {
            let mut b = StepBuilder::new(world);
            b.set(folder, "Name", Value::String(v.into())).unwrap();
            b.finish_merging("Change Name", "name").unwrap()
        };
        for v in ["A", "B", "C"] {
            let step = drag(&mut s.world, v);
            let mut buffer = Buffer::default();
            buffer.record(step);
            dock.flush(&mut s, buffer);
        }
        assert_eq!(s.history.len(), 1, "one drag, one step");
        let mut buffer = Buffer::default();
        buffer.seal();
        buffer.record(drag(&mut s.world, "D"));
        dock.flush(&mut s, buffer);
        assert_eq!(s.history.len(), 2, "a sealed gesture starts a new step");
        s.undo();
        s.undo();
        assert_eq!(
            s.world.get(folder, "Name").unwrap(),
            Value::String("Folder".into())
        );
        assert!(actions.get(TOGGLE_ID).is_some());
    }
}
