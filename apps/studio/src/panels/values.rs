//! The Properties panel's value editors (#349): a control for each kind of value, which reports what the user
//! changed this frame as an [`Edit`]. The panel applies it to each selected instance's own value ([`apply`]) and
//! writes it through the scene, so it is an undo step (a drag is one, as the button stays held; a typed value is one,
//! written when the field is left).
//!
//! - booleans: a check box alone (#378); numbers: plain text like the text fields (#381), which goes down when dragged
//!   left and up when dragged right and is typed in when clicked, within the property's range (a [`Widget::Slider`]
//!   property too: no sliders, #379);
//! - strings, vectors and colours: one plain text field each, without a box (#380), written on Enter or when it loses
//!   the focus (Escape leaves the value as it was). A vector is its numbers one after another (`-20, 40, 33`), a
//!   colour its 0-255 channels;
//! - `CFrame`: two rows, its position and its orientation in degrees ([`Part`]);
//! - `Color3`: its channels as text; a small square swatch at the right of the name column ([`swatch`], #415) opens
//!   Studio's colour picker ([`super::color_picker`], #416);
//! - enums: a drop-down of the enum's items.
//!
//! With several instances selected (#350) a value they disagree on is **mixed** ([`Mixed`]): a component shows a
//! dash (a text field "Mixed" in grey, a drop-down "Mixed", a check box a bar), and an edit of some components of a
//! vector changes those alone on each, so they keep their own others. Every control carries the property's label for
//! screen readers.

use egui::{
    Align2, Color32, ComboBox, CornerRadius, CursorIcon, FontId, Key, Response, Role, Sense, Stroke, TextEdit, Ui,
    WidgetInfo, pos2, vec2,
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
    /// Components (0, 1, 2) of a vector, of a `CFrame`'s position or orientation, or a colour's 0-255 channels (#380):
    /// each `Some` is set, each `None` left as each instance has it.
    Components([Option<f64>; 3]),
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
    match edit {
        Edit::Whole(value) => Some(value.clone()),
        Edit::Components(set) => {
            let mut value = current.clone();
            for (index, x) in set.iter().enumerate() {
                if let Some(x) = *x {
                    value = set_component(&value, part, index, x)?;
                }
            }
            components(&value, part).map(|_| value)
        }
    }
}

/// `value` with its component `index` set to `x`, as row `part` shows it (a colour's channel in 0-255).
fn set_component(value: &Value, part: Part, index: usize, x: f64) -> Option<Value> {
    let set = |mut v: Vec3| {
        v[index] = x as f32;
        v
    };
    match (value, part) {
        (Value::Vector3(v), _) if index < 3 => Some(Value::Vector3(set(*v))),
        (Value::Vector2(v), _) if index < 2 => {
            let mut v = *v;
            v[index] = x as f32;
            Some(Value::Vector2(v))
        }
        (Value::CFrame(frame), Part::Orientation) if index < 3 => {
            let turned = set(frame.to_orientation_deg());
            let rotation = CFrame::from_orientation_deg(turned.x, turned.y, turned.z).rotation();
            Some(Value::CFrame(CFrame::from_parts(frame.position(), rotation)))
        }
        (Value::CFrame(frame), _) if index < 3 => Some(Value::CFrame(CFrame::from_parts(
            set(frame.position()),
            frame.rotation(),
        ))),
        (Value::Color3(color), _) if index < 3 => {
            let mut channels = [color.r, color.g, color.b];
            channels[index] = (x / 255.0).clamp(0.0, 1.0) as f32;
            Some(Value::Color3(Color3::new(channels[0], channels[1], channels[2])))
        }
        _ => None,
    }
}

/// The components typed in `text` for a value of `count` of them, each kept in `range` (#380): numbers one after
/// another, a comma between, and a dash or nothing for one to leave as it is; or a single number, which fills every
/// component (#382: `255` is `255, 255, 255`). `None` when it is neither.
pub fn parse_components(text: &str, count: usize, range: Option<(f64, f64)>) -> Option<[Option<f64>; 3]> {
    let mut parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if let [one] = parts[..]
        && !one.is_empty()
        && one != "-"
        && one != MIXED
    {
        parts = vec![one; count];
    }
    if parts.len() != count {
        return None;
    }
    let mut out = [None; 3];
    for (slot, part) in out.iter_mut().zip(&parts) {
        if part.is_empty() || *part == "-" || *part == MIXED {
            continue;
        }
        let x = part.parse::<f64>().ok().filter(|x| x.is_finite())?;
        *slot = Some(range.map_or(x, |(min, max)| x.clamp(min, max)));
    }
    Some(out)
}

/// The size of the check box.
const SWATCH: f32 = 14.0;
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
            let speed = drag_speed(meta, 1.0);
            let field = Number {
                range: meta.range,
                speed,
                whole: true,
            };
            number(ui, n as f64, field, label, width, mixed.whole).map(|n| Edit::Whole(Value::Int(n as i64)))
        }
        Value::Number(n) => {
            let field = Number {
                range: meta.range,
                speed: drag_speed(meta, 0.1),
                whole: false,
            };
            number(ui, n, field, label, width, mixed.whole).map(|n| Edit::Whole(Value::Number(n)))
        }
        Value::String(s) => text_field(ui, label, &s, width, mixed.whole).map(|s| Edit::Whole(Value::String(s.into()))),
        Value::Vector2(v) => components_field(ui, &[v.x, v.y], &mixed.parts, meta.range, label, width),
        Value::Vector3(v) => components_field(ui, &[v.x, v.y, v.z], &mixed.parts, meta.range, label, width),
        Value::Color3(color) => {
            let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round();
            let values = [channel(color.r), channel(color.g), channel(color.b)];
            components_field(ui, &values, &mixed.parts, Some((0.0, 255.0)), label, width)
        }
        Value::Enum(item) => {
            let PropType::Enum(enum_type) = meta.ty else {
                return None;
            };
            let desc = dm.registry().enum_desc(enum_type)?;
            let mut chosen = None;
            let shown = if mixed.whole { "Mixed" } else { item.name() };
            let combo = ui.scope(|ui| {
                plain(ui);
                ComboBox::from_id_salt(label)
                    .selected_text(shown)
                    .width(width)
                    .show_ui(ui, |ui| {
                        for &name in desc.items {
                            let current = !mixed.whole && name == item.name();
                            if ui.selectable_label(current, name).clicked() && !current {
                                chosen = Some(name);
                            }
                        }
                    })
            });
            labelled(&combo.inner.response, Role::ComboBox, label);
            chosen
                .and_then(|name| dm.registry().enum_item(enum_type, name))
                .map(|item| Edit::Whole(Value::Enum(item)))
        }
        // References are read-only (the Parent changes by dragging in the Explorer); the panel draws them.
        Value::Ref(_) | Value::CFrame(_) => None,
    }
}

/// Takes the boxes off the controls `ui` draws next (#380): the row's stripe and lines frame them, and the text starts
/// where the other rows' does.
fn plain(ui: &mut Ui) {
    let widgets = &mut ui.visuals_mut().widgets;
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.weak_bg_fill = Color32::TRANSPARENT;
        state.bg_fill = Color32::TRANSPARENT;
        state.bg_stroke = Stroke::NONE;
    }
    ui.spacing_mut().button_padding.x = 0.0;
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
    // The box says it all (#378); a selection that disagrees says so.
    if mixed {
        ui.add_space(space::S2);
        ui.label(egui::RichText::new("Mixed").size(text::BODY).color(p.subtle));
    }
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

/// How a number field keeps its value.
#[derive(Clone, Copy, Debug)]
struct Number {
    /// The property's range.
    range: Option<(f64, f64)>,
    /// How much a point dragged changes it.
    speed: f64,
    /// Whether it is a whole number (an `Int`).
    whole: bool,
}

impl Number {
    /// `x` as the field keeps it: within the range, whole for an `Int`.
    fn kept(self, x: f64) -> f64 {
        let x = if self.whole { x.round() } else { x };
        self.range.map_or(x, |(min, max)| x.clamp(min, max))
    }
}

/// A single number as plain text `width` wide (#381), a dash when `mixed`: dragged left it goes down and right it goes
/// up (the resize pointer over it says so), from where the drag began; clicked, it is a text field to type in, as the
/// others. A [`Widget::Slider`] property gets one too (#379): a value is typed or dragged, never slid. Returns the
/// new value.
fn number(ui: &mut Ui, n: f64, field: Number, label: &str, width: f32, mixed: bool) -> Option<f64> {
    let shown = formatted(n, mixed);
    // The text field's id: the same place is the drag area until a click gives it the focus.
    let id = ui.make_persistent_id(("metallic.properties.text", label)).with("field");
    if ui.memory(|memory| memory.has_focus(id)) {
        let typed = text_field(ui, label, &shown, width, false)?;
        return parse_components(&typed, 1, None)
            .and_then(|[x, ..]| x)
            .map(|x| field.kept(x));
    }
    let (rect, _) = ui.allocate_exact_size(vec2(width, field_height(ui)), Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());
    labelled(&response, Role::SpinButton, label);
    let p = current_colors(ui.ctx()).palette;
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &shown,
        FontId::proportional(text::BODY),
        p.text,
    );
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let start = id.with("drag");
    if response.drag_started() {
        ui.data_mut(|data| data.insert_temp(start, (n, 0.0f64)));
    }
    if response.dragged() {
        let (from, moved) = ui.data(|data| data.get_temp::<(f64, f64)>(start)).unwrap_or((n, 0.0));
        let moved = moved + f64::from(response.drag_delta().x) * field.speed;
        ui.data_mut(|data| data.insert_temp(start, (from, moved)));
        let x = field.kept(from + moved);
        return (x != n).then_some(x);
    }
    if response.drag_stopped() {
        ui.data_mut(|data| data.remove::<(f64, f64)>(start));
    }
    if response.clicked() {
        // Typing starts with the whole value selected, to replace it.
        response.request_focus();
        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(shown.chars().count()),
        );
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), id);
    }
    None
}

/// A vector's or a colour's `values` as one plain text field `width` wide (#380): its numbers one after another
/// (`-20, 40, 33`), a dash for one the selection disagrees on (`mixed`). Returns the components typed, each kept in
/// `range`; a dash or nothing leaves that component as each instance has it.
fn components_field(
    ui: &mut Ui,
    values: &[f32],
    mixed: &[bool; 3],
    range: Option<(f64, f64)>,
    label: &str,
    width: f32,
) -> Option<Edit> {
    let shown: Vec<String> = values
        .iter()
        .zip(mixed)
        .map(|(&v, &mixed)| formatted(f64::from(v), mixed))
        .collect();
    let typed = text_field(ui, label, &shown.join(", "), width, false)?;
    parse_components(&typed, values.len(), range).map(Edit::Components)
}

/// The space round a colour's swatch inside its row's lines (#415).
pub const SWATCH_PAD: f32 = 4.0;

/// A colour row's swatch at `rect` (#415: a small square at the right of the name column): `color`, which opens a
/// picker when `editable`. Returns the colour picked.
pub fn swatch(ui: &mut Ui, rect: egui::Rect, color: Color3, label: &str, editable: bool) -> Option<Edit> {
    let p = current_colors(ui.ctx()).palette;
    let sense = if editable { Sense::click() } else { Sense::hover() };
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("metallic.properties.swatch", label)),
        sense,
    );
    labelled(&response, Role::ColorWell, label);
    let edge = if editable && response.hovered() {
        p.text
    } else {
        p.border
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(2),
        swatch_fill(color),
        Stroke::new(size::BORDER, edge),
        egui::StrokeKind::Inside,
    );
    if !editable {
        return None;
    }
    let mut picked = None;
    let id = ui.make_persistent_id(("metallic.properties.picker", label));
    egui::Popup::menu(&response)
        .id(id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let [r, g, b, _] = swatch_fill(color).to_array();
            let mut rgb = [r, g, b];
            if super::color_picker::picker(ui, id.with("picker"), &mut rgb) {
                let unit = |c: u8| f32::from(c) / 255.0;
                picked = Some(Edit::Whole(Value::Color3(Color3::new(
                    unit(rgb[0]),
                    unit(rgb[1]),
                    unit(rgb[2]),
                ))));
            }
        });
    picked
}

/// A one-line text field `width` wide showing `current` (empty, with "Mixed" in grey, when `mixed`). What is typed is
/// kept while the field has the focus and returned when it loses it (Enter, a click elsewhere); Escape drops it.
fn text_field(ui: &mut Ui, label: &str, current: &str, width: f32, mixed: bool) -> Option<String> {
    let id = ui.make_persistent_id(("metallic.properties.text", label));
    let shown = if mixed { "" } else { current };
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shown.to_owned());
    // Plain text, no box (#380): the row's stripe and lines frame it.
    let response = ui.add_sized(
        [width, field_height(ui)],
        TextEdit::singleline(&mut typed)
            .id(id.with("field"))
            .hint_text("Mixed")
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 2.0))
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
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use metallic_engine::metallic_core::glam::Vec2;

    use super::*;

    /// A row's editor of `value` (property `meta`, labelled "Value"), keeping the last edit it reported.
    pub(super) fn editor(meta: &'static PropMeta, value: Value, mixed: Mixed) -> Harness<'static, Option<Edit>> {
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(300.0, 24.0))
            .with_context_setup(crate::theme::fonts::install)
            .build_ui_state(
                move |ui, edit: &mut Option<Edit>| {
                    if let Some(made) = super::edit(ui, &dm, meta, "Value", Part::Whole, &value, mixed) {
                        *edit = Some(made);
                    }
                },
                None,
            );
        harness.run();
        harness
    }

    #[test]
    fn a_number_drags_left_down_and_right_up_from_where_it_began() {
        static SPEED: PropMeta = PropMeta::number("WalkSpeed", 16.0).widget(Widget::Drag { speed: 0.5 });
        let mut harness = editor(&SPEED, Value::Number(16.0), Mixed::default());
        let field = harness.get_by_role_and_label(Role::SpinButton, "Value").rect();
        let at = field.left_center() + vec2(10.0, 0.0);
        harness.drag_at(at);
        harness.run();
        harness.hover_at(at + vec2(20.0, 0.0));
        harness.run();
        assert_eq!(
            *harness.state(),
            Some(Edit::Whole(Value::Number(26.0))),
            "20 points right, half a unit each"
        );
        harness.hover_at(at - vec2(10.0, 0.0));
        harness.run();
        assert_eq!(
            *harness.state(),
            Some(Edit::Whole(Value::Number(11.0))),
            "from where it began, not the last frame"
        );
        harness.drop_at(at - vec2(10.0, 0.0));
        harness.run();
        // An Int stays whole.
        static COUNT: PropMeta = PropMeta::int("Count", 3).widget(Widget::Drag { speed: 0.1 });
        let mut harness = editor(&COUNT, Value::Int(3), Mixed::default());
        let field = harness.get_by_role_and_label(Role::SpinButton, "Value").rect();
        let at = field.left_center() + vec2(10.0, 0.0);
        harness.drag_at(at);
        harness.run();
        harness.hover_at(at + vec2(24.0, 0.0));
        harness.run();
        assert_eq!(*harness.state(), Some(Edit::Whole(Value::Int(5))));
    }

    #[test]
    fn a_slider_property_is_a_field_kept_in_its_range() {
        static BRIGHTNESS: PropMeta = PropMeta::number("Brightness", 0.5)
            .range(0.0, 1.0)
            .widget(Widget::Slider);
        let mut harness = editor(&BRIGHTNESS, Value::Number(0.5), Mixed::default());
        assert!(harness.query_by_role(Role::Slider).is_none());
        let field = harness.get_by_role_and_label(Role::SpinButton, "Value");
        field.click();
        harness.run();
        // A click makes it a text field, all of it selected.
        harness.get_by_role_and_label(Role::TextInput, "Value").type_text("7");
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert_eq!(
            *harness.state(),
            Some(Edit::Whole(Value::Number(1.0))),
            "kept within 0 to 1"
        );
    }

    #[test]
    fn a_check_box_stands_alone() {
        static ANCHORED: PropMeta = PropMeta::boolean("Anchored", false);
        for on in [true, false] {
            let mut harness = editor(&ANCHORED, Value::Bool(on), Mixed::default());
            assert!(harness.query_by_label("true").is_none() && harness.query_by_label("false").is_none());
            harness.get_by_role_and_label(Role::CheckBox, "Value").click();
            harness.run();
            assert_eq!(*harness.state(), Some(Edit::Whole(Value::Bool(!on))));
        }
        let mixed = Mixed {
            whole: true,
            ..Mixed::default()
        };
        editor(&ANCHORED, Value::Bool(true), mixed).get_by_label("Mixed");
    }

    /// Component `index` set to `x`, the others left.
    fn one(index: usize, x: f64) -> Edit {
        let mut set = [None; 3];
        set[index] = Some(x);
        Edit::Components(set)
    }

    #[test]
    fn typed_components_parse_with_dashes_and_blanks_left_as_they_are() {
        assert_eq!(
            parse_components("-20, 40, 33", 3, None),
            Some([Some(-20.0), Some(40.0), Some(33.0)])
        );
        assert_eq!(
            parse_components(" 1 ,\u{2013}, ", 3, None),
            Some([Some(1.0), None, None])
        );
        assert_eq!(parse_components("-, 5, -", 3, None), Some([None, Some(5.0), None]));
        assert_eq!(
            parse_components("300, 0, 0", 3, Some((0.0, 255.0))),
            Some([Some(255.0), Some(0.0), Some(0.0)])
        );
        assert_eq!(parse_components("1, 2", 2, None), Some([Some(1.0), Some(2.0), None]));
        // One number fills them all (#382); a lone dash or nothing changes nothing.
        assert_eq!(
            parse_components(" 255 ", 3, Some((0.0, 255.0))),
            Some([Some(255.0), Some(255.0), Some(255.0)])
        );
        assert_eq!(parse_components("4", 2, None), Some([Some(4.0), Some(4.0), None]));
        assert_eq!(parse_components("-", 3, None), None);
        assert_eq!(parse_components("", 3, None), None);
        for bad in ["1, 2", "1, 2, 3, 4", "a, b, c", "1, inf, 2"] {
            assert_eq!(parse_components(bad, 3, None), None, "{bad}");
        }
    }

    #[test]
    fn a_vector_is_one_plain_text_field_of_its_numbers() {
        static SIZE: PropMeta = PropMeta::vector3("Size", 4.0, 1.0, 2.0);
        let mut harness = editor(&SIZE, Value::Vector3(Vec3::new(-20.0, 40.0, 33.5)), Mixed::default());
        let field = harness.get_by_role_and_label(Role::TextInput, "Value");
        assert_eq!(field.value().as_deref(), Some("-20, 40, 33.5"));
        field.click();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness
            .get_by_role_and_label(Role::TextInput, "Value")
            .type_text("1, -, 3");
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert_eq!(*harness.state(), Some(Edit::Components([Some(1.0), None, Some(3.0)])));
        // A component the selection disagrees on is a dash.
        let mixed = Mixed {
            whole: true,
            parts: [false, true, false],
        };
        let harness = editor(&SIZE, Value::Vector3(Vec3::new(1.0, 2.0, 3.0)), mixed);
        let field = harness.get_by_role_and_label(Role::TextInput, "Value");
        assert_eq!(field.value().as_deref(), Some("1, \u{2013}, 3"));
    }

    #[test]
    fn a_colour_is_its_channels_as_text_its_swatch_drawn_by_the_row() {
        static COLOR: PropMeta = PropMeta::color3("Color", 1.0, 0.0, 0.0);
        let harness = editor(&COLOR, Value::Color3(Color3::new(1.0, 0.5, 0.0)), Mixed::default());
        let field = harness.get_by_role_and_label(Role::TextInput, "Value");
        assert_eq!(field.value().as_deref(), Some("255, 128, 0"));
        // The swatch is the row's, left of the line between the columns (#415).
        assert!(harness.query_by_role(Role::ColorWell).is_none());
    }

    #[test]
    fn a_component_edit_keeps_each_instance_s_other_components() {
        let a = Value::Vector3(Vec3::new(1.0, 2.0, 3.0));
        let b = Value::Vector3(Vec3::new(4.0, 5.0, 6.0));
        let edit = one(1, 9.0);
        assert_eq!(
            apply(&edit, Part::Whole, &a),
            Some(Value::Vector3(Vec3::new(1.0, 9.0, 3.0)))
        );
        assert_eq!(
            apply(&edit, Part::Whole, &b),
            Some(Value::Vector3(Vec3::new(4.0, 9.0, 6.0)))
        );
        assert_eq!(
            apply(&one(0, 7.0), Part::Whole, &Value::Vector2(Vec2::new(1.0, 2.0))),
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
        let Some(Value::CFrame(spun)) = apply(&one(1, 90.0), Part::Orientation, &Value::CFrame(turned)) else {
            panic!("a CFrame");
        };
        assert_eq!(spun.position(), turned.position());
        assert!((spun.to_orientation_deg().y - 90.0).abs() < 1e-3);
        // A colour's channel is in 0-255.
        let grey = Value::Color3(Color3::new(0.5, 0.5, 0.5));
        assert_eq!(
            apply(&one(0, 255.0), Part::Whole, &grey),
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
