//! The module stack inspector (the default, Niagara-like view).
//!
//! An emitter shows as four stages (Spawn, Initialize, Update, Render). Each module is a card with an
//! on/off switch, a collapsible body of property rows and a menu to move or remove it. "+" on a stage adds
//! a module with sensible defaults. Trails, beams and effect settings use the same rows.

use crate::preview::Preview;
use crate::theme::{self, color};
use crate::widgets::curve::{curve_edit, interp_combo};
use crate::widgets::gradient::gradient_edit;
use crate::widgets::{self, Icon, check, num, range, row, vec3};
use egui::{CornerRadius, Margin, RichText, Ui};
use metallic_vfx::effect::PreviewMotion;
use metallic_vfx::{
    ArtStyle, Beam, Curve, Direction, Effect, Emitter, Facing, Flipbook, FlipbookMode, Gradient,
    HitAction, InOut, InitModule, Range, RenderKind, Rgb, Shape, SimSpace, SortMode, SpawnModule,
    Texture, TextureMode, Trail, UpdateModule, Vec3,
};

/// A stage of the emitter stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Spawn modules.
    Spawn,
    /// Initialise modules.
    Init,
    /// Update modules.
    Update,
    /// The render block.
    Render,
}

impl Stage {
    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Stage::Spawn => "Spawn",
            Stage::Init => "Initialize",
            Stage::Update => "Update",
            Stage::Render => "Render",
        }
    }
}

enum CardAction {
    None,
    Up,
    Down,
    Remove,
}

/// Card chrome around one module. Returns (changed, action).
fn card(
    ui: &mut Ui,
    id: egui::Id,
    title: &str,
    enabled: &mut bool,
    focused: bool,
    body: impl FnOnce(&mut Ui) -> bool,
) -> (bool, CardAction) {
    let mut changed = false;
    let mut action = CardAction::None;
    let stroke = if focused {
        egui::Stroke::new(1.0, color::ACCENT)
    } else {
        egui::Stroke::NONE
    };
    egui::Frame::new()
        .fill(color::BG_RAISED)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut state =
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
            if focused {
                state.set_open(true);
            }
            let open = state.is_open();
            ui.horizontal(|ui| {
                let icon = if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                };
                let (r, resp) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::click());
                widgets::paint_icon(ui.painter(), r, icon, color::ICON);
                if resp.clicked() {
                    state.toggle(ui);
                }
                changed |= check(ui, enabled).changed();
                let text_color = if *enabled { color::TEXT } else { color::TEXT_3 };
                let label = ui.add(
                    egui::Label::new(RichText::new(title).font(theme::semibold(13.0)).color(text_color))
                        .sense(egui::Sense::click()),
                );
                if label.clicked() {
                    state.toggle(ui);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("···").color(color::ICON), |ui| {
                        if ui.button("Move up").clicked() {
                            action = CardAction::Up;
                            ui.close();
                        }
                        if ui.button("Move down").clicked() {
                            action = CardAction::Down;
                            ui.close();
                        }
                        if ui.button("Remove").clicked() {
                            action = CardAction::Remove;
                            ui.close();
                        }
                    });
                });
            });
            state.store(ui.ctx());
            if open {
                ui.add_space(4.0);
                ui.add_enabled_ui(*enabled, |ui| {
                    changed |= body(ui);
                });
            }
        });
    ui.add_space(4.0);
    (changed, action)
}

fn apply_action<T>(list: &mut Vec<T>, i: usize, action: CardAction) -> bool {
    match action {
        CardAction::None => false,
        CardAction::Up if i > 0 => {
            list.swap(i, i - 1);
            true
        }
        CardAction::Down if i + 1 < list.len() => {
            list.swap(i, i + 1);
            true
        }
        CardAction::Remove => {
            list.remove(i);
            true
        }
        _ => false,
    }
}

fn stage_header(ui: &mut Ui, stage: Stage, count: usize, add: impl FnOnce(&mut Ui) -> bool) -> bool {
    let mut changed = false;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(stage.label()).font(theme::semibold(14.0)).color(color::TEXT));
        if count > 0 {
            ui.label(RichText::new(count.to_string()).color(color::TEXT_3));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new("+ Add").color(color::ICON), |ui| {
                changed = add(ui);
            });
        });
    });
    ui.add_space(2.0);
    changed
}

/// Effect-level settings (name, style, cycle).
pub fn effect_settings(ui: &mut Ui, e: &mut Effect) -> bool {
    let mut changed = false;
    row(ui, "Name", |ui| {
        changed |= ui.text_edit_singleline(&mut e.name).changed();
    });
    row(ui, "Description", |ui| {
        changed |= ui.text_edit_singleline(&mut e.description).changed();
    });
    row(ui, "Authored for", |ui| {
        egui::ComboBox::from_id_salt("style")
            .selected_text(e.style.label())
            .show_ui(ui, |ui| {
                for s in ArtStyle::ALL {
                    changed |= ui.selectable_value(&mut e.style, s, s.label()).changed();
                }
            });
    });
    row(ui, "Cycle", |ui| {
        changed |= num(ui, &mut e.duration, 0.01, 0.0..=60.0).changed();
        ui.label(RichText::new("s").color(color::TEXT_3));
        changed |= ui.checkbox(&mut e.looping, "Loop").changed();
    });
    row(ui, "Preview motion", |ui| {
        egui::ComboBox::from_id_salt("motion")
            .selected_text(e.preview.label())
            .show_ui(ui, |ui| {
                for m in PreviewMotion::ALL {
                    changed |= ui.selectable_value(&mut e.preview, m, m.label()).changed();
                }
            });
    });
    changed
}

/// The full module stack for one emitter. `focus` expands and outlines one module.
pub fn emitter_stack(
    ui: &mut Ui,
    em: &mut Emitter,
    key: usize,
    focus: Option<(Stage, usize)>,
    preview: &mut Preview,
) -> bool {
    let mut changed = false;
    let base = egui::Id::unique(("emitter", key));
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(color::BG_RAISED)
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            row(ui, "Name", |ui| {
                changed |= ui.text_edit_singleline(&mut em.name).changed();
            });
            row(ui, "Capacity", |ui| {
                let mut c = em.capacity as f32;
                if num(ui, &mut c, 4.0, 1.0..=262_144.0).changed() {
                    em.capacity = c.round().max(1.0) as u32;
                    changed = true;
                }
                ui.label(RichText::new("particles").color(color::TEXT_3));
            });
            row(ui, "Space", |ui| {
                changed |= ui.selectable_value(&mut em.space, SimSpace::World, "World").changed();
                changed |= ui.selectable_value(&mut em.space, SimSpace::Local, "Local").changed();
            });
            row(ui, "Time scale", |ui| {
                changed |= num(ui, &mut em.time_scale, 0.01, 0.0..=4.0).changed();
                ui.label(RichText::new("delay").color(color::TEXT_3));
                changed |= num(ui, &mut em.delay, 0.01, 0.0..=60.0).changed();
            });
            row(ui, "Step rate", |ui| {
                changed |= num(ui, &mut em.fixed_fps, 1.0, 0.0..=60.0).changed();
                ui.label(RichText::new(if em.fixed_fps > 0.0 { "fps" } else { "every frame" }).color(color::TEXT_3));
            });
        });

    // Spawn.
    changed |= stage_header(ui, Stage::Spawn, em.spawn.len(), |ui| {
        let mut added = false;
        for (label, m) in [
            ("Spawn rate", SpawnModule::Rate { per_second: 20.0, disabled: false }),
            (
                "Burst",
                SpawnModule::Burst { time: 0.0, count: 20, cycles: 1, interval: 0.0, disabled: false },
            ),
            ("Spawn by distance", SpawnModule::Distance { per_stud: 2.0, disabled: false }),
        ] {
            if ui.button(label).clicked() {
                em.spawn.push(m);
                added = true;
                ui.close();
            }
        }
        added
    });
    let mut i = 0;
    while i < em.spawn.len() {
        let focused = focus == Some((Stage::Spawn, i));
        let m = &mut em.spawn[i];
        let title = m.label();
        let mut on = !m.is_disabled();
        let (c, action) = card(ui, base.with(("spawn", i)), title, &mut on, focused, |ui| spawn_body(ui, m));
        *em.spawn[i].disabled_mut() = !on;
        changed |= c;
        if apply_action(&mut em.spawn, i, action) {
            changed = true;
            continue;
        }
        i += 1;
    }

    // Init.
    changed |= stage_header(ui, Stage::Init, em.init.len(), |ui| {
        let mut added = false;
        for m in init_templates() {
            if ui.button(m.label()).clicked() {
                em.init.push(m);
                added = true;
                ui.close();
            }
        }
        added
    });
    let mut i = 0;
    while i < em.init.len() {
        let focused = focus == Some((Stage::Init, i));
        let m = &mut em.init[i];
        let title = m.label();
        let mut on = !m.is_disabled();
        let (c, action) =
            card(ui, base.with(("init", i)), title, &mut on, focused, |ui| init_body(ui, m, (key, i)));
        *em.init[i].disabled_mut() = !on;
        changed |= c;
        if apply_action(&mut em.init, i, action) {
            changed = true;
            continue;
        }
        i += 1;
    }

    // Update.
    changed |= stage_header(ui, Stage::Update, em.update.len(), |ui| {
        let mut added = false;
        for m in update_templates() {
            if ui.button(m.label()).clicked() {
                em.update.push(m);
                added = true;
                ui.close();
            }
        }
        added
    });
    let mut i = 0;
    while i < em.update.len() {
        let focused = focus == Some((Stage::Update, i));
        let m = &mut em.update[i];
        let title = m.label();
        let mut on = !m.is_disabled();
        let (c, action) =
            card(ui, base.with(("update", i)), title, &mut on, focused, |ui| update_body(ui, m, (key, i)));
        *em.update[i].disabled_mut() = !on;
        changed |= c;
        if apply_action(&mut em.update, i, action) {
            changed = true;
            continue;
        }
        i += 1;
    }

    // Render.
    stage_header(ui, Stage::Render, 0, |ui| {
        ui.label(RichText::new("One renderer per emitter").color(color::TEXT_3));
        false
    });
    let mut on = em.enabled;
    let focused = focus.is_some_and(|f| f.0 == Stage::Render);
    let (c, _) = card(ui, base.with("render"), "Renderer", &mut on, focused, |ui| {
        render_body(ui, &mut em.render, key, preview)
    });
    changed |= c || on != em.enabled;
    em.enabled = on;
    changed
}

fn spawn_body(ui: &mut Ui, m: &mut SpawnModule) -> bool {
    let mut changed = false;
    match m {
        SpawnModule::Rate { per_second, .. } => row(ui, "Rate", |ui| {
            changed |= num(ui, per_second, 0.5, 0.0..=10_000.0).changed();
            ui.label(RichText::new("per second").color(color::TEXT_3));
        }),
        SpawnModule::Burst {
            time,
            count,
            cycles,
            interval,
            ..
        } => {
            row(ui, "At", |ui| {
                changed |= num(ui, time, 0.01, 0.0..=60.0).changed();
                ui.label(RichText::new("s").color(color::TEXT_3));
            });
            row(ui, "Count", |ui| {
                changed |= ui.add(egui::DragValue::new(count).range(0..=100_000)).changed();
            });
            row(ui, "Repeat", |ui| {
                changed |= ui.add(egui::DragValue::new(cycles).range(1..=1000)).changed();
                ui.label(RichText::new("every").color(color::TEXT_3));
                changed |= num(ui, interval, 0.01, 0.0..=60.0).changed();
                ui.label(RichText::new("s").color(color::TEXT_3));
            });
        }
        SpawnModule::Distance { per_stud, .. } => row(ui, "Per stud", |ui| {
            changed |= num(ui, per_stud, 0.05, 0.0..=100.0).changed();
        }),
    }
    changed
}

fn init_templates() -> Vec<InitModule> {
    vec![
        InitModule::Shape {
            shape: Shape::Sphere { radius: 1.0, partial: 1.0 },
            surface: false,
            rotation: Vec3::ZERO,
            offset: Vec3::ZERO,
            disabled: false,
        },
        InitModule::Lifetime { seconds: Range::new(1.0, 2.0), disabled: false },
        InitModule::Velocity {
            speed: Range::new(2.0, 4.0),
            direction: Direction::Axis(Vec3::Y),
            spread: (30.0, 30.0),
            disabled: false,
        },
        InitModule::Size { size: Range::new(0.5, 1.0), disabled: false },
        InitModule::Color { choices: vec![Rgb::WHITE], mix: false, disabled: false },
        InitModule::Rotation { angle: Range::new(0.0, 360.0), spin: Range::new(-90.0, 90.0), disabled: false },
        InitModule::InheritVelocity { amount: 0.5, disabled: false },
    ]
}

fn update_templates() -> Vec<UpdateModule> {
    vec![
        UpdateModule::Acceleration { accel: Vec3::new(0.0, -9.8, 0.0), disabled: false },
        UpdateModule::Drag { drag: 1.0, disabled: false },
        UpdateModule::CurlNoise { strength: 2.0, scale: 2.0, scroll: Vec3::ZERO, octaves: 2, disabled: false },
        UpdateModule::Vortex { axis: Vec3::Y, center: Vec3::ZERO, swirl: 4.0, pull: 0.0, disabled: false },
        UpdateModule::Attractor { position: Vec3::new(0.0, 2.0, 0.0), strength: 5.0, radius: 2.0, kill_radius: 0.0, disabled: false },
        UpdateModule::SpeedLimit { max: 10.0, disabled: false },
        UpdateModule::CollisionPlane {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            bounce: 0.4,
            friction: 0.2,
            on_hit: HitAction::Bounce,
            disabled: false,
        },
        UpdateModule::SizeOverLife { curve: Curve::smooth(&[(0.0, 0.0), (0.2, 1.0), (1.0, 0.0)]), disabled: false },
        UpdateModule::ColorOverLife { gradient: Gradient::hex(&[(0.0, 0xFFFFFF), (1.0, 0x9D8BE0)]), disabled: false },
        UpdateModule::TransparencyOverLife { curve: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]), disabled: false },
        UpdateModule::SquashOverLife { curve: Curve::constant(0.0), disabled: false },
        UpdateModule::ErosionOverLife { curve: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]), disabled: false },
    ]
}

fn shape_label(s: &Shape) -> &'static str {
    match s {
        Shape::Point => "Point",
        Shape::Sphere { .. } => "Sphere",
        Shape::Box { .. } => "Box",
        Shape::Disc { .. } => "Disc",
        Shape::Cylinder { .. } => "Cylinder",
        Shape::Cone { .. } => "Cone",
        Shape::Edge { .. } => "Edge",
        Shape::MeshSurface { .. } => "Mesh surface",
    }
}

fn init_body(ui: &mut Ui, m: &mut InitModule, key: (usize, usize)) -> bool {
    let mut changed = false;
    match m {
        InitModule::Shape {
            shape,
            surface,
            rotation,
            offset,
            ..
        } => {
            row(ui, "Shape", |ui| {
                egui::ComboBox::from_id_salt(("shape", key))
                    .selected_text(shape_label(shape))
                    .show_ui(ui, |ui| {
                        for s in [
                            Shape::Point,
                            Shape::Sphere { radius: 1.0, partial: 1.0 },
                            Shape::Box { size: Vec3::ONE },
                            Shape::Disc { radius: 1.0, inner: 0.0 },
                            Shape::Cylinder { radius: 1.0, height: 1.0, top: 1.0 },
                            Shape::Cone { radius: 0.5, angle: 25.0 },
                            Shape::Edge { a: Vec3::new(-1.0, 0.0, 0.0), b: Vec3::new(1.0, 0.0, 0.0) },
                            Shape::MeshSurface { mesh: String::new() },
                        ] {
                            let same = std::mem::discriminant(shape) == std::mem::discriminant(&s);
                            if ui.selectable_label(same, shape_label(&s)).clicked() && !same {
                                *shape = s;
                                changed = true;
                            }
                        }
                    });
                changed |= ui.checkbox(surface, "Surface").changed();
            });
            match shape {
                Shape::Point => {}
                Shape::Sphere { radius, partial } => {
                    row(ui, "Radius", |ui| changed |= num(ui, radius, 0.02, 0.0..=100.0).changed());
                    row(ui, "Portion", |ui| {
                        changed |= ui.add(egui::Slider::new(partial, 0.0..=1.0)).changed();
                    });
                }
                Shape::Box { size } => row(ui, "Size", |ui| changed |= vec3(ui, size, 0.05)),
                Shape::Disc { radius, inner } => {
                    row(ui, "Radius", |ui| changed |= num(ui, radius, 0.02, 0.0..=100.0).changed());
                    row(ui, "Inner", |ui| changed |= ui.add(egui::Slider::new(inner, 0.0..=1.0)).changed());
                }
                Shape::Cylinder { radius, height, top } => {
                    row(ui, "Radius", |ui| changed |= num(ui, radius, 0.02, 0.0..=100.0).changed());
                    row(ui, "Height", |ui| changed |= num(ui, height, 0.02, 0.0..=100.0).changed());
                    row(ui, "Top", |ui| changed |= ui.add(egui::Slider::new(top, 0.0..=1.0)).changed());
                }
                Shape::Cone { radius, angle } => {
                    row(ui, "Radius", |ui| changed |= num(ui, radius, 0.02, 0.0..=100.0).changed());
                    row(ui, "Angle", |ui| {
                        changed |= ui.add(egui::Slider::new(angle, 0.0..=90.0).suffix("°")).changed();
                    });
                }
                Shape::Edge { a, b } => {
                    row(ui, "From", |ui| changed |= vec3(ui, a, 0.05));
                    row(ui, "To", |ui| changed |= vec3(ui, b, 0.05));
                }
                Shape::MeshSurface { mesh } => {
                    row(ui, "Mesh", |ui| changed |= ui.text_edit_singleline(mesh).changed());
                    ui.label(RichText::new("Preview emits from a sphere until mesh assets exist.").color(color::TEXT_3));
                }
            }
            row(ui, "Offset", |ui| changed |= vec3(ui, offset, 0.05));
            row(ui, "Rotation", |ui| changed |= vec3(ui, rotation, 1.0));
        }
        InitModule::Lifetime { seconds, .. } => row(ui, "Seconds", |ui| {
            changed |= range(ui, seconds, 0.01, 0.0..=20.0);
        }),
        InitModule::Velocity {
            speed,
            direction,
            spread,
            ..
        } => {
            row(ui, "Speed", |ui| changed |= range(ui, speed, 0.05, -500.0..=500.0));
            row(ui, "Direction", |ui| {
                let mut from_shape = matches!(direction, Direction::FromShape(_));
                if ui.selectable_label(!from_shape, "Axis").clicked() && from_shape {
                    *direction = Direction::Axis(Vec3::Y);
                    from_shape = false;
                    changed = true;
                }
                if ui.selectable_label(from_shape, "From shape").clicked() && !from_shape {
                    *direction = Direction::FromShape(InOut::Outward);
                    changed = true;
                }
            });
            match direction {
                Direction::Axis(v) => row(ui, "Axis", |ui| changed |= vec3(ui, v, 0.02)),
                Direction::FromShape(io) => row(ui, "Way", |ui| {
                    for (v, l) in [(InOut::Outward, "Out"), (InOut::Inward, "In"), (InOut::Both, "Both")] {
                        changed |= ui.selectable_value(io, v, l).changed();
                    }
                }),
            }
            row(ui, "Spread", |ui| {
                changed |= num(ui, &mut spread.0, 0.5, 0.0..=360.0).changed();
                changed |= num(ui, &mut spread.1, 0.5, 0.0..=360.0).changed();
                ui.label(RichText::new("degrees").color(color::TEXT_3));
            });
        }
        InitModule::Size { size, .. } => row(ui, "Studs", |ui| changed |= range(ui, size, 0.01, 0.0..=100.0)),
        InitModule::Color { choices, mix, .. } => {
            row(ui, "Colors", |ui| {
                let mut remove = None;
                for (i, c) in choices.iter_mut().enumerate() {
                    let r = widgets::color_button(ui, c);
                    changed |= r;
                    if ui.small_button("×").on_hover_text("Remove").clicked() && i > 0 {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    choices.remove(i);
                    changed = true;
                }
                if ui.small_button("+").clicked() {
                    choices.push(choices.last().copied().unwrap_or(Rgb::WHITE));
                    changed = true;
                }
            });
            row(ui, "Pick", |ui| {
                changed |= ui.selectable_value(mix, false, "One of").changed();
                changed |= ui.selectable_value(mix, true, "Blend").changed();
            });
        }
        InitModule::Rotation { angle, spin, .. } => {
            row(ui, "Angle", |ui| changed |= range(ui, angle, 1.0, -720.0..=720.0));
            row(ui, "Spin", |ui| changed |= range(ui, spin, 1.0, -2000.0..=2000.0));
        }
        InitModule::InheritVelocity { amount, .. } => row(ui, "Amount", |ui| {
            changed |= ui.add(egui::Slider::new(amount, 0.0..=1.0)).changed();
        }),
    }
    changed
}

fn curve_rows(ui: &mut Ui, label: &str, curve: &mut Curve, key: impl std::hash::Hash + std::fmt::Debug + Copy) -> bool {
    let mut changed = false;
    row(ui, label, |ui| {
        changed |= interp_combo(ui, &mut curve.interp, key);
    });
    changed |= curve_edit(ui, curve, key, 70.0);
    changed
}

fn update_body(ui: &mut Ui, m: &mut UpdateModule, key: (usize, usize)) -> bool {
    let mut changed = false;
    match m {
        UpdateModule::Acceleration { accel, .. } => row(ui, "Studs/s²", |ui| changed |= vec3(ui, accel, 0.1)),
        UpdateModule::Drag { drag, .. } => row(ui, "Drag", |ui| {
            changed |= num(ui, drag, 0.02, -20.0..=50.0).changed();
            ui.label(RichText::new("halves speed every 1/drag s").color(color::TEXT_3));
        }),
        UpdateModule::CurlNoise {
            strength,
            scale,
            scroll,
            octaves,
            ..
        } => {
            row(ui, "Strength", |ui| changed |= num(ui, strength, 0.05, 0.0..=200.0).changed());
            row(ui, "Swirl size", |ui| changed |= num(ui, scale, 0.05, 0.05..=100.0).changed());
            row(ui, "Drift", |ui| changed |= vec3(ui, scroll, 0.05));
            row(ui, "Detail", |ui| {
                changed |= ui.add(egui::Slider::new(octaves, 1..=4)).changed();
            });
        }
        UpdateModule::Vortex {
            axis,
            center,
            swirl,
            pull,
            ..
        } => {
            row(ui, "Axis", |ui| changed |= vec3(ui, axis, 0.02));
            row(ui, "Center", |ui| changed |= vec3(ui, center, 0.05));
            row(ui, "Swirl", |ui| changed |= num(ui, swirl, 0.1, -200.0..=200.0).changed());
            row(ui, "Pull", |ui| changed |= num(ui, pull, 0.1, -200.0..=200.0).changed());
        }
        UpdateModule::Attractor {
            position,
            strength,
            radius,
            kill_radius,
            ..
        } => {
            row(ui, "Position", |ui| changed |= vec3(ui, position, 0.05));
            row(ui, "Strength", |ui| changed |= num(ui, strength, 0.1, -500.0..=500.0).changed());
            row(ui, "Radius", |ui| changed |= num(ui, radius, 0.05, 0.01..=100.0).changed());
            row(ui, "Kill radius", |ui| changed |= num(ui, kill_radius, 0.02, 0.0..=100.0).changed());
        }
        UpdateModule::SpeedLimit { max, .. } => row(ui, "Max speed", |ui| {
            changed |= num(ui, max, 0.1, 0.0..=1000.0).changed();
        }),
        UpdateModule::CollisionPlane {
            point,
            normal,
            bounce,
            friction,
            on_hit,
            ..
        } => {
            row(ui, "Point", |ui| changed |= vec3(ui, point, 0.05));
            row(ui, "Normal", |ui| changed |= vec3(ui, normal, 0.02));
            row(ui, "On hit", |ui| {
                for (v, l) in [(HitAction::Bounce, "Bounce"), (HitAction::Stick, "Stick"), (HitAction::Kill, "Kill")] {
                    changed |= ui.selectable_value(on_hit, v, l).changed();
                }
            });
            row(ui, "Bounce", |ui| changed |= ui.add(egui::Slider::new(bounce, 0.0..=1.0)).changed());
            row(ui, "Friction", |ui| changed |= ui.add(egui::Slider::new(friction, 0.0..=1.0)).changed());
        }
        UpdateModule::SizeOverLife { curve, .. } => changed |= curve_rows(ui, "Size ×", curve, key),
        UpdateModule::TransparencyOverLife { curve, .. } => changed |= curve_rows(ui, "Transparency", curve, key),
        UpdateModule::SquashOverLife { curve, .. } => changed |= curve_rows(ui, "Squash", curve, key),
        UpdateModule::ErosionOverLife { curve, .. } => changed |= curve_rows(ui, "Erosion", curve, key),
        UpdateModule::ColorOverLife { gradient, .. } => changed |= gradient_edit(ui, gradient, key),
    }
    changed
}

fn kind_label(k: &RenderKind) -> &'static str {
    match k {
        RenderKind::Billboard => "Billboard",
        RenderKind::Stretched { .. } => "Stretched",
        RenderKind::Ribbon { .. } => "Ribbon",
        RenderKind::Mesh { .. } => "Mesh",
    }
}

fn facing_label(f: Facing) -> &'static str {
    match f {
        Facing::Camera => "Facing camera",
        Facing::CameraWorldUp => "Camera, upright",
        Facing::VelocityParallel => "Along velocity",
        Facing::VelocityPerpendicular => "Across velocity",
        Facing::Ground => "Flat on ground",
    }
}

/// Texture picker: a grid of tiles in a popup.
fn texture_picker(ui: &mut Ui, t: &mut Texture, key: usize, preview: &mut Preview) -> bool {
    let mut changed = false;
    let ctx = ui.ctx().clone();
    let current = preview.texture_id(&ctx, t);
    ui.horizontal(|ui| {
        let tile = egui::Image::new((current, egui::Vec2::splat(28.0))).bg_fill(color::BG_DEEP);
        ui.add(tile);
        egui::ComboBox::from_id_salt(("tex", key))
            .selected_text(t.label().to_string())
            .height(420.0)
            .show_ui(ui, |ui| {
                egui::Grid::new(("texgrid", key)).spacing([6.0, 6.0]).show(ui, |ui| {
                    for (i, cand) in Texture::BUILTIN.iter().enumerate() {
                        let id = preview.texture_id(&ctx, cand);
                        let selected = cand == t;
                        let img = egui::Image::new((id, egui::Vec2::splat(36.0))).bg_fill(color::BG_DEEP);
                        let resp = ui
                            .add(egui::Button::image(img).selected(selected))
                            .on_hover_text(cand.label());
                        if resp.clicked() {
                            *t = cand.clone();
                            changed = true;
                        }
                        if i % 5 == 4 {
                            ui.end_row();
                        }
                    }
                });
            });
    });
    changed
}

fn render_body(ui: &mut Ui, r: &mut metallic_vfx::Render, key: usize, preview: &mut Preview) -> bool {
    let mut changed = false;
    row(ui, "Draw as", |ui| {
        egui::ComboBox::from_id_salt(("kind", key))
            .selected_text(kind_label(&r.kind))
            .show_ui(ui, |ui| {
                for k in [
                    RenderKind::Billboard,
                    RenderKind::Stretched { speed_scale: 0.05, min_length: 1.0 },
                    RenderKind::Ribbon { texture_mode: TextureMode::Stretch, texture_length: 1.0 },
                    RenderKind::Mesh { mesh: String::new() },
                ] {
                    let same = std::mem::discriminant(&r.kind) == std::mem::discriminant(&k);
                    if ui.selectable_label(same, kind_label(&k)).clicked() && !same {
                        r.kind = k;
                        changed = true;
                    }
                }
            });
    });
    match &mut r.kind {
        RenderKind::Stretched { speed_scale, min_length } => {
            row(ui, "Stretch", |ui| {
                changed |= num(ui, speed_scale, 0.002, 0.0..=2.0).changed();
                ui.label(RichText::new("min").color(color::TEXT_3));
                changed |= num(ui, min_length, 0.02, 0.0..=10.0).changed();
            });
        }
        RenderKind::Ribbon { texture_mode, texture_length } => {
            row(ui, "Texture", |ui| {
                for (v, l) in [(TextureMode::Stretch, "Stretch"), (TextureMode::Wrap, "Wrap"), (TextureMode::Static, "Static")] {
                    changed |= ui.selectable_value(texture_mode, v, l).changed();
                }
                changed |= num(ui, texture_length, 0.05, 0.01..=100.0).changed();
            });
        }
        RenderKind::Mesh { mesh } => {
            row(ui, "Mesh", |ui| changed |= ui.text_edit_singleline(mesh).changed());
        }
        RenderKind::Billboard => {}
    }
    row(ui, "Facing", |ui| {
        egui::ComboBox::from_id_salt(("facing", key))
            .selected_text(facing_label(r.facing))
            .show_ui(ui, |ui| {
                for f in [
                    Facing::Camera,
                    Facing::CameraWorldUp,
                    Facing::VelocityParallel,
                    Facing::VelocityPerpendicular,
                    Facing::Ground,
                ] {
                    changed |= ui.selectable_value(&mut r.facing, f, facing_label(f)).changed();
                }
            });
    });
    row(ui, "Texture", |ui| changed |= texture_picker(ui, &mut r.texture, key, preview));
    let sheet = r.texture.sheet();
    let mut has_fb = r.flipbook.is_some();
    row(ui, "Flipbook", |ui| {
        if ui.checkbox(&mut has_fb, "").changed() {
            r.flipbook = has_fb.then(|| {
                let (columns, rows) = sheet.unwrap_or((4, 4));
                Flipbook {
                    columns,
                    rows,
                    mode: FlipbookMode::Loop,
                    fps: Range::constant(12.0),
                    start_random: false,
                    blend: true,
                }
            });
            changed = true;
        }
        if let Some(fb) = &mut r.flipbook {
            egui::ComboBox::from_id_salt(("fbmode", key))
                .width(90.0)
                .selected_text(format!("{:?}", fb.mode))
                .show_ui(ui, |ui| {
                    for m in [FlipbookMode::Loop, FlipbookMode::OneShot, FlipbookMode::PingPong, FlipbookMode::Random] {
                        changed |= ui.selectable_value(&mut fb.mode, m, format!("{m:?}")).changed();
                    }
                });
        } else if sheet.is_some() {
            ui.label(RichText::new("sheet plays once per life").color(color::TEXT_3));
        }
    });
    if let Some(fb) = &mut r.flipbook {
        row(ui, "Grid", |ui| {
            changed |= ui.add(egui::DragValue::new(&mut fb.columns).range(1..=64)).changed();
            ui.label(RichText::new("×").color(color::TEXT_3));
            changed |= ui.add(egui::DragValue::new(&mut fb.rows).range(1..=64)).changed();
            changed |= ui.checkbox(&mut fb.blend, "Blend").changed();
            changed |= ui.checkbox(&mut fb.start_random, "Random start").changed();
        });
        row(ui, "Frame rate", |ui| changed |= range(ui, &mut fb.fps, 0.1, 0.0..=60.0));
    }
    row(ui, "Light emission", |ui| {
        changed |= ui.add(egui::Slider::new(&mut r.light_emission, 0.0..=1.0)).changed();
    });
    row(ui, "Brightness", |ui| changed |= num(ui, &mut r.brightness, 0.02, 0.0..=20.0).changed());
    row(ui, "Light influence", |ui| {
        changed |= ui.add(egui::Slider::new(&mut r.light_influence, 0.0..=1.0)).changed();
    });
    row(ui, "Z offset", |ui| changed |= num(ui, &mut r.z_offset, 0.02, -50.0..=50.0).changed());
    row(ui, "Soft fade", |ui| {
        changed |= num(ui, &mut r.soft_fade, 0.02, 0.0..=10.0).changed();
        ui.label(RichText::new("studs, GPU only").color(color::TEXT_3));
    });
    row(ui, "Sort", |ui| {
        for (v, l) in [(SortMode::ByDistance, "Distance"), (SortMode::OldestOnTop, "Oldest"), (SortMode::NewestOnTop, "Newest")] {
            changed |= ui.selectable_value(&mut r.sort, v, l).changed();
        }
    });
    row(ui, "Alpha steps", |ui| {
        changed |= ui.add(egui::DragValue::new(&mut r.alpha_steps).range(0..=8)).changed();
        ui.label(RichText::new(match r.alpha_steps { 0 => "smooth", 1 => "cut-out", _ => "bands" }).color(color::TEXT_3));
    });
    row(ui, "Pixel snap", |ui| {
        changed |= num(ui, &mut r.pixel_snap, 0.5, 0.0..=16.0).changed();
        ui.label(RichText::new("px").color(color::TEXT_3));
    });
    changed
}

/// Trail properties (Roblox `Trail`).
pub fn trail_inspector(ui: &mut Ui, t: &mut Trail, key: usize, preview: &mut Preview) -> bool {
    let mut changed = false;
    row(ui, "Name", |ui| changed |= ui.text_edit_singleline(&mut t.name).changed());
    row(ui, "Enabled", |ui| changed |= check(ui, &mut t.enabled).changed());
    row(ui, "Attachment 0", |ui| changed |= vec3(ui, &mut t.attachment0, 0.05));
    row(ui, "Attachment 1", |ui| changed |= vec3(ui, &mut t.attachment1, 0.05));
    row(ui, "Lifetime", |ui| changed |= num(ui, &mut t.lifetime, 0.01, 0.01..=20.0).changed());
    row(ui, "Min length", |ui| changed |= num(ui, &mut t.min_length, 0.01, 0.0..=100.0).changed());
    row(ui, "Max length", |ui| changed |= num(ui, &mut t.max_length, 0.1, 0.0..=1000.0).changed());
    row(ui, "Face camera", |ui| changed |= check(ui, &mut t.face_camera).changed());
    row(ui, "Texture", |ui| changed |= texture_picker(ui, &mut t.texture, 10_000 + key, preview));
    row(ui, "Light emission", |ui| {
        changed |= ui.add(egui::Slider::new(&mut t.light_emission, 0.0..=1.0)).changed();
    });
    row(ui, "Brightness", |ui| changed |= num(ui, &mut t.brightness, 0.02, 0.0..=20.0).changed());
    ui.add_space(4.0);
    ui.label(RichText::new("Color, head to tail").color(color::TEXT_2));
    changed |= gradient_edit(ui, &mut t.color, ("trail-color", key));
    changed |= curve_rows(ui, "Width scale", &mut t.width_scale, ("trail-width", key));
    changed |= curve_rows(ui, "Transparency", &mut t.transparency, ("trail-transp", key));
    changed
}

/// Beam properties (Roblox `Beam`).
pub fn beam_inspector(ui: &mut Ui, b: &mut Beam, key: usize, preview: &mut Preview) -> bool {
    let mut changed = false;
    row(ui, "Name", |ui| changed |= ui.text_edit_singleline(&mut b.name).changed());
    row(ui, "Enabled", |ui| changed |= check(ui, &mut b.enabled).changed());
    row(ui, "Attachment 0", |ui| changed |= vec3(ui, &mut b.attachment0, 0.05));
    row(ui, "Attachment 1", |ui| changed |= vec3(ui, &mut b.attachment1, 0.05));
    row(ui, "Curve size", |ui| {
        changed |= num(ui, &mut b.curve_size0, 0.05, -100.0..=100.0).changed();
        changed |= num(ui, &mut b.curve_size1, 0.05, -100.0..=100.0).changed();
    });
    row(ui, "Width", |ui| {
        changed |= num(ui, &mut b.width0, 0.02, 0.0..=100.0).changed();
        changed |= num(ui, &mut b.width1, 0.02, 0.0..=100.0).changed();
    });
    row(ui, "Segments", |ui| changed |= ui.add(egui::DragValue::new(&mut b.segments).range(1..=1000)).changed());
    row(ui, "Texture", |ui| changed |= texture_picker(ui, &mut b.texture, 20_000 + key, preview));
    row(ui, "Texture speed", |ui| changed |= num(ui, &mut b.texture_speed, 0.05, -50.0..=50.0).changed());
    row(ui, "Jitter", |ui| {
        changed |= num(ui, &mut b.jitter, 0.02, 0.0..=10.0).changed();
        changed |= num(ui, &mut b.jitter_fps, 0.5, 0.0..=60.0).changed();
        ui.label(RichText::new("fps").color(color::TEXT_3));
    });
    row(ui, "Light emission", |ui| {
        changed |= ui.add(egui::Slider::new(&mut b.light_emission, 0.0..=1.0)).changed();
    });
    row(ui, "Brightness", |ui| changed |= num(ui, &mut b.brightness, 0.02, 0.0..=20.0).changed());
    row(ui, "Face camera", |ui| changed |= check(ui, &mut b.face_camera).changed());
    ui.add_space(4.0);
    ui.label(RichText::new("Color, start to end").color(color::TEXT_2));
    changed |= gradient_edit(ui, &mut b.color, ("beam-color", key));
    changed |= curve_rows(ui, "Transparency", &mut b.transparency, ("beam-transp", key));
    changed
}
