//! The panel's egui view: toolbar, tree, bindings matrix, property grid, Checks strip, the
//! Listen popover and the device preview tab ([`super::view_touch`]).
//!
//! Drawing never changes the world directly. Widgets push [`Intent`]s; after drawing,
//! [`InputActionsPanel::apply`] turns each into one [`EditStep`](super::EditStep) for the sink.

use std::collections::HashSet;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Event, EventFilter, Frame, Id, Key, Margin, Modifiers,
    Rect, Sense, Shadow, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use metallic_core::classes::{PropDef, all_properties};
use metallic_core::{ChangeCursor, CoreError, InstanceId, UDim2, Value, ValueType, Vec3, World};
use metallic_runtime::input::{
    ActionType, BindingDef, Control, ControlKind, RawEvent, Role, default_character_controls,
};

use super::capture::{
    CaptureInput, CaptureOutcome, Captured, Conflict, Listener, Resolution, capture_snapshot,
    composite_snapshot, conflicts_for, resolve_conflicts, write_capture,
};
use super::checks::{Finding, Level, NameBaseline, run_checks};
use super::commands::{CommandSink, History, StepBuilder};
use super::model::{
    DeviceColumn, INPUT_FOLDER, InputModel, ROLES, binding_device, context_snapshot, prop_string,
    role_name, touch_props, unique_child_name,
};
use super::presets::{BindingPreset, add_role_action, apply_preset, preview_preset};
use super::preview::{GlyphStyle, PreviewState};
use super::style::{self as st, ButtonKind, Icon};
use super::view_touch::TouchView;

/// Which half of the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// Tree, matrix, properties, checks.
    #[default]
    Bindings,
    /// Preview as, touch layout editing.
    Preview,
}

/// What is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Selection {
    /// A scene instance.
    Instance(InstanceId),
    /// A built-in action or context (read only), by path.
    Builtin(String),
}

/// What a Listen is for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ListenTarget {
    /// A new binding on `device`.
    Add {
        action: InstanceId,
        device: DeviceColumn,
    },
    /// A new control for one property of an existing binding.
    Replace {
        action: InstanceId,
        binding: InstanceId,
        prop: &'static str,
        old: Option<Control>,
    },
}

#[derive(Debug, Clone)]
struct Listen {
    target: ListenTarget,
    listener: Listener,
    action_name: String,
    action_type: ActionType,
    /// Keys of a four-key composite so far (Up, Down, Left, Right).
    composite: Vec<Control>,
    /// Waiting for the creator to settle a conflict.
    pending: Option<(Pending, Vec<Conflict>)>,
    focus_first: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Pending {
    One(Captured),
    Composite([Control; 4]),
}

/// A change requested while drawing.
#[derive(Debug, Clone)]
pub(super) enum Intent {
    Set {
        id: InstanceId,
        prop: String,
        value: Value,
        merge: bool,
    },
    Seal,
    AddContext,
    AddAction(Option<InstanceId>, Role),
    Delete(InstanceId),
    Materialize,
    Preset(BindingPreset),
    Listen(ListenTarget),
    Undo,
    Redo,
    TouchCommit {
        id: InstanceId,
        center: [f32; 2],
        size: f32,
        profile: super::touch::DeviceProfile,
        merge: bool,
    },
    TouchReset(Option<InstanceId>),
}

struct Cache {
    cursor: ChangeCursor,
    model: InputModel,
    findings: Vec<Finding>,
}

struct Toast {
    text: String,
    until: f64,
    undo: bool,
}

/// The Input Actions panel. Keep one per open game; call [`Self::show`] every frame.
pub struct InputActionsPanel {
    history: History,
    tab: Tab,
    pub(super) selection: Option<Selection>,
    collapsed: HashSet<InstanceId>,
    baseline: NameBaseline,
    cache: Option<Cache>,
    listen: Option<Listen>,
    pub(super) preview: PreviewState,
    pub(super) touch: TouchView,
    toast: Option<Toast>,
    text_edit: Option<(InstanceId, String, String)>,
    focus_row: Option<Selection>,
    raw_events: Vec<RawEvent>,
    prev_modifiers: Modifiers,
    checks_open: bool,
    pub(super) intents: Vec<Intent>,
    pending_step: Option<super::commands::EditStep>,
    pub(super) now: f64,
}

impl InputActionsPanel {
    /// A panel for `world`. Action names are remembered now, for the rename check.
    pub fn new(world: &World) -> Self {
        Self {
            history: History::new(),
            tab: Tab::Bindings,
            selection: None,
            collapsed: HashSet::new(),
            baseline: NameBaseline::capture(world),
            cache: None,
            listen: None,
            preview: PreviewState::default(),
            touch: TouchView::default(),
            toast: None,
            text_edit: None,
            focus_row: None,
            raw_events: Vec::new(),
            prev_modifiers: Modifiers::NONE,
            checks_open: true,
            intents: Vec::new(),
            pending_step: None,
            now: 0.0,
        }
    }

    /// The panel's own undo history (used when the host passes no sink).
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Takes back the last step of the panel's own history.
    pub fn undo(&mut self, world: &mut World) -> bool {
        let ok = self.history.undo(world);
        self.follow_remap();
        ok
    }

    /// Does the last undone step again.
    pub fn redo(&mut self, world: &mut World) -> bool {
        let ok = self.history.redo(world);
        self.follow_remap();
        ok
    }

    fn follow_remap(&mut self) {
        let map = self.history.last_remap().clone();
        self.baseline.remap(&map);
        if let Some(Selection::Instance(id)) = &mut self.selection
            && let Some(n) = map.get(id)
        {
            *id = *n;
        }
        self.touch.remap(&map);
    }

    /// Selects the first action called `name` (also as the selected touch control).
    pub fn select_action(&mut self, world: &World, name: &str) {
        let model = InputModel::read(world);
        if let Some((c, a)) = model.actions().find(|(_, a)| a.name == name) {
            self.selection = Some(match a.instance {
                Some(id) => {
                    self.touch.select(id);
                    Selection::Instance(id)
                }
                None => Selection::Builtin(format!("{}/{}", c.name, a.name)),
            });
        }
    }

    /// The scene was saved: from now on renames are measured against the saved names.
    pub fn mark_saved(&mut self, world: &World) {
        self.baseline = NameBaseline::capture(world);
    }

    /// Which tab shows.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Switches tab (the Play menu's "Edit touch layout" opens the preview tab).
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// The Preview-as state, for the Play menu caret and the Play viewport.
    pub fn preview_state(&self) -> &PreviewState {
        &self.preview
    }

    /// Mutable Preview-as state.
    pub fn preview_state_mut(&mut self) -> &mut PreviewState {
        &mut self.preview
    }

    /// Device events from real gamepads (the host owns gilrs); used by Listen and the preview.
    pub fn feed_raw(&mut self, events: &[RawEvent]) {
        self.raw_events.extend_from_slice(events);
    }

    pub(super) fn take_raw_events(&mut self) -> Vec<RawEvent> {
        std::mem::take(&mut self.raw_events)
    }

    /// The current findings of the Checks strip (also for the Accessibility check, #182).
    pub fn findings(&mut self, world: &World) -> &[Finding] {
        self.refresh(world);
        &self.cache.as_ref().expect("refreshed").findings
    }

    fn refresh(&mut self, world: &World) {
        let cursor = world.change_cursor();
        if self.cache.as_ref().is_some_and(|c| c.cursor == cursor) {
            return;
        }
        self.cache = Some(Cache {
            cursor,
            model: InputModel::read(world),
            findings: run_checks(world, &self.baseline),
        });
    }

    /// Draws the panel into `ui` and records changes in the panel's own [`History`].
    /// Ctrl+Z and Ctrl+Y (Ctrl+Shift+Z) undo and redo while the pointer is over the panel.
    pub fn show(&mut self, ui: &mut Ui, world: &mut World) {
        let mut history = std::mem::take(&mut self.history);
        self.show_inner(ui, world, &mut history, true);
        self.history = history;
    }

    /// Draws the panel and hands finished steps to `sink` (the editor's undo stack). The host
    /// owns undo and redo then; the panel shows no undo buttons.
    pub fn show_with_sink(&mut self, ui: &mut Ui, world: &mut World, sink: &mut dyn CommandSink) {
        self.show_inner(ui, world, sink, false);
    }

    fn show_inner(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        sink: &mut dyn CommandSink,
        own_history: bool,
    ) {
        self.now = ui.input(|i| i.time);
        self.refresh(world);
        let listen_inputs = if self.listen.is_some() {
            self.collect_listen_inputs(ui.ctx())
        } else {
            self.prev_modifiers = ui.input(|i| i.modifiers);
            Vec::new()
        };
        let model = self.cache.as_ref().expect("refreshed").model.clone();
        let findings = self.cache.as_ref().expect("refreshed").findings.clone();

        let outer = ui.available_rect_before_wrap();
        ui.scope(|ui| {
            st::scope_visuals(ui);
            ui.painter()
                .rect_filled(outer, CornerRadius::ZERO, st::BG_BASE);
            self.toolbar(ui, world, own_history);
            match self.tab {
                Tab::Bindings => self.bindings_tab(ui, world, &model, &findings),
                Tab::Preview => {
                    let mut touch = std::mem::take(&mut self.touch);
                    touch.show(self, ui, world, &model);
                    self.touch = touch;
                }
            }
        });

        if self.listen.is_some() {
            self.listen_popover(ui.ctx(), outer, world, listen_inputs);
        }
        self.toast_area(ui.ctx(), outer);

        if own_history
            && self.listen.is_none()
            && ui.rect_contains_pointer(outer)
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let (undo, redo) = ui.input_mut(|i| {
                let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
                let redo = i.consume_key(Modifiers::COMMAND, Key::Y)
                    || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
                (undo, redo)
            });
            if undo {
                self.intents.push(Intent::Undo);
            }
            if redo {
                self.intents.push(Intent::Redo);
            }
        }

        if let Some(step) = self.pending_step.take() {
            sink.record(step);
            sink.seal();
        }
        let intents = std::mem::take(&mut self.intents);
        for intent in intents {
            self.apply(world, sink, intent);
        }
        self.raw_events.clear();
    }

    // -----------------------------------------------------------------------------------------
    // Applying intents

    fn toast(&mut self, text: impl Into<String>, undo: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            until: self.now + 6.0,
            undo,
        });
    }

    fn commit<T>(
        &mut self,
        world: &mut World,
        sink: &mut dyn CommandSink,
        label: &str,
        merge: Option<String>,
        f: impl FnOnce(&mut StepBuilder<'_>) -> Result<T, CoreError>,
    ) -> Option<T> {
        let mut b = StepBuilder::new(world);
        match f(&mut b) {
            Ok(v) => {
                let step = match merge {
                    Some(k) => b.finish_merging(label, k),
                    None => b.finish(label),
                };
                if let Some(step) = step {
                    sink.record(step);
                }
                Some(v)
            }
            Err(e) => {
                b.abort();
                self.toast(format!("{label}: {e}"), false);
                None
            }
        }
    }

    /// Where new contexts go: `ReplicatedStorage/Input`, created on first use.
    fn input_folder(b: &mut StepBuilder<'_>) -> Result<InstanceId, CoreError> {
        let rs = b
            .world()
            .root("ReplicatedStorage")
            .ok_or(CoreError::InvalidInstance)?;
        if let Some(f) = b.world().find_first_child(rs, INPUT_FOLDER) {
            return Ok(f);
        }
        b.create(
            rs,
            super::commands::Snapshot::new("Folder")
                .with("Name", Value::String(INPUT_FOLDER.into())),
        )
    }

    fn apply(&mut self, world: &mut World, sink: &mut dyn CommandSink, intent: Intent) {
        match intent {
            Intent::Set {
                id,
                prop,
                value,
                merge,
            } => {
                let label = format!("Change {prop}");
                let key = merge.then(|| format!("{}:{prop}", id.to_bits()));
                self.commit(world, sink, &label, key, |b| b.set(id, &prop, value));
            }
            Intent::Seal => sink.seal(),
            Intent::AddContext => {
                let id = self.commit(world, sink, "Add context", None, |b| {
                    let folder = Self::input_folder(b)?;
                    let name = unique_child_name(b.world(), folder, "Gameplay");
                    b.create(
                        folder,
                        super::commands::Snapshot::new("InputContext")
                            .with("Name", Value::String(name)),
                    )
                });
                if let Some(id) = id {
                    self.selection = Some(Selection::Instance(id));
                    self.focus_row = self.selection.clone();
                }
            }
            Intent::AddAction(context, role) => {
                let label = if role == Role::None {
                    "Add action".to_string()
                } else {
                    format!("Add {} action", role_name(role))
                };
                let id = self.commit(world, sink, &label, None, |b| {
                    let ctx = match context {
                        Some(c) => c,
                        None => {
                            let folder = Self::input_folder(b)?;
                            let name = unique_child_name(b.world(), folder, "Gameplay");
                            b.create(
                                folder,
                                super::commands::Snapshot::new("InputContext")
                                    .with("Name", Value::String(name)),
                            )?
                        }
                    };
                    add_role_action(b, ctx, role)
                });
                if let Some(id) = id {
                    self.selection = Some(Selection::Instance(id));
                    self.focus_row = self.selection.clone();
                }
            }
            Intent::Delete(id) => {
                let name = prop_string(world, id, "Name");
                let parent = world.parent(id);
                if self
                    .commit(world, sink, &format!("Delete {name}"), None, |b| {
                        b.destroy(id)
                    })
                    .is_some()
                {
                    self.selection = parent
                        .filter(|p| world.is_alive(*p) && world.class_name(*p) != Some("Folder"))
                        .map(Selection::Instance);
                    self.toast(format!("Deleted {name}"), self.history_is_own());
                }
            }
            Intent::Materialize => {
                let id = self.commit(world, sink, "Add CharacterControls", None, |b| {
                    let folder = Self::input_folder(b)?;
                    b.create(folder, context_snapshot(&default_character_controls()))
                });
                if let Some(id) = id {
                    // Keep the selection on the same action, now as an instance.
                    if let Some(Selection::Builtin(path)) = &self.selection {
                        let action = path.rsplit('/').next().unwrap_or_default().to_string();
                        self.selection = Some(Selection::Instance(
                            world.find_first_child(id, &action).unwrap_or(id),
                        ));
                    } else {
                        self.selection = Some(Selection::Instance(id));
                    }
                    self.mark_saved_names_for_new(world);
                    self.toast(
                        "CharacterControls is now part of the scene",
                        self.history_is_own(),
                    );
                }
            }
            Intent::Preset(p) => {
                if let Some(report) =
                    self.commit(world, sink, p.label(), None, |b| apply_preset(b, p))
                {
                    let mut text =
                        format!("{}: {} actions changed", p.label(), report.changed.len());
                    if !report.unchanged.is_empty() {
                        text.push_str(&format!(", {} unchanged", report.unchanged.len()));
                    }
                    self.toast(text, self.history_is_own());
                }
            }
            Intent::Listen(target) => self.start_listen(world, target),
            Intent::Undo => {
                if self.history.can_undo() {
                    let label = self.history.undo_label().unwrap_or_default().to_string();
                    self.undo(world);
                    self.toast(format!("Undid {label}"), false);
                }
            }
            Intent::Redo => {
                if self.history.can_redo() {
                    self.redo(world);
                }
            }
            Intent::TouchCommit {
                id,
                center,
                size,
                profile,
                merge,
            } => {
                let name = prop_string(world, id, "Name");
                let key = merge.then(|| format!("touch:{}", id.to_bits()));
                self.commit(
                    world,
                    sink,
                    &format!("Move {name} touch control"),
                    key,
                    |b| super::touch::write_custom(b, id, center, size, &profile),
                );
            }
            Intent::TouchReset(id) => {
                let label = if id.is_some() {
                    "Reset touch control"
                } else {
                    "Reset touch layout"
                };
                self.commit(world, sink, label, None, |b| match id {
                    Some(id) => {
                        b.set(id, "TouchLayout", Value::String("Auto".into()))?;
                        b.set(
                            id,
                            "TouchPosition",
                            Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0)),
                        )?;
                        b.set(id, "TouchSize", Value::Number(0.0))
                    }
                    None => super::touch::reset_all(b).map(|_| ()),
                });
            }
        }
    }

    fn history_is_own(&self) -> bool {
        // Steps recorded into the panel's own history can be undone from the toast.
        true
    }

    /// New instances made from the built-in context keep the built-in names as their baseline,
    /// so adding CharacterControls never shows as a rename.
    fn mark_saved_names_for_new(&mut self, world: &World) {
        let fresh = NameBaseline::capture(world);
        self.baseline = fresh;
    }

    // -----------------------------------------------------------------------------------------
    // Toolbar

    fn toolbar(&mut self, ui: &mut Ui, world: &World, own_history: bool) {
        egui::Panel::top("ia-toolbar")
            .exact_size(44.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::BG_PANEL)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    st::paint_icon(
                        ui.painter(),
                        Rect::from_min_size(ui.cursor().min + vec2(0.0, 6.0), Vec2::splat(16.0)),
                        Icon::Gamepad,
                        st::ACCENT_TEXT,
                    );
                    ui.add_space(22.0);
                    ui.label(
                        egui::RichText::new("Input actions")
                            .font(st::heading())
                            .color(st::TEXT_1),
                    );
                    ui.add_space(st::S5);
                    st::segmented(
                        ui,
                        &mut self.tab,
                        &[
                            (Tab::Bindings, Some(Icon::Layers), "Bindings"),
                            (Tab::Preview, Some(Icon::Phone), "Device preview"),
                        ],
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if own_history {
                            let redo_tip = match self.history.redo_label() {
                                Some(l) => format!("Redo {l} (Ctrl+Y)"),
                                None => "Nothing to redo".into(),
                            };
                            if st::icon_button(ui, Icon::Redo, &redo_tip, self.history.can_redo())
                                .clicked()
                            {
                                self.intents.push(Intent::Redo);
                            }
                            let undo_tip = match self.history.undo_label() {
                                Some(l) => format!("Undo {l} (Ctrl+Z)"),
                                None => "Nothing to undo".into(),
                            };
                            if st::icon_button(ui, Icon::Undo, &undo_tip, self.history.can_undo())
                                .clicked()
                            {
                                self.intents.push(Intent::Undo);
                            }
                            ui.add_space(st::S2);
                        }
                        self.presets_menu(ui, world);
                        self.add_menu(ui, world);
                    });
                });
            });
    }

    fn selected_context(&self, world: &World) -> Option<InstanceId> {
        let Some(Selection::Instance(id)) = &self.selection else {
            return None;
        };
        let mut cur = Some(*id);
        while let Some(c) = cur {
            if world.class_name(c) == Some("InputContext") {
                return Some(c);
            }
            cur = world.parent(c);
        }
        None
    }

    fn selected_action(&self, world: &World) -> Option<InstanceId> {
        let Some(Selection::Instance(id)) = &self.selection else {
            return None;
        };
        match world.class_name(*id) {
            Some("InputAction") => Some(*id),
            Some("InputBinding") => world.parent(*id),
            _ => None,
        }
    }

    fn add_menu(&mut self, ui: &mut Ui, world: &World) {
        let ctx = self.selected_context(world);
        let action = self.selected_action(world);
        let resp = st::button(ui, ButtonKind::Secondary, Some(Icon::Plus), "Add");
        egui::Popup::menu(&resp).show(|ui| {
            st::scope_visuals(ui);
            ui.set_min_width(220.0);
            if ui.button("Context").clicked() {
                self.intents.push(Intent::AddContext);
            }
            ui.separator();
            let into = match ctx {
                Some(c) => format!("Action in {}", prop_string(world, c, "Name")),
                None => "Action in a new context".to_string(),
            };
            ui.label(
                egui::RichText::new(into)
                    .font(st::caption())
                    .color(st::TEXT_3),
            );
            for role in ROLES {
                let text = if role == Role::None {
                    "Empty action".to_string()
                } else {
                    format!("{} action", role_name(role))
                };
                if ui.button(text).clicked() {
                    self.intents.push(Intent::AddAction(ctx, role));
                }
            }
            if let Some(a) = action {
                ui.separator();
                for d in [DeviceColumn::KeyboardMouse, DeviceColumn::Gamepad] {
                    if ui
                        .button(format!("{} binding (Listen)", d.title()))
                        .clicked()
                    {
                        self.intents.push(Intent::Listen(ListenTarget::Add {
                            action: a,
                            device: d,
                        }));
                    }
                }
            }
        });
    }

    fn presets_menu(&mut self, ui: &mut Ui, world: &World) {
        let resp = st::button(ui, ButtonKind::Secondary, Some(Icon::Layers), "Presets");
        egui::Popup::menu(&resp).show(|ui| {
            st::scope_visuals(ui);
            ui.set_min_width(260.0);
            ui.label(
                egui::RichText::new("Replace one device's bindings of every action with a role")
                    .font(st::caption())
                    .color(st::TEXT_3),
            );
            for p in BindingPreset::ALL {
                let report = preview_preset(world, p);
                let enabled = !report.changed.is_empty();
                let tip = if enabled {
                    let mut t = format!("Changes {}", report.changed.join(", "));
                    if !report.unchanged.is_empty() {
                        t.push_str(&format!("\nLeaves {}", report.unchanged.join(", ")));
                    }
                    t
                } else {
                    "No scene action has a role this preset covers".into()
                };
                if ui
                    .add_enabled(enabled, egui::Button::new(p.label()))
                    .on_hover_text(tip)
                    .on_disabled_hover_text(
                        "Add CharacterControls to the scene, or give actions a Role",
                    )
                    .clicked()
                {
                    self.intents.push(Intent::Preset(p));
                }
            }
        });
    }

    // -----------------------------------------------------------------------------------------
    // Bindings tab

    fn bindings_tab(
        &mut self,
        ui: &mut Ui,
        world: &World,
        model: &InputModel,
        findings: &[Finding],
    ) {
        self.checks_strip(ui, findings);
        egui::Panel::left("ia-tree")
            .default_size(250.0)
            .min_size(200.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::BG_PANEL)
                    .inner_margin(Margin::symmetric(0, 8)),
            )
            .show(ui, |ui| self.tree(ui, world, model, findings));
        egui::Panel::right("ia-props")
            .default_size(300.0)
            .min_size(240.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::BG_PANEL)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.properties(ui, world, model));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(st::BG_BASE)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.matrix(ui, world, model, findings));
    }

    fn is_selected(&self, sel: &Selection) -> bool {
        self.selection.as_ref() == Some(sel)
    }

    /// One 24 px tree row. Returns its response.
    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        &mut self,
        ui: &mut Ui,
        sel: Selection,
        depth: usize,
        icon: Icon,
        name: &str,
        detail: Option<(&str, Color32)>,
        expanded: Option<bool>,
        enabled: Option<bool>,
        flagged: Option<Level>,
    ) -> (egui::Response, bool, bool) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, st::ROW), Sense::hover());
        let id = Id::unique(("ia-row", format!("{sel:?}")));
        let resp = ui.interact(rect, id, Sense::click());
        let selected = self.is_selected(&sel);
        if self.focus_row.as_ref() == Some(&sel) {
            resp.request_focus();
            resp.scroll_to_me(None);
            self.focus_row = None;
        }
        let fill = if selected {
            st::SELECT
        } else if resp.hovered() {
            st::BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::ZERO, fill);
        if selected {
            p.vline(
                rect.left() + 1.0,
                rect.y_range(),
                Stroke::new(2.0, st::ACCENT),
            );
        }
        let mut x = rect.left() + 8.0 + depth as f32 * 16.0;
        let mut toggled = false;
        if let Some(open) = expanded {
            let chev = Rect::from_min_size(pos2(x, rect.center().y - 7.0), Vec2::splat(14.0));
            st::paint_icon(
                p,
                chev,
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                st::TEXT_3,
            );
            if resp.clicked()
                && resp
                    .interact_pointer_pos()
                    .is_some_and(|pt| chev.expand(3.0).contains(pt))
            {
                toggled = true;
            }
        }
        x += 16.0;
        let dim = enabled == Some(false);
        let text_color = if dim { st::TEXT_OFF } else { st::TEXT_1 };
        st::paint_icon(
            p,
            Rect::from_min_size(pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)),
            icon,
            if selected { st::ACCENT_TEXT } else { st::ICON },
        );
        x += 20.0;
        let name_rect = p.text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            st::body(),
            text_color,
        );
        if let Some((d, c)) = detail {
            p.text(
                pos2(name_rect.right() + 6.0, rect.center().y),
                Align2::LEFT_CENTER,
                d,
                st::caption(),
                c,
            );
        }
        let mut eye_clicked = false;
        let mut right = rect.right() - 8.0;
        if let Some(on) = enabled {
            let eye =
                Rect::from_min_size(pos2(right - 18.0, rect.center().y - 9.0), Vec2::splat(18.0));
            let eid = id.with("eye");
            let er = ui.interact(eye, eid, Sense::click()).on_hover_text(if on {
                "Enabled (click to disable)"
            } else {
                "Disabled (click to enable)"
            });
            if er.hovered() {
                ui.painter()
                    .rect_filled(eye, CornerRadius::same(st::R_SM), st::BG_ACTIVE);
            }
            st::paint_icon(
                ui.painter(),
                eye.shrink(2.0),
                if on { Icon::Eye } else { Icon::EyeOff },
                if on { st::TEXT_3 } else { st::TEXT_OFF },
            );
            st::focus_ring(ui, &er, eye, st::R_SM);
            eye_clicked = er.clicked();
            right -= 22.0;
        }
        if let Some(level) = flagged {
            let (icon, color) = level_icon(level);
            st::paint_icon(
                ui.painter(),
                Rect::from_min_size(pos2(right - 14.0, rect.center().y - 7.0), Vec2::splat(14.0)),
                icon,
                color,
            );
        }
        st::focus_ring(ui, &resp, rect.shrink(2.0), st::R_SM);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, selected, name));
        (resp, toggled, eye_clicked)
    }

    fn tree(&mut self, ui: &mut Ui, world: &World, model: &InputModel, findings: &[Finding]) {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            st::section_title(ui, "Contexts");
        });
        let worst = |id: Option<InstanceId>| -> Option<Level> {
            let id = id?;
            findings
                .iter()
                .filter(|f| f.targets.contains(&id))
                .map(|f| f.level)
                .max()
        };
        let mut order: Vec<Selection> = Vec::new();
        let mut toggle_collapse = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for ctx in &model.config.contexts {
                    let csel = match ctx.instance {
                        Some(id) => Selection::Instance(id),
                        None => Selection::Builtin(ctx.name.clone()),
                    };
                    let open = ctx.instance.is_none_or(|id| !self.collapsed.contains(&id));
                    let detail = if ctx.instance.is_none() {
                        ("Built in".to_string(), st::TEXT_3)
                    } else {
                        (format!("P{}", ctx.priority), st::TEXT_3)
                    };
                    let (resp, toggled, eye) = self.tree_row(
                        ui,
                        csel.clone(),
                        0,
                        Icon::Context,
                        &ctx.name,
                        Some((&detail.0, detail.1)),
                        Some(open),
                        ctx.instance.map(|_| ctx.enabled),
                        worst(ctx.instance),
                    );
                    self.row_input(ui, &resp, &csel, ctx.instance, world, &mut toggle_collapse);
                    if toggled && let Some(id) = ctx.instance {
                        toggle_collapse = Some(id);
                    } else if resp.clicked() {
                        self.selection = Some(csel.clone());
                    }
                    if eye && let Some(id) = ctx.instance {
                        self.intents.push(Intent::Set {
                            id,
                            prop: "Enabled".into(),
                            value: Value::Bool(!ctx.enabled),
                            merge: false,
                        });
                    }
                    order.push(csel);
                    if !open {
                        continue;
                    }
                    for a in &ctx.actions {
                        let asel = match a.instance {
                            Some(id) => Selection::Instance(id),
                            None => Selection::Builtin(format!("{}/{}", ctx.name, a.name)),
                        };
                        let aopen = a.instance.is_some_and(|id| !self.collapsed.contains(&id))
                            && !a.bindings.is_empty();
                        let ty = short_type(a.ty);
                        let (resp, toggled, eye) = self.tree_row(
                            ui,
                            asel.clone(),
                            1,
                            Icon::Action,
                            &a.name,
                            Some((ty, st::TEXT_3)),
                            a.instance.filter(|_| !a.bindings.is_empty()).map(|_| aopen),
                            a.instance.map(|_| a.enabled),
                            worst(a.instance),
                        );
                        self.row_input(ui, &resp, &asel, a.instance, world, &mut toggle_collapse);
                        if toggled && let Some(id) = a.instance {
                            toggle_collapse = Some(id);
                        } else if resp.clicked() {
                            self.selection = Some(asel.clone());
                        }
                        if eye && let Some(id) = a.instance {
                            self.intents.push(Intent::Set {
                                id,
                                prop: "Enabled".into(),
                                value: Value::Bool(!a.enabled),
                                merge: false,
                            });
                        }
                        order.push(asel);
                        if !aopen {
                            continue;
                        }
                        for b in &a.bindings {
                            let Some(bid) = b.instance else { continue };
                            let bsel = Selection::Instance(bid);
                            let label = super::model::binding_label(b);
                            let (resp, _, _) = self.tree_row(
                                ui,
                                bsel.clone(),
                                2,
                                Icon::Binding,
                                &label,
                                Some((short_device(binding_device(b)), st::TEXT_3)),
                                None,
                                None,
                                worst(Some(bid)),
                            );
                            self.row_input(
                                ui,
                                &resp,
                                &bsel,
                                Some(bid),
                                world,
                                &mut toggle_collapse,
                            );
                            if resp.clicked() {
                                self.selection = Some(bsel.clone());
                            }
                            if resp.double_clicked() {
                                self.intents.push(Intent::Listen(ListenTarget::Replace {
                                    action: a.instance.unwrap_or(bid),
                                    binding: bid,
                                    prop: "KeyCode",
                                    old: b.key,
                                }));
                            }
                            order.push(bsel);
                        }
                    }
                }
                ui.add_space(st::S5);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if st::button(ui, ButtonKind::Ghost, Some(Icon::Plus), "Add context").clicked()
                    {
                        self.intents.push(Intent::AddContext);
                    }
                });
            });
        if let Some(id) = toggle_collapse
            && !self.collapsed.remove(&id)
        {
            self.collapsed.insert(id);
        }
        // Arrow keys move the selection along the visible rows.
        let focused = order.iter().position(|s| {
            let id = Id::unique(("ia-row", format!("{s:?}")));
            ui.memory(|m| m.has_focus(id))
        });
        if let Some(i) = focused {
            let id = Id::unique(("ia-row", format!("{:?}", order[i])));
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let (up, down) = ui.input_mut(|inp| {
                (
                    inp.consume_key(Modifiers::NONE, Key::ArrowUp),
                    inp.consume_key(Modifiers::NONE, Key::ArrowDown),
                )
            });
            let next = if up {
                i.checked_sub(1)
            } else if down {
                (i + 1 < order.len()).then_some(i + 1)
            } else {
                None
            };
            if let Some(n) = next {
                self.selection = Some(order[n].clone());
                self.focus_row = Some(order[n].clone());
            }
        }
    }

    /// Keys on a focused tree row: Enter selects (or listens on a binding), Delete removes,
    /// Left and Right collapse and expand, Space toggles Enabled.
    fn row_input(
        &mut self,
        ui: &mut Ui,
        resp: &egui::Response,
        sel: &Selection,
        id: Option<InstanceId>,
        world: &World,
        toggle: &mut Option<InstanceId>,
    ) {
        if !resp.has_focus() {
            return;
        }
        let (enter, del, left, right) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
            )
        });
        if enter {
            self.selection = Some(sel.clone());
            if let Some(b) = id.filter(|b| world.class_name(*b) == Some("InputBinding"))
                && let Some(a) = world.parent(b)
            {
                let old = Control::from_name(&prop_string(world, b, "KeyCode"));
                self.intents.push(Intent::Listen(ListenTarget::Replace {
                    action: a,
                    binding: b,
                    prop: "KeyCode",
                    old,
                }));
            }
        }
        if let Some(id) = id {
            if del {
                self.intents.push(Intent::Delete(id));
            }
            let collapsed = self.collapsed.contains(&id);
            if (left && !collapsed) || (right && collapsed) {
                *toggle = Some(id);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Matrix

    fn matrix(&mut self, ui: &mut Ui, world: &World, model: &InputModel, findings: &[Finding]) {
        if model.uses_builtin_character_controls() {
            self.builtin_banner(ui);
            ui.add_space(st::S4);
        }
        let name_w = 190.0;
        let avail = ui.available_width();
        let col_w = ((avail - name_w) / 3.0).max(150.0);
        // Header.
        let (hdr, _) = ui.allocate_exact_size(vec2(avail, 28.0), Sense::hover());
        let p = ui.painter();
        p.text(
            pos2(hdr.left() + 4.0, hdr.center().y),
            Align2::LEFT_CENTER,
            "ACTION",
            st::caption(),
            st::TEXT_3,
        );
        for (i, d) in DeviceColumn::ALL.iter().enumerate() {
            let x = hdr.left() + name_w + i as f32 * col_w;
            let icon = match d {
                DeviceColumn::KeyboardMouse => Icon::Keyboard,
                DeviceColumn::Gamepad => Icon::Gamepad,
                DeviceColumn::Touch => Icon::Phone,
            };
            st::paint_icon(
                p,
                Rect::from_min_size(pos2(x + 4.0, hdr.center().y - 7.0), Vec2::splat(14.0)),
                icon,
                st::TEXT_3,
            );
            p.text(
                pos2(x + 24.0, hdr.center().y),
                Align2::LEFT_CENTER,
                d.title().to_uppercase(),
                st::caption(),
                st::TEXT_3,
            );
        }
        p.hline(hdr.x_range(), hdr.bottom(), Stroke::new(1.0, st::LINE));

        let flagged: HashSet<InstanceId> = findings
            .iter()
            .filter(|f| f.level == Level::Error)
            .flat_map(|f| f.targets.iter().copied())
            .collect();
        let glyphs = self.preview.glyphs;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for ctx in &model.config.contexts {
                    ui.add_space(st::S4);
                    let (r, _) = ui.allocate_exact_size(vec2(avail, 26.0), Sense::hover());
                    let p = ui.painter();
                    st::paint_icon(
                        p,
                        Rect::from_min_size(
                            pos2(r.left() + 2.0, r.center().y - 7.0),
                            Vec2::splat(14.0),
                        ),
                        Icon::Context,
                        st::TEXT_3,
                    );
                    let t = p.text(
                        pos2(r.left() + 22.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &ctx.name,
                        st::heading(),
                        if ctx.enabled {
                            st::TEXT_1
                        } else {
                            st::TEXT_OFF
                        },
                    );
                    let mut meta = format!("Priority {}", ctx.priority);
                    if ctx.sink {
                        meta.push_str("  ·  Sink");
                    }
                    if ctx.rebind_group != "Game" && !ctx.rebind_group.is_empty() {
                        meta.push_str(&format!("  ·  Group {}", ctx.rebind_group));
                    }
                    if !ctx.enabled {
                        meta.push_str("  ·  Disabled");
                    }
                    p.text(
                        pos2(t.right() + 10.0, r.center().y + 1.0),
                        Align2::LEFT_CENTER,
                        meta,
                        st::caption(),
                        st::TEXT_3,
                    );
                    for a in &ctx.actions {
                        let sel = match a.instance {
                            Some(id) => Selection::Instance(id),
                            None => Selection::Builtin(format!("{}/{}", ctx.name, a.name)),
                        };
                        let (row, _) = ui.allocate_exact_size(vec2(avail, 36.0), Sense::hover());
                        let selected = self.is_selected(&sel)
                            || a.bindings.iter().any(|b| {
                                b.instance
                                    .is_some_and(|i| self.is_selected(&Selection::Instance(i)))
                            });
                        let p = ui.painter();
                        p.rect_filled(
                            row.shrink2(vec2(0.0, 2.0)),
                            CornerRadius::same(st::R_MD),
                            if selected {
                                st::SELECT_WEAK
                            } else {
                                st::BG_PANEL
                            },
                        );
                        // Name cell.
                        let name_rect = Rect::from_min_size(row.min, vec2(name_w, row.height()));
                        let nid = Id::unique(("ia-cell-name", format!("{sel:?}")));
                        let nresp = ui.interact(name_rect.shrink(3.0), nid, Sense::click());
                        if nresp.clicked() {
                            self.selection = Some(sel.clone());
                        }
                        let dim = !a.enabled || !ctx.enabled;
                        let p = ui.painter();
                        let nt = p.text(
                            pos2(name_rect.left() + 10.0, row.center().y),
                            Align2::LEFT_CENTER,
                            &a.name,
                            st::body(),
                            if dim { st::TEXT_OFF } else { st::TEXT_1 },
                        );
                        p.text(
                            pos2(nt.right() + 6.0, row.center().y + 1.0),
                            Align2::LEFT_CENTER,
                            short_type(a.ty),
                            st::caption(),
                            st::TEXT_3,
                        );
                        if a.instance.is_some_and(|i| flagged.contains(&i)) {
                            st::paint_icon(
                                p,
                                Rect::from_min_size(
                                    pos2(name_rect.right() - 20.0, row.center().y - 7.0),
                                    Vec2::splat(14.0),
                                ),
                                Icon::Error,
                                st::ERROR,
                            );
                        }
                        st::focus_ring(ui, &nresp, name_rect.shrink(3.0), st::R_SM);
                        for (i, d) in DeviceColumn::ALL.iter().enumerate() {
                            let cell = Rect::from_min_size(
                                pos2(row.left() + name_w + i as f32 * col_w, row.top()),
                                vec2(col_w, row.height()),
                            );
                            ui.painter().vline(
                                cell.left(),
                                cell.y_range().shrink(8.0),
                                Stroke::new(1.0, st::LINE),
                            );
                            if *d == DeviceColumn::Touch {
                                self.touch_cell(ui, world, cell, a, &sel);
                            } else {
                                self.binding_cell(ui, cell, a, *d, glyphs, &flagged);
                            }
                        }
                    }
                    if let Some(cid) = ctx.instance {
                        ui.add_space(st::S2);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            let r = st::button(ui, ButtonKind::Ghost, Some(Icon::Plus), "Action");
                            if r.clicked() {
                                self.intents.push(Intent::AddAction(Some(cid), Role::None));
                            }
                        });
                    }
                }
                ui.add_space(st::S6);
            });
    }

    fn builtin_banner(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(st::INFO_BG)
            .corner_radius(CornerRadius::same(st::R_MD))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    st::paint_icon(ui.painter(), r, Icon::Info, st::INFO);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new("This game uses the built-in CharacterControls").font(st::body()).color(st::TEXT_1));
                        ui.label(egui::RichText::new("Move, Jump, Run, Look and Zoom with Metallic's default keys. Add them to the scene to change them.").font(st::small()).color(st::TEXT_2));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if st::button(ui, ButtonKind::Primary, Some(Icon::Plus), "Add to scene").clicked() {
                            self.intents.push(Intent::Materialize);
                        }
                    });
                });
            });
    }

    fn binding_cell(
        &mut self,
        ui: &mut Ui,
        cell: Rect,
        a: &metallic_runtime::input::ActionDef,
        device: DeviceColumn,
        glyphs: GlyphStyle,
        flagged: &HashSet<InstanceId>,
    ) {
        let mut x = cell.left() + 8.0;
        let y = cell.center().y;
        let right_limit = cell.right() - 34.0;
        let bindings: Vec<&BindingDef> = a
            .bindings
            .iter()
            .filter(|b| binding_device(b) == device)
            .collect();
        let mut hidden = 0;
        for b in &bindings {
            let w = st::glyph_width(ui.painter(), b) + 10.0;
            if x + w > right_limit {
                hidden += 1;
                continue;
            }
            let chip = Rect::from_min_size(pos2(x, y - 13.0), vec2(w, 26.0));
            let sel = b.instance.map(Selection::Instance);
            let resp = match b.instance {
                Some(id) => ui.interact(chip, Id::unique(("ia-chip", id)), Sense::click()),
                None => ui.interact(
                    chip,
                    Id::unique(("ia-chip", a.name.as_str(), x as i32)),
                    Sense::hover(),
                ),
            };
            let selected = sel.as_ref().is_some_and(|s| self.is_selected(s));
            let err = b.instance.is_some_and(|i| flagged.contains(&i));
            let fill = if selected {
                st::SELECT
            } else if resp.hovered() {
                st::BG_HOVER
            } else {
                st::BG_RAISED
            };
            ui.painter()
                .rect_filled(chip, CornerRadius::same(st::R_SM), fill);
            if err {
                ui.painter().rect_stroke(
                    chip,
                    CornerRadius::same(st::R_SM),
                    Stroke::new(1.0, st::ERROR),
                    StrokeKind::Inside,
                );
            }
            st::paint_glyph(ui.painter(), pos2(x + 5.0, y), b, glyphs, !a.enabled);
            st::focus_ring(ui, &resp, chip, st::R_SM);
            let resp = resp.on_hover_text(super::model::binding_label(b));
            if let Some(id) = b.instance {
                if resp.clicked() {
                    self.selection = Some(Selection::Instance(id));
                }
                if resp.double_clicked()
                    && let Some(action) = a.instance
                {
                    self.intents.push(Intent::Listen(ListenTarget::Replace {
                        action,
                        binding: id,
                        prop: "KeyCode",
                        old: b.key,
                    }));
                }
                if resp.has_focus() {
                    let (enter, del) = ui.input_mut(|i| {
                        (
                            i.consume_key(Modifiers::NONE, Key::Enter),
                            i.consume_key(Modifiers::NONE, Key::Delete),
                        )
                    });
                    if enter && let Some(action) = a.instance {
                        self.intents.push(Intent::Listen(ListenTarget::Replace {
                            action,
                            binding: id,
                            prop: "KeyCode",
                            old: b.key,
                        }));
                    }
                    if del {
                        self.intents.push(Intent::Delete(id));
                    }
                }
            }
            x += w + 4.0;
        }
        if hidden > 0 {
            ui.painter().text(
                pos2(x + 2.0, y),
                Align2::LEFT_CENTER,
                format!("+{hidden}"),
                st::caption(),
                st::TEXT_3,
            );
        }
        if bindings.is_empty() {
            let warn = a.ty != ActionType::ViewportPosition && a.enabled;
            ui.painter().text(
                pos2(cell.left() + 10.0, y),
                Align2::LEFT_CENTER,
                "No binding",
                st::caption(),
                if warn { st::WARNING } else { st::TEXT_3 },
            );
        }
        if let Some(action) = a.instance {
            let add = Rect::from_center_size(pos2(cell.right() - 18.0, y), Vec2::splat(24.0));
            let resp = ui.interact(
                add,
                Id::unique(("ia-add", action, device as u8)),
                Sense::click(),
            );
            let hot = resp.hovered() || resp.has_focus();
            if hot || ui.rect_contains_pointer(cell) {
                ui.painter().rect_filled(
                    add,
                    CornerRadius::same(st::R_SM),
                    if hot { st::BG_ACTIVE } else { st::BG_RAISED },
                );
                st::paint_icon(ui.painter(), add.shrink(5.0), Icon::Plus, st::ICON);
            }
            st::focus_ring(ui, &resp, add, st::R_SM);
            if resp
                .on_hover_text(format!(
                    "Add a {} binding (Listen)",
                    device.title().to_lowercase()
                ))
                .clicked()
            {
                self.intents
                    .push(Intent::Listen(ListenTarget::Add { action, device }));
            }
        }
    }

    fn touch_cell(
        &mut self,
        ui: &mut Ui,
        world: &World,
        cell: Rect,
        a: &metallic_runtime::input::ActionDef,
        sel: &Selection,
    ) {
        let y = cell.center().y;
        let touchable = matches!(a.ty, ActionType::Bool | ActionType::Direction2D);
        if !touchable {
            ui.painter().text(
                pos2(cell.left() + 10.0, y),
                Align2::LEFT_CENTER,
                "Not shown on touch",
                st::caption(),
                st::TEXT_3,
            );
            return;
        }
        let props = touch_props(world, a.instance);
        let (text, on) = if a.show_on_touch {
            let kind = match (a.ty, props.slot) {
                (ActionType::Bool, _) => "button",
                (_, super::model::TouchSlot::Look) => "look pad",
                _ => "stick",
            };
            (
                format!(
                    "{}  ·  {kind}",
                    if props.custom { "Custom" } else { "Auto" }
                ),
                true,
            )
        } else {
            ("Off".to_string(), false)
        };
        let g = ui
            .painter()
            .layout_no_wrap(text.clone(), st::caption(), st::TEXT_1);
        let chip = Rect::from_min_size(
            pos2(cell.left() + 8.0, y - 12.0),
            vec2(g.size().x + 34.0, 24.0),
        );
        let resp = match a.instance {
            Some(id) => ui.interact(chip, Id::unique(("ia-touch", id)), Sense::click()),
            None => ui.interact(
                chip,
                Id::unique(("ia-touch", format!("{sel:?}"))),
                Sense::hover(),
            ),
        };
        let fill = if on {
            st::SELECT_WEAK
        } else if resp.hovered() {
            st::BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(chip, CornerRadius::same(12), fill);
        if !on {
            ui.painter().rect_stroke(
                chip,
                CornerRadius::same(12),
                Stroke::new(1.0, st::LINE_STRONG),
                StrokeKind::Inside,
            );
        }
        let dot = pos2(chip.left() + 13.0, y);
        if on {
            ui.painter().circle_filled(dot, 4.5, st::ACCENT);
        } else {
            ui.painter()
                .circle_stroke(dot, 4.0, Stroke::new(1.2, st::TEXT_3));
        }
        ui.painter().text(
            pos2(chip.left() + 24.0, y),
            Align2::LEFT_CENTER,
            text,
            st::caption(),
            if on { st::TEXT_1 } else { st::TEXT_3 },
        );
        st::focus_ring(ui, &resp, chip, 12);
        if let Some(id) = a.instance
            && resp
                .on_hover_text(if on {
                    "Shown on touch screens. Click to hide."
                } else {
                    "Click to show an on-screen control"
                })
                .clicked()
        {
            self.intents.push(Intent::Set {
                id,
                prop: "ShowOnTouch".into(),
                value: Value::Bool(!a.show_on_touch),
                merge: false,
            });
        }
    }

    // -----------------------------------------------------------------------------------------
    // Properties

    fn properties(&mut self, ui: &mut Ui, world: &World, model: &InputModel) {
        st::section_title(ui, "Properties");
        let Some(sel) = self.selection.clone() else {
            ui.add_space(st::S5);
            ui.label(
                egui::RichText::new("Select a context, action or binding.")
                    .font(st::small())
                    .color(st::TEXT_3),
            );
            return;
        };
        let id = match sel {
            Selection::Builtin(path) => {
                ui.add_space(st::S4);
                ui.label(
                    egui::RichText::new(&path)
                        .font(st::heading())
                        .color(st::TEXT_1),
                );
                ui.label(
                    egui::RichText::new(
                        "Built in and read only. Add CharacterControls to the scene to edit it.",
                    )
                    .font(st::small())
                    .color(st::TEXT_2),
                );
                ui.add_space(st::S4);
                if st::button(ui, ButtonKind::Primary, Some(Icon::Plus), "Add to scene").clicked() {
                    self.intents.push(Intent::Materialize);
                }
                if let Some((_, a)) = model.actions().find(|(c, a)| {
                    format!("{}/{}", c.name, a.name) == path || c.name == path && a.name.is_empty()
                }) {
                    ui.add_space(st::S5);
                    for b in &a.bindings {
                        let (r, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                        st::paint_glyph(
                            ui.painter(),
                            pos2(r.left(), r.center().y),
                            b,
                            self.preview.glyphs,
                            false,
                        );
                    }
                }
                return;
            }
            Selection::Instance(id) => id,
        };
        let Some(class) = world.class_name(id) else {
            self.selection = None;
            return;
        };
        ui.add_space(st::S2);
        ui.horizontal(|ui| {
            let icon = match class {
                "InputContext" => Icon::Context,
                "InputAction" => Icon::Action,
                _ => Icon::Binding,
            };
            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            st::paint_icon(ui.painter(), r, icon, st::ACCENT_TEXT);
            ui.label(
                egui::RichText::new(prop_string(world, id, "Name"))
                    .font(st::heading())
                    .color(st::TEXT_1),
            );
            ui.label(
                egui::RichText::new(class)
                    .font(st::caption())
                    .color(st::TEXT_3),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if st::icon_button(ui, Icon::Trash, "Delete (Delete)", true).clicked() {
                    self.intents.push(Intent::Delete(id));
                }
            });
        });
        ui.add_space(st::S2);
        let (roblox, metallic) = prop_groups(class);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (title, names) in [("Roblox", roblox), ("Metallic", metallic)] {
                    ui.add_space(st::S4);
                    ui.horizontal(|ui| {
                        st::section_title(ui, title);
                    });
                    for def in all_properties(class)
                        .into_iter()
                        .filter(|d| names.contains(&d.name))
                    {
                        self.prop_row(ui, world, id, def);
                    }
                }
                if class == "InputAction" {
                    ui.add_space(st::S5);
                    if st::button(
                        ui,
                        ButtonKind::Secondary,
                        Some(Icon::Listen),
                        "Listen for a binding",
                    )
                    .clicked()
                    {
                        let device = DeviceColumn::KeyboardMouse;
                        self.intents
                            .push(Intent::Listen(ListenTarget::Add { action: id, device }));
                    }
                }
            });
    }

    fn prop_row(&mut self, ui: &mut Ui, world: &World, id: InstanceId, def: &PropDef) {
        let Ok(value) = world.get(id, def.name) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.set_min_height(st::ROW + 2.0);
            let (lr, _) = ui.allocate_exact_size(vec2(118.0, st::ROW), Sense::hover());
            ui.painter().text(
                lr.left_center(),
                Align2::LEFT_CENTER,
                def.name,
                st::small(),
                st::TEXT_2,
            );
            let w = ui.available_width();
            let prop = def.name.to_string();
            let set = |v: Value, merge: bool| Intent::Set {
                id,
                prop: prop.clone(),
                value: v,
                merge,
            };
            match value {
                Value::Bool(mut b) => {
                    if st::toggle(ui, &mut b, true).changed() {
                        self.intents.push(set(Value::Bool(b), false));
                    }
                }
                Value::Number(mut n) => {
                    let r = ui.add_sized(
                        vec2(w, st::ROW),
                        egui::DragValue::new(&mut n)
                            .speed(drag_speed(def.name))
                            .max_decimals(3),
                    );
                    if r.changed() {
                        self.intents.push(set(Value::Number(n), true));
                    }
                    if r.drag_stopped() || r.lost_focus() {
                        self.intents.push(Intent::Seal);
                    }
                }
                Value::String(s) if def.allowed.is_some() => {
                    let mut cur = s.clone();
                    egui::ComboBox::from_id_salt(("ia-enum", id, def.name))
                        .width(w - 8.0)
                        .selected_text(&cur)
                        .show_ui(ui, |ui| {
                            for opt in def.allowed.unwrap_or_default() {
                                ui.selectable_value(&mut cur, opt.to_string(), *opt);
                            }
                        });
                    if cur != s {
                        self.intents.push(set(Value::String(cur), false));
                    }
                }
                Value::String(s) if is_control_prop(def.name) => {
                    let mut cur = s.clone();
                    egui::ComboBox::from_id_salt(("ia-ctl", id, def.name))
                        .width(w - 40.0)
                        .selected_text(if cur == "None" {
                            "None".to_string()
                        } else {
                            Control::from_name(&cur)
                                .map_or(cur.clone(), |c| super::model::control_label(c).to_string())
                        })
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut cur, "None".to_string(), "None");
                            for c in Control::ALL {
                                ui.selectable_value(
                                    &mut cur,
                                    c.name().to_string(),
                                    format!("{}  ({})", super::model::control_label(*c), c.name()),
                                );
                            }
                        });
                    if cur != s {
                        self.intents.push(set(Value::String(cur), false));
                    }
                    if st::icon_button(ui, Icon::Listen, "Listen", true).clicked()
                        && let Some(action) = world.parent(id)
                    {
                        let prop: &'static str = def.name;
                        self.intents.push(Intent::Listen(ListenTarget::Replace {
                            action,
                            binding: id,
                            prop,
                            old: Control::from_name(&s),
                        }));
                    }
                }
                Value::String(s) => {
                    let active = self
                        .text_edit
                        .as_ref()
                        .is_some_and(|(i, p, _)| *i == id && p == def.name);
                    let mut buf = if active {
                        self.text_edit
                            .as_ref()
                            .map(|t| t.2.clone())
                            .unwrap_or_default()
                    } else {
                        s.clone()
                    };
                    let r = ui.add_sized(
                        vec2(w, st::ROW),
                        egui::TextEdit::singleline(&mut buf)
                            .font(st::body())
                            .margin(vec2(6.0, 3.0)),
                    );
                    if r.has_focus() {
                        self.text_edit = Some((id, def.name.to_string(), buf.clone()));
                    }
                    let field = r.rect;
                    ui.painter().rect_stroke(
                        field,
                        CornerRadius::same(st::R_SM),
                        Stroke::new(
                            1.0,
                            if r.has_focus() {
                                st::FOCUS
                            } else {
                                st::LINE_CONTROL
                            },
                        ),
                        StrokeKind::Inside,
                    );
                    if r.lost_focus() {
                        self.text_edit = None;
                        if buf != s && !(def.name == "Name" && buf.trim().is_empty()) {
                            self.intents.push(set(Value::String(buf), false));
                        }
                    }
                }
                Value::UDim2(mut u) => {
                    let mut changed = false;
                    let mut sealed = false;
                    let cw = (w - 12.0) / 4.0;
                    for (v, sp) in [
                        (&mut u.x_scale, 0.01),
                        (&mut u.x_offset, 1.0),
                        (&mut u.y_scale, 0.01),
                        (&mut u.y_offset, 1.0),
                    ] {
                        let r = ui.add_sized(
                            vec2(cw, st::ROW),
                            egui::DragValue::new(v).speed(sp).max_decimals(2),
                        );
                        changed |= r.changed();
                        sealed |= r.drag_stopped() || r.lost_focus();
                    }
                    if changed {
                        self.intents.push(set(Value::UDim2(u), true));
                    }
                    if sealed {
                        self.intents.push(Intent::Seal);
                    }
                }
                Value::Vector3(mut v) => {
                    let mut changed = false;
                    let mut sealed = false;
                    let cw = (w - 8.0) / 3.0;
                    for c in [&mut v.x, &mut v.y, &mut v.z] {
                        let r = ui.add_sized(
                            vec2(cw, st::ROW),
                            egui::DragValue::new(c).speed(0.05).max_decimals(3),
                        );
                        changed |= r.changed();
                        sealed |= r.drag_stopped() || r.lost_focus();
                    }
                    if changed {
                        self.intents
                            .push(set(Value::Vector3(Vec3::new(v.x, v.y, v.z)), true));
                    }
                    if sealed {
                        self.intents.push(Intent::Seal);
                    }
                }
                Value::Ref(r) => {
                    let text = r
                        .filter(|r| world.is_alive(*r))
                        .map_or("None".to_string(), |r| world.full_name(r));
                    ui.label(
                        egui::RichText::new(text)
                            .font(st::small())
                            .color(st::TEXT_3),
                    );
                }
                other => {
                    ui.label(
                        egui::RichText::new(format!("{:?}", other.value_type()))
                            .font(st::small())
                            .color(st::TEXT_3),
                    );
                }
            }
        });
        let _ = ValueType::Bool;
    }

    // -----------------------------------------------------------------------------------------
    // Checks strip

    fn checks_strip(&mut self, ui: &mut Ui, findings: &[Finding]) {
        let errors = findings.iter().filter(|f| f.level == Level::Error).count();
        let warnings = findings
            .iter()
            .filter(|f| f.level == Level::Warning)
            .count();
        let infos = findings.len() - errors - warnings;
        let panel = egui::Panel::bottom("ia-checks")
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::BG_PANEL)
                    .inner_margin(Margin::symmetric(12, 6)),
            );
        let panel = if self.checks_open {
            panel.resizable(true).default_size(150.0).min_size(90.0)
        } else {
            panel.resizable(false).exact_size(36.0)
        };
        panel.show(ui, |ui| {
            ui.horizontal(|ui| {
                let r = st::icon_button(ui, if self.checks_open { Icon::ChevronDown } else { Icon::ChevronRight }, if self.checks_open { "Hide checks" } else { "Show checks" }, true);
                if r.clicked() {
                    self.checks_open = !self.checks_open;
                }
                ui.label(egui::RichText::new("Checks").font(st::heading()).color(st::TEXT_1));
                if findings.is_empty() {
                    st::chip(ui, "All pass", st::SUCCESS, st::SUCCESS_BG);
                }
                if errors > 0 {
                    st::chip(ui, &plural(errors, "error"), st::ERROR, st::ERROR_BG);
                }
                if warnings > 0 {
                    st::chip(ui, &plural(warnings, "warning"), st::WARNING, st::WARNING_BG);
                }
                if infos > 0 {
                    st::chip(ui, &plural(infos, "note"), st::INFO, st::INFO_BG);
                }
            });
            if !self.checks_open {
                return;
            }
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if findings.is_empty() {
                    ui.add_space(st::S3);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        st::paint_icon(ui.painter(), r, Icon::Check, st::SUCCESS);
                        ui.label(egui::RichText::new("Every action has keyboard and gamepad bindings, and the touch layout fits every device.").font(st::small()).color(st::TEXT_2));
                    });
                }
                for (i, f) in findings.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    let resp = ui.interact(r, Id::unique(("ia-finding", i, f.message.as_str())), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(st::R_SM), st::BG_HOVER);
                    }
                    let (icon, color) = level_icon(f.level);
                    let p = ui.painter();
                    st::paint_icon(p, Rect::from_min_size(pos2(r.left() + 6.0, r.center().y - 7.0), Vec2::splat(14.0)), icon, color);
                    let lt = p.text(pos2(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, f.level.label(), st::caption(), color);
                    p.text(pos2(lt.left() + 58.0, r.center().y), Align2::LEFT_CENTER, &f.message, st::small(), st::TEXT_1);
                    st::focus_ring(ui, &resp, r, st::R_SM);
                    let activate = resp.clicked() || (resp.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)));
                    if activate && let Some(&t) = f.targets.first() {
                        self.selection = Some(Selection::Instance(t));
                        if f.id.is_touch() {
                            self.tab = Tab::Preview;
                            self.touch.select(t);
                        }
                    }
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Listen

    fn start_listen(&mut self, world: &World, target: ListenTarget) {
        let action = match &target {
            ListenTarget::Add { action, .. } | ListenTarget::Replace { action, .. } => *action,
        };
        let model = InputModel::read(world);
        let Some((_, a)) = model.action(action) else {
            return;
        };
        let menu = a.role == Role::Menu;
        self.listen = Some(Listen {
            listener: Listener::new(self.now, &[]).for_menu(menu),
            action_name: a.name.clone(),
            action_type: a.ty,
            target,
            composite: Vec::new(),
            pending: None,
            focus_first: true,
        });
    }

    fn collect_listen_inputs(&mut self, ctx: &egui::Context) -> Vec<CaptureInput> {
        let mut out = Vec::new();
        let mods = ctx.input(|i| i.modifiers);
        for (was, now, c) in [
            (self.prev_modifiers.shift, mods.shift, Control::LeftShift),
            (self.prev_modifiers.ctrl, mods.ctrl, Control::LeftControl),
            (self.prev_modifiers.alt, mods.alt, Control::LeftAlt),
        ] {
            if now && !was {
                out.push(CaptureInput::Press(c, 1.0));
            } else if was && !now {
                out.push(CaptureInput::Release(c));
            }
        }
        self.prev_modifiers = mods;
        let pending = self.listen.as_ref().is_some_and(|l| l.pending.is_some());
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    ..
                } => {
                    if pending {
                        return true;
                    }
                    if !repeat && let Some(c) = egui_key_to_control(physical_key.unwrap_or(*key)) {
                        out.push(if *pressed {
                            CaptureInput::Press(c, 1.0)
                        } else {
                            CaptureInput::Release(c)
                        });
                    }
                    false
                }
                Event::Text(_) => pending,
                _ => true,
            });
            // Keys were taken out of the event list; clear the derived state too.
            if !pending {
                i.keys_down.clear();
            }
        });
        for e in self.raw_events.drain(..) {
            match e.control.kind() {
                ControlKind::Button => out.push(if e.value > 0.0 {
                    CaptureInput::Press(e.control, e.value)
                } else {
                    CaptureInput::Release(e.control)
                }),
                ControlKind::Stick => {
                    let v = e.pos.unwrap_or_default();
                    let m = (v[0] * v[0] + v[1] * v[1]).sqrt();
                    out.push(CaptureInput::Axis(e.control, m));
                }
                ControlKind::Wheel => out.push(CaptureInput::Wheel(e.value)),
                _ => {}
            }
        }
        out
    }

    fn listen_popover(
        &mut self,
        ctx: &egui::Context,
        outer: Rect,
        world: &mut World,
        mut inputs: Vec<CaptureInput>,
    ) {
        let Some(mut listen) = self.listen.take() else {
            return;
        };
        let device = match &listen.target {
            ListenTarget::Add { device, .. } => *device,
            ListenTarget::Replace { binding, .. } => InputModel::read(world)
                .binding(*binding)
                .map_or(DeviceColumn::KeyboardMouse, |(_, b)| binding_device(b)),
        };
        // Dim the panel behind.
        let dim = egui::LayerId::new(egui::Order::Middle, Id::unique("ia-listen-dim"));
        ctx.layer_painter(dim).rect_filled(
            outer,
            CornerRadius::ZERO,
            Color32::from_black_alpha(110),
        );
        let mut close = false;
        let mut finish: Option<Pending> = None;
        let mut resolution: Option<Resolution> = None;
        let width = 400.0;
        let area = egui::Area::new(Id::unique("ia-listen"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(
                outer.center().x - width / 2.0,
                outer.center().y - 150.0,
            ));
        area.show(ctx, |ui| {
            Frame::new()
                .fill(st::BG_PANEL)
                .stroke(Stroke::new(1.0, st::LINE_STRONG))
                .corner_radius(CornerRadius::same(st::R_LG))
                .shadow(Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(115),
                })
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    st::scope_visuals(ui);
                    ui.set_width(width - 32.0);
                    let slot = match &listen.target {
                        ListenTarget::Replace { prop, .. } if *prop != "KeyCode" => {
                            format!(" · {prop}")
                        }
                        _ => String::new(),
                    };
                    ui.label(
                        egui::RichText::new(format!("{}{slot}", listen.action_name))
                            .font(st::heading())
                            .color(st::TEXT_1),
                    );
                    ui.label(
                        egui::RichText::new(device.title())
                            .font(st::small())
                            .color(st::TEXT_3),
                    );
                    ui.add_space(st::S5);
                    if let Some((p, conflicts)) = &listen.pending {
                        conflict_ui(
                            ui,
                            p,
                            conflicts,
                            &listen.target,
                            listen.focus_first,
                            &mut resolution,
                            &mut close,
                        );
                        listen.focus_first = false;
                        return;
                    }
                    // Pulsing ring.
                    let t = ctx.input(|i| i.time) as f32;
                    let (ring, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
                    let c = ring.center();
                    let pulse = (t * 2.4).sin() * 0.5 + 0.5;
                    ui.painter().circle_stroke(
                        c,
                        20.0 + pulse * 6.0,
                        Stroke::new(1.5, st::ACCENT.gamma_multiply(0.35 + 0.4 * (1.0 - pulse))),
                    );
                    ui.painter().circle_filled(c, 14.0, st::SELECT);
                    st::paint_icon(
                        ui.painter(),
                        Rect::from_center_size(c, Vec2::splat(18.0)),
                        Icon::Listen,
                        st::ACCENT_TEXT,
                    );
                    let composite = listen.action_type == ActionType::Direction2D
                        && matches!(listen.target, ListenTarget::Add { .. });
                    let prompt = if composite && !listen.composite.is_empty()
                        || composite && device == DeviceColumn::KeyboardMouse
                    {
                        let dir = ["Up", "Down", "Left", "Right"][listen.composite.len().min(3)];
                        format!(
                            "Press the key for {dir} ({} of 4)",
                            listen.composite.len() + 1
                        )
                    } else {
                        match device {
                            DeviceColumn::Gamepad => "Press a button or move a stick".to_string(),
                            _ => "Press a key or click a mouse button below".to_string(),
                        }
                    };
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(prompt)
                                .font(st::body())
                                .color(st::TEXT_1),
                        );
                    });
                    if !listen.composite.is_empty() {
                        ui.horizontal(|ui| {
                            for (i, k) in listen.composite.iter().enumerate() {
                                let b = BindingDef::key(*k);
                                let (r, _) = ui.allocate_exact_size(
                                    vec2(st::glyph_width(ui.painter(), &b) + 46.0, 24.0),
                                    Sense::hover(),
                                );
                                ui.painter().text(
                                    r.left_center(),
                                    Align2::LEFT_CENTER,
                                    ["Up", "Down", "Left", "Right"][i],
                                    st::caption(),
                                    st::TEXT_3,
                                );
                                st::paint_glyph(
                                    ui.painter(),
                                    pos2(r.left() + 38.0, r.center().y),
                                    &b,
                                    self.preview.glyphs,
                                    false,
                                );
                            }
                        });
                    }
                    ui.add_space(st::S4);
                    match device {
                        DeviceColumn::Gamepad => pad_picker(ui, self.preview.glyphs, &mut inputs),
                        _ => mouse_pad(ui, &mut inputs),
                    }
                    ui.add_space(st::S4);
                    // Time left.
                    let left = listen.listener.remaining(self.now) as f32;
                    let (bar, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 3.0), Sense::hover());
                    ui.painter()
                        .rect_filled(bar, CornerRadius::same(1), st::BG_DEEP);
                    ui.painter().rect_filled(
                        Rect::from_min_size(bar.min, vec2(bar.width() * left / 8.0, 3.0)),
                        CornerRadius::same(1),
                        st::ACCENT,
                    );
                    ui.add_space(st::S4);
                    ui.horizontal(|ui| {
                        let hint = if listen.listener.clone().for_menu(false).remaining(self.now)
                            >= 0.0
                            && matches!(listen.action_type, ActionType::Bool)
                            && listen.action_name == "Menu"
                        {
                            "Esc binds · B cancels"
                        } else {
                            "Esc cancels · Backspace clears"
                        };
                        ui.label(
                            egui::RichText::new(hint)
                                .font(st::caption())
                                .color(st::TEXT_3),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if st::button(ui, ButtonKind::Ghost, None, "Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                });
        });
        ctx.request_repaint();

        if let Some(res) = resolution {
            if let Some((p, conflicts)) = listen.pending.take() {
                self.finish_listen(world, &listen, p, &conflicts, res);
            }
            self.end_listen();
            return;
        }
        if close {
            self.end_listen();
            return;
        }
        if listen.pending.is_none() {
            match listen.listener.feed(self.now, &inputs) {
                CaptureOutcome::Listening => {}
                CaptureOutcome::Cancelled | CaptureOutcome::TimedOut => {
                    self.end_listen();
                    return;
                }
                CaptureOutcome::Cleared => {
                    if let ListenTarget::Replace { binding, prop, .. } = &listen.target {
                        self.intents.push(Intent::Set {
                            id: *binding,
                            prop: prop.to_string(),
                            value: Value::String("None".into()),
                            merge: false,
                        });
                    }
                    self.end_listen();
                    return;
                }
                CaptureOutcome::Bound(cap) => {
                    let wants_composite = listen.action_type == ActionType::Direction2D
                        && matches!(listen.target, ListenTarget::Add { .. })
                        && cap.control.kind() == ControlKind::Button;
                    if wants_composite {
                        listen.composite.push(cap.control);
                        if listen.composite.len() == 4 {
                            let keys = [
                                listen.composite[0],
                                listen.composite[1],
                                listen.composite[2],
                                listen.composite[3],
                            ];
                            finish = Some(Pending::Composite(keys));
                        } else {
                            let menu = listen.action_name == "Menu";
                            listen.listener =
                                Listener::new(self.now, &[cap.control]).for_menu(menu);
                        }
                    } else {
                        finish = Some(Pending::One(cap));
                    }
                }
            }
        }
        if let Some(p) = finish {
            let conflicts = self.conflicts_of(world, &listen.target, &p);
            if conflicts.is_empty() {
                self.finish_listen(world, &listen, p, &[], Resolution::KeepBoth);
                self.end_listen();
                return;
            }
            listen.pending = Some((p, conflicts));
            listen.focus_first = true;
        }
        self.listen = Some(listen);
    }

    fn end_listen(&mut self) {
        self.listen = None;
    }

    fn conflicts_of(&self, world: &World, target: &ListenTarget, p: &Pending) -> Vec<Conflict> {
        let (action, replacing) = match target {
            ListenTarget::Add { action, .. } => (*action, None),
            ListenTarget::Replace {
                action, binding, ..
            } => (*action, Some(*binding)),
        };
        match p {
            Pending::One(c) => conflicts_for(world, action, replacing, c),
            Pending::Composite(keys) => keys
                .iter()
                .flat_map(|k| conflicts_for(world, action, replacing, &Captured::plain(*k)))
                .collect(),
        }
    }

    fn finish_listen(
        &mut self,
        world: &mut World,
        listen: &Listen,
        p: Pending,
        conflicts: &[Conflict],
        res: Resolution,
    ) {
        let label = match &p {
            Pending::One(c) => format!(
                "Bind {} to {}",
                listen.action_name,
                super::model::control_label(c.control)
            ),
            Pending::Composite(_) => format!("Bind {} to four keys", listen.action_name),
        };
        let old = match &listen.target {
            ListenTarget::Replace { old, .. } => *old,
            ListenTarget::Add { .. } => None,
        };
        let target = listen.target.clone();
        let mut b = StepBuilder::new(world);
        let result = (|| -> Result<(), CoreError> {
            resolve_conflicts(&mut b, conflicts, res, old)?;
            match (&target, &p) {
                (ListenTarget::Add { action, .. }, Pending::One(c)) => {
                    b.create(*action, capture_snapshot(c))?;
                }
                (ListenTarget::Add { action, .. }, Pending::Composite(keys)) => {
                    b.create(*action, composite_snapshot(*keys))?;
                }
                (ListenTarget::Replace { binding, prop, .. }, Pending::One(c)) => {
                    if *prop == "KeyCode" {
                        write_capture(&mut b, *binding, c)?;
                    } else {
                        b.set(*binding, prop, Value::String(c.control.name().into()))?;
                    }
                }
                (ListenTarget::Replace { .. }, Pending::Composite(_)) => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                if let Some(step) = b.finish(label) {
                    self.pending_step = Some(step);
                }
            }
            Err(e) => {
                b.abort();
                self.toast(format!("Could not bind: {e}"), false);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Toast

    fn toast_area(&mut self, ctx: &egui::Context, outer: Rect) {
        let Some(t) = &self.toast else { return };
        if self.now > t.until {
            self.toast = None;
            return;
        }
        let text = t.text.clone();
        let undo = t.undo && self.history.can_undo();
        let mut clicked_undo = false;
        egui::Area::new(Id::unique("ia-toast"))
            .order(egui::Order::Foreground)
            .anchor(
                Align2::RIGHT_BOTTOM,
                vec2(
                    -(ctx.content_rect().right() - outer.right()) - 16.0,
                    -(ctx.content_rect().bottom() - outer.bottom()) - 16.0,
                ),
            )
            .show(ctx, |ui| {
                Frame::new()
                    .fill(hex_g700())
                    .corner_radius(CornerRadius::same(st::R_MD))
                    .shadow(Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(115),
                    })
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        st::scope_visuals(ui);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&text)
                                    .font(st::small())
                                    .color(st::TEXT_1),
                            );
                            if undo
                                && st::button(ui, ButtonKind::Ghost, Some(Icon::Undo), "Undo")
                                    .clicked()
                            {
                                clicked_undo = true;
                            }
                        });
                    });
            });
        if clicked_undo {
            self.intents.push(Intent::Undo);
            self.toast = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn hex_g700() -> Color32 {
    Color32::from_rgb(0x2c, 0x2a, 0x34)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn level_icon(l: Level) -> (Icon, Color32) {
    match l {
        Level::Error => (Icon::Error, st::ERROR),
        Level::Warning => (Icon::Warning, st::WARNING),
        Level::Info => (Icon::Info, st::INFO),
    }
}

fn short_type(t: ActionType) -> &'static str {
    match t {
        ActionType::Bool => "Bool",
        ActionType::Direction1D => "1D",
        ActionType::Direction2D => "2D",
        ActionType::Direction3D => "3D",
        ActionType::ViewportPosition => "Pointer",
    }
}

fn short_device(d: DeviceColumn) -> &'static str {
    match d {
        DeviceColumn::KeyboardMouse => "Keyboard",
        DeviceColumn::Gamepad => "Gamepad",
        DeviceColumn::Touch => "Touch",
    }
}

fn is_control_prop(p: &str) -> bool {
    matches!(
        p,
        "KeyCode"
            | "Up"
            | "Down"
            | "Left"
            | "Right"
            | "Forward"
            | "Backward"
            | "PrimaryModifier"
            | "SecondaryModifier"
    )
}

fn drag_speed(p: &str) -> f64 {
    match p {
        "HoldTime" | "TapTime" | "DoubleTapWindow" | "RepeatDelay" | "RepeatRate" => 10.0,
        "TouchSize" => 1.0,
        "Priority" | "TouchOrder" | "PointerIndex" => 0.1,
        _ => 0.01,
    }
}

/// Roblox properties first, Metallic additions below (spec 10.1).
fn prop_groups(class: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match class {
        "InputContext" => (&["Name", "Enabled", "Priority", "Sink"], &["RebindGroup"]),
        "InputAction" => (
            &["Name", "Type", "Enabled", "DisplayName"],
            &[
                "Role",
                "Trigger",
                "HoldTime",
                "TapTime",
                "DoubleTapWindow",
                "RepeatDelay",
                "RepeatRate",
                "Rebindable",
                "Required",
                "CanToggle",
                "ShowOnTouch",
                "TouchOrder",
                "TouchSlot",
                "TouchLabel",
                "TouchImage",
                "TouchLayout",
                "TouchPosition",
                "TouchSize",
            ],
        ),
        _ => (
            &[
                "Name",
                "KeyCode",
                "Up",
                "Down",
                "Left",
                "Right",
                "Forward",
                "Backward",
                "PrimaryModifier",
                "SecondaryModifier",
                "Scale",
                "Vector2Scale",
                "Vector3Scale",
                "ResponseCurve",
                "ClampMagnitudeToOne",
                "PressedThreshold",
                "ReleasedThreshold",
                "UIButton",
                "PointerIndex",
                "DisplayName",
                "DisplayImage",
            ],
            &["Deadzone", "ScaleByDeltaTime"],
        ),
    }
}

fn conflict_ui(
    ui: &mut Ui,
    p: &Pending,
    conflicts: &[Conflict],
    target: &ListenTarget,
    focus_first: bool,
    resolution: &mut Option<Resolution>,
    close: &mut bool,
) {
    let what = match p {
        Pending::One(c) => super::model::binding_label(&BindingDef {
            key: Some(c.control),
            modifiers: c.modifiers,
            ..BindingDef::default()
        }),
        Pending::Composite(k) => k
            .iter()
            .map(|c| super::model::control_label(*c))
            .collect::<Vec<_>>()
            .join(" "),
    };
    Frame::new()
        .fill(st::WARNING_BG)
        .corner_radius(CornerRadius::same(st::R_MD))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                st::paint_icon(ui.painter(), r, Icon::Warning, st::WARNING);
                ui.label(
                    egui::RichText::new(format!("{what} is already in use"))
                        .font(st::body())
                        .color(st::TEXT_1),
                );
            });
            let mut paths: Vec<&str> = conflicts.iter().map(|c| c.path.as_str()).collect();
            paths.dedup();
            ui.label(
                egui::RichText::new(format!("Used by {}", paths.join(", ")))
                    .font(st::small())
                    .color(st::TEXT_2),
            );
        });
    ui.add_space(st::S5);
    let can_swap = matches!(
        (target, p),
        (ListenTarget::Replace { old: Some(_), .. }, Pending::One(_))
    );
    ui.horizontal(|ui| {
        let first = if can_swap {
            let r = st::button(ui, ButtonKind::Primary, None, "Swap")
                .on_hover_text("The other action gets this slot's old control");
            if r.clicked() {
                *resolution = Some(Resolution::Swap);
            }
            r
        } else {
            let r = st::button(ui, ButtonKind::Primary, None, "Replace")
                .on_hover_text("The other action loses this control");
            if r.clicked() {
                *resolution = Some(Resolution::Replace);
            }
            r
        };
        if focus_first {
            first.request_focus();
        }
        if can_swap
            && st::button(ui, ButtonKind::Secondary, None, "Replace")
                .on_hover_text("The other action loses this control")
                .clicked()
        {
            *resolution = Some(Resolution::Replace);
        }
        if st::button(ui, ButtonKind::Secondary, None, "Keep both")
            .on_hover_text("Both keep it; Checks shows the clash")
            .clicked()
        {
            *resolution = Some(Resolution::KeepBoth);
        }
        if st::button(ui, ButtonKind::Ghost, None, "Cancel").clicked() {
            *close = true;
        }
    });
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        *close = true;
    }
}

/// A click target for mouse buttons and the wheel.
fn mouse_pad(ui: &mut Ui, inputs: &mut Vec<CaptureInput>) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect_filled(
        r,
        CornerRadius::same(st::R_MD),
        if hot { st::BG_RAISED } else { st::BG_DEEP },
    );
    ui.painter().rect_stroke(
        r,
        CornerRadius::same(st::R_MD),
        Stroke::new(
            1.0,
            if hot {
                st::LINE_CONTROL
            } else {
                st::LINE_STRONG
            },
        ),
        StrokeKind::Inside,
    );
    ui.painter().text(
        r.center() - vec2(0.0, 7.0),
        Align2::CENTER_CENTER,
        "Click here with any mouse button",
        st::small(),
        st::TEXT_2,
    );
    ui.painter().text(
        r.center() + vec2(0.0, 9.0),
        Align2::CENTER_CENTER,
        "or turn the wheel",
        st::caption(),
        st::TEXT_3,
    );
    if hot {
        ui.input(|i| {
            for e in &i.events {
                match e {
                    Event::PointerButton {
                        button,
                        pressed: true,
                        ..
                    } => {
                        let c = match button {
                            egui::PointerButton::Primary => Control::MouseButton1,
                            egui::PointerButton::Secondary => Control::MouseButton2,
                            egui::PointerButton::Middle => Control::MouseButton3,
                            _ => continue,
                        };
                        inputs.push(CaptureInput::Press(c, 1.0));
                    }
                    Event::MouseWheel { delta, .. } if delta.y != 0.0 => {
                        inputs.push(CaptureInput::Wheel(delta.y.signum()))
                    }
                    _ => {}
                }
            }
        });
    }
}

/// Clickable gamepad controls, for creators without a pad at hand.
fn pad_picker(ui: &mut Ui, glyphs: GlyphStyle, inputs: &mut Vec<CaptureInput>) {
    use Control as C;
    ui.label(
        egui::RichText::new("No pad? Pick a control:")
            .font(st::caption())
            .color(st::TEXT_3),
    );
    let rows: [&[Control]; 2] = [
        &[
            C::ButtonA,
            C::ButtonB,
            C::ButtonX,
            C::ButtonY,
            C::ButtonL1,
            C::ButtonR1,
            C::ButtonL2,
            C::ButtonR2,
            C::ButtonL3,
        ],
        &[
            C::ButtonR3,
            C::ButtonSelect,
            C::ButtonStart,
            C::DPadUp,
            C::DPadDown,
            C::DPadLeft,
            C::DPadRight,
            C::Thumbstick1,
            C::Thumbstick2,
        ],
    ];
    for row in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for &c in row {
                let b = BindingDef::key(c);
                let w = st::glyph_width(ui.painter(), &b) + 10.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w.max(30.0), 28.0), Sense::click());
                ui.painter().rect_filled(
                    r,
                    CornerRadius::same(st::R_SM),
                    if resp.hovered() {
                        st::BG_HOVER
                    } else {
                        st::BG_RAISED
                    },
                );
                st::paint_glyph(
                    ui.painter(),
                    pos2(r.center().x - (w - 10.0) / 2.0, r.center().y),
                    &b,
                    glyphs,
                    false,
                );
                if resp.on_hover_text(super::model::control_label(c)).clicked() {
                    match c.kind() {
                        ControlKind::Stick => inputs.push(CaptureInput::Axis(c, 1.0)),
                        _ => inputs.push(CaptureInput::Press(c, 1.0)),
                    }
                }
            }
        });
    }
}

/// egui's key to our physical control name. Keys egui does not report (Shift, Ctrl, Alt) come
/// from the modifier state instead.
pub(super) fn egui_key_to_control(k: Key) -> Option<Control> {
    use Control as C;
    Some(match k {
        Key::A => C::A,
        Key::B => C::B,
        Key::C => C::C,
        Key::D => C::D,
        Key::E => C::E,
        Key::F => C::F,
        Key::G => C::G,
        Key::H => C::H,
        Key::I => C::I,
        Key::J => C::J,
        Key::K => C::K,
        Key::L => C::L,
        Key::M => C::M,
        Key::N => C::N,
        Key::O => C::O,
        Key::P => C::P,
        Key::Q => C::Q,
        Key::R => C::R,
        Key::S => C::S,
        Key::T => C::T,
        Key::U => C::U,
        Key::V => C::V,
        Key::W => C::W,
        Key::X => C::X,
        Key::Y => C::Y,
        Key::Z => C::Z,
        Key::Num0 => C::Zero,
        Key::Num1 => C::One,
        Key::Num2 => C::Two,
        Key::Num3 => C::Three,
        Key::Num4 => C::Four,
        Key::Num5 => C::Five,
        Key::Num6 => C::Six,
        Key::Num7 => C::Seven,
        Key::Num8 => C::Eight,
        Key::Num9 => C::Nine,
        Key::F1 => C::F1,
        Key::F2 => C::F2,
        Key::F3 => C::F3,
        Key::F4 => C::F4,
        Key::F5 => C::F5,
        Key::F6 => C::F6,
        Key::F7 => C::F7,
        Key::F8 => C::F8,
        Key::F9 => C::F9,
        Key::F10 => C::F10,
        Key::F11 => C::F11,
        Key::F12 => C::F12,
        Key::Space => C::Space,
        Key::Enter => C::Return,
        Key::Tab => C::Tab,
        Key::Backspace => C::Backspace,
        Key::Escape => C::Escape,
        Key::ArrowUp => C::Up,
        Key::ArrowDown => C::Down,
        Key::ArrowLeft => C::Left,
        Key::ArrowRight => C::Right,
        Key::Home => C::Home,
        Key::End => C::End,
        Key::PageUp => C::PageUp,
        Key::PageDown => C::PageDown,
        Key::Insert => C::Insert,
        Key::Delete => C::Delete,
        Key::Comma => C::Comma,
        Key::Period => C::Period,
        Key::Slash => C::Slash,
        Key::Semicolon => C::Semicolon,
        Key::Quote => C::Quote,
        Key::OpenBracket => C::LeftBracket,
        Key::CloseBracket => C::RightBracket,
        Key::Backslash => C::BackSlash,
        Key::Minus => C::Minus,
        Key::Equals => C::Equals,
        Key::Backtick => C::Backquote,
        _ => return None,
    })
}
