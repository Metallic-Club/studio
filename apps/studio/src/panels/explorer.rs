//! The Explorer panel: the game's tree (the services, then what is under them) with expand and collapse, class
//! icons, selection and a search filter.
//!
//! Clicks select as in Forge and Roblox Studio (#347): a click selects only that row, Ctrl (Cmd) adds or removes one,
//! Shift selects the rows from the anchor (the last row clicked without Shift) to it, and Ctrl+Shift adds them. A
//! closed row that holds something selected is marked with a dot.
//!
//! Each frame it lists the rows that show (only expanded instances are walked, so a large closed tree costs nothing)
//! and draws just the rows in view. A search walks the whole tree: it shows every instance whose name or class
//! matches, with its ancestors opened above it.

use std::collections::HashSet;

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Role, ScrollArea, Sense, Ui, WidgetInfo, pos2, vec2};
use metallic_engine::metallic_core::{DataModel, Id};

use super::insert::{self, Inserting};
use super::{class_icon, empty_note, ids, matches_words, search_box};
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::edit::is_movable;
use crate::editor::{Selection, TreeEdit};
use crate::keymap::Context;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{current_colors, fonts};

/// How far each level is indented.
const INDENT: f32 = 14.0;
/// The width of the expand caret's column.
const CARET: f32 = 16.0;
/// The radius of the dot on a closed row that holds something selected.
const HOLDER_DOT: f32 = 2.5;

/// The Explorer panel.
#[derive(Clone, Copy, Debug)]
pub struct Explorer;

/// What the Explorer remembers: which instances are open, and the search.
#[derive(Clone, Debug, Default)]
pub struct ExplorerState {
    expanded: HashSet<Id>,
    query: String,
    /// An instance to scroll into view next frame.
    scroll_to: Option<Id>,
    /// Where a Shift-click's range starts: the last row clicked without Shift (#347).
    anchor: Option<Id>,
    /// The row being renamed in place (#351).
    renaming: Option<Id>,
    /// The Insert Object window, while it is open (#352).
    inserting: Option<Inserting>,
}

impl ExplorerState {
    /// The Explorer of a game just opened: the workspace open, everything else closed.
    pub fn new(dm: &DataModel) -> Self {
        let mut state = Self::default();
        if let Some(workspace) = dm.registry().class_id("Workspace").and_then(|class| dm.service(class)) {
            state.expanded.insert(workspace);
        }
        state
    }

    /// The search.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Sets the search.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// Whether `id` is open.
    pub fn is_expanded(&self, id: Id) -> bool {
        self.expanded.contains(&id)
    }

    /// Opens or closes `id`.
    pub fn set_expanded(&mut self, id: Id, open: bool) {
        if open {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
    }

    /// What a click on row `id` does to `selection`, given the `rows` shown (#347): alone, it selects only that row;
    /// with `add` (Ctrl, Cmd) it adds or removes it; with `range` (Shift) it selects the rows from the anchor to it, or
    /// adds them with `add` too. The clicked row ends up the active one, unless `add` took it out.
    pub fn click(&mut self, rows: &[Row], selection: &mut Selection, id: Id, add: bool, range: bool) {
        let place = |id: Id| rows.iter().position(|row| row.id == id);
        let anchor = self.anchor.filter(|&anchor| place(anchor).is_some());
        match (range, anchor.and_then(place), place(id)) {
            (true, Some(from), Some(to)) => {
                if !add {
                    selection.clear();
                }
                // From the anchor to the clicked row, so the clicked row is added last and is the active one.
                if from <= to {
                    rows[from..=to].iter().for_each(|row| selection.add(row.id));
                } else {
                    rows[to..=from].iter().rev().for_each(|row| selection.add(row.id));
                }
            }
            _ if add => {
                selection.toggle(id);
                self.anchor = Some(id);
            }
            _ => {
                selection.set_only(Some(id));
                self.anchor = Some(id);
            }
        }
    }

    /// Edit, Select all (#347): every row the Explorer shows, the active instance kept active if it is one of them.
    pub fn select_all(&mut self, dm: &DataModel, selection: &mut Selection) {
        let active = selection.active();
        let mut all = Selection::of(rows(dm, self).into_iter().map(|row| row.id));
        if let Some(active) = active.filter(|&id| all.contains(id)) {
            all.add(active);
        }
        *selection = all;
    }

    /// Starts renaming `id` in its row (#351): its name becomes a text field, Enter keeps what is typed and Escape
    /// drops it.
    pub fn start_rename(&mut self, id: Id) {
        self.renaming = Some(id);
    }

    /// Opens the Insert Object window (#352) for `parent`: what is chosen goes inside it.
    pub fn open_insert(&mut self, parent: Id) {
        self.inserting = Some(Inserting {
            parent,
            query: String::new(),
        });
    }

    /// Whether the Insert Object window is open.
    pub fn is_inserting(&self) -> bool {
        self.inserting.is_some()
    }

    /// The row being renamed, if any.
    pub fn renaming(&self) -> Option<Id> {
        self.renaming
    }

    /// Opens every ancestor of `id` and scrolls it into view.
    pub fn reveal(&mut self, dm: &DataModel, id: Id) {
        let mut at = dm.parent(id).ok().flatten();
        while let Some(parent) = at {
            self.expanded.insert(parent);
            at = dm.parent(parent).ok().flatten();
        }
        self.scroll_to = Some(id);
    }
}

/// One row the Explorer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The instance.
    pub id: Id,
    /// How deep it is: a service is 0.
    pub depth: usize,
    /// Whether it has children the Explorer shows.
    pub has_children: bool,
    /// Whether it is open.
    pub expanded: bool,
}

/// Whether the Explorer shows `id`: not when its class is hidden from it (`RunService`, `UserInputService`, ...).
fn shown(dm: &DataModel, id: Id) -> bool {
    dm.class_of(id)
        .ok()
        .and_then(|class| dm.registry().class(class))
        .is_some_and(|info| !info.hidden_in_explorer())
}

fn shown_children(dm: &DataModel, id: Id) -> impl Iterator<Item = Id> + '_ {
    dm.children(id)
        .into_iter()
        .flatten()
        .filter(move |&child| shown(dm, child))
}

/// The rows the Explorer shows for `state`, top to bottom.
pub fn rows(dm: &DataModel, state: &ExplorerState) -> Vec<Row> {
    let mut out = Vec::new();
    let Some(root) = dm.root() else {
        return out;
    };
    let query = state.query.trim();
    for service in shown_children(dm, root) {
        if query.is_empty() {
            walk(dm, state, service, 0, &mut out);
        } else {
            search(dm, query, service, 0, &mut out);
        }
    }
    out
}

/// Adds `id` and, if it is open, what is under it.
fn walk(dm: &DataModel, state: &ExplorerState, id: Id, depth: usize, out: &mut Vec<Row>) {
    let has_children = shown_children(dm, id).next().is_some();
    let expanded = has_children && state.expanded.contains(&id);
    out.push(Row {
        id,
        depth,
        has_children,
        expanded,
    });
    if expanded {
        for child in shown_children(dm, id) {
            walk(dm, state, child, depth + 1, out);
        }
    }
}

/// Adds `id` and its matching descendants if it or any of them matches `query`; returns whether anything did.
fn search(dm: &DataModel, query: &str, id: Id, depth: usize, out: &mut Vec<Row>) -> bool {
    let at = out.len();
    out.push(Row {
        id,
        depth,
        has_children: false,
        expanded: false,
    });
    let mut any_child = false;
    for child in shown_children(dm, id) {
        any_child |= search(dm, query, child, depth + 1, out);
    }
    let name = dm.name(id).unwrap_or_default();
    let class = dm.class_name(id).unwrap_or_default();
    let matches = matches_words(&format!("{name} {class}"), query);
    if !matches && !any_child {
        out.truncate(at);
        return false;
    }
    out[at].has_children = any_child;
    out[at].expanded = any_child;
    true
}

impl Panel for Explorer {
    fn id(&self) -> &'static str {
        ids::EXPLORER
    }

    fn title(&self) -> &'static str {
        "Explorer"
    }

    fn icon(&self) -> Icon {
        Icon::PanelExplorer
    }

    fn zone(&self) -> Zone {
        Zone::Right
    }

    fn context(&self) -> Context {
        Context::Explorer
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        // Where an inserted part goes (#352): in front of the editor camera.
        let camera = cx.editor.viewport.camera;
        let place = camera.position + camera.forward() * camera.focus().clamp(4.0, 40.0);
        // The running game's tree while one runs (live editing, ADR 0010 §5), with no banner (#309).
        let editor = cx.editor.scene();
        search_box(ui, &mut editor.explorer.query, "Search the game");
        ui.add_space(space::S3);
        let rows = rows(editor.dm, editor.explorer);
        if rows.is_empty() {
            let line = if editor.explorer.query.trim().is_empty() {
                "The game is empty.".to_owned()
            } else {
                format!("Nothing is called \u{201C}{}\u{201D}.", editor.explorer.query.trim())
            };
            empty_note(ui, &line);
            return;
        }
        let searching = !editor.explorer.query.trim().is_empty();
        let row_height = size::ROW;
        let mut scroll = ScrollArea::vertical()
            .id_salt("explorer.rows")
            .auto_shrink([false, false]);
        if let Some(target) = editor.explorer.scroll_to.take()
            && let Some(index) = rows.iter().position(|row| row.id == target)
        {
            // The rows touch (#274): no spacing between them.
            let top = index as f32 * row_height;
            let view = ui.available_height();
            scroll = scroll.vertical_scroll_offset((top - view / 2.0).max(0.0));
        }
        let mut clicked = None;
        let mut toggled = None;
        let mut menu = None;
        let mut renamed = None;
        let renaming = editor
            .explorer
            .renaming
            .filter(|&id| rows.iter().any(|row| row.id == id));
        let shades = current_colors(ui.ctx()).palette.rows();
        let holders = editor.selection.holders(editor.dm);
        // The rows touch (#274), so the stripes are continuous, as the Properties panel's are.
        ui.spacing_mut().item_spacing.y = 0.0;
        scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
            for (index, row) in range.clone().zip(&rows[range]) {
                let selected = editor.selection.contains(row.id);
                let marked = !row.expanded && holders.contains(&row.id);
                let look = Look {
                    selected,
                    marked,
                    searching,
                    renaming: renaming == Some(row.id),
                    stripe: shades.fill(index),
                };
                match row_ui(ui, editor.dm, *row, look) {
                    RowAction::None => {}
                    RowAction::Select => clicked = Some(row.id),
                    RowAction::Toggle => toggled = Some(*row),
                    RowAction::Menu(edit) => menu = Some((row.id, edit)),
                    RowAction::Renamed(name) => renamed = Some((row.id, Some(name))),
                    RowAction::KeepName => renamed = Some((row.id, None)),
                    RowAction::Insert => editor.explorer.open_insert(row.id),
                }
            }
        });
        if let Some(row) = toggled {
            editor.explorer.set_expanded(row.id, !row.expanded);
        }
        if let Some(id) = clicked {
            let modifiers = ui.input(|input| input.modifiers);
            editor
                .explorer
                .click(&rows, editor.selection, id, modifiers.command, modifiers.shift);
        }
        let mut editor = editor;
        if let Some((id, name)) = renamed {
            editor.explorer.renaming = None;
            if let Some(name) = name
                && let Err(error) = editor.rename(id, &name)
            {
                log::warn!("explorer: could not rename: {error}");
            }
        }
        if let Some((id, edit)) = menu {
            // A row's menu acts on the selection, which holds the row (a right-click on another selects it).
            if !editor.selection.contains(id) {
                editor.selection.set_only(Some(id));
            }
            editor.tree_edit(edit);
        }
        if let Some(mut inserting) = editor.explorer.inserting.take() {
            let registry = std::sync::Arc::clone(editor.dm.registry());
            let (chosen, open) = insert::show(ui.ctx(), &registry, &mut inserting);
            if let Some(class) = chosen
                && let Err(error) = editor.insert(class, inserting.parent, Some(place))
            {
                log::warn!("explorer: could not insert: {error}");
            }
            if open && editor.dm.contains(inserting.parent) {
                editor.explorer.inserting = Some(inserting);
            }
        }
    }
}

/// How a row is drawn.
#[derive(Clone, Copy, Debug)]
struct Look {
    /// It is selected.
    selected: bool,
    /// It is closed and holds something selected.
    marked: bool,
    /// A search is shown (no carets).
    searching: bool,
    /// Its name is being edited.
    renaming: bool,
    /// Its stripe.
    stripe: Color32,
}

/// The items of a row's menu, in order, with whether each needs something to move (not a service) or the clipboard.
const ROW_MENU: [(&str, TreeEdit); 7] = [
    ("Cut", TreeEdit::Cut),
    ("Copy", TreeEdit::Copy),
    ("Paste", TreeEdit::Paste),
    ("Paste into", TreeEdit::PasteInto),
    ("Duplicate", TreeEdit::Duplicate),
    ("Rename", TreeEdit::Rename),
    ("Delete", TreeEdit::Delete),
];

/// What a click on a row asked for.
enum RowAction {
    None,
    Select,
    Toggle,
    /// An item of its right-click menu (#351).
    Menu(TreeEdit),
    /// Its new name, typed in place.
    Renamed(String),
    /// Renaming it ended without a new name.
    KeepName,
    /// Its `+`, or Insert Object in its menu (#352).
    Insert,
}

/// One row on its stripe (#274): the caret, the class icon and the name (a text field while it is renamed, #351); a
/// selected row on a square raised fill (#300), and a closed row that holds something selected with a dot at its end
/// (#347). A right-click opens its menu of tree edits (#351).
fn row_ui(ui: &mut Ui, dm: &DataModel, row: Row, look: Look) -> RowAction {
    let Look {
        selected,
        marked,
        searching,
        renaming,
        stripe,
    } = look;
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let name = dm.name(row.id).unwrap_or("?").to_owned();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::TreeItem, true, selected, &name));
    let indent = rect.left() + space::S3 + row.depth as f32 * INDENT;
    let caret = Rect::from_min_size(pos2(indent, rect.top()), vec2(CARET, rect.height()));
    let caret_response = if row.has_children && !searching {
        let response = ui.interact(caret, response.id.with("caret"), Sense::click());
        let verb = if row.expanded { "Collapse" } else { "Expand" };
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{verb} {name}")));
        Some(response)
    } else {
        None
    };
    if ui.is_rect_visible(rect) {
        // The stripe and the fills reach the panel's sides (#301).
        let (band_painter, band) = super::row_band(ui, rect);
        band_painter.rect_filled(band, CornerRadius::ZERO, stripe);
        let fill = if selected {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            Color32::TRANSPARENT
        };
        // Plain (#300): a square fill marks the selected row, with no accent bar.
        band_painter.rect_filled(band, CornerRadius::ZERO, fill);
        if row.has_children {
            let hovered = caret_response.as_ref().is_some_and(|response| response.hovered());
            icons::paint_at(
                ui.painter(),
                caret.center(),
                Align2::CENTER_CENTER,
                if row.expanded { Icon::UiCollapse } else { Icon::UiExpand },
                text::SMALL,
                if hovered { p.text } else { p.subtle },
            );
        }
        let class_icon = dm
            .class_of(row.id)
            .map_or(Icon::ExplorerInstance, |class| class_icon(dm.registry(), class));
        let x = caret.right() + space::S1;
        // Its group's colour (#231), selected or not (#300).
        icons::paint_at(
            ui.painter(),
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            class_icon,
            text::BODY,
            class_icon.color(&colors),
        );
        let font = if selected {
            fonts::named(fonts::STRONG, text::BODY)
        } else {
            FontId::proportional(text::BODY)
        };
        let at = pos2(x + text::BODY + space::S3, rect.center().y);
        if renaming {
            return rename_field(ui, rect, at, &name);
        }
        let label = ui.painter().text(
            at,
            Align2::LEFT_CENTER,
            &name,
            font,
            if selected { p.text } else { p.muted },
        );
        if marked {
            let accent = colors.accent;
            ui.painter().circle_filled(
                pos2(label.right() + space::S3 + HOLDER_DOT, rect.center().y),
                HOLDER_DOT,
                accent,
            );
        }
    }
    // The `+` at the end of the hovered row opens Insert Object for it (#352).
    let plus = Rect::from_center_size(
        pos2(rect.right() - space::S3 - CARET / 2.0, rect.center().y),
        vec2(CARET, CARET),
    );
    let plus_response = ui.interact(plus, response.id.with("insert"), Sense::click());
    plus_response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Insert into {name}")));
    if response.hovered() || plus_response.hovered() {
        let tint = if plus_response.hovered() { p.text } else { p.subtle };
        icons::paint_at(
            ui.painter(),
            plus.center(),
            Align2::CENTER_CENTER,
            Icon::UiAdd,
            text::BODY,
            tint,
        );
    }
    if plus_response.clicked() {
        return RowAction::Insert;
    }
    let mut insert = false;
    let mut chosen = None;
    response.context_menu(|ui| {
        if ui.button("Insert Object\u{2026}").clicked() {
            insert = true;
            ui.close();
        }
        ui.separator();
        let movable = is_movable(dm, row.id);
        for (label, edit) in ROW_MENU {
            let enabled = edit.pastes() || movable;
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                chosen = Some(edit);
                ui.close();
            }
        }
    });
    if insert {
        RowAction::Insert
    } else if let Some(edit) = chosen {
        RowAction::Menu(edit)
    } else if response.secondary_clicked() && !selected {
        RowAction::Select
    } else if caret_response.is_some_and(|response| response.clicked())
        || (row.has_children && response.double_clicked())
    {
        RowAction::Toggle
    } else if response.clicked() {
        RowAction::Select
    } else {
        RowAction::None
    }
}

/// The name of a row being renamed (#351): a text field where the name is, focused, all of it selected at first.
/// Enter, or a click elsewhere, keeps what is typed; Escape keeps the old name.
fn rename_field(ui: &mut Ui, rect: Rect, at: egui::Pos2, name: &str) -> RowAction {
    let id = ui.make_persistent_id("metallic.explorer.rename");
    let field_id = id.with("field");
    let first = ui.data_mut(|data| data.get_temp::<String>(id)).is_none();
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| name.to_owned());
    let field = Rect::from_min_max(
        pos2(at.x - space::S1, rect.top() + 2.0),
        pos2(rect.right() - space::S3, rect.bottom() - 2.0),
    );
    let response = ui.put(
        field,
        egui::TextEdit::singleline(&mut typed)
            .id(field_id)
            .margin(vec2(space::S1, 0.0))
            .font(FontId::proportional(text::BODY)),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, format!("Name of {name}")));
    if first {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field_id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(typed.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), field_id);
        }
    } else if response.lost_focus() || !response.has_focus() {
        ui.data_mut(|data| data.remove::<String>(id));
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        return if escape || typed == name {
            RowAction::KeepName
        } else {
            RowAction::Renamed(typed)
        };
    }
    ui.data_mut(|data| data.insert_temp(id, typed));
    RowAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template's game: Workspace (Camera, Baseplate, SpawnLocation), Lighting and the storage services.
    fn game() -> DataModel {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let scene = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d/Scenes/Main.scene"),
        )
        .unwrap();
        metallic_engine::metallic_project::load_scene_bytes(&mut dm, &scene).unwrap();
        dm
    }

    fn names(dm: &DataModel, rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), dm.name(row.id).unwrap()))
            .collect()
    }

    #[test]
    fn the_tree_lists_services_then_their_open_children() {
        let dm = game();
        let state = ExplorerState::new(&dm);
        let shown = names(&dm, &rows(&dm, &state));
        assert_eq!(shown[0], "Workspace");
        assert!(shown.contains(&"  Baseplate".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Lighting".to_owned()));
        // Hidden from the Explorer.
        assert!(!shown.iter().any(|name| name.contains("RunService")), "{shown:?}");

        let mut closed = state.clone();
        let workspace = rows(&dm, &state)[0].id;
        closed.set_expanded(workspace, false);
        let shown = names(&dm, &rows(&dm, &closed));
        assert!(!shown.contains(&"  Baseplate".to_owned()), "{shown:?}");
    }

    #[test]
    fn a_search_shows_the_matches_under_their_ancestors() {
        let dm = game();
        let mut state = ExplorerState::new(&dm);
        let workspace = rows(&dm, &state)[0].id;
        state.set_expanded(workspace, false);
        state.set_query("spawn");
        assert_eq!(names(&dm, &rows(&dm, &state)), ["Workspace", "  SpawnLocation"]);
        // By class too, every word.
        state.set_query("part base");
        assert_eq!(names(&dm, &rows(&dm, &state)), ["Workspace", "  Baseplate"]);
        state.set_query("teleporter");
        assert!(rows(&dm, &state).is_empty());
    }

    /// The rows' names as one string, the selected ones in brackets, the active one starred.
    fn picked(dm: &DataModel, rows: &[Row], selection: &Selection) -> String {
        rows.iter()
            .filter(|row| selection.contains(row.id))
            .map(|row| {
                let star = if selection.active() == Some(row.id) { "*" } else { "" };
                format!("{}{star}", dm.name(row.id).unwrap())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn clicks_select_one_add_one_or_a_range_from_the_anchor() {
        let dm = game();
        let mut state = ExplorerState::new(&dm);
        let rows = rows(&dm, &state);
        let at = |name: &str| rows.iter().find(|row| dm.name(row.id).unwrap() == name).unwrap().id;
        let mut selection = Selection::new();
        state.click(&rows, &mut selection, at("Baseplate"), false, false);
        assert_eq!(picked(&dm, &rows, &selection), "Baseplate*");
        // Ctrl adds, and takes out again.
        state.click(&rows, &mut selection, at("Lighting"), true, false);
        assert_eq!(picked(&dm, &rows, &selection), "Baseplate Lighting*");
        state.click(&rows, &mut selection, at("Baseplate"), true, false);
        assert_eq!(picked(&dm, &rows, &selection), "Lighting*");
        // Shift: from the anchor (the last row clicked without Shift) to the row, either way.
        state.click(&rows, &mut selection, at("Camera"), false, false);
        state.click(&rows, &mut selection, at("SpawnLocation"), false, true);
        let shown: Vec<String> = rows.iter().map(|row| dm.name(row.id).unwrap().to_owned()).collect();
        let from = shown.iter().position(|name| name == "Camera").unwrap();
        let to = shown.iter().position(|name| name == "SpawnLocation").unwrap();
        assert!(from < to, "{shown:?}");
        assert_eq!(selection.len(), to - from + 1);
        assert_eq!(selection.active(), Some(at("SpawnLocation")));
        state.click(&rows, &mut selection, at("Workspace"), false, true);
        assert_eq!(
            selection.len(),
            from + 1,
            "the range from the anchor up to Workspace, alone"
        );
        assert_eq!(selection.active(), Some(at("Workspace")));
        // Ctrl+Shift adds the range to what is selected.
        state.click(&rows, &mut selection, at("Lighting"), true, false);
        state.click(&rows, &mut selection, at("Lighting"), true, true);
        assert!(selection.contains(at("Workspace")) && selection.contains(at("Lighting")));
        // Select all: every row shown.
        state.select_all(&dm, &mut selection);
        assert_eq!(selection.len(), rows.len());
        assert_eq!(selection.active(), Some(at("Lighting")), "the active one stays active");
    }

    #[test]
    fn revealing_opens_the_ancestors() {
        let dm = game();
        let mut state = ExplorerState::default();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let baseplate = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        assert!(!state.is_expanded(workspace));
        state.reveal(&dm, baseplate);
        assert!(state.is_expanded(workspace));
        assert!(rows(&dm, &state).iter().any(|row| row.id == baseplate));
    }
}
