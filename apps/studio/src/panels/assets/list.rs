//! The browser's right side as a list (#699): a header of Name, Type, Date created and Size, whose columns sort the
//! rows (clicked again, the other way), and a row for each folder and file, which selects, opens, drags, renames and
//! has the menu a tile has ([`super::grid::behave`]). Ctrl and the wheel make the rows taller or shorter (#696).

use egui::{Align2, CornerRadius, FontId, Id, Rect, Role, ScrollArea, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

use super::grid::{self, Ground};
use super::{Act, Keys, file_size, rename_field};
use crate::editor::EditorState;
use crate::editor::assets::{Column, Entry, EntryKind, type_label};
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};

/// The Type column's width.
const TYPE_WIDTH: f32 = 120.0;
/// The Date created column's width.
const CREATED_WIDTH: f32 = 140.0;
/// The Size column's width.
const SIZE_WIDTH: f32 = 80.0;
/// The narrowest the Name column gets.
const MIN_NAME: f32 = 120.0;
/// The shortest and the tallest a row gets, however the list is scaled.
const ROW_HEIGHTS: (f32, f32) = (20.0, 60.0);

/// A row's height at the browser's scale (#696).
fn row_height(scale: f32) -> f32 {
    (size::ROW * scale).round().clamp(ROW_HEIGHTS.0, ROW_HEIGHTS.1)
}

/// The cells of the row or header `rect`, left to right: Name (what the others leave), Type, Date created and Size.
fn cells(rect: Rect) -> [Rect; 4] {
    let name = (rect.width() - TYPE_WIDTH - CREATED_WIDTH - SIZE_WIDTH).max(MIN_NAME);
    let mut x = rect.left();
    let mut cell = |width: f32| {
        let cell = Rect::from_min_max(pos2(x, rect.top()), pos2(x + width, rect.bottom()));
        x += width;
        cell
    };
    [cell(name), cell(TYPE_WIDTH), cell(CREATED_WIDTH), cell(SIZE_WIDTH)]
}

/// When `seconds` (since 1970) was, in the local time: `2026-11-15 14:03`.
pub(crate) fn date(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|utc| utc.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Draws the list.
pub(super) fn show(ui: &mut Ui, editor: &mut EditorState, keys: &Keys, acts: &mut Vec<Act>) {
    // Pictures decoded this frame, as the tiles do (#681).
    let mut decodes = 0;
    let entries = super::shown_entries(editor);
    let area = ui.available_rect_before_wrap();
    let ground = Ground::of(ui, editor, area, Role::List, acts);
    header(ui, editor);
    if ground.empty(ui, editor, &entries, acts) {
        return;
    }
    grid::zoom(ui, editor, area);
    let height = row_height(editor.browser.scale);
    let shades = current_colors(ui.ctx()).palette.rows();
    ScrollArea::vertical()
        .id_salt("assets.list")
        .auto_shrink([false, false])
        .show_rows(ui, height, entries.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let stripe = shades.fill(index);
                row(ui, editor, &entries[index], height, stripe, keys, acts, &mut decodes);
            }
        });
    ground.drop(acts);
    grid::drag_chip(ui);
}

/// The header: a column's name sorts the list by it, again the other way; the sorted column has an arrow.
fn header(ui: &mut Ui, editor: &mut EditorState) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - size::BORDER / 2.0,
        Stroke::new(size::BORDER, p.border),
    );
    for (column, cell) in Column::ALL.into_iter().zip(cells(rect)) {
        let response = ui.interact(
            cell,
            Id::unique(("metallic.assets.header", column.label())),
            Sense::click(),
        );
        let sorted = editor.browser.sort.column == column;
        response.widget_info(|| WidgetInfo::selected(Role::ColumnHeader, true, sorted, column.label()));
        if response.hovered() {
            ui.painter().rect_filled(
                cell.shrink2(vec2(0.0, 1.0)),
                CornerRadius::same(radius::TILE),
                p.surface,
            );
        }
        let ink = if sorted || response.hovered() { p.text } else { p.muted };
        let galley = ui
            .painter()
            .layout_no_wrap(column.label().to_owned(), FontId::proportional(text::SMALL), ink);
        let width = galley.size().x;
        let at = pos2(cell.left() + space::S3, cell.center().y - galley.size().y / 2.0);
        ui.painter().with_clip_rect(cell).galley(at, galley, ink);
        if sorted {
            let arrow = if editor.browser.sort.descending {
                Icon::UiMoveDown
            } else {
                Icon::UiMoveUp
            };
            icons::paint_at(
                ui.painter(),
                pos2(at.x + width + space::S1, cell.center().y),
                Align2::LEFT_CENTER,
                arrow,
                text::SMALL,
                p.subtle,
            );
        }
        if response.clicked() {
            editor.browser.sort_by(column);
        }
    }
}

/// One row on its `stripe`: the icon (or the picture) and the name, then the type, the date it was added and the size;
/// on a raised fill when selected or under the pointer.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    editor: &mut EditorState,
    entry: &Entry,
    height: f32,
    stripe: egui::Color32,
    keys: &Keys,
    acts: &mut Vec<Act>,
    decodes: &mut usize,
) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let selected = editor.browser.selected.contains(&entry.path);
    let renaming = editor.browser.renaming.as_deref() == Some(entry.path.as_str());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let name = entry.name().to_owned();
    response.widget_info(|| WidgetInfo::selected(Role::Row, true, selected, &name));
    let folder = entry.is_folder();
    let dropping = folder && super::drop_target(&response, &entry.path);
    let [name_cell, type_cell, created_cell, size_cell] = cells(rect);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, stripe);
        if selected || response.hovered() {
            let fill = if selected { p.surface_hover } else { p.surface };
            ui.painter().rect_filled(rect, CornerRadius::same(radius::TILE), fill);
        }
        if dropping {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(radius::TILE),
                Stroke::new(2.0, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        // The picture or the icon, as tall as the row leaves room for.
        let side = (height - 4.0).max(12.0);
        let picture = Rect::from_min_size(
            pos2(name_cell.left() + space::S2, rect.center().y - side / 2.0),
            vec2(side, side),
        );
        if !grid::thumbnail(ui, editor, entry, picture, decodes) {
            let (icon, ink) = grid::icon_of(entry, &colors);
            let icon_size = (side * 0.75).max(text::SMALL);
            icons::paint_at(
                ui.painter(),
                picture.center(),
                Align2::CENTER_CENTER,
                icon,
                icon_size,
                ink,
            );
        }
        let left = picture.right() + space::S3;
        if renaming {
            let field = Rect::from_min_max(
                pos2(left, rect.center().y - 10.0),
                pos2(name_cell.right() - space::S2, rect.center().y + 10.0),
            );
            let typed_name = if folder { entry.file_name() } else { entry.name() };
            if let Some(act) = rename_field(ui, field, &entry.path, typed_name, false) {
                acts.push(act);
            }
        } else {
            let ink = match entry.kind {
                EntryKind::Missing(_) => p.subtle,
                _ if selected => p.text,
                _ => p.muted,
            };
            let clip = Rect::from_min_max(
                pos2(left, rect.top()),
                pos2(name_cell.right() - space::S2, rect.bottom()),
            );
            ui.painter().with_clip_rect(clip).text(
                pos2(left, rect.center().y),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(text::BODY),
                ink,
            );
        }
        let created = editor.project.assets().added(&entry.path).map(date);
        let bytes = match entry.kind {
            EntryKind::File(_) => editor
                .project
                .fingerprint(&entry.path)
                .ok()
                .map(|fingerprint| file_size(fingerprint.size)),
            _ => None,
        };
        let kind = match entry.kind {
            EntryKind::Missing(_) => format!("{} ({})", type_label(entry), super::MISSING.to_lowercase()),
            _ => type_label(entry).to_owned(),
        };
        for (cell, value) in [(type_cell, Some(kind)), (created_cell, created), (size_cell, bytes)] {
            let Some(value) = value else {
                continue;
            };
            ui.painter().with_clip_rect(cell).text(
                pos2(cell.left() + space::S3, rect.center().y),
                Align2::LEFT_CENTER,
                value,
                FontId::proportional(text::SMALL),
                p.muted,
            );
        }
    }
    // The button on a material's or a script file's row sits at the end of its name (#703, #704).
    grid::behave(ui, editor, entry, name_cell, response, keys, acts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_scale_within_limits() {
        assert_eq!(row_height(1.0), size::ROW);
        assert_eq!(row_height(0.1), ROW_HEIGHTS.0);
        assert_eq!(row_height(10.0), ROW_HEIGHTS.1);
    }

    #[test]
    fn the_name_column_takes_what_the_others_leave() {
        let [name, kind, created, bytes] = cells(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 24.0)));
        assert_eq!(name.width(), 800.0 - TYPE_WIDTH - CREATED_WIDTH - SIZE_WIDTH);
        assert_eq!(kind.left(), name.right());
        assert_eq!(bytes.right(), 800.0);
        assert_eq!(created.width(), CREATED_WIDTH);
        // Never narrower than its least, however narrow the list.
        assert_eq!(
            cells(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 24.0)))[0].width(),
            MIN_NAME
        );
    }

    #[test]
    fn dates_read_as_local_times() {
        let shown = date(1_794_787_200);
        assert_eq!(shown.len(), "2026-11-15 14:03".len(), "{shown}");
        assert!(shown.starts_with("2026-11-"), "{shown}");
    }
}
