//! Explorer: the instance tree.
//!
//! Rows are flattened from the expanded part of the tree only when the world, the history, the
//! selection or the filter changed, and drawn with a virtualised scroll area, so large places
//! scroll at full frame rate. Click selects, Ctrl-click toggles, Shift-click selects a range,
//! double-click opens a script, F2 renames, dragging a row onto another reparents it (one undo
//! step), and right-click offers Insert, Duplicate, Group, Rename, Delete, Select children and
//! Focus in viewport. Selecting in the viewport opens the ancestors of the selection.
use std::collections::HashSet;

use eframe::egui::{self, RichText, Sense, Stroke, Vec2};
use metallic_core::{InstanceId, Value};

use crate::editor::{EditorState, INSERTABLE, UiRequest};
use crate::tokens::ui_tokens;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Row {
    id: InstanceId,
    depth: usize,
    has_children: bool,
}

/// Explorer panel state.
#[derive(Default)]
pub struct Explorer {
    expanded: HashSet<InstanceId>,
    rows: Vec<Row>,
    rows_key: Option<(metallic_core::ChangeCursor, u64, u64, u64, String)>,
    expand_revision: u64,
    filter: String,
    renaming: Option<(InstanceId, String, bool)>,
    anchor: Option<InstanceId>,
    seen_selection: u64,
    scroll_to: Option<InstanceId>,
}

/// Something the Explorer asks the editor to do after drawing.
enum Action {
    Click(InstanceId, egui::Modifiers),
    Toggle(InstanceId),
    Open(InstanceId),
    Drop(InstanceId, InstanceId),
    Insert(&'static str, InstanceId),
    Run(&'static str),
    SelectChildren(InstanceId),
    Rename(InstanceId),
    Focus(InstanceId),
}

impl Explorer {
    /// A collapsed tree with Workspace open.
    pub fn new() -> Explorer {
        Explorer::default()
    }

    /// Starts renaming `id`.
    pub fn rename(&mut self, id: InstanceId, state: &EditorState) {
        self.renaming = Some((id, state.display_name(id), true));
        self.reveal(id, state);
    }

    fn reveal(&mut self, id: InstanceId, state: &EditorState) {
        let mut cur = state.world.parent(id);
        while let Some(c) = cur {
            if self.expanded.insert(c) {
                self.expand_revision += 1;
            }
            cur = state.world.parent(c);
        }
        self.scroll_to = Some(id);
    }

    fn rebuild(&mut self, state: &EditorState) {
        self.rows.clear();
        let filter = self.filter.to_lowercase();
        let matches = |id: InstanceId| -> bool {
            filter.is_empty()
                || state.display_name(id).to_lowercase().contains(&filter)
                || state
                    .world
                    .class_name(id)
                    .is_some_and(|c| c.to_lowercase().contains(&filter))
        };
        // With a filter, show matches and their ancestors, everything open.
        let mut keep: HashSet<InstanceId> = HashSet::new();
        if !filter.is_empty() {
            let mut stack: Vec<InstanceId> = state.visible_roots();
            while let Some(id) = stack.pop() {
                if matches(id) {
                    let mut cur = Some(id);
                    while let Some(c) = cur {
                        if !keep.insert(c) {
                            break;
                        }
                        cur = state.world.parent(c);
                    }
                }
                stack.extend_from_slice(state.world.children(id));
            }
        }
        let mut stack: Vec<(InstanceId, usize)> = state
            .visible_roots()
            .into_iter()
            .rev()
            .map(|r| (r, 0))
            .collect();
        while let Some((id, depth)) = stack.pop() {
            if !filter.is_empty() && !keep.contains(&id) {
                continue;
            }
            let kids = state.world.children(id);
            self.rows.push(Row {
                id,
                depth,
                has_children: !kids.is_empty(),
            });
            let open = !filter.is_empty() || self.expanded.contains(&id);
            if open {
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
        }
    }

    /// Draws the panel.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        if self.expanded.is_empty()
            && let Some(ws) = state.world.root("Workspace")
        {
            self.expanded.insert(ws);
        }
        if state.selection.revision() != self.seen_selection {
            self.seen_selection = state.selection.revision();
            if let Some(a) = state.selection.active() {
                self.reveal(a, state);
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter by name or class")
                .desired_width(f32::INFINITY),
        );
        let key = (
            state.world.change_cursor(),
            state.history.revision(),
            self.expand_revision,
            state.selection.revision(),
            self.filter.clone(),
        );
        if self.rows_key.as_ref() != Some(&key) {
            self.rebuild(state);
            self.rows_key = Some(key);
        }

        let mut actions: Vec<Action> = Vec::new();
        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(target) = self.scroll_to.take()
            && let Some(i) = self.rows.iter().position(|r| r.id == target)
        {
            let y = i as f32 * (ui_tokens().row_height + ui.spacing().item_spacing.y);
            scroll = scroll.vertical_scroll_offset((y - 80.0).max(0.0));
        }
        let rows = self.rows.clone();
        scroll.show_rows(ui, ui_tokens().row_height, rows.len(), |ui, range| {
            for row in &rows[range] {
                self.row(ui, row, state, &mut actions);
            }
        });

        for a in actions {
            self.run(a, state);
        }
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        row: &Row,
        state: &EditorState,
        actions: &mut Vec<Action>,
    ) {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(width, ui_tokens().row_height),
            Sense::click_and_drag(),
        );
        let selected = state.selection.contains(row.id);
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, 0.0, ui_tokens().accent.gamma_multiply(0.30));
        } else if resp.hovered() {
            painter.rect_filled(rect, 0.0, ui_tokens().hover);
        }
        let x0 = rect.min.x + 4.0 + row.depth as f32 * ui_tokens().indent;
        let cy = rect.center().y;
        // Caret.
        let caret = egui::Rect::from_center_size(egui::pos2(x0 + 7.0, cy), Vec2::splat(14.0));
        if row.has_children {
            let open = self.expanded.contains(&row.id) || !self.filter.is_empty();
            let pts = if open {
                vec![
                    egui::pos2(caret.left() + 3.0, cy - 2.0),
                    egui::pos2(caret.right() - 3.0, cy - 2.0),
                    egui::pos2(caret.center().x, cy + 3.0),
                ]
            } else {
                vec![
                    egui::pos2(caret.center().x - 2.0, caret.top() + 3.0),
                    egui::pos2(caret.center().x + 3.0, cy),
                    egui::pos2(caret.center().x - 2.0, caret.bottom() - 3.0),
                ]
            };
            painter.add(egui::Shape::convex_polygon(
                pts,
                ui_tokens().text_dim,
                Stroke::NONE,
            ));
        }
        let class = state.world.class_name(row.id).unwrap_or("?");
        crate::icons::paint_class(
            &painter,
            egui::Rect::from_center_size(
                egui::pos2(x0 + 24.0, cy),
                Vec2::splat(ui_tokens().icon_size),
            ),
            class,
            ui_tokens().icon,
        );
        let name_pos = egui::pos2(x0 + 36.0, cy);
        let renaming_this = matches!(&self.renaming, Some((id, _, _)) if *id == row.id);
        if renaming_this {
            let edit_rect = egui::Rect::from_min_max(
                egui::pos2(name_pos.x, rect.top() + 2.0),
                egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
            );
            if let Some((id, text, first)) = &mut self.renaming {
                let r = ui.put(edit_rect, egui::TextEdit::singleline(text));
                if *first {
                    r.request_focus();
                    *first = false;
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if esc {
                    self.renaming = None;
                } else if enter || r.lost_focus() {
                    actions.push(Action::Rename(*id));
                }
            }
        } else {
            painter.text(
                name_pos,
                egui::Align2::LEFT_CENTER,
                state.display_name(row.id),
                ui_tokens().body(),
                ui_tokens().text,
            );
            if let Some(meta) = meta_text(state, row.id, class) {
                painter.text(
                    egui::pos2(rect.right() - 6.0, cy),
                    egui::Align2::RIGHT_CENTER,
                    meta,
                    ui_tokens().small(),
                    ui_tokens().text_dim,
                );
            }
        }

        // Drag to reparent.
        if !state.world.is_builtin(row.id) {
            resp.dnd_set_drag_payload(row.id);
        }
        if let Some(dragged) = resp.dnd_hover_payload::<InstanceId>()
            && *dragged != row.id
        {
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(2.0, ui_tokens().accent),
            );
        }
        if let Some(dragged) = resp.dnd_release_payload::<InstanceId>() {
            actions.push(Action::Drop(*dragged, row.id));
        }

        if resp.clicked() {
            let on_caret = resp
                .interact_pointer_pos()
                .is_some_and(|p| caret.contains(p));
            if on_caret && row.has_children {
                actions.push(Action::Toggle(row.id));
            } else {
                actions.push(Action::Click(row.id, ui.input(|i| i.modifiers)));
            }
        }
        if resp.double_clicked() {
            if state.world.is_a(row.id, "Script") {
                actions.push(Action::Open(row.id));
            } else if row.has_children {
                actions.push(Action::Toggle(row.id));
            }
        }
        let id = row.id;
        resp.context_menu(|ui| {
            if !state.selection.contains(id) {
                actions.push(Action::Click(id, egui::Modifiers::NONE));
            }
            ui.menu_button("Insert", |ui| {
                for &class in INSERTABLE {
                    if ui.button(class).clicked() {
                        actions.push(Action::Insert(class, id));
                        ui.close();
                    }
                }
            });
            ui.separator();
            for (label, action, shortcut) in [
                ("Duplicate", "edit.duplicate", "Ctrl+D"),
                ("Group", "edit.group", "Ctrl+G"),
                ("Delete", "edit.delete", "Delete"),
            ] {
                if ui
                    .add(egui::Button::new(label).shortcut_text(shortcut))
                    .clicked()
                {
                    actions.push(Action::Run(action));
                    ui.close();
                }
            }
            if ui
                .add(egui::Button::new("Rename").shortcut_text("F2"))
                .clicked()
            {
                actions.push(Action::Run("edit.rename"));
                ui.close();
            }
            ui.separator();
            if ui.button("Select children").clicked() {
                actions.push(Action::SelectChildren(id));
                ui.close();
            }
            if ui
                .add(egui::Button::new("Focus in viewport").shortcut_text("F"))
                .clicked()
            {
                actions.push(Action::Focus(id));
                ui.close();
            }
        });
    }

    fn run(&mut self, action: Action, state: &mut EditorState) {
        match action {
            Action::Click(id, m) => {
                if m.command {
                    state.selection.toggle(id);
                } else if m.shift
                    && let Some(anchor) = self.anchor
                {
                    let a = self.rows.iter().position(|r| r.id == anchor);
                    let b = self.rows.iter().position(|r| r.id == id);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        let ids: Vec<InstanceId> =
                            self.rows[lo..=hi].iter().map(|r| r.id).collect();
                        state.selection.set(ids);
                    }
                } else {
                    state.selection.select_only(id);
                }
                if !m.shift {
                    self.anchor = Some(id);
                }
                self.seen_selection = state.selection.revision();
            }
            Action::Toggle(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
                self.expand_revision += 1;
            }
            Action::Open(id) => state.request(UiRequest::OpenScript(id)),
            Action::Drop(dragged, onto) => {
                let ids: Vec<InstanceId> = if state.selection.contains(dragged) {
                    state.selection.ids().to_vec()
                } else {
                    vec![dragged]
                };
                state.reparent(&ids, onto);
                self.expanded.insert(onto);
                self.expand_revision += 1;
            }
            Action::Insert(class, parent) => {
                let at = state.insert_point;
                state.insert(class, Some(parent), Some(at));
                self.expanded.insert(parent);
                self.expand_revision += 1;
            }
            Action::Run(id) => state.request(UiRequest::RunAction(id)),
            Action::SelectChildren(id) => {
                let kids = state.world.children(id).to_vec();
                state.selection.set(kids);
            }
            Action::Rename(id) => {
                if let Some((_, text, _)) = self.renaming.take() {
                    let name = text.trim().to_string();
                    if !name.is_empty() && name != state.display_name(id) {
                        state.set_property(&[id], "Name", Value::String(name));
                    }
                }
            }
            Action::Focus(id) => {
                state.selection.select_only(id);
                state.request(UiRequest::FrameSelection);
            }
        }
    }
}

fn meta_text(state: &EditorState, id: InstanceId, class: &str) -> Option<String> {
    match class {
        "Folder" | "Model" => Some(state.world.children(id).len().to_string()),
        "Part" => None,
        _ if state.world.is_builtin(id) => None,
        c => Some(c.to_string()),
    }
}

/// Most rows the play tree draws (a huge place stays responsive while playing).
pub const MAX_PLAY_ROWS: usize = 2000;

/// The play world as a read-only tree, shown in place of the Explorer while a session runs.
///
/// Rows open and close (Workspace and Players start open) but nothing can be selected, renamed,
/// dragged or inserted: the play world belongs to the game and Stop throws it away. Returns
/// the number of rows drawn.
pub fn play_tree(ui: &mut egui::Ui, world: &metallic_core::World) -> usize {
    let mut budget = MAX_PLAY_ROWS;
    egui::ScrollArea::vertical()
        .id_salt("play-tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for &root in world.roots() {
                play_row(ui, world, root, &mut budget);
            }
            if budget == 0 {
                ui.label(
                    RichText::new(format!("Showing the first {MAX_PLAY_ROWS} items"))
                        .small()
                        .color(ui_tokens().text_dim),
                );
            }
        });
    MAX_PLAY_ROWS - budget
}

fn play_row(ui: &mut egui::Ui, world: &metallic_core::World, id: InstanceId, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let class = world.class_name(id).unwrap_or("?");
    let name = match world.get_ref(id, "Name") {
        Ok(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => class.to_string(),
    };
    let header = |ui: &mut egui::Ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(ui_tokens().icon_size), Sense::hover());
        crate::icons::paint_class(ui.painter(), r, class, ui_tokens().icon);
        ui.label(name.as_str());
    };
    let kids = world.children(id);
    if kids.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(ui_tokens().indent);
            header(ui);
        });
        return;
    }
    let open = world.parent(id).is_none() && matches!(class, "Workspace" | "Players");
    egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("play-row", id)),
        open,
    )
    .show_header(ui, header)
    .body(|ui| {
        for &k in kids {
            play_row(ui, world, k, budget);
        }
    });
}
