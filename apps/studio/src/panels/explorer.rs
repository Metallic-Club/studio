//! The Explorer panel: the game's tree (the services, then what is under them) with expand and collapse, class
//! icons, single selection and a search filter.
//!
//! Each frame it lists the rows that show (only expanded instances are walked, so a large closed tree costs nothing)
//! and draws just the rows in view. A search walks the whole tree: it shows every instance whose name or class
//! matches, with its ancestors opened above it.

use std::collections::HashSet;

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Role, ScrollArea, Sense, Ui, WidgetInfo, pos2, vec2};
use metallic_engine::metallic_core::{DataModel, Id};

use super::{class_icon, empty_note, icon, ids, matches_words, search_box};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// How far each level is indented.
const INDENT: f32 = 14.0;
/// The width of the expand caret's column.
const CARET: f32 = 16.0;

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

    fn icon(&self) -> char {
        icon::TREE
    }

    fn zone(&self) -> Zone {
        Zone::Left
    }

    fn context(&self) -> Context {
        Context::Explorer
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let editor = &mut *cx.editor;
        search_box(ui, &mut editor.explorer.query, "Search the game");
        ui.add_space(space::S3);
        let rows = rows(&editor.dm, &editor.explorer);
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
            let spacing = ui.spacing().item_spacing.y;
            let top = index as f32 * (row_height + spacing);
            let view = ui.available_height();
            scroll = scroll.vertical_scroll_offset((top - view / 2.0).max(0.0));
        }
        let mut clicked = None;
        let mut toggled = None;
        scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
            for row in &rows[range] {
                let selected = editor.selection == Some(row.id);
                match row_ui(ui, &editor.dm, *row, selected, searching) {
                    RowAction::None => {}
                    RowAction::Select => clicked = Some(row.id),
                    RowAction::Toggle => toggled = Some(*row),
                }
            }
        });
        if let Some(row) = toggled {
            editor.explorer.set_expanded(row.id, !row.expanded);
        }
        if let Some(id) = clicked {
            editor.selection = Some(id);
        }
    }
}

/// What a click on a row asked for.
enum RowAction {
    None,
    Select,
    Toggle,
}

/// One row: the caret, the class icon and the name; the selected row on a surface with the accent's bar.
fn row_ui(ui: &mut Ui, dm: &DataModel, row: Row, selected: bool, searching: bool) -> RowAction {
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
        let fill = if selected {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
        if selected {
            let bar = Rect::from_center_size(
                pos2(rect.left() + size::ACCENT_BAR / 2.0, rect.center().y),
                vec2(size::ACCENT_BAR, size::SELECTED_BAR),
            );
            ui.painter().rect_filled(bar, CornerRadius::same(2), colors.accent);
        }
        if row.has_children {
            let hovered = caret_response.as_ref().is_some_and(|response| response.hovered());
            ui.painter().text(
                caret.center(),
                Align2::CENTER_CENTER,
                if row.expanded {
                    icon::CARET_DOWN
                } else {
                    icon::CARET_RIGHT
                },
                fonts::named(fonts::ICONS, text::SMALL),
                if hovered { p.text } else { p.subtle },
            );
        }
        let class_icon = dm
            .class_of(row.id)
            .map_or(icon::CIRCLE, |class| class_icon(dm.registry(), class));
        let x = caret.right() + space::S1;
        ui.painter().text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            class_icon,
            fonts::named(fonts::ICONS, text::BODY),
            if selected { colors.accent } else { p.muted },
        );
        let font = if selected {
            fonts::named(fonts::STRONG, text::BODY)
        } else {
            FontId::proportional(text::BODY)
        };
        ui.painter().text(
            pos2(x + text::BODY + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            &name,
            font,
            if selected { p.text } else { p.muted },
        );
    }
    if caret_response.is_some_and(|response| response.clicked()) || (row.has_children && response.double_clicked()) {
        RowAction::Toggle
    } else if response.clicked() {
        RowAction::Select
    } else {
        RowAction::None
    }
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
