//! The Properties panel's value editors (#349): a control for each kind of value, which reports what the user
//! changed this frame as an [`Edit`]. The panel applies it to each selected instance's own value ([`apply`]) and
//! writes it through the scene, so it is an undo step (a drag is one, as the button stays held; a typed value is one,
//! written when the field is left).
//!
//! - booleans: a check box; numbers (and each component of a vector, a colour's channels): a field to drag or type
//!   in, within the property's range, or a slider for a [`Widget::Slider`] property;
//! - strings: a text field, written on Enter or when it loses the focus (Escape leaves the value as it was);
//! - `CFrame`: two rows, its position and its orientation in degrees ([`Part`]);
//! - `Color3`: the swatch opens a colour picker, and the 0-255 channels can be dragged or typed;
//! - enums: a drop-down of the enum's items.
//!
//! With several instances selected (#350) a value they disagree on is **mixed** ([`Mixed`]): its field shows a dash
//! (a drop-down "Mixed", a check box a bar), and an edit of one component of a vector changes that component alone
//! on each, so they keep their own others. Every control carries the property's label for screen readers (a
//! component's adds its axis: `Size X`).

use egui::{
    Color32, ComboBox, CornerRadius, DragValue, Key, Response, Role, Sense, Slider, Stroke, TextEdit, Ui, WidgetInfo,
    vec2,
};
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, Color3, DataModel, PropMeta, PropType, Value, Widget};

use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{size, space, text};

/// Which part of a `CFrame` a row edits: the Properties panel shows a `CFrame` as two rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The whole value (anything but a `CFrame`).
    Whole,
    /// A `CFrame`'s position.
    Position,
    /// A `CFrame`'s orientation, in degrees about X, Y and Z.
    Orientation,
}

/// What the user changed in a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// The whole value: a boolean, a number, a string, an enum item, a colour from the picker.
    Whole(Value),
    /// One component (0, 1, 2) of a vector, of a `CFrame`'s position or orientation, or a colour's 0-255 channel.
    Component(usize, f64),
}

/// Which parts of a row's value the selected instances disagree on (#350).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mixed {
    /// The value as a whole.
    pub whole: bool,
    /// Each component (of a vector, a `CFrame`'s part, a colour's channels).
    pub parts: [bool; 3],
}

impl Mixed {
    /// How `values` (each instance's, the first the active one's) disagree, as row `part` shows them.
    pub fn of(values: &[Value], part: Part) -> Self {
        let Some((first, rest)) = values.split_first() else {
            return Self::default();
        };
        let mut mixed = Self::default();
        let mine = components(first, part);
        for value in rest {
            mixed.whole |= shown(value, part) != shown(first, part);
            if let (Some(mine), Some(theirs)) = (mine, components(value, part)) {
                for (index, flag) in mixed.parts.iter_mut().enumerate() {
                    *flag |= mine[index] != theirs[index];
                }
            }
        }
        mixed
    }
}

/// The value row `part` shows: a `CFrame`'s position or orientation as a `Vector3`, anything else as it is.
pub fn shown(value: &Value, part: Part) -> Value {
    match (value, part) {
        (Value::CFrame(frame), Part::Position) => Value::Vector3(frame.position()),
        (Value::CFrame(frame), Part::Orientation) => Value::Vector3(frame.to_orientation_deg()),
        (value, _) => value.clone(),
    }
}

/// The components an [`Edit::Component`] addresses, as row `part` shows them: a colour's in 0-255.
fn components(value: &Value, part: Part) -> Option<[f64; 3]> {
    let channel = |c: f32| (f64::from(c).clamp(0.0, 1.0) * 255.0).round();
    match shown(value, part) {
        Value::Vector3(v) => Some([f64::from(v.x), f64::from(v.y), f64::from(v.z)]),
        Value::Vector2(v) => Some([f64::from(v.x), f64::from(v.y), 0.0]),
        Value::Color3(c) => Some([channel(c.r), channel(c.g), channel(c.b)]),
        _ => None,
    }
}

/// `current` (one instance's value of the property) with `edit` made to it, as row `part` shows it. `None` when the
/// edit does not fit the value.
pub fn apply(edit: &Edit, part: Part, current: &Value) -> Option<Value> {
    let set = |mut v: Vec3, index: usize, x: f64| {
        v[index] = x as f32;
        v
    };
    match (edit, current, part) {
        (Edit::Whole(value), _, _) => Some(value.clone()),
        (&Edit::Component(index, x), Value::Vector3(v), _) if index < 3 => Some(Value::Vector3(set(*v, index, x))),
        (&Edit::Component(index, x), Value::Vector2(v), _) if index < 2 => {
            let mut v = *v;
            v[index] = x as f32;
            Some(Value::Vector2(v))
        }
        (&Edit::Component(index, x), Value::CFrame(frame), Part::Orientation) if index < 3 => {
            let turned = set(frame.to_orientation_deg(), index, x);
            let rotation = CFrame::from_orientation_deg(turned.x, turned.y, turned.z).rotation();
            Some(Value::CFrame(CFrame::from_parts(frame.position(), rotation)))
        }
        (&Edit::Component(index, x), Value::CFrame(frame), _) if index < 3 => Some(Value::CFrame(CFrame::from_parts(
            set(frame.position(), index, x),
            frame.rotation(),
        ))),
        (&Edit::Component(index, x), Value::Color3(color), _) if index < 3 => {
            let mut channels = [color.r, color.g, color.b];
            channels[index] = (x / 255.0).clamp(0.0, 1.0) as f32;
            Some(Value::Color3(Color3::new(channels[0], channels[1], channels[2])))
        }
        _ => None,
    }
}

/// The size of the check box and the colour swatch.
const SWATCH: f32 = 14.0;
/// The space between the fields of a vector.
const GAP: f32 = 2.0;
/// What a mixed field shows.
const MIXED: &str = "\u{2013}";

/// Draws the editor of `value` (the active instance's value of property `meta`, shown as `label`, with what the
/// selection disagrees on `mixed`) filling `ui`, the value's cell. Returns what the user changed.
pub fn edit(
    ui: &mut Ui,
    dm: &DataModel,
    meta: &PropMeta,
    label: &str,
    part: Part,
    value: &Value,
    mixed: Mixed,
) -> Option<Edit> {
    let width = ui.available_width();
    // Controls as tall as the row leaves room for, so a row of fields is as tall as a row of text.
    let spacing = ui.spacing_mut();
    spacing.button_padding = vec2(space::S2, 0.0);
    spacing.interact_size.y = size::ROW - 4.0;
    match shown(value, part) {
        Value::Bool(on) => checkbox(ui, on, mixed.whole, label).map(|on| Edit::Whole(Value::Bool(on))),
        Value::Int(n) => {
            let mut v = n;
            let mut drag = DragValue::new(&mut v).speed(drag_speed(meta, 1.0));
            if let Some((min, max)) = meta.range {
                drag = drag.range(min as i64..=max as i64);
            }
            if mixed.whole {
                drag = drag.custom_formatter(|_, _| MIXED.to_owned());
            }
            let response = ui.add_sized([width, field_height(ui)], drag);
            labelled(&response, Role::SpinButton, label);
            (response.changed() && v != n).then_some(Edit::Whole(Value::Int(v)))
        }
        Value::Number(n) => number(ui, n, meta, label, width, mixed.whole).map(|n| Edit::Whole(Value::Number(n))),
        Value::String(s) => text_field(ui, label, &s, width, mixed.whole).map(|s| Edit::Whole(Value::String(s.into()))),
        Value::Vector2(v) => fields(ui, &[v.x, v.y], &mixed.parts, meta, label, &["X", "Y"], width),
        Value::Vector3(v) => fields(ui, &[v.x, v.y, v.z], &mixed.parts, meta, label, &["X", "Y", "Z"], width),
        Value::Color3(color) => colour(ui, color, &mixed.parts, label, width),
        Value::Enum(item) => {
            let PropType::Enum(enum_type) = meta.ty else {
                return None;
            };
            let desc = dm.registry().enum_desc(enum_type)?;
            let mut chosen = None;
            let shown = if mixed.whole { "Mixed" } else { item.name() };
            let combo = ComboBox::from_id_salt(label)
                .selected_text(shown)
                .width(width)
                .show_ui(ui, |ui| {
                    for &name in desc.items {
                        let current = !mixed.whole && name == item.name();
                        if ui.selectable_label(current, name).clicked() && !current {
                            chosen = Some(name);
                        }
                    }
                });
            labelled(&combo.response, Role::ComboBox, label);
            chosen
                .and_then(|name| dm.registry().enum_item(enum_type, name))
                .map(|item| Edit::Whole(Value::Enum(item)))
        }
        // References are read-only (the Parent changes by dragging in the Explorer); the panel draws them.
        Value::Ref(_) | Value::CFrame(_) => None,
    }
}

/// A row's field height: the row's, less a little air.
fn field_height(ui: &Ui) -> f32 {
    (size::ROW - 4.0).min(ui.available_height().max(1.0))
}

/// Gives a control `label` for screen readers.
fn labelled(response: &Response, kind: Role, label: &str) {
    response.widget_info(|| WidgetInfo::labeled(kind, response.enabled(), label));
}

/// How fast a number field changes per point dragged: the property's own, a two-hundredth of its range, or `fallback`.
fn drag_speed(meta: &PropMeta, fallback: f64) -> f64 {
    match (meta.widget, meta.range) {
        (Widget::Drag { speed }, _) => f64::from(speed),
        (_, Some((min, max))) if max > min => (max - min) / 200.0,
        _ => fallback,
    }
}

/// A check box, `on` or not (a bar when `mixed`); returns the new state when clicked (on, from mixed).
fn checkbox(ui: &mut Ui, on: bool, mixed: bool, label: &str) -> Option<bool> {
    let p = current_colors(ui.ctx()).palette;
    let accent = current_colors(ui.ctx()).accent;
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on && !mixed, label));
    let filled = on || mixed;
    let border = if response.hovered() {
        accent
    } else if filled {
        p.action
    } else {
        p.border
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        if filled { p.action } else { p.input },
        Stroke::new(size::BORDER, border),
        egui::StrokeKind::Inside,
    );
    if mixed {
        let bar = egui::Rect::from_center_size(rect.center(), vec2(SWATCH - 6.0, 2.0));
        ui.painter().rect_filled(bar, 1.0, p.action_text);
    } else if on {
        icons::paint_at(
            ui.painter(),
            rect.center(),
            egui::Align2::CENTER_CENTER,
            Icon::UiCheck,
            text::SMALL - 1.0,
            p.action_text,
        );
    }
    ui.add_space(space::S2);
    let word = match (mixed, on) {
        (true, _) => "Mixed",
        (false, true) => "true",
        (false, false) => "false",
    };
    ui.label(egui::RichText::new(word).size(text::BODY).color(p.text));
    response.clicked().then_some(mixed || !on)
}

/// How a number field shows its value: as people read it, or a dash when the selection disagrees on it.
fn formatted(v: f64, mixed: bool) -> String {
    if mixed {
        MIXED.to_owned()
    } else {
        super::properties::number(v)
    }
}

/// A number field `width` wide (a slider for a [`Widget::Slider`] property with a range), a dash when `mixed`.
fn number(ui: &mut Ui, n: f64, meta: &PropMeta, label: &str, width: f32, mixed: bool) -> Option<f64> {
    let mut v = n;
    let response = match (meta.widget, meta.range) {
        (Widget::Slider, Some((min, max))) => {
            ui.spacing_mut().slider_width = (width - 56.0).max(40.0);
            ui.add(Slider::new(&mut v, min..=max).custom_formatter(move |v, _| formatted(v, mixed)))
        }
        _ => {
            let mut drag = DragValue::new(&mut v)
                .speed(drag_speed(meta, 0.1))
                .custom_formatter(move |v, _| formatted(v, mixed));
            if let Some((min, max)) = meta.range {
                drag = drag.range(min..=max);
            }
            ui.add_sized([width, field_height(ui)], drag)
        }
    };
    let kind = if meta.widget == Widget::Slider {
        Role::Slider
    } else {
        Role::SpinButton
    };
    labelled(&response, kind, label);
    (response.changed() && v != n).then_some(v)
}

/// A field per component of `values`, side by side in `width`, each labelled with its axis and a dash where
/// `mixed`. Returns the component changed.
fn fields(
    ui: &mut Ui,
    values: &[f32],
    mixed: &[bool; 3],
    meta: &PropMeta,
    label: &str,
    axes: &[&str],
    width: f32,
) -> Option<Edit> {
    let count = values.len() as f32;
    let each = ((width - GAP * (count - 1.0)) / count).max(16.0);
    let mut edit = None;
    ui.spacing_mut().item_spacing.x = GAP;
    for (index, (value, axis)) in values.iter().zip(axes).enumerate() {
        let mut v = f64::from(*value);
        let mixed = mixed[index];
        let mut drag = DragValue::new(&mut v)
            .speed(drag_speed(meta, 0.1))
            .custom_formatter(move |v, _| formatted(v, mixed));
        if let Some((min, max)) = meta.range {
            drag = drag.range(min..=max);
        }
        let response = ui.add_sized([each, field_height(ui)], drag);
        labelled(&response, Role::SpinButton, &format!("{label} {axis}"));
        if response.changed() && v as f32 != *value {
            edit = Some(Edit::Component(index, v));
        }
    }
    edit
}

/// What a colour's channel fields are: whole numbers from 0 to 255.
static CHANNEL: PropMeta = PropMeta::number("Channel", 0.0)
    .range(0.0, 255.0)
    .widget(Widget::Drag { speed: 1.0 });

/// A colour: its swatch opens a picker (a whole colour), and its 0-255 channels are fields (a channel each).
fn colour(ui: &mut Ui, color: Color3, mixed: &[bool; 3], label: &str, width: f32) -> Option<Edit> {
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut rgb = [channel(color.r), channel(color.g), channel(color.b)];
    let before = rgb;
    ui.spacing_mut().interact_size = vec2(SWATCH + 6.0, SWATCH);
    let swatch = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
    labelled(&swatch, Role::ColorWell, label);
    let picked = (rgb != before).then(|| {
        let unit = |c: u8| f32::from(c) / 255.0;
        Edit::Whole(Value::Color3(Color3::new(unit(rgb[0]), unit(rgb[1]), unit(rgb[2]))))
    });
    ui.add_space(space::S2);
    let rest = width - SWATCH - 6.0 - space::S2 - GAP;
    let values = before.map(f32::from);
    let channels = ui
        .scope(|ui| fields(ui, &values, mixed, &CHANNEL, label, &["R", "G", "B"], rest))
        .inner;
    picked.or(channels)
}

/// A one-line text field `width` wide showing `current` (empty, with "Mixed" in grey, when `mixed`). What is typed is
/// kept while the field has the focus and returned when it loses it (Enter, a click elsewhere); Escape drops it.
fn text_field(ui: &mut Ui, label: &str, current: &str, width: f32, mixed: bool) -> Option<String> {
    let id = ui.make_persistent_id(("metallic.properties.text", label));
    let shown = if mixed { "" } else { current };
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shown.to_owned());
    let response = ui.add_sized(
        [width, field_height(ui)],
        TextEdit::singleline(&mut typed)
            .id(id.with("field"))
            .hint_text("Mixed")
            .margin(vec2(space::S2, 2.0))
            .text_color(current_colors(ui.ctx()).palette.text),
    );
    labelled(&response, Role::TextInput, label);
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, typed));
        return None;
    }
    ui.data_mut(|data| data.remove::<String>(id));
    let cancelled = ui.input(|input| input.key_pressed(Key::Escape));
    (response.lost_focus() && !cancelled && typed != shown).then_some(typed)
}

/// The swatch's fill for `color`, as the read-only panel draws it.
pub fn swatch_fill(color: Color3) -> Color32 {
    Color32::from_rgb(
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_core::glam::Vec2;

    use super::*;

    #[test]
    fn a_component_edit_keeps_each_instance_s_other_components() {
        let a = Value::Vector3(Vec3::new(1.0, 2.0, 3.0));
        let b = Value::Vector3(Vec3::new(4.0, 5.0, 6.0));
        let edit = Edit::Component(1, 9.0);
        assert_eq!(
            apply(&edit, Part::Whole, &a),
            Some(Value::Vector3(Vec3::new(1.0, 9.0, 3.0)))
        );
        assert_eq!(
            apply(&edit, Part::Whole, &b),
            Some(Value::Vector3(Vec3::new(4.0, 9.0, 6.0)))
        );
        assert_eq!(
            apply(
                &Edit::Component(0, 7.0),
                Part::Whole,
                &Value::Vector2(Vec2::new(1.0, 2.0))
            ),
            Some(Value::Vector2(Vec2::new(7.0, 2.0)))
        );
        // A CFrame's position keeps its rotation; its orientation keeps its position.
        let rotation = CFrame::from_orientation_deg(0.0, 30.0, 0.0).rotation();
        let turned = CFrame::from_parts(Vec3::new(1.0, 2.0, 3.0), rotation);
        let Some(Value::CFrame(moved)) = apply(&edit, Part::Position, &Value::CFrame(turned)) else {
            panic!("a CFrame");
        };
        assert_eq!(moved.position(), Vec3::new(1.0, 9.0, 3.0));
        assert!(moved.rotation().abs_diff_eq(turned.rotation(), 1e-6));
        let Some(Value::CFrame(spun)) = apply(&Edit::Component(1, 90.0), Part::Orientation, &Value::CFrame(turned))
        else {
            panic!("a CFrame");
        };
        assert_eq!(spun.position(), turned.position());
        assert!((spun.to_orientation_deg().y - 90.0).abs() < 1e-3);
        // A colour's channel is in 0-255.
        let grey = Value::Color3(Color3::new(0.5, 0.5, 0.5));
        assert_eq!(
            apply(&Edit::Component(0, 255.0), Part::Whole, &grey),
            Some(Value::Color3(Color3::new(1.0, 0.5, 0.5)))
        );
        // A whole value replaces; an edit that does not fit does nothing.
        assert_eq!(
            apply(&Edit::Whole(Value::Bool(true)), Part::Whole, &a),
            Some(Value::Bool(true))
        );
        assert_eq!(apply(&edit, Part::Whole, &Value::Bool(true)), None);
    }

    #[test]
    fn mixed_marks_the_whole_value_and_each_component_the_selection_disagrees_on() {
        let values = [
            Value::Vector3(Vec3::new(1.0, 2.0, 3.0)),
            Value::Vector3(Vec3::new(1.0, 5.0, 3.0)),
        ];
        assert_eq!(
            Mixed::of(&values, Part::Whole),
            Mixed {
                whole: true,
                parts: [false, true, false]
            }
        );
        assert_eq!(
            Mixed::of(&values[..1], Part::Whole),
            Mixed::default(),
            "one alone never disagrees"
        );
        let frames = [
            Value::CFrame(CFrame::new(Vec3::new(0.0, 1.0, 0.0))),
            Value::CFrame(CFrame::new(Vec3::new(0.0, 2.0, 0.0))),
        ];
        assert!(Mixed::of(&frames, Part::Position).whole);
        assert!(!Mixed::of(&frames, Part::Orientation).whole, "the same rotation");
        let names = [Value::String("A".into()), Value::String("B".into())];
        assert!(Mixed::of(&names, Part::Whole).whole);
    }
}
