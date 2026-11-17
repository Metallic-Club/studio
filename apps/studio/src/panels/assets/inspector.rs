//! The browser's inspector (#740): at its right, what the one file or folder selected is. Its picture (or icon), its
//! name, renamed in place, its path, and its type, size and date created; a folder's colour (#741). A close button at
//! its top right hides it until something else is selected.
//!
//! A material's is its editor (#742): its ball alone, drawn again only when it changes; its name, with buttons to give
//! it to the selected parts and to make a copy; its path; and its settings ([`crate::panels::material`]).

use egui::{
    Align2, CornerRadius, FontId, Id, Key, Rect, Role, ScrollArea, Sense, TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use super::{Act, AssetKind, file_size, grid};
use crate::editor::EditorState;
use crate::editor::assets::{Entry, EntryKind, parent_of, type_label};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};
use metallic_engine::metallic_project::AssetId;

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
            let material = match entry.kind {
                EntryKind::File(AssetKind::Material) => editor.project.assets().id(&entry.path),
                _ => None,
            };
            if let Some(id) = material {
                material_view(ui, editor, entry, id, rect.height(), acts);
                return;
            }
            picture(ui, editor, entry, rect.height());
            ui.add_space(space::S4);
            let width = ui.available_width();
            name_field(ui, entry, acts, width);
            path_label(ui, entry);
            ui.add_space(space::S5);
            details(ui, editor, entry);
            if entry.is_folder() {
                folder_color(ui, editor, entry);
            }
        });
}

/// The path, small and selectable, under the name.
fn path_label(ui: &mut Ui, entry: &Entry) {
    let p = current_colors(ui.ctx()).palette;
    ui.add(
        egui::Label::new(egui::RichText::new(&entry.path).size(text::SMALL).color(p.subtle))
            .wrap()
            .selectable(true),
    );
}

/// The material `id`'s editor (#742): its ball, its name with Apply and Copy, its path and its settings.
fn material_view(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, id: AssetId, height: f32, acts: &mut Vec<Act>) {
    ball(ui, editor, entry, id, height);
    ui.add_space(space::S4);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
        let buttons = 2.0 * size::ROW + 2.0 * space::S1;
        name_field(ui, entry, acts, (ui.available_width() - buttons).max(40.0));
        let can_apply = editor.can_apply_material();
        if icon_button(ui, Icon::UiApplyMaterial, "Apply to the selected parts", can_apply).clicked() {
            acts.push(Act::ApplyMaterial(entry.path.clone()));
        }
        if icon_button(ui, Icon::UiCopy, "Make a copy", true).clicked() {
            acts.push(Act::Duplicate(vec![entry.path.clone()]));
        }
    });
    path_label(ui, entry);
    let mut material = match editor.edit_material_of(id) {
        Ok(material) => material,
        Err(error) => {
            let p = current_colors(ui.ctx()).palette;
            ui.add_space(space::S4);
            ui.add(egui::Label::new(egui::RichText::new(error).size(text::SMALL).color(p.muted)).wrap());
            return;
        }
    };
    let before = material;
    let files = crate::panels::values::Files::with_pictures(ui.ctx(), editor);
    crate::panels::material::sections(ui, &files, &mut material);
    if material != before
        && let Err(error) = editor.edit_material(id, material)
    {
        editor.browser.error = Some(error);
    }
}

/// The material `id`'s ball alone, no background (#742): the preview renderer draws it after the frame, from the
/// thumbnails' camera, and it is cut to the ball as a tile's is (#697). Until it is drawn, the tile's picture.
fn ball(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, id: AssetId, height: f32) {
    let side = picture_side(ui, height);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::hover());
    let area = Rect::from_center_size(row.center(), vec2(side, side));
    ui.interact(area, Id::unique("metallic.assets.inspector.ball"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(Role::Image, true, "Material preview"));
    let pixels = (side * ui.ctx().pixels_per_point()).round() as u32;
    editor.materials.preview = Some((id, (pixels, pixels)));
    match editor.materials.preview_texture {
        Some((shown, texture, uv)) if shown == id => grid::paint_ball(ui.painter(), texture, area, uv),
        _ => {
            let mut decodes = 0;
            grid::thumbnail(ui, editor, entry, area, &mut decodes);
            ui.ctx().request_repaint();
        }
    }
    if editor.materials.preview_loading {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// A small button of `icon`, `label` for screen readers and in its tooltip; greyed out when not `enabled`.
fn icon_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(size::ROW, size::ROW), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::TILE), p.surface_hover);
    }
    let ink = match (enabled, response.hovered()) {
        (false, _) => p.text.gamma_multiply(0.48),
        (true, true) => colors.accent,
        (true, false) => p.text,
    };
    icons::paint_at(
        ui.painter(),
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        text::BODY,
        ink,
    );
    response.on_hover_text(label)
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
    let side = picture_side(ui, height);
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

/// The picture's side: as wide as the inspector leaves room for, at most [`PICTURE`] and a third of its `height`.
fn picture_side(ui: &Ui, height: f32) -> f32 {
    (ui.available_width() - 2.0 * space::S6)
        .min(PICTURE)
        .min(height / 3.0)
        .max(48.0)
}

/// The name, as a heading `width` wide that is edited in place: Enter, or leaving it, renames; Escape keeps the name.
/// A missing file's is not edited.
fn name_field(ui: &mut Ui, entry: &Entry, acts: &mut Vec<Act>, width: f32) {
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
            .desired_width(width),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::TextInput, !missing, "Asset name"));
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
