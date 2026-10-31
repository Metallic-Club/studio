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
//!     Action::new("play.team_test", "Team Test", Icon::Players, ToolbarGroup::Play, move |s| s.start_play("play.team_test"))
//!         .enabled(|s| !s.play.is_playing())
//!         .accessory(move |ui, _| { let mut n = p.get(); ui.add(egui::DragValue::new(&mut n).range(1..=8)); p.set(n); }),
//! );
//! ```
//!
//! The shortcut shown in the tooltip comes from the [`Keymap`], so rebinding a key updates every
//! place that shows it.
use eframe::egui::{self, RichText};

use crate::editor::{EditorState, Space, Tool, UiRequest};
use crate::icons::{self, Icon};
use crate::keymap::Keymap;
use crate::theme;
use crate::tokens::ui_tokens;

/// Where an action appears on the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarGroup {
    /// Not on the toolbar (menus and keys only).
    Hidden,
    /// Select / Move / Rotate / Scale.
    Tools,
    /// Snap and space settings.
    Settings,
    /// Undo, redo.
    Edit,
    /// Building operations on the selection: align, mirror, array, select similar.
    Build,
    /// Panels that open and close (Input actions).
    View,
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
            let tip = if shortcut.is_empty() {
                a.label.clone()
            } else {
                format!("{} ({shortcut})", a.label)
            };
            let button = egui::Button::new("")
                .selected(checked)
                .min_size(egui::vec2(28.0, 26.0));
            let resp = theme::add_enabled(ui, enabled, button)
                .on_hover_text(&tip)
                .on_disabled_hover_text(&tip);
            let colour = if !enabled {
                theme::text_off()
            } else if a.id == "play.start" || (a.group == ToolbarGroup::Play && a.id != "play.stop")
            {
                ui_tokens().success
            } else {
                ui_tokens().text
            };
            icons::paint(ui.painter(), resp.rect, a.icon, colour);
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
    ] {
        r.register(
            Action::new(id, label, icon, ToolbarGroup::Tools, move |s| s.tool = tool)
                .checked(move |s| s.tool == tool),
        );
    }
    r.register(
        Action::new(
            "snap.toggle",
            "Snap to increments",
            Icon::Snap,
            ToolbarGroup::Settings,
            |s| {
                s.snapping.enabled = !s.snapping.enabled;
            },
        )
        .checked(|s| s.snapping.enabled)
        .accessory(|ui, s| {
            ui.add(
                egui::DragValue::new(&mut s.snapping.move_step)
                    .range(0.05..=64.0)
                    .speed(0.05)
                    .suffix(" stud"),
            )
            .on_hover_text("Move and scale increment");
            ui.add(
                egui::DragValue::new(&mut s.snapping.rotate_step)
                    .range(1.0..=90.0)
                    .speed(0.5)
                    .suffix("°"),
            )
            .on_hover_text("Rotate increment");
        }),
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
            s.start_play("play.start")
        })
        .enabled(|s| !s.play.is_playing() && s.play.modes().any(|(id, _)| id == "play.start")),
    );
    r.register(
        Action::new("play.stop", "Stop", Icon::Stop, ToolbarGroup::Play, |s| {
            s.play.stop()
        })
        .enabled(|s| s.play.is_playing()),
    );
    r
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
        // The editor's registry: the built-in actions plus the building tools and panels.
        let ui = crate::editor_ui::EditorUi::new(state(), crate::output::OutputPanel::new());
        let r = &ui.actions;
        for b in Keymap::metallic().bindings() {
            if b.action.starts_with("drag.") || b.action.starts_with("output.") {
                continue; // handled by the viewport drag and the Output panel
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
    fn tools_switch_and_disabled_actions_do_not_run() {
        let mut r = default_actions();
        let mut s = state();
        assert!(r.run("tool.rotate", &mut s));
        assert_eq!(s.tool, Tool::Rotate);
        assert!(!r.run("edit.undo", &mut s), "nothing to undo");
        assert!(!r.run("play.start", &mut s), "no Play mode registered");
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
