//! The browser's right side: the open folder's folders and files (or the search's results) as tiles that wrap with
//! the panel's width (#676). A tile is its icon and its name; it is selected by a click, opened by a double-click,
//! and dragged onto a folder to move it there.

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, FontId, Id, Order, Rect, Role, ScrollArea, Sense, Stroke, TextFormat, Ui, WidgetInfo,
};
use egui::{pos2, vec2};

use super::{Act, AssetDrag, Keys, MISSING, context_menu, drop_target, dropped_on, file_size, rename_field};
use crate::editor::EditorState;
use crate::editor::assets::{Entry, EntryKind};
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, space, text};

/// A tile's size.
pub(super) const TILE: egui::Vec2 = egui::vec2(84.0, 82.0);
/// The space between tiles.
const GAP: f32 = space::S2;
/// A tile's icon.
const ICON: f32 = 30.0;
/// The height of a tile's icon area, above its name.
const ICON_AREA: f32 = 44.0;

/// Draws the tiles.
pub(super) fn show(ui: &mut Ui, editor: &EditorState, keys: &Keys, acts: &mut Vec<Act>) {
    let entries = super::shown_entries(editor);
    let area = ui.available_rect_before_wrap();
    // The empty space: a click there clears the selection, a right-click offers a new folder or file.
    let ground = ui.interact(area, Id::unique("metallic.assets.ground"), Sense::click());
    ground.widget_info(|| WidgetInfo::labeled(Role::Grid, true, "Assets"));
    let folder = editor.browser.open.clone();
    let searching = !editor.browser.query.trim().is_empty();
    let ground_menu: Vec<super::MenuLine<'_>> = super::NEW_ITEMS
        .iter()
        .map(|(label, _)| (*label, None, !searching))
        .collect();
    if let Some(new) = context_menu(&ground, &ground_menu).and_then(|i| super::new_item(ground_menu[i].0)) {
        acts.push(Act::New(folder.clone(), new));
    }
    if ground.clicked() && !editor.browser.selected.is_empty() {
        acts.push(Act::Click {
            path: String::new(),
            add: false,
            range: false,
        });
    }
    // Tiles dropped on the empty space go into the open folder (not while a search shows files from anywhere). Taken
    // after the tiles, so a folder's tile under the pointer takes a drop first.
    let ground_drop = |acts: &mut Vec<Act>| {
        if let Some(paths) = dropped_on(&ground).filter(|_| !searching) {
            acts.push(Act::Move(paths, folder.clone()));
        }
    };
    if entries.is_empty() {
        let line = if searching {
            format!("Nothing is called \u{201C}{}\u{201D}.", editor.browser.query.trim())
        } else {
            "This folder is empty. Right-click to add a folder or a script.".to_owned()
        };
        crate::panels::empty_note(ui, &line);
        ground_drop(acts);
        return;
    }
    let columns = (((area.width() + GAP) / (TILE.x + GAP)).floor() as usize).max(1);
    let lines = entries.len().div_ceil(columns);
    ScrollArea::vertical()
        .id_salt("assets.tiles")
        .auto_shrink([false, false])
        .show_rows(ui, TILE.y + GAP, lines, |ui, range| {
            for line in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    let start = line * columns;
                    for entry in &entries[start..(start + columns).min(entries.len())] {
                        tile(ui, editor, entry, keys, acts);
                    }
                });
                ui.add_space(GAP);
            }
        });
    ground_drop(acts);
    drag_chip(ui);
}

/// One tile: its icon on a fill when selected or under the pointer, its name under it (two lines at most).
fn tile(ui: &mut Ui, editor: &EditorState, entry: &Entry, keys: &Keys, acts: &mut Vec<Act>) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let selected = editor.browser.selected.contains(&entry.path);
    let renaming = editor.browser.renaming.as_deref() == Some(entry.path.as_str());
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click_and_drag());
    let name = entry.name().to_owned();
    response.widget_info(|| WidgetInfo::selected(Role::GridCell, true, selected, &name));
    let folder = entry.is_folder();
    let dropping = folder && drop_target(&response, &entry.path);
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            Some(p.surface_hover)
        } else if response.hovered() {
            Some(p.surface)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
        }
        if dropping {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(radius::ITEM),
                Stroke::new(2.0, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        let (icon, ink) = match entry.kind {
            EntryKind::Folder => (Icon::AssetFolder, Icon::AssetFolder.color(&colors)),
            EntryKind::File(kind) => (kind.icon(), kind.icon().color(&colors)),
            EntryKind::Missing(_) => (Icon::UiWarning, colors.accent),
        };
        let centre = pos2(rect.center().x, rect.top() + space::S2 + ICON_AREA / 2.0);
        icons::paint_at(ui.painter(), centre, Align2::CENTER_CENTER, icon, ICON, ink);
        let label = Rect::from_min_max(
            pos2(rect.left() + space::S1, rect.top() + space::S2 + ICON_AREA),
            pos2(rect.right() - space::S1, rect.bottom() - space::S1),
        );
        if renaming {
            let field = Rect::from_min_size(label.min, vec2(label.width(), 20.0));
            let typed_name = if folder { entry.file_name() } else { entry.name() };
            if let Some(act) = rename_field(ui, field, &entry.path, typed_name, true) {
                acts.push(act);
            }
        } else {
            let ink = match entry.kind {
                EntryKind::Missing(_) => p.subtle,
                _ if selected => p.text,
                _ => p.muted,
            };
            let mut job =
                LayoutJob::single_section(name.clone(), TextFormat::simple(FontId::proportional(text::SMALL), ink));
            job.wrap = TextWrapping {
                max_width: label.width(),
                max_rows: 2,
                break_anywhere: true,
                overflow_character: Some('\u{2026}'),
            };
            job.halign = egui::Align::Center;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(pos2(label.center().x, label.top()), galley, ink);
        }
    }
    let response = response.on_hover_text(hover_text(editor, entry));
    if let Some(paths) = dropped_on(&response).filter(|_| folder) {
        acts.push(Act::Move(paths, entry.path.clone()));
    }
    // Dragging the tile drags the selection, or the tile alone when it is not selected.
    let dragged = if selected {
        editor.browser.selected.iter().cloned().collect()
    } else {
        vec![entry.path.clone()]
    };
    if !renaming && !matches!(entry.kind, EntryKind::Missing(_)) {
        response.dnd_set_drag_payload(AssetDrag { paths: dragged });
    }
    let present = !matches!(entry.kind, EntryKind::Missing(_));
    let menu: Vec<super::MenuLine<'_>> = vec![
        ("Open", None, folder),
        ("", None, true),
        ("Rename", keys.rename.clone(), present),
        ("Duplicate", keys.duplicate.clone(), present),
        ("Delete", keys.delete.clone(), present),
    ];
    let targets: Vec<String> = if selected {
        editor.browser.selected.iter().cloned().collect()
    } else {
        vec![entry.path.clone()]
    };
    match context_menu(&response, &menu).map(|i| menu[i].0) {
        Some("Open") => acts.push(Act::Activate(entry.clone())),
        Some("Rename") => acts.push(Act::StartRename(entry.path.clone())),
        Some("Duplicate") => acts.push(Act::Duplicate(targets)),
        Some("Delete") => acts.push(Act::Delete(targets)),
        _ => {
            let modifiers = ui.input(|input| input.modifiers);
            if response.double_clicked() && !renaming {
                acts.push(Act::Activate(entry.clone()));
            } else if response.clicked() || (response.secondary_clicked() && !selected) {
                acts.push(Act::Click {
                    path: entry.path.clone(),
                    add: modifiers.command && response.clicked(),
                    range: modifiers.shift && response.clicked(),
                });
            }
        }
    }
}

/// What a tile's tooltip says: its path, and its kind and size (or that it is missing).
fn hover_text(editor: &EditorState, entry: &Entry) -> String {
    match entry.kind {
        EntryKind::Folder => format!("{}\nFolder", entry.path),
        EntryKind::Missing(kind) => format!("{}\n{} \u{2022} {MISSING}", entry.path, kind.label()),
        EntryKind::File(kind) => {
            let size = editor
                .project
                .fingerprint(&entry.path)
                .map(|fingerprint| format!(" \u{2022} {}", file_size(fingerprint.size)))
                .unwrap_or_default();
            format!("{}\n{}{size}", entry.path, kind.label())
        }
    }
}

/// While tiles are dragged, a chip beside the pointer saying what is dragged.
fn drag_chip(ui: &Ui) {
    let Some(drag) = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx()) else {
        return;
    };
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let label = match drag.paths.as_slice() {
        [one] => crate::editor::assets::file_name(one).to_owned(),
        many => format!("{} items", many.len()),
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(Order::Tooltip, Id::unique("metallic.assets.drag")));
    let galley = painter.layout_no_wrap(label, FontId::proportional(text::SMALL), p.text);
    let rect = Rect::from_min_size(
        pointer + vec2(space::S6, space::S4),
        vec2(galley.size().x + 2.0 * space::S3, galley.size().y + 2.0 * space::S2),
    );
    painter.rect(
        rect,
        CornerRadius::same(radius::ITEM),
        p.surface,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(space::S3, space::S2), galley, p.text);
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
}
