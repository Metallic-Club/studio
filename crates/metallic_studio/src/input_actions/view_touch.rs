//! The device preview tab: "Preview as", the phone frame with the touch overlay, "Edit touch
//! layout" (drag, resize, 4 dp snap, guides, keyboard nudge), the virtual gamepad and a live
//! readout of every action evaluated by the real [`InputRuntime`].

use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Event, EventFilter, Frame, Id, Key, Margin, Modifiers,
    Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use metallic_core::{ChangeCursor, InstanceId, World};
use metallic_runtime::input::{ActionType, ActionValue, Control, Device, InputRuntime, RawEvent};

use super::commands::IdMap;
use super::model::InputModel;
use super::preview::{GlyphStyle, PAD_BUTTONS, PreviewAs, TouchSim, VirtualPad};
use super::style::{self as st, ButtonKind, Icon};
use super::touch::{
    self, BIG_NUDGE, ControlRole, DevicePreset, DeviceProfile, GRID, MIN_GAP, Orientation, Placed,
    Violation, clamp_center, clamp_size, snap,
};
use super::view::{InputActionsPanel, Intent, Selection};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Resize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    id: InstanceId,
    kind: DragKind,
    start_center: [f32; 2],
    start_size: f32,
    moved: [f32; 2],
    center: [f32; 2],
    size: f32,
}

/// State of the preview tab.
#[derive(Default)]
pub(super) struct TouchView {
    sim: TouchSim,
    selected: Option<InstanceId>,
    drag: Option<Drag>,
    runtime: Option<(ChangeCursor, InputRuntime)>,
    pad: VirtualPad,
    pad_drag: [bool; 2],
    keys_down: Vec<Control>,
    last_time: f64,
}

impl TouchView {
    /// Follows ids that an undo rebuilt.
    pub(super) fn remap(&mut self, map: &IdMap) {
        if let Some(id) = &mut self.selected
            && let Some(n) = map.get(id)
        {
            *id = *n;
        }
        self.drag = None;
    }

    /// The selected touch control.
    #[cfg(test)]
    pub(super) fn selected(&self) -> Option<InstanceId> {
        self.selected
    }

    /// Selects a touch control (from the Checks strip).
    pub(super) fn select(&mut self, id: InstanceId) {
        self.selected = Some(id);
    }

    fn runtime(&mut self, world: &World) -> &mut InputRuntime {
        let cursor = world.change_cursor();
        let stale = self.runtime.as_ref().is_none_or(|(c, _)| *c != cursor);
        if stale {
            match &mut self.runtime {
                Some((c, rt)) => {
                    rt.sync(world);
                    *c = cursor;
                }
                None => self.runtime = Some((cursor, InputRuntime::from_world(world))),
            }
        }
        &mut self.runtime.as_mut().expect("just set").1
    }

    pub(super) fn show(
        &mut self,
        panel: &mut InputActionsPanel,
        ui: &mut Ui,
        world: &World,
        model: &InputModel,
    ) {
        let now = panel.now;
        let dt = ((now - self.last_time) as f32).clamp(0.0, 0.1);
        self.last_time = now;

        egui::Panel::left("ia-preview-side")
            .default_size(250.0)
            .min_size(220.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::bg_panel())
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| self.side(panel, ui, model));

        // Build the layout once per frame (with the control being dragged at its draft spot).
        let profile = panel.preview.profile();
        let actions = touch::touch_actions(world);
        let mut placed = profile
            .map(|p| touch::layout(&p, &actions))
            .unwrap_or_default();
        if let Some(d) = &self.drag
            && let Some(p) = placed.iter_mut().find(|p| p.id == Some(d.id))
        {
            p.center = d.center;
            p.size = d.size;
            p.custom = true;
        }

        // Touch values for the readout.
        let mut touch_values: HashMap<String, ActionValue> = HashMap::new();
        if let Some(prof) = &profile
            && !panel.preview.editing_layout
        {
            let state = self.sim.take_state(prof, &placed);
            if let Some(stick) = placed.iter().find(|p| p.role == ControlRole::Stick) {
                touch_values.insert(stick.path.clone(), ActionValue::Axis2(state.stick));
            }
            for f in [self.sim.mouse, self.sim.pinned].into_iter().flatten() {
                if let super::preview::FingerTarget::Button(i) = f.target
                    && let Some(p) = placed.get(i)
                {
                    touch_values.insert(p.path.clone(), ActionValue::Bool(true));
                }
            }
        }

        // Feed the runtime: virtual pad, host pads, preview keys.
        let mut events = self.pad.events(now);
        events.append(&mut panel_raw(panel));
        let rt = self.runtime(world);
        for e in events {
            rt.push(e);
        }

        egui::Panel::right("ia-preview-info")
            .default_size(280.0)
            .min_size(240.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(st::bg_panel())
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                if panel.preview.editing_layout
                    && let Some(prof) = &profile
                {
                    self.inspector(panel, ui, world, &placed, prof);
                    ui.add_space(st::S5);
                }
                self.readout(panel, ui, model, &touch_values);
            });

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(st::bg_base())
                    .inner_margin(Margin::same(16)),
            )
            .show(ui, |ui| match (panel.preview.mode, profile) {
                (PreviewAs::Phone | PreviewAs::Tablet, Some(p)) => {
                    self.stage(panel, ui, model, &p, &placed);
                }
                (PreviewAs::Gamepad, _) => self.gamepad(ui, panel.preview.glyphs),
                _ => self.keyboard(ui, now),
            });

        let rt = self.runtime(world);
        rt.evaluate(now, dt);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    // -----------------------------------------------------------------------------------------

    fn side(&mut self, panel: &mut InputActionsPanel, ui: &mut Ui, model: &InputModel) {
        st::section_title(ui, "Preview as");
        ui.spacing_mut().item_spacing.y = 2.0;
        for mode in PreviewAs::ALL {
            let icon = match mode {
                PreviewAs::KeyboardMouse => Icon::Keyboard,
                PreviewAs::Phone => Icon::Phone,
                PreviewAs::Tablet => Icon::Tablet,
                PreviewAs::Gamepad => Icon::Gamepad,
            };
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let resp = ui.interact(r, Id::unique(("ia-mode", mode as u8)), Sense::click());
            let on = panel.preview.mode == mode;
            ui.painter().rect_filled(
                r,
                CornerRadius::same(st::R_MD),
                if on {
                    st::select()
                } else if resp.hovered() {
                    st::bg_hover()
                } else {
                    Color32::TRANSPARENT
                },
            );
            st::paint_icon(
                ui.painter(),
                Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 8.0), Vec2::splat(16.0)),
                icon,
                if on { st::accent_text() } else { st::icon() },
            );
            ui.painter().text(
                pos2(r.left() + 36.0, r.center().y),
                Align2::LEFT_CENTER,
                mode.label(),
                st::body(),
                st::text_1(),
            );
            if on {
                ui.painter()
                    .circle_filled(pos2(r.right() - 12.0, r.center().y), 3.0, st::accent());
            }
            st::focus_ring(ui, &resp, r, st::R_MD);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, mode.label())
            });
            if resp.clicked() && !on {
                panel.preview.mode = mode;
                self.sim.clear();
                self.drag = None;
            }
        }
        ui.spacing_mut().item_spacing.y = st::S3;
        let forced = match panel.preview.mode.forced_device() {
            metallic_runtime::input::DeviceKind::KeyboardMouse => "Keyboard and mouse",
            metallic_runtime::input::DeviceKind::Gamepad => "Gamepad",
            metallic_runtime::input::DeviceKind::Touch => "Touch",
        };
        ui.label(
            egui::RichText::new(format!(
                "Play shows {} prompts. The scene is not changed.",
                forced.to_lowercase()
            ))
            .font(st::caption())
            .color(st::text_3()),
        );

        if panel.preview.mode.is_touch() {
            ui.add_space(st::S5);
            st::section_title(ui, "Screen");
            if panel.preview.mode == PreviewAs::Phone {
                let mut preset = panel.preview.phone;
                st::segmented(
                    ui,
                    &mut preset,
                    &[
                        (DevicePreset::SmallPhone, None, "Small"),
                        (DevicePreset::Phone, None, "Phone"),
                    ],
                );
                if preset != panel.preview.phone {
                    panel.preview.phone = preset;
                    self.sim.clear();
                }
            }
            let mut o = panel.preview.orientation;
            st::segmented(
                ui,
                &mut o,
                &[
                    (Orientation::Landscape, None, "Landscape"),
                    (Orientation::Portrait, None, "Portrait"),
                ],
            );
            if o != panel.preview.orientation {
                panel.preview.orientation = o;
                self.sim.clear();
            }
            if let Some(p) = panel.preview.profile() {
                ui.label(
                    egui::RichText::new(format!(
                        "{:.0} x {:.0} dp, insets {:.0} {:.0} {:.0} {:.0}",
                        p.width,
                        p.height,
                        p.insets.left,
                        p.insets.top,
                        p.insets.right,
                        p.insets.bottom
                    ))
                    .font(st::mono())
                    .color(st::text_3()),
                );
            }
            switch_row(ui, "Show safe area", &mut panel.preview.show_safe_area);
            switch_row(ui, "Guides", &mut panel.preview.show_guides);

            ui.add_space(st::S5);
            st::section_title(ui, "Touch layout");
            let editable = !model.uses_builtin_character_controls();
            let mut editing = panel.preview.editing_layout;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Edit touch layout")
                        .font(st::body())
                        .color(if editable {
                            st::text_1()
                        } else {
                            st::text_off()
                        }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if st::toggle(ui, &mut editing, editable).changed() {
                        panel.preview.editing_layout = editing;
                        self.sim.clear();
                        self.drag = None;
                    }
                });
            });
            if editable {
                let help = if panel.preview.editing_layout {
                    "Drag to move (4 dp grid, Alt moves freely). Drag the corner handle to resize. Tab picks a control, arrows nudge 4 dp, Shift 16 dp, [ and ] resize."
                } else {
                    "The left mouse button is a finger. Right click pins a second finger so a stick and a button can be held together."
                };
                ui.label(
                    egui::RichText::new(help)
                        .font(st::caption())
                        .color(st::text_3()),
                );
                if panel.preview.editing_layout
                    && st::button(ui, ButtonKind::Secondary, Some(Icon::Reset), "Reset layout")
                        .on_hover_text("Every control back to TouchLayout = Auto")
                        .clicked()
                {
                    panel.intents.push(Intent::TouchReset(None));
                }
            } else {
                ui.label(
                    egui::RichText::new("The built-in controls are read only.")
                        .font(st::caption())
                        .color(st::text_3()),
                );
                if st::button(
                    ui,
                    ButtonKind::Primary,
                    Some(Icon::Plus),
                    "Add CharacterControls",
                )
                .clicked()
                {
                    panel.intents.push(Intent::Materialize);
                }
            }
        }
        if matches!(
            panel.preview.mode,
            PreviewAs::Gamepad | PreviewAs::KeyboardMouse
        ) || panel.preview.mode.is_touch()
        {
            ui.add_space(st::S5);
            st::section_title(ui, "Gamepad glyphs");
            st::segmented(
                ui,
                &mut panel.preview.glyphs,
                &[
                    (GlyphStyle::Letters, None, GlyphStyle::Letters.label()),
                    (GlyphStyle::Shapes, None, GlyphStyle::Shapes.label()),
                ],
            );
        }
    }

    // -----------------------------------------------------------------------------------------
    // The phone frame

    fn stage(
        &mut self,
        panel: &mut InputActionsPanel,
        ui: &mut Ui,
        model: &InputModel,
        profile: &DeviceProfile,
        placed: &[Placed],
    ) {
        let avail = ui.available_rect_before_wrap();
        let bezel = 14.0;
        let scale = ((avail.width() - 2.0 * bezel - 8.0) / profile.width)
            .min((avail.height() - 2.0 * bezel - 40.0) / profile.height)
            .max(0.2);
        let screen_size = vec2(profile.width * scale, profile.height * scale);
        let screen = Rect::from_center_size(avail.center() - vec2(0.0, 12.0), screen_size);
        let frame = screen.expand(bezel);
        let to_px = |p: [f32; 2]| pos2(screen.left() + p[0] * scale, screen.top() + p[1] * scale);
        let to_dp = |p: Pos2| [(p.x - screen.left()) / scale, (p.y - screen.top()) / scale];
        let painter = ui.painter().with_clip_rect(avail);

        // Device body and screen.
        painter.rect_filled(frame, CornerRadius::same(28), st::g950());
        painter.rect_stroke(
            frame,
            CornerRadius::same(28),
            Stroke::new(1.0, st::line_strong()),
            StrokeKind::Inside,
        );
        painter.rect_filled(screen, CornerRadius::same(10), st::bg_deep());
        // A quiet floor so the overlay reads against "a game".
        let horizon = screen.top() + screen.height() * 0.55;
        for i in 0..7 {
            let t = i as f32 / 6.0;
            let y = egui::lerp(horizon..=screen.bottom(), t * t);
            painter.hline(
                screen.x_range(),
                y,
                Stroke::new(1.0, st::line().gamma_multiply(0.7)),
            );
        }
        for i in -6..=6 {
            let x0 = screen.center().x + i as f32 * screen.width() * 0.04;
            let x1 = screen.center().x + i as f32 * screen.width() * 0.22;
            painter.line_segment(
                [pos2(x0, horizon), pos2(x1, screen.bottom())],
                Stroke::new(1.0, st::line().gamma_multiply(0.7)),
            );
        }

        let safe = profile.safe_area();
        if panel.preview.show_safe_area {
            let ins = profile.insets;
            let band = st::with_alpha(st::g950(), 190);
            let bands = [
                Rect::from_min_max(screen.min, pos2(to_px([ins.left, 0.0]).x, screen.bottom())),
                Rect::from_min_max(
                    pos2(to_px([profile.width - ins.right, 0.0]).x, screen.top()),
                    screen.max,
                ),
                Rect::from_min_max(screen.min, pos2(screen.right(), to_px([0.0, ins.top]).y)),
                Rect::from_min_max(
                    pos2(screen.left(), to_px([0.0, profile.height - ins.bottom]).y),
                    screen.max,
                ),
            ];
            for b in bands {
                if b.width() > 0.5 && b.height() > 0.5 {
                    painter.rect_filled(b, CornerRadius::ZERO, band);
                }
            }
            let sr = Rect::from_min_max(
                to_px([safe.x, safe.y]),
                to_px([safe.right(), safe.bottom()]),
            );
            painter.add(Shape::dashed_line(
                &rect_points(sr),
                Stroke::new(1.0, st::info().gamma_multiply(0.7)),
                5.0,
                4.0,
            ));
            painter.text(
                sr.left_top() + vec2(6.0, 4.0),
                Align2::LEFT_TOP,
                "Safe area",
                st::caption(),
                st::info(),
            );
        }
        let editing = panel.preview.editing_layout && !model.uses_builtin_character_controls();
        if panel.preview.show_guides || editing {
            let inner = safe.shrink(touch::SAFE_MARGIN);
            let ir = Rect::from_min_max(
                to_px([inner.x, inner.y]),
                to_px([inner.right(), inner.bottom()]),
            );
            painter.add(Shape::dashed_line(
                &rect_points(ir),
                Stroke::new(1.0, st::line_control().gamma_multiply(0.6)),
                3.0,
                5.0,
            ));
            let z = profile.stick_zone();
            let zr = Rect::from_min_max(to_px([z.x, z.y]), to_px([z.right(), z.bottom()]));
            painter.rect_filled(
                zr.shrink(1.0),
                CornerRadius::same(6),
                st::accent().gamma_multiply(0.05),
            );
            painter.text(
                zr.left_bottom() + vec2(8.0, -6.0),
                Align2::LEFT_BOTTOM,
                "Stick zone",
                st::caption(),
                st::accent_text().gamma_multiply(0.7),
            );
        }

        let violations = touch::violations(profile, placed);
        let bad = |i: usize| {
            violations.iter().any(|v| match v {
                Violation::TooSmall { control } | Violation::OutsideSafeArea { control } => {
                    *control == i
                }
                Violation::TooClose { a, b, .. } => *a == i || *b == i,
            })
        };

        // Fingers and held buttons (simulation only).
        let held: Vec<usize> = [self.sim.mouse, self.sim.pinned]
            .into_iter()
            .flatten()
            .filter_map(|f| match f.target {
                super::preview::FingerTarget::Button(i) => Some(i),
                _ => None,
            })
            .collect();
        let stick_finger = [self.sim.mouse, self.sim.pinned]
            .into_iter()
            .flatten()
            .find_map(|f| match f.target {
                super::preview::FingerTarget::Stick { base } => Some((base, f.pos)),
                _ => None,
            });

        for (i, p) in placed.iter().enumerate() {
            let c = to_px(p.center);
            let r = p.radius() * scale;
            match p.role {
                ControlRole::Stick => {
                    let (base, knob) = match stick_finger {
                        Some((b, f)) => {
                            let d = [f[0] - b[0], f[1] - b[1]];
                            let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
                            let k = len.min(p.radius()) / len;
                            (to_px(b), to_px([b[0] + d[0] * k, b[1] + d[1] * k]))
                        }
                        None => (c, c),
                    };
                    let active = stick_finger.is_some();
                    painter.circle_filled(
                        base,
                        r,
                        st::with_alpha(st::bg_panel(), if active { 150 } else { 100 }),
                    );
                    painter.circle_stroke(
                        base,
                        r,
                        Stroke::new(
                            1.5,
                            st::silver().gamma_multiply(if active { 0.8 } else { 0.45 }),
                        ),
                    );
                    let kr = (p.size * 0.47).max(touch::MIN_SIZE) / 2.0 * scale;
                    painter.circle_filled(
                        knob,
                        kr,
                        if active {
                            st::accent().gamma_multiply(0.85)
                        } else {
                            st::with_alpha(st::keycap(), 215)
                        },
                    );
                    painter.circle_stroke(
                        knob,
                        kr,
                        Stroke::new(1.2, st::silver().gamma_multiply(0.8)),
                    );
                    painter.text(
                        base + vec2(0.0, r + 10.0),
                        Align2::CENTER_CENTER,
                        &p.label,
                        st::caption(),
                        st::text_2(),
                    );
                }
                ControlRole::Menu => {
                    painter.circle_filled(c, r, st::with_alpha(st::bg_panel(), 200));
                    painter.circle_stroke(c, r, Stroke::new(1.0, st::silver().gamma_multiply(0.5)));
                    for dx in [-3.0, 3.0] {
                        painter.rect_filled(
                            Rect::from_center_size(
                                c + vec2(dx * scale.max(0.6), 0.0),
                                vec2(2.5, 10.0) * scale.max(0.6),
                            ),
                            CornerRadius::same(1),
                            st::text_2(),
                        );
                    }
                }
                _ => {
                    let down = held.contains(&i);
                    let rr = if down { r * 0.94 } else { r };
                    let primary = p.role == ControlRole::Primary;
                    let fill = match (primary, down) {
                        (true, true) => st::accent_hover(),
                        (true, false) => st::with_alpha(st::accent(), 225),
                        (false, true) => st::bg_active(),
                        (false, false) => st::with_alpha(st::bg_raised(), 225),
                    };
                    painter.circle_filled(c, rr, fill);
                    painter.circle_stroke(
                        c,
                        rr,
                        Stroke::new(
                            1.2,
                            if primary {
                                st::accent_hover()
                            } else {
                                st::silver().gamma_multiply(0.7)
                            },
                        ),
                    );
                    let fg = if primary {
                        st::on_accent()
                    } else {
                        st::text_1()
                    };
                    let font =
                        egui::FontId::proportional((11.0 * scale.max(0.8)).clamp(10.0, 15.0));
                    painter.text(c, Align2::CENTER_CENTER, &p.label, font, fg);
                }
            }
            if editing && bad(i) {
                painter.circle_stroke(c, r + 3.0, Stroke::new(1.5, st::error()));
            }
            if p.custom && editing {
                painter.circle_filled(c + vec2(r * 0.72, -r * 0.72), 3.0, st::accent_text());
            }
        }
        // Gap warnings between controls.
        if editing {
            for v in &violations {
                if let Violation::TooClose { a, b, gap } = v {
                    let (pa, pb) = (&placed[*a], &placed[*b]);
                    let ca = to_px(pa.center);
                    let cb = to_px(pb.center);
                    let dir = (cb - ca).normalized();
                    let ea = ca + dir * pa.radius() * scale;
                    let eb = cb - dir * pb.radius() * scale;
                    painter.line_segment([ea, eb], Stroke::new(1.5, st::error()));
                    let text = if *gap < 0.0 {
                        "Overlap".to_string()
                    } else {
                        format!("{gap:.0} dp, needs {MIN_GAP:.0}")
                    };
                    warning_chip(&painter, ea.lerp(eb, 0.5) + vec2(0.0, -14.0), &text);
                }
            }
        }

        // Fingers.
        for (f, pinned) in [(self.sim.mouse, false), (self.sim.pinned, true)] {
            if let Some(f) = f {
                let fp = to_px(f.pos);
                painter.circle_filled(fp, 16.0 * scale.max(0.7), Color32::from_white_alpha(28));
                painter.circle_stroke(
                    fp,
                    16.0 * scale.max(0.7),
                    Stroke::new(1.0, Color32::from_white_alpha(90)),
                );
                if pinned {
                    painter.circle_filled(fp, 3.0, st::accent_text());
                }
            }
        }

        // Interaction.
        if editing {
            self.edit_controls(panel, ui, profile, placed, scale, &to_px, &to_dp);
        } else {
            let resp = ui.interact(
                screen,
                Id::unique("ia-stage-touch"),
                Sense::click_and_drag(),
            );
            if let Some(pos) = resp.interact_pointer_pos() {
                let dp = to_dp(pos);
                if resp.drag_started_by(egui::PointerButton::Primary)
                    || (resp.is_pointer_button_down_on()
                        && self.sim.mouse.is_none()
                        && ui.input(|i| i.pointer.primary_pressed()))
                {
                    self.sim.mouse_down(profile, placed, dp);
                } else if resp.dragged_by(egui::PointerButton::Primary) {
                    self.sim.mouse_move(dp);
                }
            }
            if ui.input(|i| i.pointer.primary_released()) {
                self.sim.mouse_up();
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                self.sim.toggle_pin(profile, placed, to_dp(pos));
            }
        }

        // Caption under the device.
        let mut caption = format!("{}  ·  {:.0}%", profile.label(), scale * 100.0);
        if editing {
            caption.push_str("  ·  editing layout");
        }
        ui.painter().text(
            pos2(frame.center().x, frame.bottom() + 16.0),
            Align2::CENTER_CENTER,
            caption,
            st::caption(),
            st::text_3(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_controls(
        &mut self,
        panel: &mut InputActionsPanel,
        ui: &mut Ui,
        profile: &DeviceProfile,
        placed: &[Placed],
        scale: f32,
        to_px: &dyn Fn([f32; 2]) -> Pos2,
        to_dp: &dyn Fn(Pos2) -> [f32; 2],
    ) {
        let _ = to_dp;
        let safe = profile.safe_area();
        let alt = ui.input(|i| i.modifiers.alt);
        for p in placed.iter().filter(|p| p.editable()) {
            let Some(id) = p.id else { continue };
            let c = to_px(p.center);
            let r = p.radius() * scale;
            let body = Rect::from_center_size(c, Vec2::splat(2.0 * r));
            let rid = Id::unique(("ia-touch-ctl", id));
            let resp = ui.interact(body, rid, Sense::click_and_drag());
            let handle_c = c + vec2(r, r) * std::f32::consts::FRAC_1_SQRT_2;
            let handle = Rect::from_center_size(handle_c, Vec2::splat(12.0));
            let hresp = ui.interact(handle.expand(3.0), rid.with("resize"), Sense::drag());
            let selected = self.selected == Some(id);
            if resp.clicked() || resp.drag_started() || resp.gained_focus() {
                if self.selected != Some(id) {
                    panel.intents.push(Intent::Seal);
                }
                self.selected = Some(id);
                panel.selection = Some(Selection::Instance(id));
            }
            if selected || resp.hovered() || resp.has_focus() {
                let p_ = ui.painter();
                p_.circle_stroke(
                    c,
                    r + 2.0,
                    Stroke::new(
                        if selected { 2.0 } else { 1.0 },
                        if selected {
                            st::accent()
                        } else {
                            st::line_control()
                        },
                    ),
                );
                if selected {
                    // The 8 dp spacing guide.
                    p_.add(Shape::dashed_line(
                        &circle_points(c, r + MIN_GAP * scale, 48),
                        Stroke::new(1.0, st::accent().gamma_multiply(0.45)),
                        3.0,
                        3.0,
                    ));
                    p_.rect_filled(handle, CornerRadius::same(2), st::accent());
                    p_.rect_stroke(
                        handle,
                        CornerRadius::same(2),
                        Stroke::new(1.0, st::on_accent()),
                        StrokeKind::Inside,
                    );
                }
            }
            if resp.has_focus() {
                ui.painter()
                    .circle_stroke(c, r + 5.0, Stroke::new(2.0, st::focus()));
            }
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("{} touch control", p.label),
                )
            });

            // Mouse move and resize.
            let start = |kind| Drag {
                id,
                kind,
                start_center: p.center,
                start_size: p.size,
                moved: [0.0, 0.0],
                center: p.center,
                size: p.size,
            };
            if resp.drag_started() {
                self.drag = Some(start(DragKind::Move));
            }
            if selected && hresp.drag_started() {
                self.drag = Some(start(DragKind::Resize));
            }
            let active = resp.dragged() || (selected && hresp.dragged());
            if active
                && let Some(d) = &mut self.drag
                && d.id == id
            {
                let delta = if d.kind == DragKind::Move {
                    resp.drag_delta()
                } else {
                    hresp.drag_delta()
                } / scale;
                d.moved[0] += delta.x;
                d.moved[1] += delta.y;
                match d.kind {
                    DragKind::Move => {
                        let mut cpos = [
                            d.start_center[0] + d.moved[0],
                            d.start_center[1] + d.moved[1],
                        ];
                        if !alt {
                            cpos = [
                                snap(cpos[0] - safe.x) + safe.x,
                                snap(cpos[1] - safe.y) + safe.y,
                            ];
                        }
                        d.center = clamp_center(cpos, d.size, &safe);
                    }
                    DragKind::Resize => {
                        let grow =
                            (d.moved[0] + d.moved[1]) * std::f32::consts::FRAC_1_SQRT_2 * 2.0;
                        d.size = clamp_size(d.start_size + grow);
                        d.center = clamp_center(d.start_center, d.size, &safe);
                    }
                }
            }
            if (resp.drag_stopped() || hresp.drag_stopped())
                && let Some(d) = self.drag.take_if(|d| d.id == id)
                && (d.center != d.start_center || d.size != d.start_size || !p.custom)
            {
                panel.intents.push(Intent::Seal);
                panel.intents.push(Intent::TouchCommit {
                    id,
                    center: d.center,
                    size: d.size,
                    profile: *profile,
                    merge: false,
                });
                panel.intents.push(Intent::Seal);
            }

            // Keyboard: arrows nudge, Shift for 16 dp, [ and ] resize.
            if resp.has_focus() {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        rid,
                        EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                let (mut dx, mut dy, mut ds) = (0.0, 0.0, 0.0);
                ui.input_mut(|i| {
                    for (key, x, y) in [
                        (Key::ArrowLeft, -1.0, 0.0),
                        (Key::ArrowRight, 1.0, 0.0),
                        (Key::ArrowUp, 0.0, -1.0),
                        (Key::ArrowDown, 0.0, 1.0),
                    ] {
                        if i.consume_key(Modifiers::NONE, key) {
                            dx += x * GRID;
                            dy += y * GRID;
                        }
                        if i.consume_key(Modifiers::SHIFT, key) {
                            dx += x * BIG_NUDGE;
                            dy += y * BIG_NUDGE;
                        }
                    }
                    if i.consume_key(Modifiers::NONE, Key::OpenBracket) {
                        ds -= GRID;
                    }
                    if i.consume_key(Modifiers::NONE, Key::CloseBracket) {
                        ds += GRID;
                    }
                });
                if dx != 0.0 || dy != 0.0 || ds != 0.0 {
                    let size = clamp_size(p.size + ds);
                    let center = clamp_center([p.center[0] + dx, p.center[1] + dy], size, &safe);
                    panel.intents.push(Intent::TouchCommit {
                        id,
                        center,
                        size,
                        profile: *profile,
                        merge: true,
                    });
                }
            }
        }
        // Clicking the empty screen clears the control selection.
        if ui.input(|i| i.pointer.primary_clicked()) && !ui.ctx().is_pointer_over_egui() {
            self.selected = None;
        }
    }

    fn inspector(
        &mut self,
        panel: &mut InputActionsPanel,
        ui: &mut Ui,
        world: &World,
        placed: &[Placed],
        profile: &DeviceProfile,
    ) {
        st::section_title(ui, "Selected control");
        let Some((i, p)) = placed
            .iter()
            .enumerate()
            .find(|(_, p)| p.id.is_some() && p.id == self.selected)
        else {
            ui.label(
                egui::RichText::new("Click a control, or press Tab to reach one.")
                    .font(st::small())
                    .color(st::text_3()),
            );
            return;
        };
        let id = p.id.expect("filtered");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&p.label)
                    .font(st::heading())
                    .color(st::text_1()),
            );
            if p.custom {
                st::chip(ui, "Custom", st::accent_text(), st::select_weak());
            } else {
                st::chip(ui, "Auto", st::text_2(), st::bg_raised());
            }
        });
        let safe = profile.safe_area();
        let u = touch::to_udim(p.center, &safe);
        let rows = [
            ("Size", format!("{:.0} dp", p.size)),
            (
                "Centre",
                format!("{:.0}, {:.0} dp", p.center[0], p.center[1]),
            ),
            (
                "TouchPosition",
                format!(
                    "{{{}, {:.0}}}, {{{}, {:.0}}}",
                    u.x_scale, u.x_offset, u.y_scale, u.y_offset
                ),
            ),
        ];
        for (k, v) in rows {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(100.0, 20.0), Sense::hover());
                ui.painter().text(
                    r.left_center(),
                    Align2::LEFT_CENTER,
                    k,
                    st::small(),
                    st::text_2(),
                );
                ui.label(egui::RichText::new(v).font(st::mono()).color(st::text_1()));
            });
        }
        let vs = touch::violations(profile, placed);
        for v in vs {
            let text = match v {
                Violation::TooSmall { control } if control == i => {
                    Some("Smaller than 48 dp".to_string())
                }
                Violation::OutsideSafeArea { control } if control == i => {
                    Some("Less than 16 dp inside the safe area".to_string())
                }
                Violation::TooClose { a, b, gap } if a == i || b == i => {
                    let other = &placed[if a == i { b } else { a }];
                    Some(format!(
                        "{:.0} dp from {} (needs 8)",
                        gap.max(0.0),
                        other.label
                    ))
                }
                _ => None,
            };
            if let Some(t) = text {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                    st::paint_icon(ui.painter(), r, Icon::Error, st::error());
                    ui.label(egui::RichText::new(t).font(st::small()).color(st::error()));
                });
            }
        }
        if p.custom
            && st::button(
                ui,
                ButtonKind::Ghost,
                Some(Icon::Reset),
                "Back to automatic",
            )
            .clicked()
        {
            panel.intents.push(Intent::TouchReset(Some(id)));
        }
        let _ = world;
    }

    fn readout(
        &mut self,
        panel: &InputActionsPanel,
        ui: &mut Ui,
        model: &InputModel,
        touch: &HashMap<String, ActionValue>,
    ) {
        st::section_title(ui, "Live actions");
        ui.label(
            egui::RichText::new("Evaluated by the Player's input runtime.")
                .font(st::caption())
                .color(st::text_3()),
        );
        ui.add_space(st::S2);
        let rt = self.runtime.as_ref().map(|(_, r)| r);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut last_ctx = "";
                for (ctx, a) in model.actions() {
                    if ctx.name != last_ctx {
                        ui.add_space(st::S3);
                        ui.label(
                            egui::RichText::new(&ctx.name)
                                .font(st::caption())
                                .color(st::text_3()),
                        );
                        last_ctx = &ctx.name;
                    }
                    let path = format!("{}/{}", ctx.name, a.name);
                    let value = touch.get(&path).copied().or_else(|| {
                        let rt = rt?;
                        rt.find_action(&path).map(|id| rt.state(id))
                    });
                    let (r, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    let active = value.is_some_and(|v| is_active(&v));
                    if active {
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(st::R_SM),
                            st::select_weak(),
                        );
                    }
                    ui.painter().text(
                        pos2(r.left() + 6.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &a.name,
                        st::body(),
                        if a.enabled {
                            st::text_1()
                        } else {
                            st::text_off()
                        },
                    );
                    let vis = Rect::from_min_max(
                        pos2(r.right() - 96.0, r.top() + 4.0),
                        pos2(r.right() - 6.0, r.bottom() - 4.0),
                    );
                    paint_value(ui.painter(), vis, a.ty, value);
                }
            });
        let _ = panel;
    }

    // -----------------------------------------------------------------------------------------
    // Gamepad and keyboard stages

    fn gamepad(&mut self, ui: &mut Ui, glyphs: GlyphStyle) {
        let avail = ui.available_rect_before_wrap();
        let body = Rect::from_center_size(
            avail.center() - vec2(0.0, 20.0),
            vec2(520.0_f32.min(avail.width() - 20.0), 260.0),
        );
        let p = ui.painter();
        // A generic pad: one rounded body, no product outline.
        p.rect_filled(body, CornerRadius::same(110), st::bg_raised());
        p.rect_stroke(
            body,
            CornerRadius::same(110),
            Stroke::new(1.0, st::line_strong()),
            StrokeKind::Inside,
        );
        p.hline(
            body.x_range().shrink(110.0),
            body.top() + 0.5,
            Stroke::new(1.0, st::highlight()),
        );
        let s = body.width() / 520.0;
        let at = |x: f32, y: f32| pos2(body.left() + x * s, body.top() + y * s);
        let mut button = |ui: &mut Ui, c: Control, pos: Pos2, size: Vec2| {
            let r = Rect::from_center_size(pos, size);
            let resp = ui.interact(
                r,
                Id::unique(("ia-vpad", c.name())),
                Sense::click_and_drag(),
            );
            let down = resp.is_pointer_button_down_on()
                || self.pad.held.contains(&c)
                    && resp.has_focus()
                    && ui.input(|i| i.key_down(Key::Space));
            let space =
                resp.has_focus() && ui.input(|i| i.key_down(Key::Space) || i.key_down(Key::Enter));
            self.pad
                .set_button(c, resp.is_pointer_button_down_on() || space);
            let fill = if down || space {
                st::accent()
            } else if resp.hovered() {
                st::bg_hover()
            } else {
                st::bg_panel()
            };
            ui.painter()
                .rect_filled(r, CornerRadius::same((size.y / 2.0) as u8), fill);
            let b = metallic_runtime::input::BindingDef::key(c);
            let w = st::glyph_width(ui.painter(), &b);
            st::paint_glyph(
                ui.painter(),
                pos2(r.center().x - w / 2.0, r.center().y),
                &b,
                glyphs,
                false,
            );
            st::focus_ring(ui, &resp, r, (size.y / 2.0) as u8);
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, super::model::control_label(c))
            });
        };
        let face = 34.0 * s;
        button(ui, Control::ButtonA, at(410.0, 140.0), Vec2::splat(face));
        button(ui, Control::ButtonB, at(446.0, 104.0), Vec2::splat(face));
        button(ui, Control::ButtonX, at(374.0, 104.0), Vec2::splat(face));
        button(ui, Control::ButtonY, at(410.0, 68.0), Vec2::splat(face));
        button(ui, Control::DPadUp, at(150.0, 150.0), Vec2::splat(face));
        button(ui, Control::DPadDown, at(150.0, 218.0), Vec2::splat(face));
        button(ui, Control::DPadLeft, at(116.0, 184.0), Vec2::splat(face));
        button(ui, Control::DPadRight, at(184.0, 184.0), Vec2::splat(face));
        button(ui, Control::ButtonL1, at(110.0, 22.0), vec2(70.0, 26.0) * s);
        button(ui, Control::ButtonL2, at(190.0, 22.0), vec2(70.0, 26.0) * s);
        button(ui, Control::ButtonR2, at(330.0, 22.0), vec2(70.0, 26.0) * s);
        button(ui, Control::ButtonR1, at(410.0, 22.0), vec2(70.0, 26.0) * s);
        button(
            ui,
            Control::ButtonSelect,
            at(230.0, 96.0),
            vec2(56.0, 24.0) * s,
        );
        button(
            ui,
            Control::ButtonStart,
            at(290.0, 96.0),
            vec2(56.0, 24.0) * s,
        );
        let _ = PAD_BUTTONS;
        // Sticks.
        for (i, (cx, cy)) in [(110.0, 96.0), (330.0, 184.0)].into_iter().enumerate() {
            let c = at(cx, cy);
            let r = 40.0 * s;
            let ring = Rect::from_center_size(c, Vec2::splat(2.0 * r));
            let resp = ui.interact(ring, Id::unique(("ia-vstick", i)), Sense::drag());
            if resp.dragged()
                && let Some(pp) = resp.interact_pointer_pos()
            {
                let d = (pp - c) / r;
                let len = d.length().max(1.0);
                self.pad.sticks[i] = [d.x / len, -d.y / len];
                self.pad_drag[i] = true;
            } else if self.pad_drag[i] && !resp.dragged() {
                self.pad.sticks[i] = [0.0, 0.0];
                self.pad_drag[i] = false;
            }
            let v = self.pad.sticks[i];
            let painter = ui.painter();
            painter.circle_filled(c, r, st::bg_deep());
            painter.circle_stroke(c, r, Stroke::new(1.0, st::line_control()));
            let knob = c + vec2(v[0], -v[1]) * r * 0.6;
            painter.circle_filled(
                knob,
                r * 0.55,
                if self.pad_drag[i] {
                    st::accent()
                } else {
                    st::bg_active()
                },
            );
            painter.text(
                knob,
                Align2::CENTER_CENTER,
                if i == 0 { "L" } else { "R" },
                st::caption(),
                st::text_1(),
            );
        }
        ui.painter().text(
            pos2(body.center().x, body.bottom() + 22.0),
            Align2::CENTER_CENTER,
            "Virtual gamepad: click buttons, drag the sticks. A connected pad works too.",
            st::caption(),
            st::text_3(),
        );
    }

    fn keyboard(&mut self, ui: &mut Ui, now: f64) {
        let avail = ui.available_rect_before_wrap();
        let card = Rect::from_center_size(
            avail.center(),
            vec2(460.0_f32.min(avail.width() - 20.0), 200.0),
        );
        let resp = ui.interact(card, Id::unique("ia-kb-try"), Sense::click());
        if resp.clicked() {
            resp.request_focus();
        }
        let focused = resp.has_focus();
        ui.painter()
            .rect_filled(card, CornerRadius::same(st::R_LG), st::bg_panel());
        ui.painter().rect_stroke(
            card,
            CornerRadius::same(st::R_LG),
            Stroke::new(
                if focused { 2.0 } else { 1.0 },
                if focused {
                    st::focus()
                } else {
                    st::line_strong()
                },
            ),
            StrokeKind::Inside,
        );
        st::paint_icon(
            ui.painter(),
            Rect::from_center_size(card.center() - vec2(0.0, 52.0), Vec2::splat(28.0)),
            Icon::Keyboard,
            st::accent_text(),
        );
        let title = if focused {
            "Press keys to try your bindings"
        } else {
            "Click here, then press keys"
        };
        ui.painter().text(
            card.center() - vec2(0.0, 18.0),
            Align2::CENTER_CENTER,
            title,
            st::body(),
            st::text_1(),
        );
        ui.painter().text(
            card.center() + vec2(0.0, 4.0),
            Align2::CENTER_CENTER,
            "Tab leaves. The readout on the right shows each action.",
            st::caption(),
            st::text_3(),
        );
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                        tab: false,
                    },
                )
            });
            let mods = ui.input(|i| i.modifiers);
            let mut events = Vec::new();
            ui.input(|i| {
                for e in &i.events {
                    if let Event::Key {
                        key,
                        physical_key,
                        pressed,
                        repeat: false,
                        ..
                    } = e
                        && let Some(c) =
                            super::view::egui_key_to_control(physical_key.unwrap_or(*key))
                    {
                        events.push((c, *pressed));
                    }
                }
            });
            for (flag, c) in [
                (mods.shift, Control::LeftShift),
                (mods.ctrl, Control::LeftControl),
                (mods.alt, Control::LeftAlt),
            ] {
                let was = self.keys_down.contains(&c);
                if flag != was {
                    events.push((c, flag));
                }
            }
            if let Some((_, rt)) = &mut self.runtime {
                for (c, down) in &events {
                    rt.push(RawEvent::button(
                        Device::KeyboardMouse,
                        *c,
                        if *down { 1.0 } else { 0.0 },
                        now,
                    ));
                }
            }
            for (c, down) in events {
                if down {
                    if !self.keys_down.contains(&c) {
                        self.keys_down.push(c);
                    }
                } else {
                    self.keys_down.retain(|k| *k != c);
                }
            }
        } else if !self.keys_down.is_empty() {
            if let Some((_, rt)) = &mut self.runtime {
                rt.release_all(now);
            }
            self.keys_down.clear();
        }
        let mut x = card.center().x - 120.0;
        for c in &self.keys_down {
            let b = metallic_runtime::input::BindingDef::key(*c);
            x += st::paint_glyph(
                ui.painter(),
                pos2(x, card.center().y + 50.0),
                &b,
                GlyphStyle::Letters,
                false,
            ) + 6.0;
        }
    }
}

fn panel_raw(panel: &mut InputActionsPanel) -> Vec<RawEvent> {
    panel.take_raw_events()
}

fn switch_row(ui: &mut Ui, label: &str, on: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .font(st::body())
                .color(st::text_1()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            st::toggle(ui, on, true);
        });
    });
}

fn is_active(v: &ActionValue) -> bool {
    match v {
        ActionValue::Bool(b) => *b,
        ActionValue::Axis1(x) => x.abs() > 1e-3,
        ActionValue::Axis2(v) => v[0].abs() + v[1].abs() > 1e-3,
        ActionValue::Axis3(v) => v.iter().any(|c| c.abs() > 1e-3),
    }
}

fn paint_value(p: &egui::Painter, r: Rect, ty: ActionType, v: Option<ActionValue>) {
    match (ty, v) {
        (ActionType::Bool, v) => {
            let on = matches!(v, Some(ActionValue::Bool(true)));
            let c = pos2(r.right() - 8.0, r.center().y);
            if on {
                p.circle_filled(c, 5.0, st::accent());
            } else {
                p.circle_stroke(c, 4.5, Stroke::new(1.2, st::line_control()));
            }
            p.text(
                c - vec2(14.0, 0.0),
                Align2::RIGHT_CENTER,
                if on { "Down" } else { "Up" },
                st::caption(),
                if on { st::accent_text() } else { st::text_3() },
            );
        }
        (_, Some(ActionValue::Axis2(xy)))
        | (ActionType::Direction2D, Some(ActionValue::Axis3([xy @ .., _]))) => {
            let c = pos2(r.right() - 9.0, r.center().y);
            p.circle_stroke(c, 8.5, Stroke::new(1.0, st::line_control()));
            let len = (xy[0] * xy[0] + xy[1] * xy[1]).sqrt().max(1.0);
            p.circle_filled(c + vec2(xy[0] / len, -xy[1] / len) * 7.0, 2.5, st::accent());
            p.text(
                c - vec2(16.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("{:+.2} {:+.2}", xy[0], xy[1]),
                st::mono(),
                st::text_2(),
            );
        }
        (_, Some(ActionValue::Axis1(x))) => {
            let bar = Rect::from_min_max(
                pos2(r.left() + 30.0, r.center().y - 2.0),
                pos2(r.right(), r.center().y + 2.0),
            );
            p.rect_filled(bar, CornerRadius::same(2), st::bg_deep());
            let mid = bar.center().x;
            let end = mid + x.clamp(-1.0, 1.0) * bar.width() / 2.0;
            p.rect_filled(
                Rect::from_min_max(
                    pos2(mid.min(end), bar.top()),
                    pos2(mid.max(end), bar.bottom()),
                ),
                CornerRadius::same(2),
                st::accent(),
            );
        }
        (_, Some(ActionValue::Axis3(v))) => {
            p.text(
                r.right_center(),
                Align2::RIGHT_CENTER,
                format!("{:+.1} {:+.1} {:+.1}", v[0], v[1], v[2]),
                st::mono(),
                st::text_2(),
            );
        }
        (_, Some(ActionValue::Bool(b))) => {
            p.text(
                r.right_center(),
                Align2::RIGHT_CENTER,
                if b { "Down" } else { "Up" },
                st::caption(),
                st::text_2(),
            );
        }
        (_, None) => {
            p.text(
                r.right_center(),
                Align2::RIGHT_CENTER,
                "-",
                st::caption(),
                st::text_3(),
            );
        }
    }
}

fn warning_chip(p: &egui::Painter, at: Pos2, text: &str) {
    let g = p.layout_no_wrap(text.to_string(), st::caption(), st::error());
    let r = Rect::from_center_size(at, g.size() + vec2(12.0, 6.0));
    p.rect_filled(r, CornerRadius::same(st::R_SM), st::error_bg());
    p.galley(r.min + vec2(6.0, 3.0), g, st::error());
}

fn rect_points(r: Rect) -> Vec<Pos2> {
    vec![
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ]
}

fn circle_points(c: Pos2, r: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            c + vec2(t.cos(), t.sin()) * r
        })
        .collect()
}
