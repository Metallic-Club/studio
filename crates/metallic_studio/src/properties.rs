//! Properties: every registry property shared by the selected instances, in sections.
//!
//! Sections come in a fixed order: the class's own (Data), Appearance, Transform, Physics,
//! Behaviour. Values that differ across a multi-selection show "Mixed". Number fields scrub on
//! drag and accept expressions (`3*4`, `+2` adds to the current value, see [`crate::expr`]).
//!
//! **One gesture, one undo step:** while the pointer is held (scrubbing a number, dragging in the
//! colour picker) changes go through the editor's [`crate::preview::PreviewOverlay`], which writes
//! them as a live preview and remembers the first value of each touched property. When the
//! pointer is released the whole gesture is recorded as one `SetProperties` step. Typed values
//! and toggles apply at once as one step each.
use std::collections::HashMap;

use eframe::egui::{self, RichText};
use metallic_core::classes::{PropDef, all_properties, find_property};
use metallic_core::{CFrame, Color3, InstanceId, UDim2, Value, Vec3};

use crate::commands::{Command, PropName};
use crate::editor::{EditorState, UiRequest};
use crate::expr::eval_relative;
use crate::tokens::ui_tokens;
use metallic_core::transform::{clamp_part_size, orthonormal};

/// Formats a number field like egui does, but never as "-0" (a negative zero or a tiny negative
/// value that rounds to zero shows as "0").
fn fmt_drag(n: f64, decimals: std::ops::RangeInclusive<usize>) -> String {
    let text = egui::emath::format_with_decimals_in_range(n, decimals.clone());
    match text.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => {
            egui::emath::format_with_decimals_in_range(0.0, decimals)
        }
        _ => text,
    }
}

/// What a number attribute's drag asked for this frame.
enum AttributeScrub {
    /// The attribute now has this value (a frame of a drag).
    Value(String, f64),
    /// The drag ended.
    End,
}

/// A short text for an attribute value the panel cannot edit.
fn attribute_summary(value: &Value) -> String {
    match value {
        Value::Vector3(v) => format!("Vector3({}, {}, {})", v.x, v.y, v.z),
        Value::Color3(c) => format!("Color3({}, {}, {})", c.r, c.g, c.b),
        Value::UDim2(u) => format!(
            "UDim2({}, {}, {}, {})",
            u.x_scale, u.x_offset, u.y_scale, u.y_offset
        ),
        Value::CFrame(_) => "CFrame".to_string(),
        _ => String::new(),
    }
}

/// Section of a property, in display order.
fn section(prop: &str) -> usize {
    match prop {
        "Color"
        | "Material"
        | "Transparency"
        | "Shape"
        | "BackgroundColor3"
        | "BackgroundTransparency"
        | "TextColor3"
        | "TextSize"
        | "Text"
        | "Visible" => 1,
        "Position" | "Orientation" | "Size" => 2,
        "Anchored" | "CanCollide" => 3,
        "Enabled" => 4,
        _ => 0,
    }
}

const SECTION_NAMES: [&str; 5] = ["Data", "Appearance", "Transform", "Physics", "Behaviour"];

/// Properties panel state.
#[derive(Default)]
pub struct Properties {
    filter: String,
    /// The property of the open live-preview gesture (its values live in the overlay).
    gesture: Option<String>,
    /// Esc cancelled the gesture; ignore the widget until the pointer is released.
    cancelled: bool,
    /// Text being typed into a string field, by property name.
    text: HashMap<String, String>,
    /// The Attributes section: text being typed into string attributes, by attribute name.
    attr_text: HashMap<String, String>,
    /// The Attributes section: name and type (0 Bool, 1 Number, 2 String) of the attribute to add.
    attr_new_name: String,
    attr_new_kind: usize,
    /// A number attribute being scrubbed: its first value, so the whole drag is one undo step.
    attr_gesture: Option<AttrGesture>,
}

/// The open scrub of a number attribute.
struct AttrGesture {
    id: InstanceId,
    name: String,
    /// The value before the drag (`Nil` when it did not exist).
    old: Value,
}

/// What a widget asked for this frame.
enum Edit {
    /// Replace the value on every selected instance.
    Set(String, Value),
    /// Change one component of a vector-like value on each instance (keeps the others).
    Component(String, usize, f64),
}

impl Properties {
    /// An empty panel.
    pub fn new() -> Properties {
        Properties::default()
    }

    /// True while a live-preview gesture is open.
    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// True while a scrub is open or was cancelled with the pointer still held: global keys wait.
    pub fn is_busy(&self) -> bool {
        self.gesture.is_some() || self.cancelled
    }

    /// Esc during a scrub: every previewed value goes back and no step is recorded.
    pub fn cancel_gesture(&mut self, state: &mut EditorState) {
        if self.gesture.take().is_some() {
            state.preview.cancel(&mut state.world);
            self.cancelled = true;
        }
    }

    /// Live value of a number attribute while it is dragged: the world takes it at once and the
    /// history gets nothing until [`Self::end_attribute_scrub`], which records the whole drag as
    /// one step (from the value before the first call).
    pub fn scrub_attribute(
        &mut self,
        state: &mut EditorState,
        id: InstanceId,
        name: &str,
        value: f64,
    ) {
        if self
            .attr_gesture
            .as_ref()
            .is_some_and(|g| g.id != id || g.name != name)
        {
            self.end_attribute_scrub(state);
        }
        if self.attr_gesture.is_none() {
            let old = state
                .world
                .attribute(id, name)
                .cloned()
                .unwrap_or(Value::Nil);
            self.attr_gesture = Some(AttrGesture {
                id,
                name: name.to_string(),
                old,
            });
        }
        let _ = state.world.set_attribute(id, name, Value::Number(value));
    }

    /// Ends the drag of a number attribute and records it as one undo step, if it changed.
    pub fn end_attribute_scrub(&mut self, state: &mut EditorState) {
        let Some(g) = self.attr_gesture.take() else {
            return;
        };
        let new = state
            .world
            .attribute(g.id, &g.name)
            .cloned()
            .unwrap_or(Value::Nil);
        if new != g.old {
            state.record(Command::SetAttribute {
                id: g.id,
                name: g.name,
                old: g.old,
                new,
            });
        }
    }

    /// Shows `value` for `prop` on the selection as a live scrub would (pointer held): it is
    /// previewed, and recorded as one step when the pointer is released. For scripted checks.
    pub fn scrub(&mut self, state: &mut EditorState, prop: &str, value: Value) {
        let ids = state.selection.ids().to_vec();
        self.commit(state, &ids, Edit::Set(prop.to_string(), value), true);
    }

    /// Records the open live-preview gesture as one undo step (pointer released, or Play
    /// starting).
    pub fn finish_gesture(&mut self, state: &mut EditorState) {
        let Some(prop) = self.gesture.take() else {
            return;
        };
        let n = state.preview.len();
        let name = if n == 1 {
            format!("Set {prop}")
        } else {
            format!("Set {prop} ({n} changes)")
        };
        if let Some(cmd) = state.preview.commit(name) {
            state.record(cmd);
        }
    }

    /// Sets `prop` to `value` on every selected instance as one step, exactly as typing the value
    /// into the panel does (clamps and pivot rules included). For scripted checks and commands.
    pub fn set(&mut self, state: &mut EditorState, prop: &str, value: Value) {
        let ids = state.selection.ids().to_vec();
        self.commit(state, &ids, Edit::Set(prop.to_string(), value), false);
    }

    /// Draws the panel.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let pointer_down = ui.input(|i| i.pointer.primary_down());
        if !pointer_down {
            self.finish_gesture(state);
            self.end_attribute_scrub(state);
            self.cancelled = false;
        } else if self.gesture.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_gesture(state);
        }
        let ids: Vec<InstanceId> = state.selection.ids().to_vec();
        let Some(&active) = ids.last() else {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Select something to see its properties.")
                    .color(ui_tokens().text_dim),
            );
            return;
        };
        let class = state.world.class_name(active).unwrap_or("?");
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(
                egui::Vec2::splat(ui_tokens().icon_size),
                egui::Sense::hover(),
            );
            crate::icons::paint_class(ui.painter(), r, class, ui_tokens().icon);
            let title = if ids.len() == 1 {
                state.display_name(active)
            } else {
                format!("{} objects", ids.len())
            };
            ui.label(RichText::new(title).size(ui_tokens().title_size).strong());
            ui.label(
                RichText::new(if ids.len() == 1 { class } else { "" }).color(ui_tokens().text_dim),
            );
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter properties")
                .desired_width(f32::INFINITY),
        );

        // Properties every selected instance has.
        let mut props: Vec<&'static PropDef> = all_properties(class);
        for &id in &ids {
            let c = state.world.class_name(id).unwrap_or("?");
            props.retain(|p| find_property(c, p.name).is_some());
        }
        // Engine bookkeeping (for example a Model's scale) is not shown.
        props.retain(|p| !p.hidden);
        let filter = self.filter.to_lowercase();
        props.retain(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter));

        let mut edits: Vec<Edit> = Vec::new();
        let mut attribute_edit: Option<Command> = None;
        let mut attribute_scrub: Option<AttributeScrub> = None;
        // Inputs in the property column are 24 px rows (spec 4.3).
        ui.spacing_mut().interact_size.y = crate::theme::ROW_HEIGHT;
        ui.spacing_mut().item_spacing.y = 2.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (s, title) in SECTION_NAMES.iter().enumerate() {
                    let in_section: Vec<&PropDef> = props
                        .iter()
                        .copied()
                        .filter(|p| section(p.name) == s)
                        .collect();
                    if in_section.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(
                        RichText::new(*title)
                            .font(crate::theme::fonts::strong())
                            .color(crate::theme::text_2()),
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        for p in in_section {
                            self.row(ui, state, &ids, active, p, &mut edits);
                        }
                    });
                }
                if ids.len() == 1 {
                    let (attr, scrub) = self.attributes_section(ui, state, active);
                    attribute_edit = attr;
                    attribute_scrub = scrub;
                }
            });

        for e in edits {
            self.commit(state, &ids, e, pointer_down);
        }
        match attribute_scrub {
            Some(AttributeScrub::Value(name, value)) => {
                self.scrub_attribute(state, active, &name, value);
            }
            Some(AttributeScrub::End) => self.end_attribute_scrub(state),
            None => {}
        }
        if let Some(cmd) = attribute_edit {
            self.end_attribute_scrub(state);
            let _ = state.apply(cmd);
        }
    }

    /// The Attributes section of one instance: edit, remove and add attributes. Returns the
    /// command for the one change made this frame, if any (each change is one undo step).
    fn attributes_section(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        id: InstanceId,
    ) -> (Option<Command>, Option<AttributeScrub>) {
        let mut out = None;
        let mut scrub = None;
        let attributes: Vec<(String, Value)> = state
            .world
            .attributes(id)
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let set = |name: &str, old: &Value, new: Value| Command::SetAttribute {
            id,
            name: name.to_string(),
            old: old.clone(),
            new,
        };
        egui::CollapsingHeader::new(
            RichText::new("Attributes")
                .font(crate::theme::fonts::strong())
                .color(crate::theme::text_2()),
        )
        .default_open(true)
        .show(ui, |ui| {
            for (name, value) in &attributes {
                ui.horizontal(|ui| {
                    ui.label(name.as_str());
                    match value {
                        Value::Bool(b) => {
                            let mut now = *b;
                            if ui.checkbox(&mut now, "").changed() {
                                out = Some(set(name, value, Value::Bool(now)));
                            }
                        }
                        Value::Number(n) => {
                            let mut now = *n;
                            let response = ui.add(egui::DragValue::new(&mut now).speed(0.1));
                            if response.changed() {
                                if response.dragged() {
                                    // Every frame of a drag: live, one undo step at the end.
                                    scrub = Some(AttributeScrub::Value(name.clone(), now));
                                } else {
                                    out = Some(set(name, value, Value::Number(now)));
                                }
                            }
                            if response.drag_stopped() {
                                scrub = Some(AttributeScrub::End);
                            }
                        }
                        Value::String(text) => {
                            let buffer = self
                                .attr_text
                                .entry(name.clone())
                                .or_insert_with(|| text.clone());
                            let response = ui.add(
                                egui::TextEdit::singleline(buffer).desired_width(f32::INFINITY),
                            );
                            if response.lost_focus() {
                                let typed = std::mem::take(buffer);
                                self.attr_text.remove(name);
                                if typed != *text {
                                    out = Some(set(name, value, Value::String(typed)));
                                }
                            }
                        }
                        other => {
                            ui.label(
                                RichText::new(format!(
                                    "{} (edit in a script)",
                                    attribute_summary(other)
                                ))
                                .color(ui_tokens().text_dim),
                            );
                        }
                    }
                    if ui.small_button("Remove").clicked() {
                        out = Some(set(name, value, Value::Nil));
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.attr_new_name)
                        .hint_text("New attribute name")
                        .desired_width(140.0),
                );
                egui::ComboBox::from_id_salt("attribute-type")
                    .selected_text(["Bool", "Number", "String"][self.attr_new_kind])
                    .show_ui(ui, |ui| {
                        for (i, label) in ["Bool", "Number", "String"].iter().enumerate() {
                            ui.selectable_value(&mut self.attr_new_kind, i, *label);
                        }
                    });
                let name = self.attr_new_name.trim().to_string();
                let problem = metallic_core::check_attribute_name(&name)
                    .err()
                    .or_else(|| {
                        state
                            .world
                            .attribute(id, &name)
                            .map(|_| "that attribute already exists".to_string())
                    });
                let add = ui.add_enabled(problem.is_none(), egui::Button::new("Add"));
                if let Some(problem) = &problem
                    && !name.is_empty()
                {
                    add.on_disabled_hover_text(problem);
                } else if add.clicked() {
                    let value = match self.attr_new_kind {
                        0 => Value::Bool(false),
                        1 => Value::Number(0.0),
                        _ => Value::String(String::new()),
                    };
                    out = Some(set(&name, &Value::Nil, value));
                    self.attr_new_name.clear();
                }
            });
        });
        (out, scrub)
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut EditorState,
        ids: &[InstanceId],
        active: InstanceId,
        p: &PropDef,
        edits: &mut Vec<Edit>,
    ) {
        let Some(value) = read(&state.world, active, p.name) else {
            return;
        };
        let mixed = if p.name == "WorldPivot" {
            ids.iter()
                .any(|&id| read(&state.world, id, p.name).as_ref() != Some(&value))
        } else {
            ids.iter()
                .any(|&id| state.world.get_ref(id, p.name).ok() != Some(&value))
        };
        let playing = state.play.is_playing();
        let hover_bg = ui.painter().add(egui::Shape::Noop);
        let numeric = matches!(
            value,
            Value::Number(_) | Value::Vector3(_) | Value::CFrame(_) | Value::UDim2(_)
        );
        let row = ui.horizontal(|ui| {
            ui.set_min_height(crate::theme::ROW_HEIGHT);
            // Names sit left aligned in a fixed column (about 40 % of the default panel).
            let label_w = ui_tokens().label_width;
            ui.allocate_ui_with_layout(
                egui::vec2(label_w, crate::theme::ROW_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(label_w, crate::theme::ROW_HEIGHT));
                    let label = ui.add(
                        egui::Label::new(RichText::new(p.name).color(crate::theme::text_2()))
                            .truncate(),
                    );
                    label.on_hover_text(format!(
                        "{}.{}",
                        state.world.class_name(active).unwrap_or(""),
                        p.name
                    ));
                },
            );
            if mixed {
                ui.label(RichText::new("Mixed").color(ui_tokens().text_dim).italics());
            }
            let name = p.name.to_string();
            // While playing, values are read-only and drawn disabled.
            ui.add_enabled_ui(!playing, |ui| {
                if numeric {
                    ui.style_mut()
                        .text_styles
                        .insert(egui::TextStyle::Button, ui_tokens().mono());
                }
                match (&value, p.allowed) {
                    (Value::Bool(b), _) => {
                        let mut v = *b;
                        if ui.checkbox(&mut v, "").changed() {
                            edits.push(Edit::Set(name, Value::Bool(v)));
                        }
                    }
                    (Value::Number(n), _) => {
                        let mut v = *n;
                        let cur = *n;
                        let r = ui.add(
                            egui::DragValue::new(&mut v)
                                .speed(drag_speed(p.name))
                                .max_decimals(3)
                                .custom_formatter(fmt_drag)
                                .custom_parser(move |s| eval_relative(s, cur)),
                        );
                        if r.changed() {
                            edits.push(Edit::Set(name, Value::Number(v)));
                        }
                    }
                    (Value::String(s), Some(allowed)) => {
                        let mut chosen = s.clone();
                        egui::ComboBox::from_id_salt(p.name)
                            .selected_text(&chosen)
                            .show_ui(ui, |ui| {
                                for &a in allowed {
                                    ui.selectable_value(&mut chosen, a.to_string(), a);
                                }
                            });
                        if &chosen != s {
                            edits.push(Edit::Set(name, Value::String(chosen)));
                        }
                    }
                    (Value::String(s), None) if p.name == "Source" => {
                        let lines = s.lines().count();
                        if ui.button(format!("Edit script ({lines} lines)")).clicked() {
                            state.request(UiRequest::OpenScript(active));
                        }
                    }
                    (Value::String(s), None) => {
                        let buf = self.text.entry(name.clone()).or_insert_with(|| s.clone());
                        let r =
                            ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
                        if r.lost_focus() {
                            if buf != s {
                                edits.push(Edit::Set(name.clone(), Value::String(buf.clone())));
                            }
                            self.text.remove(&name);
                        } else if !r.has_focus() {
                            *buf = s.clone();
                        }
                    }
                    (Value::Vector3(v), _) => {
                        let comps = [v.x, v.y, v.z];
                        for (axis, &c) in comps.iter().enumerate() {
                            ui.label(
                                RichText::new(["X", "Y", "Z"][axis])
                                    .color(ui_tokens().axis[axis])
                                    .strong(),
                            );
                            let mut x = c;
                            let r = ui.add(
                                egui::DragValue::new(&mut x)
                                    .speed(drag_speed(p.name))
                                    .max_decimals(3)
                                    .custom_formatter(fmt_drag)
                                    .custom_parser(move |s| eval_relative(s, c)),
                            );
                            if r.changed() {
                                edits.push(Edit::Component(name.clone(), axis, x));
                            }
                        }
                    }
                    (Value::CFrame(cf), _) => {
                        // Shown and edited as position plus orientation (degrees, Y then X then Z).
                        let o = cf.orientation_degrees();
                        let rows = [
                            ("Position", [cf.position.x, cf.position.y, cf.position.z], 0),
                            ("Rotation", [o.x, o.y, o.z], 3),
                        ];
                        ui.vertical(|ui| {
                            for (label, comps, base) in rows {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(label).color(ui_tokens().text_dim));
                                    for (axis, &c) in comps.iter().enumerate() {
                                        ui.label(
                                            RichText::new(["X", "Y", "Z"][axis])
                                                .color(ui_tokens().axis[axis])
                                                .strong(),
                                        );
                                        let mut x = c;
                                        let r = ui.add(
                                            egui::DragValue::new(&mut x)
                                                .speed(if base == 0 { 0.1 } else { 1.0 })
                                                .max_decimals(3)
                                                .custom_formatter(fmt_drag)
                                                .custom_parser(move |s| eval_relative(s, c)),
                                        );
                                        if r.changed() {
                                            edits.push(Edit::Component(
                                                name.clone(),
                                                base + axis,
                                                x,
                                            ));
                                        }
                                    }
                                });
                            }
                        });
                    }
                    (Value::Color3(c), _) => {
                        let mut rgb = [to_u8(c.r), to_u8(c.g), to_u8(c.b)];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            edits.push(Edit::Set(
                                name,
                                Value::Color3(Color3::new(
                                    f64::from(rgb[0]),
                                    f64::from(rgb[1]),
                                    f64::from(rgb[2]),
                                )),
                            ));
                        }
                        ui.label(
                            RichText::new(format!("{}, {}, {}", rgb[0], rgb[1], rgb[2]))
                                .monospace()
                                .color(ui_tokens().text_dim),
                        );
                    }
                    (Value::UDim2(u), _) => {
                        let comps = [u.x_scale, u.x_offset, u.y_scale, u.y_offset];
                        for (i, &c) in comps.iter().enumerate() {
                            let mut x = c;
                            let speed = if i % 2 == 0 { 0.01 } else { 1.0 };
                            let r = ui.add(
                                egui::DragValue::new(&mut x)
                                    .speed(speed)
                                    .max_decimals(3)
                                    .custom_formatter(fmt_drag)
                                    .custom_parser(move |s| eval_relative(s, c)),
                            );
                            if r.changed() {
                                edits.push(Edit::Component(name.clone(), i, x));
                            }
                        }
                    }
                    (Value::Ref(r), _) => {
                        let text = match r {
                            Some(id) if state.world.is_alive(*id) => state.display_name(*id),
                            _ => "None".to_string(),
                        };
                        ui.label(RichText::new(text).color(ui_tokens().text_dim));
                    }
                    (Value::Nil, _) => {
                        ui.label(RichText::new("nil").color(ui_tokens().text_dim));
                    }
                }
            });
        });
        if !playing && ui.rect_contains_pointer(row.response.rect) {
            ui.painter().set(
                hover_bg,
                egui::Shape::rect_filled(
                    row.response.rect.expand2(egui::vec2(4.0, 1.0)),
                    egui::CornerRadius::same(crate::theme::R_XS),
                    crate::theme::bg_hover(),
                ),
            );
        }
    }

    fn commit(&mut self, state: &mut EditorState, ids: &[InstanceId], edit: Edit, live: bool) {
        if state.play.is_playing() || (live && self.cancelled) {
            return;
        }
        // A viewport drag owns the overlay; a Properties edit waits for it.
        if !live && state.preview.is_active() {
            return;
        }
        let (prop, values): (String, Vec<(InstanceId, Value)>) = match edit {
            Edit::Set(prop, v) => {
                let vals = ids.iter().map(|&id| (id, v.clone())).collect();
                (prop, vals)
            }
            Edit::Component(prop, axis, x) => {
                let vals = ids
                    .iter()
                    .filter_map(|&id| {
                        let old = read(&state.world, id, &prop)?;
                        Some((id, with_component(&old, axis, x)?))
                    })
                    .collect();
                (prop, vals)
            }
        };
        // Part sizes stay inside the engine's limits, as a script's write would.
        let mut refused = None;
        let values: Vec<(InstanceId, Value)> = values
            .into_iter()
            .filter_map(|(id, v)| match v {
                Value::Vector3(s) if prop == "Size" && state.world.is_a(id, "BasePart") => {
                    Some((id, Value::Vector3(clamp_part_size(s))))
                }
                // A Model's WorldPivot is made orthonormal like core's `set_world_pivot`; one that
                // cannot be (degenerate axes, not finite) is refused, as core refuses it.
                Value::CFrame(cf) if prop == "WorldPivot" => match orthonormal(*cf, "WorldPivot") {
                    Ok(cf) => Some((id, Value::CFrame(Box::new(cf)))),
                    Err(e) => {
                        refused = Some(e.to_string());
                        None
                    }
                },
                v => Some((id, v)),
            })
            .collect();
        if let Some(why) = refused {
            state.note(format!("WorldPivot not set: {why}"));
        }
        // Setting WorldPivot also marks it set (`WorldPivotSet`), as core's `set_world_pivot`
        // does; without that the pivot keeps following the bounding box.
        let marks: Vec<InstanceId> = if prop == "WorldPivot" {
            values
                .iter()
                .map(|(id, _)| *id)
                .filter(|&id| state.world.is_a(id, "Model"))
                .collect()
        } else {
            Vec::new()
        };
        if live {
            let name: PropName = prop.as_str().into();
            let set: PropName = "WorldPivotSet".into();
            for (id, v) in values {
                let _ = state.preview.set(&mut state.world, id, &name, v);
            }
            for id in marks {
                let _ = state
                    .preview
                    .set(&mut state.world, id, &set, Value::Bool(true));
            }
            self.gesture = Some(prop);
            return;
        }
        let mut commands: Vec<Command> = values
            .into_iter()
            .filter_map(|(id, v)| {
                let old = state.world.get(id, &prop).ok()?;
                (old != v).then(|| Command::set(id, &prop, old, v))
            })
            .collect();
        for id in marks {
            if let Ok(old) = state.world.get(id, "WorldPivotSet")
                && old != Value::Bool(true)
            {
                commands.push(Command::set(id, "WorldPivotSet", old, Value::Bool(true)));
            }
        }
        match commands.len() {
            0 => {}
            1 => {
                let _ = state.apply(commands.remove(0));
            }
            n => {
                let _ = state.apply(Command::Batch {
                    name: format!("Set {prop} of {n} objects"),
                    commands,
                });
            }
        }
    }
}

/// The value Properties shows and edits: a Model's `WorldPivot` is its pivot as Luau reads it
/// (the bounding-box centre until set); everything else is the stored value.
fn read(world: &metallic_core::World, id: InstanceId, prop: &str) -> Option<Value> {
    if prop == "WorldPivot" && world.is_a(id, "Model") {
        let cf = metallic_core::transform::world_pivot(world, id).ok()?;
        return Some(Value::CFrame(Box::new(cf)));
    }
    world.get(id, prop).ok()
}

fn drag_speed(prop: &str) -> f64 {
    match prop {
        "Orientation" => 1.0,
        "Transparency" | "BackgroundTransparency" => 0.01,
        _ => 0.1,
    }
}

fn to_u8(c: f64) -> u8 {
    c.round().clamp(0.0, 255.0) as u8
}

/// `value` with component `i` replaced by `x` (Vector3: x, y, z; UDim2: xs, xo, ys, yo;
/// CFrame: position x, y, z then orientation x, y, z in degrees).
fn with_component(value: &Value, i: usize, x: f64) -> Option<Value> {
    match *value {
        Value::Vector3(v) => {
            let mut c = [v.x, v.y, v.z];
            *c.get_mut(i)? = x;
            Some(Value::Vector3(Vec3::new(c[0], c[1], c[2])))
        }
        Value::UDim2(u) => {
            let mut c = [u.x_scale, u.x_offset, u.y_scale, u.y_offset];
            *c.get_mut(i)? = x;
            Some(Value::UDim2(UDim2::new(c[0], c[1], c[2], c[3])))
        }
        // 0..3 position, 3..6 orientation in degrees.
        Value::CFrame(ref cf) => {
            let o = cf.orientation_degrees();
            let mut c = [cf.position.x, cf.position.y, cf.position.z, o.x, o.y, o.z];
            *c.get_mut(i)? = x;
            Some(Value::CFrame(Box::new(CFrame::from_position_orientation(
                Vec3::new(c[0], c[1], c[2]),
                Vec3::new(c[3], c[4], c[5]),
            ))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cframe_edits_position_and_orientation_separately() {
        let cf =
            CFrame::from_position_orientation(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 30.0, 0.0));
        let v = Value::CFrame(Box::new(cf));
        let Some(Value::CFrame(moved)) = with_component(&v, 1, 5.0) else {
            panic!()
        };
        assert_eq!(moved.position, Vec3::new(1.0, 5.0, 3.0));
        assert!((moved.orientation_degrees().y - 30.0).abs() < 1e-9);
        let Some(Value::CFrame(turned)) = with_component(&v, 4, 90.0) else {
            panic!()
        };
        assert_eq!(turned.position, Vec3::new(1.0, 2.0, 3.0));
        assert!((turned.orientation_degrees().y - 90.0).abs() < 1e-9);
    }

    #[test]
    fn component_edit_keeps_other_axes() {
        let v = Value::Vector3(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            with_component(&v, 1, 9.0),
            Some(Value::Vector3(Vec3::new(1.0, 9.0, 3.0)))
        );
        assert_eq!(with_component(&Value::Bool(true), 0, 1.0), None);
    }

    #[test]
    fn sections_keep_transform_together() {
        assert_eq!(section("Position"), section("Size"));
        assert_eq!(SECTION_NAMES[section("Anchored")], "Physics");
        assert_eq!(SECTION_NAMES[section("Name")], "Data");
    }

    #[test]
    fn negative_zero_shows_as_zero() {
        assert_eq!(fmt_drag(-0.0, 0..=3), "0");
        assert_eq!(fmt_drag(-0.0001, 0..=3), "0");
        assert_eq!(fmt_drag(-0.5, 0..=3), "-0.5");
        assert_eq!(fmt_drag(8.0, 1..=3), "8.0");
        assert_eq!(fmt_drag(-22.0, 1..=3), "-22.0");
    }
}
