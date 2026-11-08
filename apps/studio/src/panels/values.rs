//! The Properties panel's value editors (#349): a control for each kind of value, which returns the new value when
//! the user changed it this frame. The panel writes it through the scene, so it is an undo step (a drag is one, as
//! the button stays held; a typed value is one, written when the field is left).
//!
//! - booleans: a check box; numbers (and each component of a vector, a colour's channels): a field to drag or type
//!   in, within the property's range, or a slider for a [`Widget::Slider`] property;
//! - strings: a text field, written on Enter or when it loses the focus (Escape leaves the value as it was);
//! - `CFrame`: two rows, its position and its orientation in degrees ([`Part`]);
//! - `Color3`: the swatch opens a colour picker, and the 0-255 channels can be dragged or typed;
//! - enums: a drop-down of the enum's items.
//!
//! Every control carries the property's label for screen readers (a component's adds its axis: `Size X`).

use egui::{
    Color32, ComboBox, CornerRadius, DragValue, Key, Response, Role, Sense, Slider, Stroke, TextEdit, Ui, WidgetInfo,
    vec2,
};
use metallic_engine::metallic_core::glam::{Vec2, Vec3};
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

/// The size of the check box and the colour swatch.
const SWATCH: f32 = 14.0;
/// The space between the fields of a vector.
const GAP: f32 = 2.0;

/// Draws the editor of `value` (property `meta`, shown as `label`) filling `ui`, the value's cell. Returns the new
/// value when the user changed it.
pub fn edit(ui: &mut Ui, dm: &DataModel, meta: &PropMeta, label: &str, part: Part, value: &Value) -> Option<Value> {
    let width = ui.available_width();
    // Controls as tall as the row leaves room for, so a row of fields is as tall as a row of text.
    let spacing = ui.spacing_mut();
    spacing.button_padding = vec2(space::S2, 0.0);
    spacing.interact_size.y = size::ROW - 4.0;
    match (value, part) {
        (Value::Bool(on), _) => checkbox(ui, *on, label).map(Value::Bool),
        (Value::Int(n), _) => {
            let mut v = *n;
            let mut drag = DragValue::new(&mut v).speed(drag_speed(meta, 1.0));
            if let Some((min, max)) = meta.range {
                drag = drag.range(min as i64..=max as i64);
            }
            let response = ui.add_sized([width, field_height(ui)], drag);
            labelled(&response, Role::SpinButton, label);
            (response.changed() && v != *n).then_some(Value::Int(v))
        }
        (Value::Number(n), _) => number(ui, *n, meta, label, width).map(Value::Number),
        (Value::String(s), _) => text_field(ui, label, s, width).map(|s| Value::String(s.into())),
        (Value::Vector2(v), _) => {
            let parts = components(ui, &[v.x, v.y], meta, label, &["X", "Y"], width)?;
            Some(Value::Vector2(Vec2::new(parts[0], parts[1])))
        }
        (Value::Vector3(v), _) => vector3(ui, *v, meta, label, width).map(Value::Vector3),
        (Value::CFrame(frame), Part::Orientation) => {
            let turned = vector3(ui, frame.to_orientation_deg(), meta, label, width)?;
            let rotation = CFrame::from_orientation_deg(turned.x, turned.y, turned.z).rotation();
            Some(Value::CFrame(CFrame::from_parts(frame.position(), rotation)))
        }
        (Value::CFrame(frame), _) => {
            let at = vector3(ui, frame.position(), meta, label, width)?;
            Some(Value::CFrame(CFrame::from_parts(at, frame.rotation())))
        }
        (Value::Color3(color), _) => colour(ui, *color, label, width).map(Value::Color3),
        (Value::Enum(item), _) => {
            let PropType::Enum(enum_type) = meta.ty else {
                return None;
            };
            let desc = dm.registry().enum_desc(enum_type)?;
            let mut chosen = None;
            let combo = ComboBox::from_id_salt(label)
                .selected_text(item.name())
                .width(width)
                .show_ui(ui, |ui| {
                    for &name in desc.items {
                        if ui.selectable_label(name == item.name(), name).clicked() && name != item.name() {
                            chosen = Some(name);
                        }
                    }
                });
            labelled(&combo.response, Role::ComboBox, label);
            chosen
                .and_then(|name| dm.registry().enum_item(enum_type, name))
                .map(Value::Enum)
        }
        // References are read-only (the Parent changes by dragging in the Explorer); the panel draws them.
        (Value::Ref(_), _) => None,
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

/// A check box, `on` or not; returns the new state when clicked.
fn checkbox(ui: &mut Ui, on: bool, label: &str) -> Option<bool> {
    let p = current_colors(ui.ctx()).palette;
    let accent = current_colors(ui.ctx()).accent;
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on, label));
    let border = if response.hovered() {
        accent
    } else if on {
        p.action
    } else {
        p.border
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        if on { p.action } else { p.input },
        Stroke::new(size::BORDER, border),
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
    ui.add_space(space::S2);
    ui.label(
        egui::RichText::new(if on { "true" } else { "false" })
            .size(text::BODY)
            .color(p.text),
    );
    response.clicked().then_some(!on)
}

/// A number field `width` wide (a slider for a [`Widget::Slider`] property with a range).
fn number(ui: &mut Ui, n: f64, meta: &PropMeta, label: &str, width: f32) -> Option<f64> {
    let mut v = n;
    let response = match (meta.widget, meta.range) {
        (Widget::Slider, Some((min, max))) => {
            ui.spacing_mut().slider_width = (width - 56.0).max(40.0);
            ui.add(Slider::new(&mut v, min..=max).custom_formatter(|v, _| super::properties::number(v)))
        }
        _ => {
            let mut drag = DragValue::new(&mut v)
                .speed(drag_speed(meta, 0.1))
                .custom_formatter(|v, _| super::properties::number(v));
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

/// A field per component of `values`, side by side in `width`, each labelled with its axis. Returns every component
/// when one changed.
fn components(
    ui: &mut Ui,
    values: &[f32],
    meta: &PropMeta,
    label: &str,
    axes: &[&str],
    width: f32,
) -> Option<Vec<f32>> {
    let count = values.len() as f32;
    let each = ((width - GAP * (count - 1.0)) / count).max(16.0);
    let mut out = values.to_vec();
    let mut changed = false;
    ui.spacing_mut().item_spacing.x = GAP;
    for (value, axis) in out.iter_mut().zip(axes) {
        let mut v = f64::from(*value);
        let mut drag = DragValue::new(&mut v)
            .speed(drag_speed(meta, 0.1))
            .custom_formatter(|v, _| super::properties::number(v));
        if let Some((min, max)) = meta.range {
            drag = drag.range(min..=max);
        }
        let response = ui.add_sized([each, field_height(ui)], drag);
        labelled(&response, Role::SpinButton, &format!("{label} {axis}"));
        if response.changed() && v as f32 != *value {
            *value = v as f32;
            changed = true;
        }
    }
    changed.then_some(out)
}

/// A `Vector3` as three fields.
fn vector3(ui: &mut Ui, v: Vec3, meta: &PropMeta, label: &str, width: f32) -> Option<Vec3> {
    let parts = components(ui, &[v.x, v.y, v.z], meta, label, &["X", "Y", "Z"], width)?;
    Some(Vec3::new(parts[0], parts[1], parts[2]))
}

/// A colour: its swatch opens a picker, and its 0-255 channels are fields.
fn colour(ui: &mut Ui, color: Color3, label: &str, width: f32) -> Option<Color3> {
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut rgb = [channel(color.r), channel(color.g), channel(color.b)];
    let before = rgb;
    ui.spacing_mut().interact_size = vec2(SWATCH + 4.0, SWATCH);
    let swatch = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
    labelled(&swatch, Role::ColorWell, label);
    ui.add_space(space::S2);
    let rest = width - SWATCH - 4.0 - space::S2 - GAP;
    let each = ((rest - GAP * 2.0) / 3.0).max(16.0);
    ui.spacing_mut().item_spacing.x = GAP;
    for (value, axis) in rgb.iter_mut().zip(["R", "G", "B"]) {
        let response = ui.add_sized([each, field_height(ui)], DragValue::new(value).range(0..=255));
        labelled(&response, Role::SpinButton, &format!("{label} {axis}"));
    }
    (rgb != before).then(|| {
        let unit = |c: u8| f32::from(c) / 255.0;
        // A channel left alone keeps its exact value, not its rounding to 0-255.
        let keep = |new: u8, old: u8, exact: f32| if new == old { exact } else { unit(new) };
        Color3::new(
            keep(rgb[0], before[0], color.r),
            keep(rgb[1], before[1], color.g),
            keep(rgb[2], before[2], color.b),
        )
    })
}

/// A one-line text field `width` wide showing `current`. What is typed is kept while the field has the focus and
/// returned when it loses it (Enter, a click elsewhere); Escape drops it.
fn text_field(ui: &mut Ui, label: &str, current: &str, width: f32) -> Option<String> {
    let id = ui.make_persistent_id(("metallic.properties.text", label));
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_owned());
    let response = ui.add_sized(
        [width, field_height(ui)],
        TextEdit::singleline(&mut typed)
            .id(id.with("field"))
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
    (response.lost_focus() && !cancelled && typed != current).then_some(typed)
}

/// The swatch's fill for `color`, as the read-only panel drew it.
pub fn swatch_fill(color: Color3) -> Color32 {
    Color32::from_rgb(
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}
