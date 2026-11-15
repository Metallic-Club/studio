//! The material editor (#680, ADR 0012): a material in a tab of its own beside the viewport, opened by a double-click
//! on it in the Assets panel.
//!
//! On the left, a preview sphere the renderer draws with the material, turned by dragging. On the right, the
//! material: its colour, roughness, metallic, emission and glass; a slot per texture channel (base colour, normal,
//! roughness, metallic, emission, ambient occlusion), each a picker of the project's pictures that also takes one
//! dragged from the Assets panel, with a thumbnail; the packed occlusion-roughness-metallic map and its channels; and
//! how the textures are mapped (tile size, scale, offset, filtering, wrapping).
//!
//! Every change is written into the material's file at once and is an undo step of the tab (its Undo and Redo
//! buttons): the preview and every part using the material follow.

use egui::{Align2, ComboBox, CornerRadius, Grid, Rect, RichText, ScrollArea, Sense, Slider, Ui, Vec2, pos2, vec2};
use metallic_engine::metallic_core::{Color3, Widget};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Channel, Filter, Material, Packed, Wrap};

use super::empty_note;
use super::values::{self, Edit, Files};
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::EditorState;
use crate::editor::materials::{PANEL, material_of};
use crate::keymap::Context;
use crate::theme::icons::Icon;
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// The pictures a texture slot takes.
const IMAGES: Widget = Widget::AssetId {
    extensions: &["png", "jpg", "jpeg"],
};

/// The preview's largest side.
const PREVIEW: f32 = 280.0;

/// A slot's thumbnail side.
const THUMB: f32 = 28.0;

/// The material editor.
#[derive(Clone, Copy, Debug)]
pub struct MaterialEditor;

impl Panel for MaterialEditor {
    fn id(&self) -> &'static str {
        PANEL
    }

    fn title(&self) -> &'static str {
        "Material"
    }

    fn icon(&self) -> Icon {
        Icon::AssetMaterial
    }

    fn zone(&self) -> Zone {
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn transient(&self) -> bool {
        true
    }

    fn ui(&self, ui: &mut Ui, _cx: &mut PanelContext<'_>) {
        empty_note(ui, "Double-click a material in the Assets panel to edit it.");
    }

    fn pane_title(&self, pane: &str, cx: &PanelContext<'_>) -> String {
        material_of(pane)
            .and_then(|id| cx.editor.material_title(id))
            .unwrap_or_else(|| self.title().to_owned())
    }

    fn pane_ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>, pane: &str) {
        let Some(id) = material_of(pane).filter(|&id| cx.editor.materials.get(id).is_some()) else {
            empty_note(ui, "This material is no longer in the project.");
            return;
        };
        editor_ui(ui, cx.editor, id);
    }
}

/// The tab of the open material `id`.
fn editor_ui(ui: &mut Ui, editor: &mut EditorState, id: AssetId) {
    let p = current_colors(ui.ctx()).palette;
    let title = editor.material_title(id).unwrap_or_default();
    let (can_undo, can_redo) = editor.materials.can_undo_redo(id);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .font(fonts::named(fonts::STRONG, text::BODY_LG))
                .color(p.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(can_redo, |ui| {
                if button(ui, ButtonKind::Quiet, "Redo").clicked() {
                    editor.undo_material(id, true);
                }
            });
            ui.add_enabled_ui(can_undo, |ui| {
                if button(ui, ButtonKind::Quiet, "Undo").clicked() {
                    editor.undo_material(id, false);
                }
            });
        });
    });
    ui.add_space(space::S3);
    let wide = ui.available_width() > PREVIEW * 2.0;
    let side = if wide {
        PREVIEW.min(ui.available_height())
    } else {
        (ui.available_width() * 0.6).min(PREVIEW)
    }
    .max(64.0);
    let Some(mut material) = editor.materials.get(id).map(|open| open.material) else {
        return;
    };
    let files = Files::of(&editor.project);
    let mut layout = |ui: &mut Ui, editor: &mut EditorState| {
        preview(ui, editor, id, side);
        ui.add_space(space::S5);
        ScrollArea::vertical()
            .id_salt(("material.form", id))
            .auto_shrink([false, false])
            .show(ui, |ui| ui.vertical(|ui| form(ui, editor, &files, &mut material)));
    };
    if wide {
        ui.horizontal_top(|ui| layout(ui, editor));
    } else {
        ui.vertical(|ui| layout(ui, editor));
    }
    if editor.materials.get(id).is_some_and(|open| open.material != material)
        && let Err(error) = editor.edit_material(id, material)
    {
        log::warn!("materials: {error}");
    }
}

/// The preview: the sphere the renderer drew with the material, turned by dragging.
fn preview(ui: &mut Ui, editor: &mut EditorState, id: AssetId, side: f32) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Material preview"));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface);
    if response.dragged()
        && let Some(open) = editor.materials.get_mut(id)
    {
        let delta = response.drag_delta();
        open.yaw += delta.x * 0.01;
        open.pitch = (open.pitch + delta.y * 0.01).clamp(-1.4, 1.4);
    }
    // The renderer draws it after the frame, at this size (app/viewport.rs).
    let pixels = (side * ui.ctx().pixels_per_point()).round() as u32;
    editor.materials.preview = Some((id, (pixels, pixels)));
    match editor.materials.preview_texture {
        Some((shown, texture)) if shown == id => {
            ui.painter().image(
                texture,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        _ => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Drawing\u{2026}",
                egui::FontId::proportional(text::SMALL),
                p.subtle,
            );
        }
    }
}

/// A section's heading.
fn heading(ui: &mut Ui, label: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.add_space(space::S4);
    ui.label(crate::projects::section_label(label, p.subtle));
    ui.add_space(space::S2);
}

/// The material's values, changed in place.
fn form(ui: &mut Ui, editor: &mut EditorState, files: &Files, material: &mut Material) {
    ui.set_max_width(460.0);
    heading(ui, "Surface");
    Grid::new("material.surface")
        .num_columns(2)
        .spacing([space::S5, space::S3])
        .show(ui, |ui| {
            color_row(ui, "Colour", &mut material.color);
            number_row(ui, "Roughness", &mut material.roughness, 0.0..=1.0);
            number_row(ui, "Metallic", &mut material.metallic, 0.0..=1.0);
            color_row(ui, "Emission", &mut material.emission);
            number_row(ui, "Emission strength", &mut material.emission_strength, 0.0..=10.0);
            ui.label("Glass");
            ui.checkbox(&mut material.glass, "")
                .on_hover_text("When the part is see-through, its reflections stay at full strength.");
            ui.end_row();
        });

    heading(ui, "Textures");
    Grid::new("material.textures")
        .num_columns(2)
        .spacing([space::S5, space::S3])
        .show(ui, |ui| {
            let textures = &mut material.textures;
            for (label, slot) in [
                ("Base colour", &mut textures.color),
                ("Normal", &mut textures.normal),
                ("Roughness", &mut textures.roughness),
                ("Metallic", &mut textures.metallic),
                ("Emission", &mut textures.emission),
                ("Ambient occlusion", &mut textures.occlusion),
            ] {
                texture_row(ui, editor, files, label, slot);
            }
            number_row(ui, "Normal strength", &mut material.normal_strength, 0.0..=2.0);
        });

    heading(ui, "Packed map");
    ui.label(
        RichText::new("One picture holding ambient occlusion, roughness and metallic: one sample for three.")
            .size(text::SMALL)
            .color(current_colors(ui.ctx()).palette.subtle),
    );
    Grid::new("material.packed")
        .num_columns(2)
        .spacing([space::S5, space::S3])
        .show(ui, |ui| {
            let mut texture = material.packed.map(|packed| packed.texture);
            texture_row(ui, editor, files, "Packed map", &mut texture);
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

    heading(ui, "Mapping");
    Grid::new("material.mapping")
        .num_columns(2)
        .spacing([space::S5, space::S3])
        .show(ui, |ui| {
            ui.label("Tile size (m)");
            ui.add(
                egui::DragValue::new(&mut material.tile_size)
                    .speed(0.05)
                    .range(0.01..=1000.0),
            );
            ui.end_row();
            pair_row(ui, "Scale", &mut material.uv_scale, 0.01);
            pair_row(ui, "Offset", &mut material.uv_offset, 0.01);
            ui.label("Filtering");
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
            ui.label("Wrapping");
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
    *material = material.clamped();
}

fn number_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.label(label);
    let response = ui.add(Slider::new(value, range).step_by(0.01));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, label));
    ui.end_row();
}

fn pair_row(ui: &mut Ui, label: &str, value: &mut [f32; 2], speed: f64) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut value[0]).speed(speed).prefix("U "));
        ui.add(egui::DragValue::new(&mut value[1]).speed(speed).prefix("V "));
    });
    ui.end_row();
}

/// A colour's row: its swatch, which opens Studio's colour picker.
fn color_row(ui: &mut Ui, label: &str, color: &mut [f32; 3]) {
    ui.label(label);
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
    ui.label(label);
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

/// A texture slot: its thumbnail, and a picker of the project's pictures (None clears it; a picture dragged from the
/// Assets panel onto it is taken).
fn texture_row(ui: &mut Ui, editor: &mut EditorState, files: &Files, label: &str, slot: &mut Option<AssetId>) {
    ui.label(label);
    ui.horizontal(|ui| {
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
        let width = 200.0f32.min(ui.available_width()).max(100.0);
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
