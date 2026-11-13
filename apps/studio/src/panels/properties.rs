//! The Properties panel: the selected instance's properties as the class registry describes them (`PropMeta`), in
//! their groups and order, **edited in place** (#349, see [`super::values`]): each change is written through the
//! scene, so it is an undo step.
//!
//! **Rows open in place** (#417): a vector opens into its X, Y and Z, a colour into its R, G and B, each on a row of its
//! own that edits that part alone; a `CFrame` (`CFrame`, `PivotOffset`) shows whole on one row and opens into its
//! Position and Rotation (its orientation, #455), which open in turn. What is open is kept while Studio runs.
//!
//! What cannot be edited is formatted for reading: references as the instance's name, read-only properties as their
//! value (vectors as their components, a colour as a swatch and its 0-255 channels, a boolean as a check box).
//!
//! With several instances selected (#350) it lists the properties they all have (the same name and type), as Forge's
//! inspector does: a value they disagree on is mixed, and an edit is made to each of them (a vector's component alone
//! on each, so they keep their others), as one undo step. The header names the count ("3 Parts", "4 objects").
//!
//! **Attributes** (#591): with one instance selected, its attributes follow its properties, by name, edited as
//! properties are (a vector or a colour opens into its parts). The Add attribute button asks for a name and a type; a
//! right click on an attribute's name renames or deletes it. Every change goes through the scene, so it is an undo step
//! and an unsaved change, and scenes save attributes.
//!
//! Each group's rows are striped (#230): alternately a little darker and a little brighter than the panel, with faint
//! 1 px lines between the rows and between the name and value columns ([`crate::theme::tokens::RowShades`]). The line
//! between the columns can be dragged left and right (#324), and stays where it was left.

use std::collections::HashSet;

use egui::{
    Align, Button, Color32, CornerRadius, CursorIcon, Key, Label, Layout, Popup, PopupCloseBehavior, Rect, RichText,
    Role, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};
use metallic_engine::metallic_core::{
    Color3, DataModel, Id, PropFlags, PropId, PropMeta, PropType, ResolvedProp, Value, Vector2, Vector3,
};

use super::values::{self, Edit, Mixed, Part};
use super::{class_icon, empty_note, ids};
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::Scene;
use crate::keymap::Context;
use crate::settings::window::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};
use crate::theme::{ButtonKind, current_colors, fonts};

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

/// One row of the panel: a property, or a part of one (#417: a vector's X, a `CFrame`'s Position).
#[derive(Clone, Debug)]
pub struct PropertyRow {
    /// Its full name, which screen readers hear and which names its controls (`PropMeta::label`, or the name, then
    /// its parts: `CFrame Position X`).
    pub label: String,
    /// The name shown: the property's, or the part's alone (`X`) on a part's row.
    pub shown: String,
    /// Where it is in the tree of rows (`CFrame/Position/X`), which open state is kept by.
    pub key: String,
    /// The row it opens from, if it is a part.
    pub parent: Option<String>,
    /// How deep it is: 0 for a property, 1 for its parts, 2 for theirs.
    pub depth: u8,
    /// Whether it opens into rows of its parts.
    pub opens: bool,
    /// The component it edits alone (0, 1, 2), on a part's row.
    pub component: Option<usize>,
    /// What it does.
    pub description: &'static str,
    /// Its value: the active instance's.
    pub value: Value,
    /// What the selected instances disagree on.
    pub mixed: Mixed,
    /// What it edits.
    pub source: Source,
    /// Its declaration.
    pub meta: &'static PropMeta,
    /// Which part of the value the row shows.
    pub part: Part,
    /// Whether the panel edits it: not a read-only property, a computed one with no setter, or a reference (in any
    /// of the selected instances' classes).
    pub editable: bool,
}

/// What a row edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A property, as the active instance's class numbers it.
    Property(PropId),
    /// The active instance's attribute of this name (#591).
    Attribute(String),
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
        let mut all = vec![value];
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
        let source = Source::Property(PropId::new(index as u16));
        let rows = value_rows(label, label, meta, &all, &source, shared_editable);
        let keyed = rows.into_iter().map(|row| (meta.order, index, row));
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, list)) => list.extend(keyed),
            None => groups.push((name, keyed.collect())),
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

/// The rows of a value shown as `label` (#417): its own, then its parts' (a vector's axes, a colour's channels, a
/// `CFrame`'s position and rotation, and theirs), keyed by their path from `key`. `all` holds each selected instance's
/// value, the active one's first.
fn value_rows(
    label: &str,
    key: &str,
    meta: &'static PropMeta,
    all: &[Value],
    source: &Source,
    editable: bool,
) -> Vec<PropertyRow> {
    let Some(value) = all.first() else {
        return Vec::new();
    };
    let row = |shown: &str, path: &[&str], part: Part, component: Option<usize>, opens: bool| {
        let full: Vec<&str> = std::iter::once(label).chain(path.iter().copied()).collect();
        let keys: Vec<&str> = std::iter::once(key).chain(path.iter().copied()).collect();
        let parent = (!path.is_empty()).then(|| keys[..keys.len() - 1].join("/"));
        let mixed = Mixed::of(all, part);
        PropertyRow {
            label: full.join(" "),
            shown: shown.to_owned(),
            key: keys.join("/"),
            parent,
            depth: path.len() as u8,
            opens,
            component,
            description: meta.description,
            value: value.clone(),
            mixed: match component {
                // A part's row is mixed where its component is.
                Some(i) => Mixed {
                    whole: mixed.parts[i],
                    parts: [mixed.parts[i]; 3],
                },
                None => mixed,
            },
            source: source.clone(),
            meta,
            part,
            editable,
        }
    };
    let mut rows = Vec::new();
    let axes: &[&str] = match value {
        Value::Vector2(_) => &["X", "Y"],
        Value::Color3(_) => &["R", "G", "B"],
        _ => &["X", "Y", "Z"],
    };
    match value {
        Value::CFrame(_) => {
            rows.push(row(label, &[], Part::Whole, None, true));
            for (part, name) in [(Part::Position, "Position"), (Part::Orientation, "Rotation")] {
                rows.push(row(name, &[name], part, None, true));
                for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                    rows.push(row(axis, &[name, axis], part, Some(i), false));
                }
            }
        }
        Value::Vector3(_) | Value::Vector2(_) | Value::Color3(_) => {
            rows.push(row(label, &[], Part::Whole, None, true));
            for (i, &axis) in axes.iter().enumerate() {
                rows.push(row(axis, &[axis], Part::Whole, Some(i), false));
            }
        }
        _ => rows.push(row(label, &[], Part::Whole, None, false)),
    }
    rows
}

/// What an attribute's row is declared as (#591): attributes have no declaration, and the editors only read a range,
/// a speed and an enum's type from one, none of which an attribute has.
static ATTRIBUTE: PropMeta = PropMeta::string("Attribute", "");

/// The rows of `id`'s attributes (#591), by name. One that holds an enum item or an instance is shown, not edited:
/// the editors need a declared enum type, and Studio does not make either.
pub fn attribute_rows(dm: &DataModel, id: Id) -> Vec<PropertyRow> {
    let Ok(attributes) = dm.attributes(id) else {
        return Vec::new();
    };
    let mut named: Vec<(&str, &Value)> = attributes.iter().map(|(name, value)| (&**name, value)).collect();
    named.sort_by(|a, b| a.0.cmp(b.0));
    named
        .into_iter()
        .flat_map(|(name, value)| {
            let editable = !matches!(value, Value::Enum(_) | Value::Ref(_));
            let source = Source::Attribute(name.to_owned());
            value_rows(
                name,
                &format!("@{name}"),
                &ATTRIBUTE,
                std::slice::from_ref(value),
                &source,
                editable,
            )
        })
        .collect()
}

/// The types a new attribute can have (#591), as Roblox names them, in its order.
pub const ATTRIBUTE_TYPES: [&str; 6] = ["string", "boolean", "number", "Color3", "Vector2", "Vector3"];

/// The value a new attribute of `kind` (one of [`ATTRIBUTE_TYPES`]) starts with.
fn starting_value(kind: &str) -> Value {
    match kind {
        "boolean" => Value::Bool(false),
        "number" => Value::Number(0.0),
        "Color3" => Value::Color3(Color3::new(0.0, 0.0, 0.0)),
        "Vector2" => Value::Vector2(Vector2::new(0.0, 0.0)),
        "Vector3" => Value::Vector3(Vector3::new(0.0, 0.0, 0.0)),
        _ => Value::String("".into()),
    }
}

/// The longest an attribute's name can be, as in Roblox.
const NAME_LIMIT: usize = 100;

/// Why `name` cannot name an attribute beside those `taken` (#591), or `None` when it can: as in Roblox, up to 100
/// letters, digits and underscores.
pub fn name_problem(name: &str, taken: &[String]) -> Option<&'static str> {
    if name.is_empty() {
        Some("Give it a name.")
    } else if name.chars().count() > NAME_LIMIT {
        Some("At most 100 characters.")
    } else if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some("Letters, digits and _ only.")
    } else if taken.iter().any(|other| other == name) {
        Some("It has one by that name.")
    } else {
        None
    }
}

/// A change to the active instance's attributes other than a value (#591).
#[derive(Clone, Debug, PartialEq)]
enum AttributeAction {
    /// Add one of this name with this value.
    Add(String, Value),
    /// Give one another name, keeping its value.
    Rename {
        /// Its name.
        from: String,
        /// The new one.
        to: String,
    },
    /// Remove one.
    Delete(String),
}

/// Makes `action` to `id`'s attributes through `scene`, so it is an undo step; a rename is two changes in one step.
fn apply_attribute_action(scene: &mut Scene<'_>, id: Id, action: AttributeAction) {
    let done = match action {
        AttributeAction::Add(name, value) => scene.set_attribute(id, &name, Some(value)),
        AttributeAction::Rename { from, to } => {
            let value = scene.dm.attribute(id, &from).ok().flatten().cloned();
            scene
                .set_attribute(id, &from, None)
                .and_then(|()| scene.set_attribute(id, &to, value))
        }
        AttributeAction::Delete(name) => scene.set_attribute(id, &name, None),
    };
    if let Err(error) = done {
        log::warn!("properties: {error}");
    }
}

/// How wide the fields of the attribute menus are.
const MENU_FIELD: f32 = 180.0;

/// A line of `note` under a field saying what is wrong with it.
fn field_note(ui: &mut Ui, note: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.label(RichText::new(note).size(text::SMALL).color(p.subtle));
}

/// The menu a right click on attribute `name`'s row opens (#591): a field that renames it, and Delete. It closes on a
/// click outside it, not on one in its field.
fn attribute_menu(response: &egui::Response, name: &str, taken: &[String]) -> Option<AttributeAction> {
    let field = response.id.with("rename");
    let mut action = None;
    let shown = Popup::context_menu(response)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut typed = ui
                .data(|data| data.get_temp::<String>(field))
                .unwrap_or_else(|| name.to_owned());
            let others: Vec<String> = taken.iter().filter(|other| *other != name).cloned().collect();
            let edit = ui.add(
                TextEdit::singleline(&mut typed)
                    .id(field.with("field"))
                    .desired_width(MENU_FIELD),
            );
            edit.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Attribute name"));
            let problem = name_problem(&typed, &others);
            if let Some(problem) = problem {
                field_note(ui, problem);
            }
            let ready = problem.is_none() && typed != name;
            let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let rename = ui.add_enabled(ready, Button::new("Rename"));
            rename.widget_info(|| WidgetInfo::labeled(Role::Button, ready, "Rename attribute"));
            if ready && (rename.clicked() || entered) {
                action = Some(AttributeAction::Rename {
                    from: name.to_owned(),
                    to: typed.clone(),
                });
                ui.close();
            }
            ui.separator();
            let delete = ui.button("Delete");
            delete.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Delete attribute"));
            if delete.clicked() {
                action = Some(AttributeAction::Delete(name.to_owned()));
                ui.close();
            }
            ui.data_mut(|data| data.insert_temp(field, typed));
        });
    // What was typed is forgotten once the menu closes: it opens on the name again.
    if shown.is_none() || action.is_some() {
        response.ctx.data_mut(|data| data.remove::<String>(field));
    }
    action
}

/// The Add attribute button under the attributes (#591), and the form it opens: a name, a type and Add. Returns the
/// attribute to add when one is asked for.
fn add_attribute(ui: &mut Ui, taken: &[String]) -> Option<AttributeAction> {
    let id = egui::Id::unique("metallic.properties.add_attribute");
    let button = crate::theme::button(ui, ButtonKind::Secondary, "Add attribute");
    let mut action = None;
    let shown = Popup::menu(&button)
        .id(id.with("menu"))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let (mut name, mut kind) = ui.data(|data| data.get_temp::<(String, usize)>(id)).unwrap_or_default();
            let field = ui.add(
                TextEdit::singleline(&mut name)
                    .id(id.with("name"))
                    .hint_text("Name")
                    .desired_width(MENU_FIELD),
            );
            field.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "New attribute name"));
            let problem = name_problem(&name, taken);
            if let Some(problem) = problem.filter(|_| !name.is_empty()) {
                field_note(ui, problem);
            }
            // The types as buttons, one of them chosen: a drop-down would be a menu in a menu.
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(MENU_FIELD);
                for (index, &label) in ATTRIBUTE_TYPES.iter().enumerate() {
                    if ui.selectable_label(kind == index, label).clicked() {
                        kind = index;
                    }
                }
            });
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let add = ui.add_enabled(problem.is_none(), Button::new("Add"));
            add.widget_info(|| WidgetInfo::labeled(Role::Button, problem.is_none(), "Add the attribute"));
            if problem.is_none() && (add.clicked() || entered) {
                let kind_name = ATTRIBUTE_TYPES.get(kind).copied().unwrap_or("string");
                action = Some(AttributeAction::Add(name.clone(), starting_value(kind_name)));
                name.clear();
                ui.close();
            }
            ui.data_mut(|data| data.insert_temp(id, (name, kind)));
        });
    if shown.is_none() {
        ui.data_mut(|data| data.remove::<(String, usize)>(id));
    }
    action
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
        let mut edits: Vec<RowEdit> = Vec::new();
        let mut attribute_action = None;
        // Attributes are the active instance's, listed when it is the only one selected (#591).
        let attributes = (ids.len() == 1).then(|| attribute_rows(dm, id));
        let taken: Vec<String> = dm
            .attributes(id)
            .map(|list| list.iter().map(|(name, _)| name.to_string()).collect())
            .unwrap_or_default();
        ScrollArea::vertical()
            .id_salt("properties.rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let share = name_share(ui.ctx());
                let top = ui.cursor().top();
                let mut open = open_rows(ui.ctx());
                for group in groups(dm, &ids) {
                    section_label(ui, group.name);
                    edits.extend(rows_ui(ui, dm, &group.rows, &mut open, share, &taken).edits);
                    ui.add_space(space::S5);
                }
                let mut bottom = ui.cursor().top();
                if let Some(rows) = &attributes {
                    section_label(ui, "Attributes");
                    let drawn = rows_ui(ui, dm, rows, &mut open, share, &taken);
                    edits.extend(drawn.edits);
                    attribute_action = drawn.action;
                    bottom = ui.cursor().top();
                    if !rows.is_empty() {
                        ui.add_space(space::S3);
                    }
                    if let Some(added) = add_attribute(ui, &taken) {
                        attribute_action = Some(added);
                    }
                    ui.add_space(space::S5);
                }
                ui.data_mut(|data| data.insert_temp(open_id(), open));
                // The line between the columns is dragged anywhere down the rows, not over the button under them.
                let rows = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=bottom);
                split_grip(ui, rows, share);
            });
        // To each selected instance's own value (an attribute, to the active instance's), through the scene, so the
        // frame's edits are one undo step (#348); a value a property refuses is logged.
        for (source, name, part, edit) in edits {
            match source {
                Source::Property(_) => {
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
                Source::Attribute(attribute) => {
                    let Some(current) = scene.dm.attribute(id, &attribute).ok().flatten().cloned() else {
                        continue;
                    };
                    let Some(value) = values::apply(&edit, part, &current) else {
                        continue;
                    };
                    if let Err(error) = scene.set_attribute(id, &attribute, Some(value)) {
                        log::warn!("properties: {error}");
                    }
                }
            }
        }
        if let Some(action) = attribute_action {
            apply_attribute_action(&mut scene, id, action);
        }
    }
}

/// Which rows are open (#417), by key, kept while Studio runs.
fn open_id() -> egui::Id {
    egui::Id::unique("metallic.properties.open")
}

/// The rows open now.
fn open_rows(ctx: &egui::Context) -> std::collections::HashSet<String> {
    ctx.data(|data| data.get_temp(open_id())).unwrap_or_default()
}

/// A row's edit: what it edits, the property's name (an attribute's is in its source), the part and the change.
type RowEdit = (Source, &'static str, Part, Edit);

/// What a group's rows did this frame.
#[derive(Default)]
struct Drawn {
    /// Their edits.
    edits: Vec<RowEdit>,
    /// What an attribute's menu asked for.
    action: Option<AttributeAction>,
}

/// Draws `rows` (a group's) touching and striped, a part's row only while every row it opens from is `open` (#417),
/// and opens or closes what was clicked. `taken` are the active instance's attribute names, which a rename checks.
fn rows_ui(
    ui: &mut Ui,
    dm: &DataModel,
    rows: &[PropertyRow],
    open: &mut HashSet<String>,
    share: f32,
    taken: &[String],
) -> Drawn {
    let p = current_colors(ui.ctx()).palette;
    let shown: Vec<&PropertyRow> = rows
        .iter()
        .filter(|row| {
            let mut at = row.parent.as_deref();
            while let Some(parent) = at {
                if !open.contains(parent) {
                    return false;
                }
                at = parent.rsplit_once('/').map(|(up, _)| up);
            }
            true
        })
        .collect();
    let mut drawn = Drawn::default();
    // The rows touch, so the stripes and the lines between them are continuous.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let shades = p.rows();
        for (index, row) in shown.iter().enumerate() {
            let last = index + 1 == shown.len();
            let is_open = row.opens.then(|| open.contains(&row.key));
            let out = ui
                .push_id(&row.key, |ui| {
                    row_ui(ui, dm, row, shades.fill(index), last, share, is_open, taken)
                })
                .inner;
            if let Some(edit) = out.edit {
                drawn.edits.push((row.source.clone(), row.meta.name, row.part, edit));
            }
            if out.toggled && !open.remove(&row.key) {
                open.insert(row.key.clone());
            }
            if out.action.is_some() {
                drawn.action = out.action;
            }
        }
    });
    drawn
}

/// What a row did this frame.
struct RowOut {
    /// What the user changed.
    edit: Option<Edit>,
    /// Its arrow was clicked: open or close it.
    toggled: bool,
    /// What its menu asked for (an attribute's, #591).
    action: Option<AttributeAction>,
}

/// How far a part's row is indented per level.
const INDENT: f32 = 14.0;

/// One row of a fixed height striped with `fill`: its name on the left (indented by its depth, after an arrow that
/// opens or closes it when `open` says it opens, #417), cut short with an ellipsis (and shown whole on hover) when it
/// does not fit, and its editor (or, when it cannot be edited, its value) on the right. A faint line sits above the
/// row and between its columns, and under it when it is the group's `last`. The names take `share` of its width. An
/// attribute's name opens its menu on a right click (#591); `taken` are the names a rename cannot take.
#[allow(clippy::too_many_arguments)]
fn row_ui(
    ui: &mut Ui,
    dm: &DataModel,
    row: &PropertyRow,
    fill: Color32,
    last: bool,
    share: f32,
    open: Option<bool>,
    taken: &[String],
) -> RowOut {
    let mut out = RowOut {
        edit: None,
        toggled: false,
        action: None,
    };
    let p = current_colors(ui.ctx()).palette;
    let line = Stroke::new(size::BORDER, p.rows().line);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, size::ROW), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return out;
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
    let (mut name_cell, value_cell) = (cell(rect.left(), split), cell(split, rect.right()));
    // Indented by its depth, after its arrow when it opens (#417).
    name_cell.min.x += f32::from(row.depth) * INDENT;
    if let Some(is_open) = open {
        let arrow = Rect::from_min_size(
            pos2(name_cell.left() - space::S1, rect.center().y - INDENT / 2.0),
            vec2(INDENT, INDENT),
        );
        let response = ui.interact(arrow, ui.scope_id().with("open"), Sense::click());
        let verb = if is_open { "Close" } else { "Open" };
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{verb} {}", row.label)));
        let icon = if is_open { Icon::UiCollapse } else { Icon::UiExpand };
        let ink = if response.hovered() { p.text } else { p.subtle };
        icons::paint_at(
            ui.painter(),
            arrow.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            text::SMALL,
            ink,
        );
        out.toggled = response.clicked();
    }
    name_cell.min.x += INDENT;
    // A colour's swatch (#415): a small square at the right of the name column, inside the row's lines with room round
    // it; the name stops short of it, and the value's text starts where every other value's does.
    let mut picked = None;
    if let (Value::Color3(color), None) = (values::shown(&row.value, row.part), row.component) {
        let side = size::ROW - 2.0 * values::SWATCH_PAD;
        let swatch = Rect::from_min_size(
            pos2(
                split - size::BORDER - values::SWATCH_PAD - side,
                rect.top() + values::SWATCH_PAD,
            ),
            vec2(side, side),
        );
        name_cell.max.x = name_cell.max.x.min(swatch.left() - space::S2);
        picked = values::swatch(ui, swatch, color, &row.label, row.editable);
    }
    let in_cell = |cell: Rect| {
        UiBuilder::new()
            .max_rect(cell)
            .layout(Layout::left_to_right(Align::Center))
    };
    ui.scope_builder(in_cell(name_cell), |ui| {
        ui.set_clip_rect(name_cell.intersect(ui.clip_rect()));
        let name = ui.add(
            Label::new(RichText::new(&row.shown).size(text::BODY).color(p.muted))
                .truncate()
                .selectable(false),
        );
        if !row.description.is_empty() {
            name.on_hover_text(row.description);
        }
    });
    if let (Source::Attribute(attribute), 0) = (&row.source, row.depth) {
        let response = ui.interact(name_cell, ui.scope_id().with("attribute"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("Attribute {attribute}")));
        out.action = attribute_menu(&response, attribute, taken);
    }
    let typed = ui
        .scope_builder(in_cell(value_cell), |ui| {
            ui.set_clip_rect(value_cell.intersect(ui.clip_rect()));
            match row.component {
                // A part's row edits its component alone (#417).
                Some(index) if row.editable => {
                    values::edit_component(ui, row.meta, &row.label, row.part, &row.value, index, row.mixed.whole)
                }
                Some(index) => {
                    let shown = values::component_text(&row.value, row.part, index).unwrap_or_default();
                    ui.label(RichText::new(shown).size(text::BODY).color(p.text));
                    None
                }
                None if row.editable => values::edit(ui, dm, row.meta, &row.label, row.part, &row.value, row.mixed),
                None if row.mixed.whole => {
                    ui.label(RichText::new("Mixed").size(text::BODY).color(p.subtle));
                    None
                }
                None => {
                    value_ui(ui, dm, &values::shown(&row.value, row.part));
                    None
                }
            }
        })
        .inner;
    out.edit = typed.or(picked);
    out
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

/// A property's value: a check box alone (a boolean, #378), or text (a colour's swatch is the row's, #415).
fn value_ui(ui: &mut Ui, dm: &DataModel, value: &Value) {
    let p = current_colors(ui.ctx()).palette;
    let shown = format_value(dm, value);
    if let &Value::Bool(on) = value {
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
        // #589: a part's properties, group by group, as the owner laid them out.
        let layout: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|group| {
                let rows = group.rows.iter().filter(|row| row.depth == 0);
                (group.name, rows.map(|row| row.label.as_str()).collect())
            })
            .collect();
        assert_eq!(
            layout,
            [
                ("General", vec!["Name", "Parent", "Archivable"]),
                ("Transform", vec!["Position", "Rotation", "Size", "PivotOffset"]),
                (
                    "Appearance",
                    vec!["Shape", "Material", "Color", "Transparency", "CastShadow"]
                ),
                ("Collision", vec!["CanCollide"]),
                ("Physics", vec!["Anchored"]),
            ]
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

    /// #591: an instance's attributes are rows by name, a vector's opening into its axes; one holding an enum item is shown,
    /// not edited. A new name is checked as Roblox checks one.
    #[test]
    fn attributes_are_rows_by_name_and_their_names_are_checked() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        dm.set_parent(part, Some(workspace)).unwrap();
        dm.set_attribute(part, "Speed", Some(Value::Number(3.0))).unwrap();
        dm.set_attribute(part, "Aim", Some(Value::Vector3(Vector3::new(0.0, 1.0, 0.0))))
            .unwrap();
        let wood = metallic_engine::metallic_core::EnumItem::new("Material", "Wood");
        dm.set_attribute(part, "Kind", Some(Value::Enum(wood))).unwrap();

        let rows = attribute_rows(&dm, part);
        let top: Vec<(&str, bool)> = rows
            .iter()
            .filter(|row| row.depth == 0)
            .map(|row| (row.label.as_str(), row.editable))
            .collect();
        assert_eq!(top, [("Aim", true), ("Kind", false), ("Speed", true)]);
        let axes: Vec<&str> = rows
            .iter()
            .filter(|row| row.parent.as_deref() == Some("@Aim"))
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(axes, ["Aim X", "Aim Y", "Aim Z"]);
        assert!(rows.iter().all(|row| matches!(&row.source, Source::Attribute(_))));

        let taken = ["Speed".to_owned()];
        for wrong in ["", "Speed", "two words", "dash-ed", &"a".repeat(101)] {
            assert!(name_problem(wrong, &taken).is_some(), "{wrong:?}");
        }
        assert_eq!(name_problem("Max_Speed2", &taken), None);
        assert_eq!(name_problem(&"a".repeat(100), &taken), None);
    }
}
