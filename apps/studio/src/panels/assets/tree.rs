//! The browser's left side: the project's folders and files as a tree (#676). The root is the project; a folder's
//! caret opens it here, a click opens it on the right. The open folder is highlighted, and a folder takes tiles
//! dropped on it.

use egui::Role;
use egui::{Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

use super::{Act, Keys, MenuLine, ROOT, context_menu, drop_target, dropped_on};
use crate::editor::EditorState;
use crate::editor::assets::{self as browser, Entry, EntryKind};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{current_colors, fonts};

/// How far each level is indented.
const INDENT: f32 = 14.0;
/// The caret's width.
const CARET: f32 = 16.0;

/// One row of the tree.
struct Row {
    entry: Entry,
    depth: usize,
    has_children: bool,
    expanded: bool,
}

/// The rows shown: the root, then each open folder's folders and files, depth first.
fn rows(editor: &EditorState) -> Vec<Row> {
    let root = Entry {
        path: String::new(),
        kind: EntryKind::Folder,
        id: None,
    };
    let mut out = vec![Row {
        entry: root,
        depth: 0,
        has_children: true,
        expanded: true,
    }];
    // Script files take their scripts' types (#701), worked out once for the whole tree.
    let types = browser::script_types(&editor.dm);
    walk(editor, &types, "", 1, &mut out);
    out
}

fn walk(
    editor: &EditorState,
    types: &std::collections::HashMap<String, crate::panels::assets::ScriptType>,
    folder: &str,
    depth: usize,
    out: &mut Vec<Row>,
) {
    let mut entries = browser::children(&editor.project, folder);
    browser::retype(&mut entries, types);
    for entry in entries {
        let is_folder = entry.is_folder();
        let expanded = is_folder && editor.browser.expanded.contains(&entry.path);
        let has_children = is_folder && !browser::children(&editor.project, &entry.path).is_empty();
        let path = entry.path.clone();
        out.push(Row {
            entry,
            depth,
            has_children,
            expanded,
        });
        if expanded {
            walk(editor, types, &path, depth + 1, out);
        }
    }
}

/// Draws the tree.
pub(super) fn show(ui: &mut Ui, editor: &EditorState, keys: &Keys, acts: &mut Vec<Act>) {
    let rows = rows(editor);
    let shades = current_colors(ui.ctx()).palette.rows();
    ui.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical()
        .id_salt("assets.tree")
        .auto_shrink([false, false])
        .show_rows(ui, size::ROW, rows.len(), |ui, range| {
            for (index, row) in range.clone().zip(&rows[range]) {
                row_ui(ui, editor, row, shades.fill(index), keys, acts);
            }
        });
}

/// One row: the caret, the icon and the name; the open folder on a raised fill.
fn row_ui(ui: &mut Ui, editor: &EditorState, row: &Row, stripe: Color32, keys: &Keys, acts: &mut Vec<Act>) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let entry = &row.entry;
    let is_root = entry.path.is_empty();
    let is_folder = entry.is_folder();
    // A file's row shows its whole file name, a folder's its name.
    let name = if is_root {
        ROOT
    } else if is_folder {
        entry.name()
    } else {
        entry.file_name()
    };
    let open = is_folder && entry.path == editor.browser.open && editor.browser.query.trim().is_empty();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::TreeItem, true, open, name));
    let indent = rect.left() + space::S3 + row.depth as f32 * INDENT;
    let caret = Rect::from_min_size(pos2(indent, rect.top()), vec2(CARET, rect.height()));
    let caret_response = (row.has_children && !is_root).then(|| {
        let response = ui.interact(caret, response.id.with("caret"), Sense::click());
        let verb = if row.expanded { "Collapse" } else { "Expand" };
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{verb} {name}")));
        response
    });
    let dropping = is_folder && drop_target(&response, &entry.path);
    if ui.is_rect_visible(rect) {
        let (band_painter, mut band) = crate::panels::row_band(ui, rect);
        // To the panel's left edge, but not past the tree's right one: the line beside it stays whole (#694).
        band.max.x = band.max.x.min(rect.right());
        band_painter.rect_filled(band, CornerRadius::ZERO, stripe);
        let fill = if open {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            Color32::TRANSPARENT
        };
        band_painter.rect_filled(band, CornerRadius::ZERO, fill);
        if dropping {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                CornerRadius::ZERO,
                Stroke::new(2.0, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(caret_response) = &caret_response {
            let tint = if caret_response.hovered() { p.text } else { p.subtle };
            let icon = if row.expanded { Icon::UiCollapse } else { Icon::UiExpand };
            icons::paint_at(
                ui.painter(),
                caret.center(),
                Align2::CENTER_CENTER,
                icon,
                text::SMALL,
                tint,
            );
        }
        let icon = match entry.kind {
            EntryKind::Folder if is_root => Icon::PanelAssets,
            EntryKind::Folder => Icon::AssetFolder,
            EntryKind::File(kind) => kind.icon(),
            EntryKind::Missing(_) => Icon::UiWarning,
        };
        let ink = match entry.kind {
            EntryKind::Missing(_) => colors.accent,
            _ => icon.color(&colors),
        };
        let x = caret.right() + space::S1;
        icons::paint_at(
            ui.painter(),
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            icon,
            text::BODY,
            ink,
        );
        let font = if open {
            fonts::named(fonts::STRONG, text::BODY)
        } else {
            FontId::proportional(text::BODY)
        };
        let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(rect.right() - space::S2, rect.bottom()));
        ui.painter().with_clip_rect(clip).text(
            pos2(x + text::BODY + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            font,
            if open { p.text } else { p.muted },
        );
    }
    if let Some(paths) = dropped_on(&response).filter(|_| is_folder) {
        acts.push(Act::Move(paths, entry.path.clone()));
    }
    let edits = [
        MenuLine::Separator,
        MenuLine::Item("Rename", keys.rename.clone(), true),
        MenuLine::Item("Duplicate", keys.duplicate.clone(), true),
        MenuLine::Item("Delete", keys.delete.clone(), true),
    ];
    let menu: Vec<MenuLine<'_>> = if is_root {
        vec![super::new_submenu(true)]
    } else if is_folder {
        std::iter::once(super::new_submenu(true)).chain(edits).collect()
    } else {
        std::iter::once(MenuLine::Item("Show in Folder", None, true))
            .chain(edits)
            .collect()
    };
    let path = entry.path.clone();
    let chosen = context_menu(&response, &menu);
    match chosen {
        Some(label) if super::new_item(label).is_some() => {
            if let Some(new) = super::new_item(label) {
                acts.push(Act::New(path, new));
            }
        }
        Some("Show in Folder") => acts.push(Act::Show(path)),
        Some("Rename") => {
            // Named on its tile, in its folder.
            acts.push(Act::Show(path.clone()));
            acts.push(Act::StartRename(path));
        }
        Some("Duplicate") => acts.push(Act::Duplicate(vec![path])),
        Some("Delete") => acts.push(Act::Delete(vec![path])),
        _ => {
            if caret_response.is_some_and(|caret| caret.clicked()) {
                acts.push(Act::Toggle(path));
            } else if response.double_clicked() && !is_folder {
                acts.push(Act::Activate(entry.clone()));
            } else if response.double_clicked() && !is_root {
                acts.push(Act::Toggle(path));
            } else if response.clicked() {
                acts.push(if is_folder { Act::Open(path) } else { Act::Show(path) });
            }
        }
    }
}
