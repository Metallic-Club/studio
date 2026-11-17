//! The material editor (#680, ADR 0012), in the Properties panel when a material is selected in the Assets browser
//! (#742, #779): its settings in categories, in the panel's rows and with its value inputs.
//!
//! - **Texture Maps** (#762): Color, Normal, Roughness, Metalness, Emissive and Ambient occlusion, in that order. Each
//!   is a picture field ([`values::picture_field`]): a small preview, the picture's name (its path while it is
//!   edited, with completions), and a button that opens a window to choose one, as a list or a grid. A picture
//!   dragged from the Assets panel is taken too.
//! - **Surface**: the tint and Glass, how strongly the normal map bends the light, the roughness and metalness, and
//!   the glow's colour and strength, as numbers and colours are typed and dragged in the panel.
//! - **Packed Map**: one picture holding ambient occlusion, roughness and metallic, and which channel holds each.
//! - **Mapping**: how the textures are laid on a part (tile size, scale, offset, filtering, wrapping).
//!
//! Every change is written into the material's file at once and is one of its undo steps.

use egui::Ui;
use metallic_engine::metallic_core::glam::Vec2;
use metallic_engine::metallic_core::{Color3, DataModel, PropMeta, Value};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Channel, Filter, Material, Packed, Wrap};

use super::properties::{Mark, category_heading, plain_row};
use super::values::{self, Edit, Files, Mixed, Part};
use crate::editor::EditorState;
use crate::theme::current_colors;
use crate::theme::tokens::space;

/// Which normal maps the renderer reads (#773), said beside the Normal map.
pub(crate) const NORMAL_CONVENTION: &str = "Normal maps use the OpenGL convention: green points up the picture, as \
                                            in glTF and Roblox. For a DirectX map, invert its green channel.";

/// A packed map's channels, as the drop-downs name them.
const CHANNELS: [(Option<Channel>, &str); 5] = [
    (None, "None"),
    (Some(Channel::R), "Red"),
    (Some(Channel::G), "Green"),
    (Some(Channel::B), "Blue"),
    (Some(Channel::A), "Alpha"),
];

/// One setting's row, and the part of the material it changes.
enum Field<'a> {
    /// A picture (#762): its row's name, its field's label for screen readers, its name's tooltip, and the slot.
    Picture(&'static str, &'static str, &'static str, &'a mut Option<AssetId>),
    /// A number: what it is (its name and range) and the value.
    Number(PropMeta, &'a mut f32),
    /// A colour, with its swatch.
    Color(PropMeta, &'a mut [f32; 3]),
    /// A check box: what it is, its name's tooltip, and the value.
    Check(PropMeta, &'static str, &'a mut bool),
    /// Two numbers (U and V).
    Pair(PropMeta, &'a mut [f32; 2]),
    /// A choice among `items`: its name, the items, which is chosen, and where the choice goes.
    Choice(&'static str, &'static [&'static str], usize, Box<dyn FnMut(usize) + 'a>),
}

/// The material's settings, changed in place, in categories. `dm` names nothing here but is what the panel's inputs
/// read enums from; `files` has the pictures' thumbnails.
pub(crate) fn categories(ui: &mut Ui, dm: &DataModel, files: &Files, material: &mut Material) {
    let textures = &mut material.textures;
    category(
        ui,
        dm,
        files,
        "Texture Maps",
        vec![
            Field::Picture("Color", "Color map", "", &mut textures.color),
            Field::Picture("Normal", "Normal map", NORMAL_CONVENTION, &mut textures.normal),
            Field::Picture("Roughness", "Roughness map", "", &mut textures.roughness),
            Field::Picture("Metalness", "Metalness map", "", &mut textures.metallic),
            Field::Picture("Emissive", "Emissive map", "", &mut textures.emission),
            Field::Picture(
                "Ambient occlusion",
                "Ambient occlusion map",
                "",
                &mut textures.occlusion,
            ),
        ],
    );
    category(
        ui,
        dm,
        files,
        "Surface",
        vec![
            Field::Color(PropMeta::color3("Tint", 1.0, 1.0, 1.0), &mut material.color),
            Field::Check(
                PropMeta::boolean("Glass", false),
                "When the part is see-through, its reflections stay at full strength.",
                &mut material.glass,
            ),
            Field::Number(
                PropMeta::number("Normal strength", 1.0).range(0.0, 2.0),
                &mut material.normal_strength,
            ),
            Field::Number(
                PropMeta::number("Roughness", 0.5).range(0.0, 1.0),
                &mut material.roughness,
            ),
            Field::Number(
                PropMeta::number("Metalness", 0.0).range(0.0, 1.0),
                &mut material.metallic,
            ),
            Field::Color(PropMeta::color3("Emissive", 0.0, 0.0, 0.0), &mut material.emission),
            Field::Number(
                PropMeta::number("Emissive strength", 0.0).range(0.0, 10.0),
                &mut material.emission_strength,
            ),
        ],
    );
    packed_category(ui, dm, files, material);
    let filter = match material.filter {
        Filter::Linear => 0,
        Filter::Nearest => 1,
    };
    let wrap = match material.wrap {
        Wrap::Repeat => 0,
        Wrap::Mirror => 1,
        Wrap::Clamp => 2,
    };
    category(
        ui,
        dm,
        files,
        "Mapping",
        vec![
            Field::Number(
                PropMeta::number("Tile size", 4.0).range(0.01, 1000.0),
                &mut material.tile_size,
            ),
            Field::Pair(PropMeta::vector2("Scale", 1.0, 1.0), &mut material.uv_scale),
            Field::Pair(PropMeta::vector2("Offset", 0.0, 0.0), &mut material.uv_offset),
            Field::Choice(
                "Filtering",
                &["Smooth", "Nearest"],
                filter,
                Box::new(|index| {
                    material.filter = if index == 1 { Filter::Nearest } else { Filter::Linear };
                }),
            ),
            Field::Choice(
                "Wrapping",
                &["Repeat", "Mirror", "Clamp"],
                wrap,
                Box::new(|index| {
                    material.wrap = match index {
                        1 => Wrap::Mirror,
                        2 => Wrap::Clamp,
                        _ => Wrap::Repeat,
                    };
                }),
            ),
        ],
    );
    *material = material.clamped();
}

/// The packed map: one picture for three channels, and which of its channels holds each.
fn packed_category(ui: &mut Ui, dm: &DataModel, files: &Files, material: &mut Material) {
    let mut texture = material.packed.map(|packed| packed.texture);
    let mut fields = vec![Field::Picture(
        "Texture",
        "Packed map",
        "One picture holding ambient occlusion, roughness and metallic: one sample for three.",
        &mut texture,
    )];
    let mut packed = material.packed;
    if let Some(packed) = &mut packed {
        let index = |channel: Option<Channel>| CHANNELS.iter().position(|(c, _)| *c == channel).unwrap_or(0);
        let names: &'static [&'static str] = &["None", "Red", "Green", "Blue", "Alpha"];
        let (occlusion, roughness, metallic) =
            (index(packed.occlusion), index(packed.roughness), index(packed.metallic));
        let Packed {
            occlusion: o,
            roughness: r,
            metallic: m,
            ..
        } = packed;
        fields.push(Field::Choice(
            "Occlusion in",
            names,
            occlusion,
            Box::new(|i| *o = CHANNELS[i].0),
        ));
        fields.push(Field::Choice(
            "Roughness in",
            names,
            roughness,
            Box::new(|i| *r = CHANNELS[i].0),
        ));
        fields.push(Field::Choice(
            "Metallic in",
            names,
            metallic,
            Box::new(|i| *m = CHANNELS[i].0),
        ));
    }
    category(ui, dm, files, "Packed Map", fields);
    material.packed = match (texture, packed) {
        (None, _) => None,
        (Some(texture), Some(packed)) => Some(Packed { texture, ..packed }),
        (Some(texture), None) => Some(Packed::new(texture)),
    };
}

/// A category: its heading, which collapses it (#778), then a row for each of its `fields`.
fn category(ui: &mut Ui, dm: &DataModel, files: &Files, name: &str, fields: Vec<Field<'_>>) {
    if category_heading(ui, name, 0.0) {
        let shades = current_colors(ui.ctx()).palette.rows();
        let count = fields.len();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, field) in fields.into_iter().enumerate() {
                field_row(ui, dm, files, field, shades.fill(index), index + 1 == count);
            }
        });
    }
    ui.add_space(space::S5);
}

/// One field's row, with the Properties panel's input for its kind; what it changes goes into the material.
fn field_row(ui: &mut Ui, dm: &DataModel, files: &Files, field: Field<'_>, fill: egui::Color32, last: bool) {
    // The panel's input for `value`, as `meta` describes it; the new value when it changed.
    let edit = |ui: &mut Ui, meta: &PropMeta, value: &Value| {
        values::edit(ui, dm, files, meta, meta.name, Part::Whole, value, Mixed::default())
    };
    let changed = |edit: Option<Edit>, value: &Value| edit.and_then(|edit| values::apply(&edit, Part::Whole, value));
    match field {
        Field::Picture(name, label, tip, slot) => {
            let current = slot.map(|id| id.to_string()).unwrap_or_default();
            // Its preview at the right of the name, as a colour's swatch (#780): a click opens the chooser.
            let thumbnail = files.path_of(&current).and_then(|path| {
                values::want_picture(ui.ctx(), path);
                files.picture(path)
            });
            let (chosen, marked) = plain_row(ui, name, tip, fill, last, Mark::Picture(thumbnail, label), |ui| {
                values::picture_field(ui, files, label, &current, ui.available_width())
            });
            if marked.clicked {
                values::open_picture_window(ui.ctx(), label);
            }
            if let Some(chosen) = chosen.flatten() {
                *slot = chosen.parse().ok();
            }
        }
        Field::Number(meta, value) => {
            let current = Value::Number(f64::from(*value));
            let (typed, _) = plain_row(ui, meta.name, "", fill, last, Mark::None, |ui| {
                edit(ui, &meta, &current)
            });
            if let Some(Value::Number(n)) = changed(typed.flatten(), &current) {
                *value = n as f32;
            }
        }
        Field::Color(meta, value) => {
            let color = Color3::new(value[0], value[1], value[2]);
            let current = Value::Color3(color);
            let (typed, marked) = plain_row(ui, meta.name, "", fill, last, Mark::Color(color, meta.name), |ui| {
                edit(ui, &meta, &current)
            });
            if let Some(Value::Color3(c)) = changed(typed.flatten().or(marked.picked), &current) {
                *value = [c.r, c.g, c.b];
            }
        }
        Field::Check(meta, tip, value) => {
            let current = Value::Bool(*value);
            let (typed, _) = plain_row(ui, meta.name, tip, fill, last, Mark::None, |ui| {
                edit(ui, &meta, &current)
            });
            if let Some(Value::Bool(on)) = changed(typed.flatten(), &current) {
                *value = on;
            }
        }
        Field::Pair(meta, value) => {
            let current = Value::Vector2(Vec2::new(value[0], value[1]));
            let (typed, _) = plain_row(ui, meta.name, "", fill, last, Mark::None, |ui| {
                edit(ui, &meta, &current)
            });
            if let Some(Value::Vector2(v)) = changed(typed.flatten(), &current) {
                *value = [v.x, v.y];
            }
        }
        Field::Choice(name, items, current, mut set) => {
            let (chosen, _) = plain_row(ui, name, "", fill, last, Mark::None, |ui| {
                values::choice_field(ui, name, items, current, ui.available_width())
            });
            if let Some(index) = chosen.flatten() {
                set(index);
            }
        }
    }
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
