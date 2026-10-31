//! Properties: every registry property shared by the selected instances, in sections.
//!
//! Sections come in a fixed order: the class's own (Data), Appearance, Transform, Physics,
//! Behaviour. Values that differ across a multi-selection show "Mixed". Number fields scrub on
//! drag and accept expressions (`3*4`, `+2` adds to the current value, see [`crate::expr`]).
//!
//! **One gesture, one undo step:** while the pointer is held (scrubbing a number) changes write
//! the world directly as a live preview, and the first value of each touched property is
//! remembered. When the pointer is released the whole gesture is recorded as one step. Typed
//! values and toggles apply at once as one step each. A Color3 row's swatch opens the colour
//! popover ([`crate::editor_colour`]), which previews and commits the same way.
use std::collections::HashMap;

use eframe::egui::{self, RichText};
use metallic_core::classes::{PropDef, all_properties, find_property};
use metallic_core::{CFrame, InstanceId, UDim2, Value, Vec3};

use crate::commands::Command;
use crate::editor::{EditorState, UiRequest};
use crate::expr::eval_relative;
use crate::tokens::ui_tokens;
use metallic_core::transform::clamp_part_size;

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
    /// Live-preview gesture: first value of every (instance, property) touched.
    gesture: Vec<(InstanceId, String, Value)>,
    /// Text being typed into a string field, by property name.
    text: HashMap<String, String>,
    /// A Color3 row was clicked: the property and the swatch's rectangle.
    colour_request: Option<(String, egui::Rect)>,
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

    /// The Color3 row clicked this frame, if any: the editor opens the colour popover
    /// ([`crate::editor_colour`]) next to `rect` for that property.
    pub fn take_colour_request(&mut self) -> Option<(String, egui::Rect)> {
        self.colour_request.take()
    }

    /// True while a live-preview gesture is open.
    pub fn in_gesture(&self) -> bool {
        !self.gesture.is_empty()
    }

    fn finish_gesture(&mut self, state: &mut EditorState) {
        if self.gesture.is_empty() {
            return;
        }
        let mut commands: Vec<Command> = self
            .gesture
            .drain(..)
            .filter_map(|(id, prop, old)| {
                let new = state.world.get(id, &prop).ok()?;
                (new != old).then(|| Command::set(id, &prop, old, new))
            })
            .collect();
        match commands.len() {
            0 => {}
            1 => state.record(commands.remove(0)),
            n => {
                let prop = match &commands[0] {
                    Command::SetProperty { prop, .. } => prop.clone(),
                    _ => String::new(),
                };
                state.record(Command::Batch {
                    name: format!("Set {prop} ({n} changes)"),
                    commands,
                });
            }
        }
    }

    /// Draws the panel.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let pointer_down = ui.input(|i| i.pointer.primary_down());
        if !pointer_down {
            self.finish_gesture(state);
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
                    egui::CollapsingHeader::new(RichText::new(*title).strong())
                        .default_open(true)
                        .show(ui, |ui| {
                            for p in in_section {
                                self.row(ui, state, &ids, active, p, &mut edits);
                            }
                        });
                }
            });

        for e in edits {
            self.commit(state, &ids, e, pointer_down);
        }
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
        let Ok(value) = state.world.get(active, p.name) else {
            return;
        };
        let mixed = ids
            .iter()
            .any(|&id| state.world.get_ref(id, p.name).ok() != Some(&value));
        ui.horizontal(|ui| {
            let label = ui.add_sized(
                [ui_tokens().label_width, 20.0],
                egui::Label::new(RichText::new(p.name).color(ui_tokens().text_dim)).truncate(),
            );
            label.on_hover_text(format!(
                "{}.{}",
                state.world.class_name(active).unwrap_or(""),
                p.name
            ));
            if mixed {
                ui.label(RichText::new("Mixed").color(ui_tokens().text_dim).italics());
            }
            let name = p.name.to_string();
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
                    let r = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
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
                                            .custom_parser(move |s| eval_relative(s, c)),
                                    );
                                    if r.changed() {
                                        edits.push(Edit::Component(name.clone(), base + axis, x));
                                    }
                                }
                            });
                        }
                    });
                }
                (Value::Color3(c), _) => {
                    let rgb = [to_u8(c.r), to_u8(c.g), to_u8(c.b)];
                    // The swatch shows the property's own colour; a click opens the colour popover.
                    let swatch = egui::Button::new("")
                        .fill(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                        .min_size(egui::vec2(36.0, 18.0));
                    let r = ui
                        .add(swatch)
                        .on_hover_text(format!("Edit {name} in the colour popover"));
                    let label = format!("{name} swatch");
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                    if r.clicked() && !state.play.is_playing() {
                        self.colour_request = Some((name.clone(), r.rect));
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
    }

    fn commit(&mut self, state: &mut EditorState, ids: &[InstanceId], edit: Edit, live: bool) {
        if state.play.is_playing() {
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
                        let old = state.world.get(id, &prop).ok()?;
                        Some((id, with_component(&old, axis, x)?))
                    })
                    .collect();
                (prop, vals)
            }
        };
        // Part sizes stay inside the engine's limits, as a script's write would.
        let values: Vec<(InstanceId, Value)> = values
            .into_iter()
            .map(|(id, v)| match v {
                Value::Vector3(s) if prop == "Size" && state.world.is_a(id, "BasePart") => {
                    (id, Value::Vector3(clamp_part_size(s)))
                }
                v => (id, v),
            })
            .collect();
        if live {
            for (id, v) in values {
                if !self
                    .gesture
                    .iter()
                    .any(|(gid, gp, _)| *gid == id && *gp == prop)
                    && let Ok(old) = state.world.get(id, &prop)
                {
                    self.gesture.push((id, prop.clone(), old));
                }
                let _ = state.world.set(id, &prop, v);
            }
            return;
        }
        let mut commands: Vec<Command> = values
            .into_iter()
            .filter_map(|(id, v)| {
                let old = state.world.get(id, &prop).ok()?;
                (old != v).then(|| Command::set(id, &prop, old, v))
            })
            .collect();
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
}
