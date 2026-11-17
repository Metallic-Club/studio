//! An asset's properties (#740, #779), in the Properties panel when a file or folder was selected in the Assets
//! browser after the instances: its icon, name and type at the top as an instance's are, a preview under them (a
//! picture, a material's ball, a scene's view, else its icon), then its properties in categories, in the panel's rows
//! and with its value inputs.
//!
//! - **File**: the name (renamed in place), the path, the type, the size or what a folder holds, and the date
//!   created; a folder's colour (#741).
//! - A material (#742) is edited here: Apply and Copy beside its name, then its categories
//!   ([`crate::panels::material`]).

use egui::{Align2, CornerRadius, Id, Layout, Rect, RichText, Role, ScrollArea, Sense, Ui, WidgetInfo, vec2};
use metallic_engine::metallic_core::{Color3, Value};
use metallic_engine::metallic_project::AssetId;

use super::{Act, AssetKind, file_size, grid};
use crate::editor::EditorState;
use crate::editor::assets::{Entry, EntryKind, parent_of, type_label};
use crate::panels::properties::{category_heading, plain_row, plain_value};
use crate::panels::values;
use crate::theme::Tip as _;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// The preview's largest side.
const PREVIEW: f32 = 180.0;

/// The entry at `path`, as the browser shows it, if the project has it.
fn entry(editor: &EditorState, path: &str) -> Option<Entry> {
    editor
        .asset_children(parent_of(path))
        .into_iter()
        .find(|entry| entry.path == path)
}

/// Draws the properties of the file or folder at `path` in the Properties panel's `ui`.
pub(crate) fn show(ui: &mut Ui, editor: &mut EditorState, path: &str) {
    let Some(entry) = entry(editor, path) else {
        crate::panels::empty_note(ui, "That file is gone.");
        return;
    };
    let material = match entry.kind {
        EntryKind::File(AssetKind::Material) => editor.project.assets().id(&entry.path),
        _ => None,
    };
    let mut acts = Vec::new();
    header(ui, editor, &entry, material.is_some(), &mut acts);
    ui.add_space(space::S4);
    let height = ui.available_height();
    ScrollArea::vertical()
        .id_salt("properties.asset")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match material {
                Some(id) => ball(ui, editor, &entry, id, height),
                None => picture(ui, editor, &entry, height),
            }
            ui.add_space(space::S4);
            if category_heading(ui, "File", 0.0) {
                file_rows(ui, editor, &entry, &mut acts);
            }
            ui.add_space(space::S5);
            if let Some(id) = material {
                material_rows(ui, editor, id);
            }
        });
    for act in acts {
        super::apply(ui, editor, act);
    }
}

/// The top line: the file's icon, its name and its type, as an instance's class; a material's Apply and Copy at the
/// right.
fn header(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, material: bool, acts: &mut Vec<Act>) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    ui.horizontal(|ui| {
        let (icon, ink) = grid::icon_of(editor, entry, &colors);
        icons::show(ui, icon, text::BODY_LG, ink);
        let name = if entry.is_folder() {
            entry.file_name()
        } else {
            entry.name()
        };
        ui.label(
            RichText::new(name)
                .font(fonts::named(fonts::HEADING, text::BODY_LG))
                .color(p.text),
        );
        ui.label(RichText::new(type_label(entry)).size(text::SMALL).color(p.subtle));
        if material {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::S1;
                if icon_button(ui, Icon::UiCopy, "Make a copy", true).clicked() {
                    acts.push(Act::Duplicate(vec![entry.path.clone()]));
                }
                let can_apply = editor.can_apply_material();
                if icon_button(ui, Icon::UiApplyMaterial, "Apply to the selected parts", can_apply).clicked() {
                    acts.push(Act::ApplyMaterial(entry.path.clone()));
                }
            });
        }
    });
}

/// The File category: the name (renamed in place: Enter, or leaving it), the path, the type, the size or what a
/// folder holds, the date created, and a folder's colour.
fn file_rows(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, acts: &mut Vec<Act>) {
    let p = current_colors(ui.ctx()).palette;
    let shades = p.rows();
    let mut rows: Vec<(&str, String)> = vec![("Path", entry.path.clone()), ("Type", type_label(entry).to_owned())];
    match entry.kind {
        EntryKind::Folder => {
            let inside = editor.asset_children(&entry.path).len();
            rows.push((
                "Contents",
                match inside {
                    1 => "1 item".to_owned(),
                    n => format!("{n} items"),
                },
            ));
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
    let folder = entry.is_folder();
    let count = 1 + rows.len() + usize::from(folder);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // What is typed is a folder's whole name, a file's without its type's extension, as on a tile.
        let name = if folder { entry.file_name() } else { entry.name() };
        let missing = matches!(entry.kind, EntryKind::Missing(_));
        let (typed, _) = plain_row(ui, "Name", "", shades.fill(0), count == 1, None, |ui| {
            if missing {
                plain_value(ui, "Name", name);
                return None;
            }
            values::text_field(ui, "Name", name, ui.available_width(), false)
        });
        if let Some(typed) = typed.flatten()
            && typed.trim() != name
            && !typed.trim().is_empty()
        {
            acts.push(Act::Rename(entry.path.clone(), typed));
        }
        for (index, (label, value)) in rows.iter().enumerate() {
            let last = index + 2 == count;
            plain_row(ui, label, "", shades.fill(index + 1), last, None, |ui| {
                plain_value(ui, label, value);
            });
        }
        if folder {
            folder_color(ui, editor, entry, shades.fill(count - 1));
        }
    });
}

/// A folder's colour (#741): its swatch, which opens Studio's colour picker, its hex (or that it is the usual one),
/// and a button back to the usual colour when it has one of its own.
fn folder_color(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, fill: egui::Color32) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let own = editor.folder_color(&entry.path);
    let [r, g, b, _] = grid::folder_ink(editor, &entry.path, &colors).to_array();
    let current = Color3::from_rgb_u8(r, g, b);
    let (reset, picked) = plain_row(ui, "Colour", "", fill, true, Some((current, "Folder colour")), |ui| {
        let hex = if own.is_some() {
            crate::panels::color_picker::hex([r, g, b])
        } else {
            "Usual".to_owned()
        };
        ui.label(RichText::new(hex).size(text::BODY).color(p.text));
        if own.is_none() {
            return false;
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            icon_button(ui, Icon::UiClose, "Use the usual colour", true).clicked()
        })
        .inner
    });
    let chosen = match (picked, reset) {
        (Some(values::Edit::Whole(Value::Color3(color))), _) => Some(Some(color.to_rgb_u8())),
        (_, Some(true)) => Some(None),
        _ => None,
    };
    if let Some(color) = chosen
        && let Err(error) = editor.set_folder_color(&entry.path, color)
    {
        editor.browser.error = Some(error);
    }
}

/// A material's categories (#742, #779), edited in place: each change is written into its file and is one of its
/// undo steps.
fn material_rows(ui: &mut Ui, editor: &mut EditorState, id: AssetId) {
    let mut material = match editor.edit_material_of(id) {
        Ok(material) => material,
        Err(error) => {
            let p = current_colors(ui.ctx()).palette;
            ui.add(egui::Label::new(RichText::new(error).size(text::SMALL).color(p.muted)).wrap());
            return;
        }
    };
    let before = material;
    let files = values::Files::with_pictures(ui.ctx(), editor);
    crate::panels::material::categories(ui, &editor.dm, &files, &mut material);
    if material != before
        && let Err(error) = editor.edit_material(id, material)
    {
        editor.browser.error = Some(error);
    }
}

/// The preview's side: as wide as the panel leaves room for, at most [`PREVIEW`] and a third of its `height`.
fn preview_side(ui: &Ui, height: f32) -> f32 {
    (ui.available_width() - 2.0 * space::S6)
        .min(PREVIEW)
        .min(height / 3.0)
        .max(48.0)
}

/// The material `id`'s ball alone, no background (#742): the preview renderer draws it after the frame, from the
/// thumbnails' camera, and it is cut to the ball as a tile's is (#697, #757). Until it is drawn, the tile's picture.
fn ball(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, id: AssetId, height: f32) {
    let side = preview_side(ui, height);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::hover());
    let area = Rect::from_center_size(row.center(), vec2(side, side));
    ui.interact(area, Id::unique("metallic.properties.asset.ball"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(Role::Image, true, "Material preview"));
    let pixels = (side * ui.ctx().pixels_per_point()).round() as u32;
    editor.materials.preview = Some((id, (pixels, pixels)));
    match editor.materials.preview_texture {
        Some((shown, texture, uv)) if shown == id => grid::paint_ball(ui.painter(), texture, area, uv),
        _ => {
            grid::thumbnail(ui, editor, entry, area);
            ui.ctx().request_repaint();
        }
    }
    if editor.materials.preview_loading {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// The preview of anything else: a picture's image, a scene's view, or the kind's icon, centred.
fn picture(ui: &mut Ui, editor: &mut EditorState, entry: &Entry, height: f32) {
    let side = preview_side(ui, height);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side), Sense::hover());
    let area = Rect::from_center_size(row.center(), vec2(side, side));
    ui.interact(area, Id::unique("metallic.properties.asset.preview"), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(Role::Image, true, "Preview"));
    if !grid::thumbnail(ui, editor, entry, area) {
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

/// A small button of `icon`, `label` for screen readers and in its tooltip; greyed out when not `enabled`.
fn icon_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let side = size::ROW - 4.0;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), sense);
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
    response.tip(label)
}
