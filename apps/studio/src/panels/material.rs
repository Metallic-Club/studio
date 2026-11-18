//! The material editor (#680, ADR 0012), in the Assets browser's inspector (#742): a selected material's settings, in
//! sections, in Studio's style.
//!
//! - **Texture maps** (#762): Color, Normal, Roughness, Metalness, Emissive and Ambient occlusion, in that order. Each
//!   is a picture field ([`values::picture_field`]): a small preview, the picture's name (its path while it is
//!   edited, with completions), and a button that opens a window to choose one, as a list or a grid. A picture
//!   dragged from the Assets panel is taken too.
//! - **Values**: the tint and Glass, how strongly the normal map bends the light, the roughness and metalness, and
//!   the glow's colour and strength.
//! - **Packed map**: one picture holding ambient occlusion, roughness and metallic, and which channel holds each.
//! - **Mapping**: how the textures are laid on a part (tile size, scale, offset, filtering, wrapping).
//!
//! Every change is written into the material's file at once and is one of its undo steps.

use egui::{ComboBox, Grid, RichText, Sense, Slider, Ui, vec2};
use metallic_engine::metallic_core::Color3;
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Channel, Filter, Material, Packed, Wrap};

use super::values::{self, Edit, Files};
use crate::editor::EditorState;
use crate::theme::current_colors;
use crate::theme::tokens::{size, space, text};

/// The width of the labels' column.
const LABELS: f32 = 84.0;

/// The material's settings, changed in place, in sections. `files` has the pictures' thumbnails.
pub(crate) fn sections(ui: &mut Ui, files: &Files, material: &mut Material) {
    let textures = &mut material.textures;
    section(ui, "Texture maps", |ui| {
        texture_row(ui, files, "Color", "Color map", &mut textures.color);
        texture_row(ui, files, "Normal", "Normal map", &mut textures.normal);
        texture_row(ui, files, "Roughness", "Roughness map", &mut textures.roughness);
        texture_row(ui, files, "Metalness", "Metalness map", &mut textures.metallic);
        texture_row(ui, files, "Emissive", "Emissive map", &mut textures.emission);
        texture_row(
            ui,
            files,
            "Ambient occlusion",
            "Ambient occlusion map",
            &mut textures.occlusion,
        );
    });
    section(ui, "Values", |ui| {
        color_row(ui, "Tint", "Colour", &mut material.color);
        ui.label(row_label("Glass"));
        ui.checkbox(&mut material.glass, "")
            .on_hover_text("When the part is see-through, its reflections stay at full strength.")
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::CheckBox, true, "Glass"));
        ui.end_row();
        number_row(
            ui,
            "Normal strength",
            "Normal strength",
            &mut material.normal_strength,
            0.0..=2.0,
        );
        number_row(ui, "Roughness", "Roughness", &mut material.roughness, 0.0..=1.0);
        number_row(ui, "Metalness", "Metalness", &mut material.metallic, 0.0..=1.0);
        color_row(ui, "Emissive", "Emissive colour", &mut material.emission);
        number_row(
            ui,
            "Emissive strength",
            "Emissive strength",
            &mut material.emission_strength,
            0.0..=10.0,
        );
    });
    packed_section(ui, files, material);
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
fn packed_section(ui: &mut Ui, files: &Files, material: &mut Material) {
    section(ui, "Packed map", |ui| {
        let mut texture = material.packed.map(|packed| packed.texture);
        texture_row(ui, files, "Texture", "Packed map", &mut texture);
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

/// A texture map: `shown`, then a picture field (#762, [`values::picture_field`]) labelled `label` for screen
/// readers; emptied, it clears the slot.
fn texture_row(ui: &mut Ui, files: &Files, shown: &str, label: &str, slot: &mut Option<AssetId>) {
    ui.label(row_label(shown));
    let current = slot.map(|id| id.to_string()).unwrap_or_default();
    let width = ui.available_width().max(120.0);
    if let Some(chosen) = values::picture_field(ui, files, label, &current, width) {
        *slot = chosen.parse().ok();
    }
    ui.end_row();
}

/// A small picture of the project's image `id`, once it is decoded (off the UI thread, #771, then kept); `None`
/// meanwhile, and for one that cannot be read. Asking draws nothing: only what is on screen should ask.
pub(crate) fn thumbnail(ctx: &egui::Context, editor: &mut EditorState, id: AssetId) -> Option<egui::TextureHandle> {
    editor.picture_loader.receive(ctx, &mut editor.thumbnails);
    if let Some(known) = editor.thumbnails.get(&id) {
        return known.clone();
    }
    editor.picture_loader.want(ctx, &editor.project, id);
    None
}
