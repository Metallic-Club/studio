//! The Properties panel: the selected instance's properties as the class registry describes them (`PropMeta`), in
//! their groups and order, **read-only** in this slice (editing comes with undo).
//!
//! Values are formatted for reading: vectors as their components, a colour as a swatch and its 0-255 channels, an
//! enum as its item, a reference as the instance's name, a boolean as a check box.
//!
//! Each group's rows are striped (#230): alternately a little darker and a little brighter than the panel, with faint
//! 1 px lines between the rows and between the name and value columns ([`crate::theme::tokens::RowShades`]).

use egui::{
    Align, Color32, CornerRadius, Label, Layout, Rect, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use metallic_engine::metallic_core::{DataModel, Id, PropFlags, PropId, ResolvedProp, Value};

use super::{class_icon, empty_note, ids, playing_banner};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::settings::window::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{current_colors, fonts};

/// The group of properties declared without one.
const DEFAULT_GROUP: &str = "Data";
/// The share of a row's width its name takes.
const NAME_SHARE: f32 = 0.42;

/// The Properties panel.
#[derive(Clone, Copy, Debug)]
pub struct Properties;

/// One property as the panel lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyRow {
    /// The name shown (`PropMeta::label`, or the name).
    pub label: &'static str,
    /// What it does.
    pub description: &'static str,
    /// Its value.
    pub value: Value,
}

/// A group of properties, in the order the panel lists them.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Its name.
    pub name: &'static str,
    /// Its properties, by `PropMeta::order` (ties in declaration order).
    pub rows: Vec<PropertyRow>,
}

/// `id`'s visible properties by group: groups in the order the class's properties first name them (inherited ones
/// first), properties by their order in the group. Hidden properties are left out.
pub fn groups(dm: &DataModel, id: Id) -> Vec<Group> {
    let Ok(class) = dm.class_of(id) else {
        return Vec::new();
    };
    // Each row with its sort key: its order, then its place in the class.
    type Keyed = (i16, usize, PropertyRow);
    let mut groups: Vec<(&'static str, Vec<Keyed>)> = Vec::new();
    for (index, prop) in dm.registry().props(class).iter().enumerate() {
        if !visible(prop) {
            continue;
        }
        let Ok(value) = dm.get(id, PropId::new(index as u16)) else {
            continue;
        };
        let meta = prop.meta;
        let name = if meta.group.is_empty() {
            DEFAULT_GROUP
        } else {
            meta.group
        };
        let row = PropertyRow {
            label: if meta.label.is_empty() { meta.name } else { meta.label },
            description: meta.description,
            value,
        };
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, rows)) => rows.push((meta.order, index, row)),
            None => groups.push((name, vec![(meta.order, index, row)])),
        }
    }
    groups
        .into_iter()
        .map(|(name, mut rows)| {
            rows.sort_by_key(|(order, index, _)| (*order, *index));
            Group {
                name,
                rows: rows.into_iter().map(|(_, _, row)| row).collect(),
            }
        })
        .collect()
}

fn visible(prop: &ResolvedProp) -> bool {
    !prop.hidden && !prop.meta.flags.contains(PropFlags::HIDDEN)
}

/// A number as people read it: at most three decimals, without trailing zeros.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".to_owned()
        } else if value > 0.0 {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        };
    }
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

/// A value as text: what the panel shows beside a swatch or check box, and what a test reads.
pub fn format_value(dm: &DataModel, value: &Value) -> String {
    let vector = |parts: &[f32]| {
        parts
            .iter()
            .map(|part| number(f64::from(*part)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match value {
        Value::Bool(on) => if *on { "true" } else { "false" }.to_owned(),
        Value::Int(n) => n.to_string(),
        Value::Number(n) => number(*n),
        Value::String(s) => s.to_string(),
        Value::Vector2(v) => vector(&[v.x, v.y]),
        Value::Vector3(v) => vector(&[v.x, v.y, v.z]),
        Value::CFrame(frame) => {
            let at = frame.position();
            let turn = frame.to_orientation_deg();
            format!(
                "{}  \u{2022}  {}\u{B0}",
                vector(&[at.x, at.y, at.z]),
                vector(&[turn.x, turn.y, turn.z])
            )
        }
        Value::Color3(color) => {
            let channel = |c: f32| ((c.clamp(0.0, 1.0) * 255.0).round() as u8).to_string();
            format!("{}, {}, {}", channel(color.r), channel(color.g), channel(color.b))
        }
        Value::Enum(item) => item.name().to_owned(),
        Value::Ref(None) => "None".to_owned(),
        Value::Ref(Some(target)) => dm.name(*target).map_or_else(|_| "None".to_owned(), str::to_owned),
    }
}

impl Panel for Properties {
    fn id(&self) -> &'static str {
        ids::PROPERTIES
    }

    fn title(&self) -> &'static str {
        "Properties"
    }

    fn icon(&self) -> Icon {
        Icon::PanelProperties
    }

    fn zone(&self) -> Zone {
        Zone::RightBelow
    }

    fn context(&self) -> Context {
        Context::Properties
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        // The running game's values while one runs, live (ADR 0010 §5).
        let scene = cx.editor.scene();
        if scene.playing {
            playing_banner(ui);
            // The values change as the game runs.
            ui.ctx().request_repaint();
        }
        let dm: &_ = scene.dm;
        let Some(id) = scene.selection.filter(|&id| dm.contains(id)) else {
            empty_note(ui, "Select something in the Explorer to see its properties.");
            return;
        };
        let p = current_colors(ui.ctx()).palette;
        ui.horizontal(|ui| {
            let icon = dm
                .class_of(id)
                .map_or(Icon::ExplorerInstance, |class| class_icon(dm.registry(), class));
            icons::show(ui, icon, text::BODY_LG, icon.color(&current_colors(ui.ctx())));
            ui.label(
                RichText::new(dm.name(id).unwrap_or_default())
                    .font(fonts::named(fonts::HEADING, text::BODY_LG))
                    .color(p.text),
            );
            ui.label(
                RichText::new(dm.class_name(id).unwrap_or_default())
                    .size(text::SMALL)
                    .color(p.subtle),
            );
        });
        ui.add_space(space::S4);
        ScrollArea::vertical()
            .id_salt("properties.rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for group in groups(dm, id) {
                    section_label(ui, group.name);
                    // The rows touch, so the stripes and the lines between them are continuous.
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let shades = p.rows();
                        for (index, row) in group.rows.iter().enumerate() {
                            let last = index + 1 == group.rows.len();
                            ui.push_id(row.label, |ui| row_ui(ui, dm, row, shades.fill(index), last));
                        }
                    });
                    ui.add_space(space::S5);
                }
            });
    }
}

/// One property, a row of a fixed height striped with `fill`: its name on the left and its value on the right, each
/// cut short with an ellipsis (and shown whole on hover) when it does not fit. A faint line sits above the row and
/// between its columns, and under it when it is the group's `last`.
fn row_ui(ui: &mut Ui, dm: &DataModel, row: &PropertyRow, fill: Color32, last: bool) {
    let p = current_colors(ui.ctx()).palette;
    let line = Stroke::new(size::BORDER, p.rows().line);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, size::ROW), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let split = rect.left() + (width * NAME_SHARE).max(80.0).min(width);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    // The lines are drawn inside the row (half a point in), so the next row's fill never covers them.
    let half = size::BORDER / 2.0;
    ui.painter().hline(rect.x_range(), rect.top() + half, line);
    if last {
        ui.painter().hline(rect.x_range(), rect.bottom() - half, line);
    }
    ui.painter().vline(split - half, rect.y_range(), line);

    let cell = |left: f32, right: f32| {
        Rect::from_min_max(
            pos2(left + space::S3, rect.top()),
            pos2(right - space::S3, rect.bottom()),
        )
    };
    let (name_cell, value_cell) = (cell(rect.left(), split), cell(split, rect.right()));
    let in_cell = |cell: Rect| {
        UiBuilder::new()
            .max_rect(cell)
            .layout(Layout::left_to_right(Align::Center))
    };
    ui.scope_builder(in_cell(name_cell), |ui| {
        ui.set_clip_rect(name_cell.intersect(ui.clip_rect()));
        let name = ui.add(
            Label::new(RichText::new(row.label).size(text::BODY).color(p.muted))
                .truncate()
                .selectable(false),
        );
        if !row.description.is_empty() {
            name.on_hover_text(row.description);
        }
    });
    ui.scope_builder(in_cell(value_cell), |ui| {
        ui.set_clip_rect(value_cell.intersect(ui.clip_rect()));
        value_ui(ui, dm, &row.value);
    });
}

/// A property's value, as text after a swatch (a colour) or a check box (a boolean).
fn value_ui(ui: &mut Ui, dm: &DataModel, value: &Value) {
    let p = current_colors(ui.ctx()).palette;
    let shown = format_value(dm, value);
    match value {
        Value::Color3(color) => {
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            let fill = Color32::from_rgb(
                (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
                (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
                (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
            );
            ui.painter().rect(
                rect,
                CornerRadius::same(3),
                fill,
                Stroke::new(size::BORDER, p.border),
                egui::StrokeKind::Inside,
            );
        }
        Value::Bool(on) => {
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(3),
                if *on { p.action } else { p.input },
                Stroke::new(size::BORDER, if *on { p.action } else { p.border }),
                egui::StrokeKind::Inside,
            );
            if *on {
                icons::paint_at(
                    ui.painter(),
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    Icon::UiCheck,
                    text::SMALL - 1.0,
                    p.action_text,
                );
            }
        }
        _ => {}
    }
    let color = match value {
        Value::Ref(None) => p.subtle,
        _ => p.text,
    };
    ui.add(
        Label::new(RichText::new(shown).size(text::BODY).color(color))
            .truncate()
            .selectable(true),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_trailing_zeros() {
        assert_eq!(number(512.0), "512");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(1.0 / 3.0), "0.333");
        assert_eq!(number(-0.0001), "0");
    }

    #[test]
    fn a_parts_properties_are_grouped_and_formatted() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let scene = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d/Scenes/Main.scene"),
        )
        .unwrap();
        metallic_engine::metallic_project::load_scene_bytes(&mut dm, &scene).unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let baseplate = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();

        let groups = groups(&dm, baseplate);
        let names: Vec<&str> = groups.iter().map(|group| group.name).collect();
        assert!(names.contains(&"Data") && names.contains(&"Transform"), "{names:?}");
        let mut seen = std::collections::HashSet::new();
        assert!(
            names.iter().all(|name| seen.insert(*name)),
            "each group once: {names:?}"
        );
        let value = |label: &str| {
            groups
                .iter()
                .flat_map(|group| &group.rows)
                .find(|row| row.label == label)
                .map(|row| format_value(&dm, &row.value))
                .unwrap_or_else(|| panic!("no {label}"))
        };
        assert_eq!(value("Name"), "Baseplate");
        assert_eq!(value("Size"), "512, 16, 512");
        assert_eq!(value("Anchored"), "true");
        assert_eq!(value("Material"), "Concrete");
        assert_eq!(value("Color"), "99, 95, 98");
        assert_eq!(value("Parent"), "Workspace");

        // The workspace's camera is a reference, shown by name.
        let workspace_groups = super::groups(&dm, workspace);
        let camera = workspace_groups
            .iter()
            .flat_map(|group| &group.rows)
            .find(|row| row.label == "CurrentCamera")
            .unwrap();
        assert_eq!(format_value(&dm, &camera.value), "Camera");
    }
}
