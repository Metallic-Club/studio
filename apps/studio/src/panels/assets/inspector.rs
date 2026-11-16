//! The browser's inspector (#740): at its right, what the one file or folder selected is. Its picture (or icon), its
//! name, renamed in place, its path, and its type, size and date created; a folder's colour (#741). A close button at
//! its top right hides it until something else is selected.

use egui::{
    Align2, CornerRadius, FontId, Id, Key, Rect, Role, ScrollArea, Sense, TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use super::{Act, file_size, grid};
use crate::editor::EditorState;
use crate::editor::assets::{Entry, EntryKind, parent_of, type_label};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// The inspector's width, before it is dragged.
pub(super) const WIDTH: f32 = 280.0;
/// The narrowest it gets.
pub(super) const MIN_WIDTH: f32 = 200.0;
/// Its picture's largest side.
const PICTURE: f32 = 180.0;

/// The entry at `path`, as the browser shows it, if the project has it.
pub(super) fn entry(editor: &EditorState, path: &str) -> Option<Entry> {
    editor
        .asset_children(parent_of(path))
        .into_iter()
        .find(|entry| entry.path == path)
}

/// Draws the inspector of `entry` in `ui`.
pub(super) fn show(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, acts: &mut Vec<Act>) {
    let rect = ui.max_rect();
    let pane = ui.interact(rect, Id::unique("metallic.assets.inspector"), Sense::hover());
    pane.widget_info(|| WidgetInfo::labeled(Role::Group, true, "Inspector"));
    close_button(ui, editor, entry, rect);
    ScrollArea::vertical()
        .id_salt("assets.inspector")
        .auto_shrink([false, false])
        .content_margin(egui::Margin::symmetric(space::S5 as i8, space::S4 as i8))
        .show(ui, |ui| {
            picture(ui, editor, entry, rect.height());
            ui.add_space(space::S4);
            name_field(ui, entry, acts);
            let p = current_colors(ui.ctx()).palette;
            ui.add(
                egui::Label::new(egui::RichText::new(&entry.path).size(text::SMALL).color(p.subtle))
                    .wrap()
                    .selectable(true),
            );
            ui.add_space(space::S5);
            details(ui, editor, entry);
            if entry.is_folder() {
                folder_color(ui, editor, entry);
            }
        });
}

/// A folder's colour (#741): a swatch that opens Studio's colour picker, its hex, and a button back to the usual
/// colour when it has one of its own.
fn folder_color(ui: &mut Ui, editor: &mut EditorState, entry: &Entry) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let own = editor.folder_color(&entry.path);
    let shown = super::grid::folder_ink(editor, &entry.path, &colors);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::hover());
    let label_width = (rect.width() * 0.4).min(110.0);
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        "Colour",
        FontId::proportional(text::SMALL),
        p.muted,
    );
    let side = size::ROW - 8.0;
    let swatch = Rect::from_min_size(
        pos2(rect.left() + label_width, rect.center().y - side / 2.0),
        vec2(side * 1.6, side),
    );
    let [r, g, b, _] = shown.to_array();
    let current =
        metallic_engine::metallic_core::Color3::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
    let mut chosen = None;
    if let Some(crate::panels::values::Edit::Whole(metallic_engine::metallic_core::Value::Color3(picked))) =
        crate::panels::values::swatch(ui, swatch, current, "Folder colour", true)
    {
        let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        chosen = Some(Some([channel(picked.r), channel(picked.g), channel(picked.b)]));
    }
    let hex = if own.is_some() {
        crate::panels::color_picker::hex([r, g, b])
    } else {
        "Usual".to_owned()
    };
    ui.painter().text(
        pos2(swatch.right() + space::S3, rect.center().y),
        Align2::LEFT_CENTER,
        hex,
        FontId::proportional(text::SMALL),
        p.text,
    );
    if own.is_some() {
        let side = size::ROW;
        let reset = Rect::from_min_size(pos2(rect.right() - side, rect.top()), vec2(side, side));
        let response = ui.interact(
            reset,
            Id::unique("metallic.assets.inspector.usual_color"),
            Sense::click(),
        );
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Use the usual colour"));
        if response.hovered() {
            ui.painter()
                .rect_filled(reset, CornerRadius::same(radius::TILE), p.surface_hover);
        }
        let ink = if response.hovered() { p.text } else { p.muted };
        icons::paint_at(
            ui.painter(),
            reset.center(),
            Align2::CENTER_CENTER,
            Icon::UiClose,
            text::SMALL,
            ink,
        );
        if response.on_hover_text("Use the usual colour").clicked() {
            chosen = Some(None);
        }
    }
    if let Some(color) = chosen
        && let Err(error) = editor.set_folder_color(&entry.path, color)
    {
        editor.browser.error = Some(error);
    }
}

/// The close button at the top right of `rect`.
fn close_button(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, rect: Rect) {
    let p = current_colors(ui.ctx()).palette;
    let side = size::ROW;
    let button = Rect::from_min_size(
        pos2(rect.right() - side - space::S2, rect.top() + space::S2),
        vec2(side, side),
    );
    let response = ui.interact(button, Id::unique("metallic.assets.inspector.close"), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Close the inspector"));
    if response.hovered() {
        ui.painter()
            .rect_filled(button, CornerRadius::same(radius::TILE), p.surface_hover);
    }
    let ink = if response.hovered() { p.text } else { p.muted };
    icons::paint_at(
        ui.painter(),
        button.center(),
        Align2::CENTER_CENTER,
        Icon::UiClose,
        text::BODY,
        ink,
    );
    if response.on_hover_text("Close the inspector").clicked() {
        editor.browser.inspector_closed = Some(entry.path.clone());
    }
}

/// The picture: a picture's image, a material's ball, a scene's view, or the kind's icon, centred. At most a third
/// of the inspector's `height`, so the name under it shows in a short panel.
fn picture(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, height: f32) {
    let side = (ui.available_width() - 2.0 * space::S6)
        .min(PICTURE)
        .min(height / 3.0)
        .max(48.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::hover());
    let area = Rect::from_center_size(row.center(), vec2(side, side));
    let mut decodes = 0;
    if !grid::thumbnail(ui, editor, entry, area, &mut decodes) {
        let colors = current_colors(ui.ctx());
        let (icon, ink) = grid::icon_of(editor, entry, &colors);
        icons::paint_at(
            ui.painter(),
            area.center(),
            Align2::CENTER_CENTER,
            icon,
            side * 0.5,
            ink,
        );
    }
}

/// The name, as a heading that is edited in place: Enter, or leaving it, renames; Escape keeps the name. A missing
/// file's is not edited.
fn name_field(ui: &mut Ui, entry: &Entry, acts: &mut Vec<Act>) {
    let p = current_colors(ui.ctx()).palette;
    // What is typed is a folder's whole name, a file's without its type's extension, as on a tile.
    let name = if entry.is_folder() {
        entry.file_name()
    } else {
        entry.name()
    };
    let id = Id::unique(("metallic.assets.inspector.name", entry.path.as_str()));
    let field_id = id.with("field");
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| name.to_owned());
    let missing = matches!(entry.kind, EntryKind::Missing(_));
    let response = ui.add_enabled(
        !missing,
        TextEdit::singleline(&mut typed)
            .id(field_id)
            .font(fonts::named(fonts::STRONG, text::BODY_LG))
            .text_color(p.text)
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, space::S1))
            .desired_width(f32::INFINITY),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::TextInput, !missing, "Name"));
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, typed));
        return;
    }
    ui.data_mut(|data| data.remove::<String>(id));
    let cancelled = ui.input(|input| input.key_pressed(Key::Escape));
    if response.lost_focus() && !cancelled && typed.trim() != name && !typed.trim().is_empty() {
        acts.push(Act::Rename(entry.path.clone(), typed));
    }
}

/// The details: type, size and date created for a file; what is in it for a folder.
fn details(ui: &mut Ui, editor: &EditorState, entry: &Entry) {
    let mut rows: Vec<(&str, String)> = vec![("Type", type_label(entry).to_owned())];
    match entry.kind {
        EntryKind::Folder => {
            let inside = editor.asset_children(&entry.path).len();
            let items = match inside {
                1 => "1 item".to_owned(),
                n => format!("{n} items"),
            };
            rows.push(("Contents", items));
        }
        EntryKind::File(_) => {
            if let Ok(fingerprint) = editor.project.fingerprint(&entry.path) {
                rows.push(("Size", file_size(fingerprint.size)));
            }
        }
        EntryKind::Missing(_) => rows.push(("Status", super::MISSING.to_owned())),
    }
    if let Some(added) = editor.project.assets().added(&entry.path) {
        rows.push(("Date created", super::list::date(added)));
    }
    detail_rows(ui, &rows);
}

/// Rows of a label and its value, the labels in a column of their own.
pub(super) fn detail_rows(ui: &mut Ui, rows: &[(&str, String)]) {
    let p = current_colors(ui.ctx()).palette;
    let label_width = (ui.available_width() * 0.4).min(110.0);
    for (label, value) in rows {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::hover());
        let font = FontId::proportional(text::SMALL);
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font.clone(),
            p.muted,
        );
        let value_rect = Rect::from_min_max(pos2(rect.left() + label_width, rect.top()), rect.max);
        ui.painter().with_clip_rect(value_rect).text(
            pos2(value_rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            value,
            font,
            p.text,
        );
        let line = format!("{label}: {value}");
        ui.interact(
            rect,
            Id::unique(("metallic.assets.inspector.detail", *label)),
            Sense::hover(),
        )
        .widget_info(|| WidgetInfo::labeled(Role::Label, true, &line));
    }
}
