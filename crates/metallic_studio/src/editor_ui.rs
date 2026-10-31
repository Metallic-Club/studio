//! The editor window layout: menu bar, toolbar, Explorer (left), Properties (right), Output
//! (bottom), the scene/script tabs in the centre and the status bar.
//!
//! [`EditorUi`] owns the panels, the [`ActionRegistry`](crate::actions::ActionRegistry) and the
//! [`Keymap`]; [`EditorState`] owns the data. Keys are dispatched once per frame through the
//! keymap to actions: global keys always (except while typing in a text field, where only Save
//! and Play keys work), viewport keys only while the pointer is over the 3D view and the right
//! button is up (so WASD flies the camera while it is held).
use eframe::egui::{self, RichText};
use eframe::egui_wgpu;

use crate::actions::{ActionRegistry, ToolbarGroup, default_actions};
use crate::editor::{EditorState, INSERTABLE, UiRequest};
use crate::explorer::Explorer;
use crate::keymap::{KeyContext, Keymap};
use crate::output::OutputPanel;
use crate::properties::Properties;
use crate::script_editor::ScriptTabs;
use crate::theme;
use crate::tokens::ui_tokens;
use crate::viewport::Viewport;

/// What the editor asks the app shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorExit {
    /// Keep editing.
    Stay,
    /// Close the game and go back to the start screen.
    Close,
    /// Quit Studio (the window's close button or Alt+F4, after the unsaved-changes prompt).
    Quit,
}

/// What the unsaved-changes prompt is guarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leave {
    /// File > Close game.
    Close,
    /// Closing the window.
    Quit,
}

/// The editor window for one open game.
pub struct EditorUi {
    /// Data and edit entry points.
    pub state: EditorState,
    /// Actions run by toolbar, menus and keys.
    pub actions: ActionRegistry,
    /// Shortcuts.
    pub keymap: Keymap,
    /// Output panel (owns the log the state's sink feeds).
    pub output: OutputPanel,
    /// 3D view.
    pub viewport: Viewport,
    explorer: Explorer,
    properties: Properties,
    scripts: ScriptTabs,
    pressed: Vec<&'static str>,
    /// Open while the unsaved-changes prompt shows.
    confirm: Option<Leave>,
    /// Why the last Save from the prompt failed (shown in the prompt).
    save_error: Option<String>,
}

impl EditorUi {
    /// Builds the editor around `state`, whose Output sink must come from `output`.
    pub fn new(state: EditorState, output: OutputPanel) -> EditorUi {
        let mut ui = EditorUi {
            state,
            actions: default_actions(),
            keymap: Keymap::metallic(),
            output,
            viewport: Viewport::new(),
            explorer: Explorer::new(),
            properties: Properties::new(),
            scripts: ScriptTabs::new(),
            pressed: Vec::new(),
            confirm: None,
            save_error: None,
        };
        ui.viewport.frame_all(&ui.state);
        ui
    }

    /// Opens a game folder in a new editor.
    pub fn open(path: &std::path::Path) -> Result<EditorUi, crate::StudioError> {
        let output = OutputPanel::new();
        let state = EditorState::open(path, output.log.sink())?;
        Ok(EditorUi::new(state, output))
    }

    /// The editor on a template folder for screenshots: one part selected, nothing is written to
    /// disk.
    pub fn demo(template: &std::path::Path) -> Option<EditorUi> {
        let mut editor = EditorUi::open(template).ok()?;
        editor.state.can_save = false;
        let ws = editor.state.world.root("Workspace")?;
        let course = editor.state.world.find_first_child(ws, "Course")?;
        let part = editor.state.world.children(course).get(2).copied()?;
        editor.state.selection.select_only(part);
        editor.viewport.frame_selection(&editor.state);
        let back = -editor.viewport.camera.forward();
        editor.viewport.camera.dolly(back, 6.0);
        Some(editor)
    }

    /// The window is asked to close (close button, Alt+F4). Returns true when it may close now;
    /// with unsaved changes it opens the Save / Don't save / Cancel prompt instead and returns
    /// false, and a later frame returns [`EditorExit::Quit`] if the user agrees.
    pub fn request_quit(&mut self) -> bool {
        // Unsaved script text counts: put the open tabs into the world first.
        self.scripts.flush(&mut self.state);
        if !self.state.is_dirty() {
            return true;
        }
        self.confirm = Some(Leave::Quit);
        false
    }

    /// True while the unsaved-changes prompt is open.
    pub fn is_confirming(&self) -> bool {
        self.confirm.is_some()
    }

    /// Why the last Save from the prompt failed, if it did.
    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    /// Saves now (script buffers first). On failure the error goes to Output and is returned.
    pub fn save(&mut self) -> Result<(), String> {
        self.scripts.flush(&mut self.state);
        self.state.save().map_err(|e| {
            let text = e.to_string();
            self.state.output.push(
                crate::output::Tag::Studio,
                crate::output::Level::Error,
                text.clone(),
            );
            text
        })
    }

    /// Runs action `id`, flushing script buffers first for Save.
    pub fn run_action(&mut self, id: &str) {
        if id == "file.save" {
            self.scripts.flush(&mut self.state);
        }
        if id == "output.clear" {
            self.output.log.clear();
            return;
        }
        self.actions.run(id, &mut self.state);
    }

    fn keys(&mut self, ctx: &egui::Context, viewport_hovered: bool, right_down: bool) {
        let typing = ctx.egui_wants_keyboard_input();
        let mut contexts: Vec<KeyContext> = Vec::with_capacity(2);
        if !typing {
            contexts.push(KeyContext::Global);
            if viewport_hovered && !right_down && !self.viewport.is_dragging() {
                contexts.push(KeyContext::Viewport);
            }
        }
        let mut pressed = std::mem::take(&mut self.pressed);
        if typing {
            // Only Save and Play keys work while a text field has focus.
            self.keymap
                .pressed(ctx, &[KeyContext::Global], &mut pressed);
            pressed.retain(|a| a.starts_with("file.") || a.starts_with("play."));
        } else if !self.viewport.is_dragging() {
            self.keymap.pressed(ctx, &contexts, &mut pressed);
        } else {
            pressed.clear();
        }
        for &id in &pressed {
            self.run_action(id);
        }
        self.pressed = pressed;
    }

    fn requests(&mut self) {
        for r in self.state.take_requests() {
            match r {
                UiRequest::FrameSelection | UiRequest::FrameAll => {
                    self.viewport.handle(r, &self.state)
                }
                UiRequest::Rename(id) => self.explorer.rename(id, &self.state),
                UiRequest::OpenScript(id) => self.scripts.open(id, &self.state),
                UiRequest::RunAction(id) => self.run_action(id),
            }
        }
    }

    /// Draws one frame. `render_state` is eframe's wgpu state (None without a GPU).
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        render_state: Option<&egui_wgpu::RenderState>,
    ) -> EditorExit {
        let ctx = ui.ctx().clone();
        let dt = f64::from(ctx.input(|i| i.stable_dt));
        self.state.play.update(dt);
        let mut exit = EditorExit::Stay;

        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(
                            egui::Button::new("Save").shortcut_text(self.keymap.label("file.save")),
                        )
                        .clicked()
                    {
                        self.run_action("file.save");
                        ui.close();
                    }
                    if ui.button("Close game").clicked() {
                        self.scripts.flush(&mut self.state);
                        if self.state.is_dirty() {
                            self.confirm = Some(Leave::Close);
                            self.save_error = None;
                        } else {
                            exit = EditorExit::Close;
                        }
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let undo = self.state.history.undo_name(&self.state.world);
                    let redo = self.state.history.redo_name(&self.state.world);
                    let u = undo.map_or("Undo".to_string(), |n| format!("Undo {n}"));
                    let r = redo.map_or("Redo".to_string(), |n| format!("Redo {n}"));
                    if let Some(a) = self.actions.get("edit.undo") {
                        let enabled = a.is_enabled(&self.state);
                        if theme::add_enabled(
                            ui,
                            enabled,
                            egui::Button::new(u).shortcut_text(self.keymap.label("edit.undo")),
                        )
                        .clicked()
                        {
                            self.run_action("edit.undo");
                            ui.close();
                        }
                    }
                    if let Some(a) = self.actions.get("edit.redo") {
                        let enabled = a.is_enabled(&self.state);
                        if theme::add_enabled(
                            ui,
                            enabled,
                            egui::Button::new(r).shortcut_text(self.keymap.label("edit.redo")),
                        )
                        .clicked()
                        {
                            self.run_action("edit.redo");
                            ui.close();
                        }
                    }
                    ui.separator();
                    self.actions.menu_items(
                        ui,
                        &[
                            "edit.duplicate",
                            "edit.delete",
                            "edit.group",
                            "edit.rename",
                            "edit.select_all",
                        ],
                        &mut self.state,
                        &self.keymap,
                    );
                });
                ui.menu_button("View", |ui| {
                    self.actions.menu_items(
                        ui,
                        &["view.frame_selection", "view.frame_all"],
                        &mut self.state,
                        &self.keymap,
                    );
                });
                ui.menu_button("Insert", |ui| {
                    for &class in INSERTABLE {
                        if ui.button(class).clicked() {
                            let parent = self
                                .state
                                .selection
                                .active()
                                .filter(|&a| !self.state.world.is_a(a, "BasePart"));
                            let at = self.state.insert_point;
                            self.state.insert(class, parent, Some(at));
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Test", |ui| {
                    let ids: Vec<&'static str> = self
                        .actions
                        .iter()
                        .filter(|a| a.group == ToolbarGroup::Play)
                        .map(|a| a.id)
                        .collect();
                    self.actions
                        .menu_items(ui, &ids, &mut self.state, &self.keymap);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dot = if self.state.is_dirty() { " •" } else { "" };
                    ui.label(RichText::new(format!("{}{dot}", self.state.name)).strong());
                });
            });
        });

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                self.actions
                    .toolbar(ui, ToolbarGroup::Tools, &mut self.state, &self.keymap);
                ui.label(RichText::new(self.state.tool.label()).color(ui_tokens().text_dim));
                ui.separator();
                self.actions
                    .toolbar(ui, ToolbarGroup::Settings, &mut self.state, &self.keymap);
                ui.separator();
                self.actions
                    .toolbar(ui, ToolbarGroup::Edit, &mut self.state, &self.keymap);
                ui.separator();
                self.actions
                    .toolbar(ui, ToolbarGroup::Play, &mut self.state, &self.keymap);
                if let Some(status) = self.state.play.status() {
                    let text = match status {
                        crate::play::PlayStatus::Starting => "Starting…".to_string(),
                        crate::play::PlayStatus::Running(t) => t,
                        crate::play::PlayStatus::Ended(t) => t,
                    };
                    ui.label(RichText::new(text).color(ui_tokens().success));
                }
            });
        });

        egui::Panel::bottom("status")
            .exact_size(24.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let save = if self.state.is_dirty() {
                        "Unsaved changes"
                    } else {
                        "Saved"
                    };
                    ui.label(RichText::new(save).small().color(ui_tokens().text_dim));
                    if let Some(note) = self.state.current_note() {
                        ui.separator();
                        ui.label(RichText::new(note).small());
                        ctx.request_repaint_after(crate::editor::NOTE_TIME);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let n = self.state.selection.len();
                        if n > 0 {
                            let mut text = format!("{n} selected");
                            if let Some(b) = self.state.selection_bounds() {
                                let s = b.size();
                                text.push_str(&format!(
                                    " · {} x {} x {}",
                                    fmt(s.x),
                                    fmt(s.y),
                                    fmt(s.z)
                                ));
                            }
                            ui.label(RichText::new(text).small().monospace());
                        }
                        if let Some(h) = self.viewport.hovered() {
                            ui.label(
                                RichText::new(self.state.display_name(h))
                                    .small()
                                    .color(ui_tokens().text_dim),
                            );
                        }
                    });
                });
            });

        egui::Panel::left("explorer")
            .default_size(280.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Explorer").strong());
                self.explorer.ui(ui, &mut self.state);
            });
        egui::Panel::right("properties")
            .default_size(304.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Properties").strong());
                self.properties.ui(ui, &mut self.state);
            });
        egui::Panel::bottom("output")
            .default_size(200.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Output").strong());
                self.output.ui(ui);
            });

        let mut viewport_hovered = false;
        egui::CentralPanel::default().show(ui, |ui| {
            self.scripts.prune(&self.state);
            let scene = self.scripts.tab_strip(ui, &mut self.state);
            if scene {
                let before = ui.available_rect_before_wrap();
                self.viewport.ui(ui, &mut self.state, render_state);
                viewport_hovered = ui.rect_contains_pointer(before);
            } else {
                self.scripts.editor(ui, &mut self.state);
            }
        });

        let right_down = ctx.input(|i| i.pointer.secondary_down());
        self.keys(&ctx, viewport_hovered, right_down);
        self.requests();

        if let Some(leave) = self.confirm {
            let done = match leave {
                Leave::Close => EditorExit::Close,
                Leave::Quit => EditorExit::Quit,
            };
            egui::Window::new("Save changes?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&ctx, |ui| {
                    ui.label(format!("{} has unsaved changes.", self.state.name));
                    if let Some(err) = &self.save_error {
                        ui.label(
                            RichText::new(format!("Could not save: {err}"))
                                .color(ui_tokens().error),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            match self.save() {
                                Ok(()) => {
                                    self.confirm = None;
                                    self.save_error = None;
                                    exit = done;
                                }
                                // Stay open and show why; the game is not closed unsaved.
                                Err(e) => self.save_error = Some(e),
                            }
                        }
                        if ui.button("Don't save").clicked() {
                            self.confirm = None;
                            self.save_error = None;
                            exit = done;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm = None;
                            self.save_error = None;
                        }
                    });
                });
        }
        exit
    }
}

fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}
