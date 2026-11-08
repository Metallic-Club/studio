//! The Properties panel: the selected instance's properties as the class registry describes them (`PropMeta`), in
//! their groups and order, **edited in place** (#349, see [`super::values`]): each change is written through the
//! scene, so it is an undo step. A `CFrame` is two rows, its position and its orientation.
//!
//! What cannot be edited is formatted for reading: references as the instance's name, read-only properties as their
//! value (vectors as their components, a colour as a swatch and its 0-255 channels, a boolean as a check box).
//!
//! With several instances selected (#350) it lists the properties they all have (the same name and type), as Forge's
//! inspector does: a value they disagree on is mixed, and an edit is made to each of them (a vector's component alone
//! on each, so they keep their others), as one undo step. The header names the count ("3 Parts", "4 objects").
//!
//! Each group's rows are striped (#230): alternately a little darker and a little brighter than the panel, with faint
//! 1 px lines between the rows and between the name and value columns ([`crate::theme::tokens::RowShades`]). The line
//! between the columns can be dragged left and right (#324), and stays where it was left.

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Label, Layout, Rect, RichText, Role, ScrollArea, Sense, Stroke, Ui,
    UiBuilder, WidgetInfo, pos2, vec2,
};
use metallic_engine::metallic_core::{DataModel, Id, PropFlags, PropId, PropMeta, PropType, ResolvedProp, Value};

use super::values::{self, Edit, Mixed, Part};
use super::{class_icon, empty_note, ids};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::settings::window::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{current_colors, fonts};

/// The group of properties declared without one.
const DEFAULT_GROUP: &str = "Data";
/// The share of a row's width its name takes, until the line between the columns is dragged.
const NAME_SHARE: f32 = 0.42;
/// The narrowest either column can be dragged to.
const MIN_COLUMN: f32 = 60.0;
/// The width of the strip around the line between the columns that drags it.
const GRIP: f32 = 6.0;

/// Where the line between the columns was left (#324), kept in egui's memory.
fn share_id() -> egui::Id {
    egui::Id::unique("metallic.properties.name_share")
}

/// The share of a row's width the names take: where the line was dragged to, else [`NAME_SHARE`].
pub fn name_share(ctx: &egui::Context) -> f32 {
    ctx.data_mut(|data| *data.get_persisted_mut_or(share_id(), NAME_SHARE))
}

/// Where the line between the columns is in a row from `left`, `width` wide, the names taking `share` of it: each column
/// at least [`MIN_COLUMN`] wide where the row has room for both.
fn split_x(left: f32, width: f32, share: f32) -> f32 {
    let narrowest = MIN_COLUMN.min(width / 2.0);
    left + (width * share).clamp(narrowest, width - narrowest)
}

/// The Properties panel.
#[derive(Clone, Copy, Debug)]
pub struct Properties;

/// One property as the panel lists it (a `CFrame` is two: its position and its orientation).
#[derive(Clone, Debug)]
pub struct PropertyRow {
    /// The name shown (`PropMeta::label`, or the name; a `CFrame`'s rows add Position and Orientation).
    pub label: String,
    /// What it does.
    pub description: &'static str,
    /// Its value: the active instance's.
    pub value: Value,
    /// What the selected instances disagree on.
    pub mixed: Mixed,
    /// The property, as the active instance's class numbers it.
    pub prop: PropId,
    /// Its declaration.
    pub meta: &'static PropMeta,
    /// Which part of the value the row shows.
    pub part: Part,
    /// Whether the panel edits it: not a read-only property, a computed one with no setter, or a reference (in any
    /// of the selected instances' classes).
    pub editable: bool,
}

/// A group of properties, in the order the panel lists them.
#[derive(Clone, Debug)]
pub struct Group {
    /// Its name.
    pub name: &'static str,
    /// Its properties, by `PropMeta::order` (ties in declaration order).
    pub rows: Vec<PropertyRow>,
}

/// The resolved property `name` of `id`'s class, if it has one Studio shows.
fn shown_prop(dm: &DataModel, id: Id, name: &str) -> Option<&'static PropMeta> {
    let class = dm.class_of(id).ok()?;
    let info = dm.registry().class(class)?;
    let prop = info.prop(info.prop_id(name)?)?;
    visible(prop).then_some(prop.meta)
}

/// The visible properties of `ids` (the selection, the active instance last) by group: the active instance's that
/// every other has too, with the same type (#350). Groups come in the order the class's properties first name them
/// (inherited ones first), properties by their order in the group. Hidden properties are left out.
pub fn groups(dm: &DataModel, ids: &[Id]) -> Vec<Group> {
    let Some((&id, others)) = ids.split_last() else {
        return Vec::new();
    };
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
        // The others' values, the active one's first; a property one of them lacks is not shared.
        let mut all = vec![value.clone()];
        let mut shared_editable = editable(meta);
        let shared = others.iter().all(|&other| {
            let Some(theirs) = shown_prop(dm, other, meta.name).filter(|theirs| theirs.ty == meta.ty) else {
                return false;
            };
            shared_editable &= editable(theirs);
            dm.get_by_name(other, meta.name).map(|value| all.push(value)).is_ok()
        });
        if !shared {
            continue;
        }
        let name = if meta.group.is_empty() {
            DEFAULT_GROUP
        } else {
            meta.group
        };
        let label = if meta.label.is_empty() { meta.name } else { meta.label };
        let parts: &[(Part, &str)] = if matches!(value, Value::CFrame(_)) {
            &[(Part::Position, " Position"), (Part::Orientation, " Orientation")]
        } else {
            &[(Part::Whole, "")]
        };
        for &(part, suffix) in parts {
            let row = PropertyRow {
                label: format!("{label}{suffix}"),
                description: meta.description,
                value: value.clone(),
                mixed: Mixed::of(&all, part),
                prop: PropId::new(index as u16),
                meta,
                part,
                editable: shared_editable,
            };
            match groups.iter_mut().find(|(group, _)| *group == name) {
                Some((_, rows)) => rows.push((meta.order, index, row)),
                None => groups.push((name, vec![(meta.order, index, row)])),
            }
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

/// Whether the panel edits a property: not when it is read-only, computed without a setter, or a reference (the
/// Parent changes by dragging in the Explorer).
fn editable(meta: &PropMeta) -> bool {
    !meta.flags.contains(PropFlags::READ_ONLY)
        && meta.computed.is_none_or(|computed| computed.set.is_some())
        && !matches!(meta.ty, PropType::Ref(_))
}

/// What the header calls several selected instances: "3 Parts" when they share a class, else "4 objects".
pub fn count_title(dm: &DataModel, ids: &[Id]) -> String {
    let classes: Vec<&str> = ids.iter().filter_map(|&id| dm.class_name(id).ok()).collect();
    match classes.split_first() {
        Some((first, rest)) if rest.iter().all(|class| class == first) => format!("{} {first}s", ids.len()),
        _ => format!("{} objects", ids.len()),
    }
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
        let mut scene = cx.editor.scene();
        if scene.playing {
            // The values change as the game runs. No banner says so (#309).
            ui.ctx().request_repaint();
        }
        let dm: &DataModel = scene.dm;
        // Every selected instance, the active one last (#350).
        let ids: Vec<Id> = scene
            .selection
            .ids()
            .iter()
            .copied()
            .filter(|&id| dm.contains(id))
            .collect();
        let Some(&id) = ids.last() else {
            empty_note(ui, "Select something in the Explorer to see its properties.");
            return;
        };
        let p = current_colors(ui.ctx()).palette;
        ui.horizontal(|ui| {
            let icon = dm
                .class_of(id)
                .map_or(Icon::ExplorerInstance, |class| class_icon(dm.registry(), class));
            icons::show(ui, icon, text::BODY_LG, icon.color(&current_colors(ui.ctx())));
            let title = match ids.len() {
                1 => dm.name(id).unwrap_or_default().to_owned(),
                _ => count_title(dm, &ids),
            };
            ui.label(
                RichText::new(title)
                    .font(fonts::named(fonts::HEADING, text::BODY_LG))
                    .color(p.text),
            );
            if ids.len() == 1 {
                ui.label(
                    RichText::new(dm.class_name(id).unwrap_or_default())
                        .size(text::SMALL)
                        .color(p.subtle),
                );
            }
        });
        ui.add_space(space::S4);
        let mut edits: Vec<(&'static str, Part, Edit)> = Vec::new();
        ScrollArea::vertical()
            .id_salt("properties.rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let share = name_share(ui.ctx());
                let top = ui.cursor().top();
                for group in groups(dm, &ids) {
                    section_label(ui, group.name);
                    // The rows touch, so the stripes and the lines between them are continuous.
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let shades = p.rows();
                        for (index, row) in group.rows.iter().enumerate() {
                            let last = index + 1 == group.rows.len();
                            let edited = ui
                                .push_id(&row.label, |ui| row_ui(ui, dm, row, shades.fill(index), last, share))
                                .inner;
                            if let Some(edit) = edited {
                                edits.push((row.meta.name, row.part, edit));
                            }
                        }
                    });
                    ui.add_space(space::S5);
                }
                let rows = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
                split_grip(ui, rows, share);
            });
        // To each selected instance's own value, through the scene, so the frame's edits are one undo step (#348); a
        // value a property refuses is logged.
        for (name, part, edit) in edits {
            for &target in &ids {
                let Ok(current) = scene.dm.get_by_name(target, name) else {
                    continue;
                };
                let Some(value) = values::apply(&edit, part, &current) else {
                    continue;
                };
                if let Err(error) = scene.set_by_name(target, name, value) {
                    log::warn!("properties: {error}");
                }
            }
        }
    }
}

/// One property, a row of a fixed height striped with `fill`: its name on the left, cut short with an ellipsis (and
/// shown whole on hover) when it does not fit, and its editor (or, when it cannot be edited, its value) on the right.
/// A faint line sits above the row and between its columns, and under it when it is the group's `last`. The names
/// take `share` of its width. Returns what the user changed.
fn row_ui(ui: &mut Ui, dm: &DataModel, row: &PropertyRow, fill: Color32, last: bool, share: f32) -> Option<Edit> {
    let p = current_colors(ui.ctx()).palette;
    let line = Stroke::new(size::BORDER, p.rows().line);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, size::ROW), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let split = split_x(rect.left(), width, share);
    // The stripe and the lines between rows reach the panel's sides (#301).
    let (band_painter, band) = super::row_band(ui, rect);
    band_painter.rect_filled(band, CornerRadius::ZERO, fill);
    // The lines are drawn inside the row (half a point in), so the next row's fill never covers them.
    let half = size::BORDER / 2.0;
    band_painter.hline(band.x_range(), rect.top() + half, line);
    if last {
        band_painter.hline(band.x_range(), rect.bottom() - half, line);
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
            Label::new(RichText::new(&row.label).size(text::BODY).color(p.muted))
                .truncate()
                .selectable(false),
        );
        if !row.description.is_empty() {
            name.on_hover_text(row.description);
        }
    });
    ui.scope_builder(in_cell(value_cell), |ui| {
        ui.set_clip_rect(value_cell.intersect(ui.clip_rect()));
        if row.editable {
            values::edit(ui, dm, row.meta, &row.label, row.part, &row.value, row.mixed)
        } else if row.mixed.whole {
            ui.label(RichText::new("Mixed").size(text::BODY).color(p.subtle));
            None
        } else {
            value_ui(ui, dm, &values::shown(&row.value, row.part));
            None
        }
    })
    .inner
}

/// The strip over the line between the columns, down every row in `rows` (#324): dragging it moves the line, which
/// follows the pointer and stops short of either side. Under the pointer and while dragged, the line shows in the
/// accent and the pointer as a resize arrow.
fn split_grip(ui: &mut Ui, rows: Rect, share: f32) {
    let width = rows.width();
    if width <= 2.0 * MIN_COLUMN || rows.height() <= 0.0 {
        return;
    }
    let x = split_x(rows.left(), width, share);
    let grip = Rect::from_x_y_ranges(x - GRIP / 2.0..=x + GRIP / 2.0, rows.y_range());
    let response = ui.interact(grip, ui.scope_id().with("metallic.properties.split"), Sense::drag());
    response.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, "Name column width"));
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let moved = split_x(rows.left(), width, (pointer.x - rows.left()) / width);
        let share = (moved - rows.left()) / width;
        ui.data_mut(|data| data.insert_persisted(share_id(), share));
        ui.ctx().request_repaint();
    }
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        let accent = current_colors(ui.ctx()).accent;
        let half = size::BORDER / 2.0;
        ui.painter()
            .vline(x - half, rows.y_range(), Stroke::new(size::BORDER, accent));
    }
}

/// A property's value: a check box alone (a boolean, #378), or text after a swatch (a colour).
fn value_ui(ui: &mut Ui, dm: &DataModel, value: &Value) {
    let p = current_colors(ui.ctx()).palette;
    let shown = format_value(dm, value);
    match value {
        Value::Color3(color) => {
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            let fill = values::swatch_fill(*color);
            ui.painter().rect(
                rect,
                CornerRadius::same(3),
                fill,
                Stroke::new(size::BORDER, p.border),
                egui::StrokeKind::Inside,
            );
        }
        &Value::Bool(on) => {
            let (rect, response) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            response.widget_info(|| WidgetInfo::selected(Role::CheckBox, false, on, &shown));
            ui.painter().rect(
                rect,
                CornerRadius::same(3),
                if on { p.action } else { p.input },
                Stroke::new(size::BORDER, if on { p.action } else { p.border }),
                egui::StrokeKind::Inside,
            );
            if on {
                icons::paint_at(
                    ui.painter(),
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    Icon::UiCheck,
                    text::SMALL - 1.0,
                    p.action_text,
                );
            }
            return;
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

        let groups = groups(&dm, &[baseplate]);
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

        // With another part: only what both have, marked where they disagree; with a folder, only what every
        // instance has.
        let spawn = dm.find_first_child(workspace, "SpawnLocation", false).unwrap().unwrap();
        let both = super::groups(&dm, &[spawn, baseplate]);
        let row = |groups: &[Group], label: &str| {
            groups
                .iter()
                .flat_map(|group| &group.rows)
                .find(|row| row.label == label)
                .cloned()
        };
        assert!(row(&both, "Size").unwrap().mixed.whole);
        assert!(!row(&both, "Anchored").unwrap().mixed.whole);
        assert_eq!(count_title(&dm, &[spawn, baseplate]), "2 Parts");
        let folder = dm.registry().class_id("Folder").unwrap();
        let folder = dm.create(folder).unwrap();
        let mixed = super::groups(&dm, &[folder, baseplate]);
        assert!(row(&mixed, "Name").is_some_and(|row| row.mixed.whole));
        assert!(row(&mixed, "Size").is_none(), "a folder has no size");
        assert_eq!(count_title(&dm, &[folder, baseplate]), "2 objects");

        // The workspace's camera is a reference, shown by name.
        let workspace_groups = super::groups(&dm, &[workspace]);
        let camera = workspace_groups
            .iter()
            .flat_map(|group| &group.rows)
            .find(|row| row.label == "CurrentCamera")
            .unwrap();
        assert_eq!(format_value(&dm, &camera.value), "Camera");
    }
}
