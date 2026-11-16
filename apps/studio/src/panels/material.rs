//! The material editor (#680, ADR 0012), in the Assets browser's inspector (#742): a selected material's settings, a
//! section each, in Studio's style.
//!
//! - **Colour**: the tint, its texture (the base colour), and Glass.
//! - **Normal**: the normal map and how strongly it bends the light.
//! - **Roughness**, **Metallic**: the value and its texture.
//! - **Emission**: the glow's colour, its strength and its texture.
//! - **Ambient occlusion**: its texture.
//! - **Packed map**: one picture holding ambient occlusion, roughness and metallic, and which channel holds each.
//! - **Mapping**: how the textures are laid on a part (tile size, scale, offset, filtering, wrapping).
//!
//! Each texture is a picker of the project's pictures that also takes one dragged from the Assets panel, with a
//! thumbnail. Every change is written into the material's file at once and is one of its undo steps.

use egui::{ComboBox, CornerRadius, Grid, Rect, RichText, Sense, Slider, Ui, Vec2, pos2, vec2};
use metallic_engine::metallic_core::{Color3, Widget};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Channel, Filter, Material, Packed, Wrap};

use super::values::{self, Edit, Files};
use crate::editor::EditorState;
use crate::theme::current_colors;
use crate::theme::tokens::{size, space, text};

/// The pictures a texture slot takes.
const IMAGES: Widget = Widget::AssetId {
    extensions: &["png", "jpg", "jpeg"],
};

/// A slot's thumbnail side.
const THUMB: f32 = 22.0;
/// The width of the labels' column.
const LABELS: f32 = 84.0;

/// The material's settings, changed in place, a section each.
pub(crate) fn sections(ui: &mut Ui, editor: &mut EditorState, files: &Files, material: &mut Material) {
    let textures = &mut material.textures;
    section(ui, "Colour", |ui| {
        color_row(ui, "Tint", "Colour", &mut material.color);
        texture_row(ui, editor, files, "Texture", "Colour texture", &mut textures.color);
        ui.label(row_label("Glass"));
        ui.checkbox(&mut material.glass, "")
            .on_hover_text("When the part is see-through, its reflections stay at full strength.")
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::CheckBox, true, "Glass"));
        ui.end_row();
    });
    section(ui, "Normal", |ui| {
        texture_row(ui, editor, files, "Texture", "Normal texture", &mut textures.normal);
        number_row(
            ui,
            "Strength",
            "Normal strength",
            &mut material.normal_strength,
            0.0..=2.0,
        );
    });
    section(ui, "Roughness", |ui| {
        number_row(ui, "Value", "Roughness", &mut material.roughness, 0.0..=1.0);
        texture_row(
            ui,
            editor,
            files,
            "Texture",
            "Roughness texture",
            &mut textures.roughness,
        );
    });
    section(ui, "Metallic", |ui| {
        number_row(ui, "Value", "Metallic", &mut material.metallic, 0.0..=1.0);
        texture_row(ui, editor, files, "Texture", "Metallic texture", &mut textures.metallic);
    });
    section(ui, "Emission", |ui| {
        color_row(ui, "Colour", "Emission", &mut material.emission);
        number_row(
            ui,
            "Strength",
            "Emission strength",
            &mut material.emission_strength,
            0.0..=10.0,
        );
        texture_row(ui, editor, files, "Texture", "Emission texture", &mut textures.emission);
    });
    section(ui, "Ambient occlusion", |ui| {
        texture_row(
            ui,
            editor,
            files,
            "Texture",
            "Ambient occlusion texture",
            &mut textures.occlusion,
        );
    });
    packed_section(ui, editor, files, material);
    mapping_section(ui, material);
    *material = material.clamped();
}

/// A section: its heading, then its rows (a label and a control each) as `rows` lays them out.
fn section(ui: &mut Ui, title: &str, rows: impl FnOnce(&mut Ui)) {
    let p = current_colors(ui.ctx()).palette;
    ui.add_space(space::S5);
    ui.label(crate::projects::section_label(title, p.subtle));
    ui.add_space(space::S2);
    // The sliders fill what the labels leave.
    ui.spacing_mut().slider_width = (ui.available_width() - LABELS - 64.0).max(60.0);
    Grid::new(("material.section", title))
        .num_columns(2)
        .min_col_width(LABELS)
        .spacing([space::S3, space::S2])
        .show(ui, rows);
}

/// A row's label.
fn row_label(label: &str) -> RichText {
    RichText::new(label).size(text::SMALL)
}

/// The packed map: one picture for three channels, and which of its channels holds each.
fn packed_section(ui: &mut Ui, editor: &mut EditorState, files: &Files, material: &mut Material) {
    section(ui, "Packed map", |ui| {
        let mut texture = material.packed.map(|packed| packed.texture);
        texture_row(ui, editor, files, "Texture", "Packed map", &mut texture);
        match (texture, material.packed) {
            (None, _) => material.packed = None,
            (Some(texture), Some(packed)) => material.packed = Some(Packed { texture, ..packed }),
            (Some(texture), None) => material.packed = Some(Packed::new(texture)),
        }
        if let Some(packed) = &mut material.packed {
            channel_row(ui, "Occlusion in", &mut packed.occlusion);
            channel_row(ui, "Roughness in", &mut packed.roughness);
            channel_row(ui, "Metallic in", &mut packed.metallic);
        }
    });
    let p = current_colors(ui.ctx()).palette;
    ui.label(
        RichText::new("One picture holding ambient occlusion, roughness and metallic: one sample for three.")
            .size(text::SMALL)
            .color(p.subtle),
    );
}

/// How the textures are laid on a part.
fn mapping_section(ui: &mut Ui, material: &mut Material) {
    section(ui, "Mapping", |ui| {
        ui.label(row_label("Tile size"));
        ui.add(
            egui::DragValue::new(&mut material.tile_size)
                .speed(0.05)
                .range(0.01..=1000.0)
                .suffix(" units"),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Tile size"));
        ui.end_row();
        pair_row(ui, "Scale", &mut material.uv_scale, 0.01);
        pair_row(ui, "Offset", &mut material.uv_offset, 0.01);
        ui.label(row_label("Filtering"));
        ComboBox::from_id_salt("material.filter")
            .selected_text(match material.filter {
                Filter::Linear => "Smooth",
                Filter::Nearest => "Nearest",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut material.filter, Filter::Linear, "Smooth");
                ui.selectable_value(&mut material.filter, Filter::Nearest, "Nearest");
            });
        ui.end_row();
        ui.label(row_label("Wrapping"));
        ComboBox::from_id_salt("material.wrap")
            .selected_text(match material.wrap {
                Wrap::Repeat => "Repeat",
                Wrap::Mirror => "Mirror",
                Wrap::Clamp => "Clamp",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut material.wrap, Wrap::Repeat, "Repeat");
                ui.selectable_value(&mut material.wrap, Wrap::Mirror, "Mirror");
                ui.selectable_value(&mut material.wrap, Wrap::Clamp, "Clamp");
            });
        ui.end_row();
    });
}

/// A value's row: `shown` beside a slider over `range`, labelled `label` for screen readers.
fn number_row(ui: &mut Ui, shown: &str, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.label(row_label(shown));
    let response = ui.add(Slider::new(value, range).step_by(0.01));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, label));
    ui.end_row();
}

fn pair_row(ui: &mut Ui, label: &str, value: &mut [f32; 2], speed: f64) {
    ui.label(row_label(label));
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut value[0]).speed(speed).prefix("U "));
        ui.add(egui::DragValue::new(&mut value[1]).speed(speed).prefix("V "));
    });
    ui.end_row();
}

/// A colour's row: `shown` beside its swatch, which opens Studio's colour picker; `label` for screen readers.
fn color_row(ui: &mut Ui, shown: &str, label: &str, color: &mut [f32; 3]) {
    ui.label(row_label(shown));
    let (rect, _) = ui.allocate_exact_size(vec2(size::ROW * 1.6, size::ROW - 6.0), Sense::hover());
    let current = Color3::new(color[0], color[1], color[2]);
    if let Some(Edit::Whole(metallic_engine::metallic_core::Value::Color3(picked))) =
        values::swatch(ui, rect, current, label, true)
    {
        *color = [picked.r, picked.g, picked.b];
    }
    ui.end_row();
}

/// A packed map's channel: none, or red, green, blue or alpha.
fn channel_row(ui: &mut Ui, label: &str, channel: &mut Option<Channel>) {
    ui.label(row_label(label));
    let name = |channel: Option<Channel>| match channel {
        None => "None",
        Some(Channel::R) => "Red",
        Some(Channel::G) => "Green",
        Some(Channel::B) => "Blue",
        Some(Channel::A) => "Alpha",
    };
    let combo = ComboBox::from_id_salt(("material.channel", label))
        .selected_text(name(*channel))
        .show_ui(ui, |ui| {
            for choice in [
                None,
                Some(Channel::R),
                Some(Channel::G),
                Some(Channel::B),
                Some(Channel::A),
            ] {
                ui.selectable_value(channel, choice, name(choice));
            }
        });
    combo
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
    ui.end_row();
}

/// A texture slot: `shown`, then its thumbnail and a picker of the project's pictures (None clears it; a picture
/// dragged from the Assets panel onto it is taken), labelled `label` for screen readers.
fn texture_row(
    ui: &mut Ui,
    editor: &mut EditorState,
    files: &Files,
    shown: &str,
    label: &str,
    slot: &mut Option<AssetId>,
) {
    ui.label(row_label(shown));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S2;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::hover());
        let p = current_colors(ui.ctx()).palette;
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.surface);
        if let Some(id) = *slot
            && let Some(thumbnail) = thumbnail(ui.ctx(), editor, id)
        {
            ui.painter().image(
                thumbnail.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        let current = slot.map(|id| id.to_string()).unwrap_or_default();
        let width = ui.available_width().max(80.0);
        if let Some(chosen) = values::file_picker(ui, files, IMAGES, label, &current, width, false) {
            *slot = chosen.parse().ok();
        }
    });
    ui.end_row();
}

/// A small picture of the project's image `id` (decoded once, then kept), or `None` when it cannot be read.
pub(crate) fn thumbnail(ctx: &egui::Context, editor: &mut EditorState, id: AssetId) -> Option<egui::TextureHandle> {
    if let Some(known) = editor.thumbnails.get(&id) {
        return known.clone();
    }
    let mut reader = editor.project.asset_reader();
    let made = reader.load_image(&id.to_string()).ok().map(|mut image| {
        image.fit(64);
        let picture =
            egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
        ctx.load_texture(
            format!("metallic.thumbnail.{id}"),
            picture,
            egui::TextureOptions::LINEAR,
        )
    });
    editor.thumbnails.insert(id, made.clone());
    made
}
