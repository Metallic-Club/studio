//! Editor actions as data: the toolbar, menus and keyboard all run actions from one registry.
//!
//! # Seam: adding a toolbar action
//!
//! An [`Action`] has an id (also its keymap id, see [`crate::keymap`]), a label, an icon glyph,
//! a toolbar group, an `enabled` predicate, an optional `checked` predicate (for tool toggles)
//! and a handler, plus an optional *accessory*: a small widget drawn right after its toolbar
//! button that keeps its own state. Team Test (task A12) registers its action with a player-count
//! accessory, without touching the toolbar code:
//!
//! ```ignore
//! let players = Rc::new(Cell::new(2u8));
//! let p = players.clone();
//! registry.register(
//!     Action::new("play.team_test", "Team Test", Icon::Players, ToolbarGroup::Play, move |s| s.request(UiRequest::StartPlay("play.team_test")))
//!         .enabled(|s| !s.play.is_playing())
//!         .accessory(move |ui, _| { let mut n = p.get(); ui.add(egui::DragValue::new(&mut n).range(1..=8)); p.set(n); }),
//! );
//! ```
//!
//! A play action asks the UI with [`UiRequest::StartPlay`] rather than calling
//! [`EditorState::start_play`] itself, so the UI first commits open script tabs and ends any
//! live-preview gesture: Play runs what is on screen. While a session runs, every editing
//! action (tools, snapping, undo, delete...) is disabled, accessories included.
//!
//! The shortcut shown in the tooltip comes from the [`Keymap`], so rebinding a key updates every
//! place that shows it.
use eframe::egui::{self, RichText};

use metallic_render::glam::DVec3;

use crate::editor::{EditorState, MOVE_STEPS, PivotMode, ROTATE_STEPS, Space, Tool, UiRequest};
use crate::icons::Icon;
use crate::keymap::Keymap;
use crate::theme;
use crate::tokens::ui_tokens;
use crate::widgets::button::Button;

/// Where an action appears on the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarGroup {
    /// Not on the toolbar (menus and keys only).
    Hidden,
    /// Select / Move / Rotate / Scale.
    Tools,
    /// Increment, space and pivot settings.
    Settings,
    /// Snap target chips (Grid, Surface, Vertex, Edge, Align), drawn as text toggles.
    Snap,
    /// Undo, redo.
    Edit,
    /// Play cluster (centred): play modes and Stop.
    Play,
}

type Predicate = Box<dyn Fn(&EditorState) -> bool>;
type Handler = Box<dyn FnMut(&mut EditorState)>;
type Accessory = Box<dyn FnMut(&mut egui::Ui, &mut EditorState)>;

/// One editor action.
pub struct Action {
    /// Stable id, e.g. `edit.undo`.
    pub id: &'static str,
    /// Label in menus and the toolbar tooltip.
    pub label: String,
    /// Icon on the toolbar button.
    pub icon: Icon,
    /// Toolbar placement.
    pub group: ToolbarGroup,
    enabled: Option<Predicate>,
    checked: Option<Predicate>,
    handler: Handler,
    accessory: Option<Accessory>,
}

impl Action {
    /// A new action that is always enabled.
    pub fn new(
        id: &'static str,
        label: &str,
        icon: Icon,
        group: ToolbarGroup,
        handler: impl FnMut(&mut EditorState) + 'static,
    ) -> Action {
        Action {
            id,
            label: label.to_string(),
            icon,
            group,
            enabled: None,
            checked: None,
            handler: Box::new(handler),
            accessory: None,
        }
    }

    /// Sets the enabled predicate.
    pub fn enabled(mut self, f: impl Fn(&EditorState) -> bool + 'static) -> Action {
        self.enabled = Some(Box::new(f));
        self
    }

    /// Makes the action a toggle whose state `f` reports.
    pub fn checked(mut self, f: impl Fn(&EditorState) -> bool + 'static) -> Action {
        self.checked = Some(Box::new(f));
        self
    }

    /// Adds a widget drawn after the toolbar button.
    pub fn accessory(mut self, f: impl FnMut(&mut egui::Ui, &mut EditorState) + 'static) -> Action {
        self.accessory = Some(Box::new(f));
        self
    }

    /// True when the action can run now.
    pub fn is_enabled(&self, s: &EditorState) -> bool {
        self.enabled.as_ref().is_none_or(|f| f(s))
    }

    /// True for a toggle that is on.
    pub fn is_checked(&self, s: &EditorState) -> bool {
        self.checked.as_ref().is_some_and(|f| f(s))
    }
}

/// All actions, in registration order.
#[derive(Default)]
pub struct ActionRegistry {
    actions: Vec<Action>,
}

impl ActionRegistry {
    /// An empty registry.
    pub fn new() -> ActionRegistry {
        ActionRegistry::default()
    }

    /// Adds an action, replacing one with the same id.
    pub fn register(&mut self, action: Action) {
        if let Some(i) = self.actions.iter().position(|a| a.id == action.id) {
            self.actions[i] = action;
        } else {
            self.actions.push(action);
        }
    }

    /// The action with `id`.
    pub fn get(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// Every action.
    pub fn iter(&self) -> impl Iterator<Item = &Action> {
        self.actions.iter()
    }

    /// Runs `id` if it exists and is enabled; returns whether it ran.
    pub fn run(&mut self, id: &str, state: &mut EditorState) -> bool {
        match self.actions.iter_mut().find(|a| a.id == id) {
            Some(a) if a.is_enabled(state) => {
                (a.handler)(state);
                true
            }
            _ => false,
        }
    }

    /// Draws the buttons of `group` into a horizontal toolbar.
    pub fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        group: ToolbarGroup,
        state: &mut EditorState,
        keymap: &Keymap,
    ) {
        let mut clicked = None;
        // Stop always sits after the play modes, however many register.
        let mut order: Vec<usize> = (0..self.actions.len())
            .filter(|&i| self.actions[i].group == group)
            .collect();
        order.sort_by_key(|&i| self.actions[i].id == "play.stop");
        for i in order {
            let a = &mut self.actions[i];
            let enabled = a.is_enabled(state);
            let checked = a.is_checked(state);
            let shortcut = keymap.label(a.id);
            let mut tip = if shortcut.is_empty() {
                a.label.clone()
            } else {
                format!("{} ({shortcut})", a.label)
            };
            if a.id == "play.start" && !enabled && !state.play.is_playing() {
                tip.push_str(
                    "\nPlay is not connected yet: no play mode is registered in this build.",
                );
            }
            // Tools, Play and Stop show their key as a cap; the rest are icon buttons whose
            // tooltip names the key.
            let labelled = matches!(a.group, ToolbarGroup::Tools | ToolbarGroup::Play);
            let key = shortcut.as_str();
            let name = a.label.as_str();
            let mut button = if a.id == "play.start" {
                Button::primary(name)
            } else if a.group == ToolbarGroup::Play {
                Button::secondary(name)
            } else {
                Button::ghost(if labelled { name } else { "" })
            };
            button = button.compact().glyph(a.icon).selected(checked);
            if !labelled {
                button = button.alt(&tip);
            }
            if labelled && !key.is_empty() {
                button = button.keycap(key);
            }
            let mut resp = ui.add_enabled(enabled, button);
            if labelled {
                resp = resp.on_hover_text(&tip);
            }
            let resp = resp.on_disabled_hover_text(&tip);
            if resp.clicked() {
                clicked = Some(a.id);
            }
            if let Some(acc) = &mut a.accessory {
                acc(ui, state);
            }
        }
        if let Some(id) = clicked {
            self.run(id, state);
        }
    }

    /// Draws the actions of `group` as text toggle chips (the snap targets).
    pub fn chips(
        &mut self,
        ui: &mut egui::Ui,
        group: ToolbarGroup,
        state: &mut EditorState,
        keymap: &Keymap,
    ) {
        let mut clicked = None;
        for a in self.actions.iter_mut().filter(|a| a.group == group) {
            let shortcut = keymap.label(a.id);
            let tip = if shortcut.is_empty() {
                a.label.clone()
            } else {
                format!("{} ({shortcut})", a.label)
            };
            let checked = a.is_checked(state);
            let button = Button::ghost(short_label(&a.label))
                .compact()
                .selected(checked);
            let enabled = a.is_enabled(state);
            let resp = ui
                .add_enabled(enabled, button)
                .on_hover_text(&tip)
                .on_disabled_hover_text(&tip);
            if resp.clicked() {
                clicked = Some(a.id);
            }
            if let Some(acc) = &mut a.accessory {
                ui.add_enabled_ui(enabled, |ui| acc(ui, state));
            }
        }
        if let Some(id) = clicked {
            self.run(id, state);
        }
    }

    /// Draws menu entries for `ids` (label and shortcut).
    pub fn menu_items(
        &mut self,
        ui: &mut egui::Ui,
        ids: &[&str],
        state: &mut EditorState,
        keymap: &Keymap,
    ) {
        let mut clicked = None;
        for &id in ids {
            let Some(a) = self.get(id) else { continue };
            let button = egui::Button::new(&a.label).shortcut_text(keymap.label(id));
            if theme::add_enabled(ui, a.is_enabled(state), button).clicked() {
                clicked = Some(id);
            }
        }
        if let Some(id) = clicked {
            self.run(id, state);
            ui.close();
        }
    }
}

/// The chip text: the first word of the label ("Grid snapping" shows "Grid").
fn short_label(label: &str) -> &str {
    label.split(' ').next().unwrap_or(label)
}

/// An increment without trailing zeros ("0.25", "1", "11.25").
fn fmt_step(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The built-in editor actions.
pub fn default_actions() -> ActionRegistry {
    let mut r = ActionRegistry::new();
    let editing = |s: &EditorState| !s.play.is_playing();
    let has_selection = |s: &EditorState| !s.play.is_playing() && !s.selection.is_empty();

    for (id, label, icon, tool) in [
        ("tool.select", "Select", Icon::Select, Tool::Select),
        ("tool.move", "Move", Icon::Move, Tool::Move),
        ("tool.rotate", "Rotate", Icon::Rotate, Tool::Rotate),
        ("tool.scale", "Scale", Icon::Scale, Tool::Scale),
        (
            "tool.transform",
            "Transform",
            Icon::Text("T"),
            Tool::Transform,
        ),
    ] {
        r.register(
            Action::new(id, label, icon, ToolbarGroup::Tools, move |s| s.tool = tool)
                .checked(move |s| s.tool == tool)
                .enabled(editing),
        );
    }
    let mut off_grid: Option<((metallic_core::ChangeCursor, u64, u64, u64), bool)> = None;
    r.register(
        Action::new(
            "snap.grid",
            "Grid snapping",
            Icon::Snap,
            ToolbarGroup::Snap,
            |s| s.snapping.grid = !s.snapping.grid,
        )
        .checked(|s| s.snapping.grid)
        .accessory(move |ui, s| {
            ui.menu_button(format!("{} stud", fmt_step(s.snapping.move_step)), |ui| {
                for &step in MOVE_STEPS {
                    if ui
                        .selectable_label(
                            s.snapping.move_step == step,
                            format!("{} stud", fmt_step(step)),
                        )
                        .clicked()
                    {
                        s.snapping.move_step = step;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Move and scale increment ([ and ])");
            ui.menu_button(format!("{}°", fmt_step(s.snapping.rotate_step)), |ui| {
                for &step in ROTATE_STEPS {
                    if ui
                        .selectable_label(
                            s.snapping.rotate_step == step,
                            format!("{}°", fmt_step(step)),
                        )
                        .clicked()
                    {
                        s.snapping.rotate_step = step;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Rotate increment (Shift+[ and Shift+])");
            // Off-grid check, recomputed only when the world, selection or increments change.
            let key = (
                s.world.change_cursor(),
                s.selection.revision(),
                s.snapping.move_step.to_bits(),
                s.snapping.rotate_step.to_bits(),
            );
            let off = match off_grid {
                Some((k, v)) if k == key => v,
                _ => {
                    let v = s.off_grid();
                    off_grid = Some((key, v));
                    v
                }
            };
            if off
                && ui
                    .add(
                        egui::Button::new(
                            RichText::new("Off grid").small().color(theme::warning()),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Snap selection to grid (Ctrl+Shift+Home)")
                    .clicked()
            {
                s.snap_selection_to_grid();
            }
        }),
    );
    for (id, label, get, flip) in [
        (
            "snap.surface",
            "Surface snapping",
            (|s: &EditorState| s.snapping.surface) as fn(&EditorState) -> bool,
            (|s: &mut EditorState| s.snapping.surface = !s.snapping.surface)
                as fn(&mut EditorState),
        ),
        (
            "snap.vertex",
            "Vertex snapping (hold V for one drag)",
            |s| s.snapping.vertex,
            |s| s.snapping.vertex = !s.snapping.vertex,
        ),
        (
            "snap.edge",
            "Edge snapping",
            |s| s.snapping.edge,
            |s| s.snapping.edge = !s.snapping.edge,
        ),
        (
            "snap.align",
            "Align to surface",
            |s| s.snapping.align,
            |s| s.snapping.align = !s.snapping.align,
        ),
    ] {
        r.register(Action::new(id, label, Icon::Snap, ToolbarGroup::Snap, flip).checked(get));
    }
    r.register(
        Action::new(
            "snap.soft",
            "Soft snap",
            Icon::Snap,
            ToolbarGroup::Hidden,
            |s| {
                s.snapping.soft = !s.snapping.soft;
                let on = if s.snapping.soft { "on" } else { "off" };
                s.note(format!("Soft snap {on}"));
            },
        )
        .checked(|s| s.snapping.soft),
    );
    for (id, label, up, rotate) in [
        ("increment.move_down", "Halve move increment", false, false),
        ("increment.move_up", "Double move increment", true, false),
        (
            "increment.rotate_down",
            "Smaller rotate increment",
            false,
            true,
        ),
        ("increment.rotate_up", "Larger rotate increment", true, true),
    ] {
        r.register(Action::new(
            id,
            label,
            Icon::Snap,
            ToolbarGroup::Hidden,
            move |s| {
                if rotate {
                    s.step_rotate_increment(up);
                } else {
                    s.step_move_increment(up);
                }
            },
        ));
    }
    r.register(
        Action::new(
            "pivot.mode",
            "Pivot or selection centre",
            Icon::Text("Pv"),
            ToolbarGroup::Settings,
            |s| {
                s.pivot_mode = match s.pivot_mode {
                    PivotMode::Pivot => PivotMode::Centre,
                    PivotMode::Centre => PivotMode::Pivot,
                };
            },
        )
        .checked(|s| s.pivot_mode == PivotMode::Centre)
        .accessory(|ui, s| {
            ui.label(
                RichText::new(match s.pivot_mode {
                    PivotMode::Pivot => "Pivot",
                    PivotMode::Centre => "Centre",
                })
                .color(ui_tokens().text_dim),
            );
        }),
    );
    r.register(
        Action::new(
            "pivot.edit",
            "Edit pivot",
            Icon::Text("P"),
            ToolbarGroup::Settings,
            |s| s.pivot_edit = !s.pivot_edit,
        )
        .checked(|s| s.pivot_edit)
        .enabled(editing),
    );
    r.register(
        Action::new(
            "pivot.reset",
            "Reset pivot",
            Icon::Text("P"),
            ToolbarGroup::Hidden,
            |s| {
                s.reset_pivot();
            },
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "build.rotate90",
            "Rotate 90°",
            Icon::Rotate,
            ToolbarGroup::Hidden,
            |s| {
                s.rotate_selection(DVec3::Y, 90.0, "Rotate");
            },
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "build.tilt90",
            "Tilt 90°",
            Icon::Rotate,
            ToolbarGroup::Hidden,
            |s| {
                let axis = s.tilt_axis();
                s.rotate_selection(axis, 90.0, "Tilt");
            },
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "build.drop",
            "Drop onto surface below",
            Icon::Text("v"),
            ToolbarGroup::Hidden,
            |s| {
                s.drop_selection();
            },
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "build.snap_to_grid",
            "Snap selection to grid",
            Icon::Snap,
            ToolbarGroup::Hidden,
            |s| {
                s.snap_selection_to_grid();
            },
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "space.toggle",
            "World or local axes",
            Icon::Space,
            ToolbarGroup::Settings,
            |s| {
                s.space = match s.space {
                    Space::World => Space::Local,
                    Space::Local => Space::World,
                };
            },
        )
        .checked(|s| s.space == Space::Local)
        .enabled(editing)
        .accessory(|ui, s| {
            ui.label(
                RichText::new(match s.space {
                    Space::World => "World",
                    Space::Local => "Local",
                })
                .color(ui_tokens().text_dim),
            );
        }),
    );
    r.register(
        Action::new(
            "edit.undo",
            "Undo",
            Icon::Undo,
            ToolbarGroup::Edit,
            EditorState::undo,
        )
        .enabled(move |s| editing(s) && !s.history.is_empty()),
    );
    r.register(
        Action::new(
            "edit.redo",
            "Redo",
            Icon::Redo,
            ToolbarGroup::Edit,
            EditorState::redo,
        )
        .enabled(move |s| editing(s) && s.history.redo_name(&s.world).is_some()),
    );
    r.register(
        Action::new(
            "edit.duplicate",
            "Duplicate",
            Icon::Text("D"),
            ToolbarGroup::Hidden,
            EditorState::duplicate_selection,
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "edit.delete",
            "Delete",
            Icon::Text("X"),
            ToolbarGroup::Hidden,
            EditorState::delete_selection,
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "edit.group",
            "Group",
            Icon::Text("G"),
            ToolbarGroup::Hidden,
            EditorState::group_selection,
        )
        .enabled(has_selection),
    );
    r.register(
        Action::new(
            "edit.rename",
            "Rename",
            Icon::Text("R"),
            ToolbarGroup::Hidden,
            |s| {
                if let Some(id) = s.selection.active() {
                    s.request(UiRequest::Rename(id));
                }
            },
        )
        .enabled(has_selection),
    );
    r.register(Action::new(
        "edit.select_all",
        "Select all in Workspace",
        Icon::Text("A"),
        ToolbarGroup::Hidden,
        |s| {
            if let Some(ws) = s.world.root("Workspace") {
                let kids = s.world.children(ws).to_vec();
                s.selection.set(kids);
            }
        },
    ));
    r.register(Action::new(
        "file.save",
        "Save",
        Icon::Text("S"),
        ToolbarGroup::Hidden,
        |s| {
            if let Err(e) = s.save() {
                s.output.push(
                    crate::output::Tag::Studio,
                    crate::output::Level::Error,
                    e.to_string(),
                );
            }
        },
    ));
    r.register(Action::new(
        "file.close",
        "Close game",
        Icon::Text("W"),
        ToolbarGroup::Hidden,
        |s| s.request(UiRequest::CloseGame),
    ));
    r.register(Action::new(
        "edit.deselect",
        "Clear selection",
        Icon::Text("Esc"),
        ToolbarGroup::Hidden,
        |s| s.selection.clear(),
    ));
    r.register(Action::new(
        "settings.open",
        "Settings",
        Icon::Text(","),
        ToolbarGroup::Hidden,
        |s| s.request(UiRequest::OpenSettings),
    ));
    r.register(Action::new(
        "view.frame_selection",
        "Frame selection",
        Icon::Text("F"),
        ToolbarGroup::Hidden,
        |s| s.request(UiRequest::FrameSelection),
    ));
    r.register(Action::new(
        "view.frame_all",
        "Frame all",
        Icon::Text("H"),
        ToolbarGroup::Hidden,
        |s| s.request(UiRequest::FrameAll),
    ));
    r.register(
        Action::new("play.start", "Play", Icon::Play, ToolbarGroup::Play, |s| {
            s.request(UiRequest::StartPlay("play.start"))
        })
        .enabled(|s| !s.play.is_playing() && s.play.has_mode("play.start")),
    );
    r.register(
        Action::new(
            "play.stop",
            "Stop",
            Icon::Stop,
            ToolbarGroup::Play,
            EditorState::stop_play,
        )
        .enabled(|s| s.play.is_playing()),
    );
    lock_while_playing(&mut r);
    r
}

/// Actions that stay usable while a play session runs: the play controls, Save (it writes the
/// edit world, which Play never changes), Settings, and Close game (it stops Play, then asks
/// about unsaved changes).
pub const PLAY_SAFE: &[&str] = &[
    "play.start",
    "play.stop",
    "file.save",
    "file.close",
    "settings.open",
];

/// Disables every action not in [`PLAY_SAFE`] while playing (tools, snapping, pivots, edit
/// commands, selection, view framing). A disabled action also leaves its shortcut to the game.
/// Actions registered later decide for themselves.
fn lock_while_playing(r: &mut ActionRegistry) {
    for a in &mut r.actions {
        if PLAY_SAFE.contains(&a.id) {
            continue;
        }
        let before = a.enabled.take();
        a.enabled = Some(Box::new(move |s: &EditorState| {
            !s.play.is_playing() && before.as_ref().is_none_or(|f| f(s))
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputLog;
    use std::path::Path;

    fn state() -> EditorState {
        let log = OutputLog::new(16);
        EditorState::new(
            metallic_core::World::new_with_services(),
            Path::new("."),
            Path::new("main.mscene"),
            "Test",
            log.sink(),
        )
    }

    #[test]
    fn every_bound_key_has_an_action() {
        let r = default_actions();
        for b in Keymap::metallic().bindings() {
            if b.action.starts_with("drag.")
                || b.action.starts_with("output.")
                || b.action.starts_with("hold.")
            {
                continue; // handled by the viewport drag, held keys and the Output panel
            }
            assert!(r.get(b.action).is_some(), "no action for {}", b.action);
        }
    }

    #[test]
    fn settings_open_is_bound_to_ctrl_comma_and_asks_the_ui() {
        let keymap = Keymap::metallic();
        let chord = keymap.chord("settings.open").expect("bound");
        assert_eq!(
            chord,
            crate::keymap::KeyChord::parse("Ctrl+,").expect("parses")
        );
        assert_eq!(keymap.label("settings.open"), "Ctrl+,");
        let mut r = default_actions();
        let mut s = state();
        assert!(r.run("settings.open", &mut s));
        assert!(matches!(
            s.take_requests().as_slice(),
            [UiRequest::OpenSettings]
        ));
    }

    #[test]
    fn ctrl_w_closes_the_game_and_escape_only_clears_the_selection() {
        let keymap = Keymap::metallic();
        assert_eq!(keymap.label("file.close"), "Ctrl+W");
        let esc = crate::keymap::KeyChord::parse("Escape").expect("parses");
        let on_esc: Vec<_> = keymap
            .bindings()
            .iter()
            .filter(|b| b.chord == esc)
            .map(|b| b.action)
            .collect();
        // play.stop only in the Play context (while a game runs).
        assert_eq!(on_esc, ["play.stop", "edit.deselect", "drag.cancel"]);
        let mut r = default_actions();
        let mut s = state();
        assert!(r.run("file.close", &mut s));
        assert!(matches!(
            s.take_requests().as_slice(),
            [UiRequest::CloseGame]
        ));
        assert!(r.run("edit.deselect", &mut s));
        assert!(s.take_requests().is_empty(), "Esc asks for no close");
    }

    #[test]
    fn tools_switch_and_disabled_actions_do_not_run() {
        let mut r = default_actions();
        let mut s = state();
        assert!(r.run("tool.rotate", &mut s));
        assert_eq!(s.tool, Tool::Rotate);
        assert!(!r.run("edit.undo", &mut s), "nothing to undo");
        assert!(r.run("play.start", &mut s), "Play is registered");
        assert!(matches!(
            s.take_requests().as_slice(),
            [UiRequest::StartPlay("play.start")]
        ));
        s.play = crate::play::PlayController::new();
        assert!(!r.run("play.start", &mut s), "no Play mode registered");
    }

    #[test]
    fn editing_actions_are_off_while_playing() {
        let mut r = default_actions();
        let mut s = state();
        s.start_play("play.start");
        assert!(s.play.is_playing());
        for id in [
            "tool.move",
            "snap.surface",
            "pivot.mode",
            "edit.delete",
            "edit.select_all",
            "view.frame_all",
            "play.start",
        ] {
            assert!(r.get(id).is_some(), "{id} exists");
            assert!(
                !r.get(id).is_some_and(|a| a.is_enabled(&s)),
                "{id} runs while playing"
            );
        }
        for id in ["file.save", "file.close", "settings.open", "play.stop"] {
            assert!(r.get(id).is_some_and(|a| a.is_enabled(&s)), "{id} is off");
        }
        assert!(!r.run("tool.rotate", &mut s));
        assert_eq!(s.tool, Tool::Move);
        assert!(r.run("play.stop", &mut s));
        assert!(!s.play.is_playing());
        assert!(r.run("tool.rotate", &mut s));
    }

    #[test]
    fn registering_an_action_adds_it_without_toolbar_changes() {
        let mut r = default_actions();
        let before = r.iter().filter(|a| a.group == ToolbarGroup::Play).count();
        r.register(Action::new(
            "play.team_test",
            "Team Test",
            Icon::Players,
            ToolbarGroup::Play,
            |_| {},
        ));
        assert_eq!(
            r.iter().filter(|a| a.group == ToolbarGroup::Play).count(),
            before + 1
        );
    }
}
