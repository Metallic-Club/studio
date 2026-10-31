//! The curve editor: value-over-time graphs of the active track with Bezier handles.
//!
//! Mouse: click a key to select (Shift adds), drag keys to change value and time (time snaps to
//! frames; hold Alt to move freely), drag a handle to shape the curve (an automatic handle
//! becomes Aligned), drag on empty space to box-select, click or drag the ruler to scrub. Wheel
//! zooms both axes (Ctrl: time only, Shift: value only), middle-drag pans. Keys: F frames all,
//! Delete removes keys. The toolbar sets tangent modes and interpolation.

use crate::paint::{self, Icon};
use crate::scalar::{self, HandleSide};
use crate::state::{DragOp, EditorState, KeyRef};
use crate::theme::Theme;
use egui::{Align2, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use metallic_anim::{Interpolation, TangentMode};

const GUTTER: f32 = 56.0;
const TOOLBAR: f32 = 34.0;
const RULER: f32 = 24.0;
const KEY_HIT: f32 = 8.0;

/// The curve editor widget.
#[derive(Debug, Clone, Copy)]
pub struct CurveEditor {
    theme: Theme,
}

impl Default for CurveEditor {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

#[derive(Debug, Clone, Copy)]
struct Plot {
    rect: Rect,
    toolbar: Rect,
    ruler: Rect,
    gutter: Rect,
    area: Rect,
}

impl Plot {
    fn new(rect: Rect) -> Self {
        let toolbar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + TOOLBAR));
        let ruler = Rect::from_min_max(
            pos2(rect.left() + GUTTER, toolbar.bottom()),
            pos2(rect.right(), toolbar.bottom() + RULER),
        );
        let gutter = Rect::from_min_max(
            pos2(rect.left(), ruler.bottom()),
            pos2(rect.left() + GUTTER, rect.bottom()),
        );
        let area = Rect::from_min_max(pos2(ruler.left(), ruler.bottom()), rect.max);
        Self {
            rect,
            toolbar,
            ruler,
            gutter,
            area,
        }
    }

    fn y(&self, s: &EditorState, v: f32) -> f32 {
        let vv = s.value_view;
        let span = (vv.max - vv.min).max(1e-6);
        self.area.bottom() - (v - vv.min) / span * self.area.height()
    }

    fn value(&self, s: &EditorState, y: f32) -> f32 {
        let vv = s.value_view;
        vv.min + (self.area.bottom() - y) / self.area.height().max(1.0) * (vv.max - vv.min)
    }

    fn x(&self, s: &EditorState, t: f32) -> f32 {
        s.time_view.x(self.area.left(), t)
    }

    fn time(&self, s: &EditorState, x: f32) -> f32 {
        s.time_view.time(self.area.left(), x)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    Key(KeyRef, usize),
    Handle(KeyRef, usize, HandleSide),
}

impl CurveEditor {
    /// A curve editor with these colours.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Shows the curve editor in all available space.
    pub fn show(self, ui: &mut egui::Ui, state: &mut EditorState) -> egui::Response {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::hover());
        let plot = Plot::new(rect);
        let id = ui.scope_id().with("curve_editor");
        self.toolbar(ui, state, &plot);
        self.view_input(ui, state, &plot);
        let area = ui.interact(plot.area, id.with("area"), Sense::click_and_drag());
        let ruler = ui.interact(plot.ruler, id.with("ruler"), Sense::click_and_drag());
        if (ruler.clicked() || ruler.dragged())
            && let Some(p) = ruler.interact_pointer_pos()
        {
            let t = plot.time(state, p.x);
            state.time = state.snap_time(t).clamp(0.0, state.clip.duration);
        }
        self.area_input(ui, state, &plot, &area);
        if resp.hovered() || area.hovered() {
            let (f, del) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::F),
                    i.key_pressed(egui::Key::Delete),
                )
            });
            if f {
                self.frame_all(state, &plot);
            }
            if del {
                state.delete_selected();
            }
        }
        self.paint(ui, state, &plot, area.hover_pos());
        resp
    }

    fn frame_all(&self, state: &mut EditorState, plot: &Plot) {
        state.frame_all_values();
        let end = state.clip.duration.max(state.clip.last_key_time());
        state.time_view.fit(0.0, end, plot.area.width());
    }

    fn toolbar(&self, ui: &mut egui::Ui, state: &mut EditorState, plot: &Plot) {
        let t = &self.theme;
        ui.painter().rect_filled(plot.toolbar, 0.0, t.panel);
        ui.painter().hline(
            plot.toolbar.x_range(),
            plot.toolbar.bottom() - 0.5,
            t.hairline(),
        );
        let inner = plot.toolbar.shrink2(vec2(10.0, 5.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        let name = state
            .active_track
            .and_then(|i| state.clip.tracks.get(i))
            .map(|tr| tr.target.label())
            .unwrap_or_else(|| "No track".to_owned());
        ui.label(egui::RichText::new(name).strong().color(t.text));
        if let Some(track) = state.active_track.and_then(|i| state.clip.tracks.get(i)) {
            for (c, label) in scalar::labels(&track.channel).iter().enumerate() {
                let col = if scalar::labels(&track.channel).len() == 1 {
                    t.silver
                } else {
                    t.axis[c % 3]
                };
                let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, col);
                ui.label(egui::RichText::new(*label).size(11.0).color(t.text_dim));
            }
        }
        ui.add_space(12.0);
        let has_sel = !state.selection.is_empty();
        let mode = state.selected_tangent_mode();
        ui.add_enabled_ui(has_sel, |ui| {
            ui.label(egui::RichText::new("Tangents").size(11.0).color(t.text_dim));
            for m in TangentMode::ALL {
                if ui
                    .selectable_label(mode == Some(m), egui::RichText::new(m.label()).size(11.5))
                    .clicked()
                {
                    state.set_selected_tangent_mode(m);
                }
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Interpolation")
                    .size(11.0)
                    .color(t.text_dim),
            );
            let interp = state.selected_interpolation();
            for (label, i) in [
                ("Bezier", Interpolation::Bezier),
                ("Linear", Interpolation::Linear),
                ("Step", Interpolation::Constant),
            ] {
                if ui
                    .selectable_label(interp == Some(i), egui::RichText::new(label).size(11.5))
                    .clicked()
                {
                    state.set_selected_interpolation(i);
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if paint::icon_button(ui, t, Icon::FrameAll, false, "Frame all (F)").clicked() {
                self.frame_all(state, plot);
            }
        });
    }

    fn view_input(&self, ui: &egui::Ui, state: &mut EditorState, plot: &Plot) {
        if !ui.rect_contains_pointer(plot.area) {
            return;
        }
        let (scroll, zoom, mods, hover, middle, delta) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
                i.pointer.middle_down(),
                i.pointer.delta(),
            )
        });
        let factor = if zoom != 1.0 {
            zoom
        } else {
            (scroll / 240.0).exp()
        };
        if factor != 1.0
            && let Some(h) = hover
        {
            if !mods.shift {
                state.time_view.zoom_at(plot.area.left(), h.x, factor);
            }
            if !mods.command {
                let v = plot.value(state, h.y);
                let vv = &mut state.value_view;
                vv.min = v - (v - vv.min) / factor;
                vv.max = v + (vv.max - v) / factor;
            }
        }
        if middle {
            state.time_view.start -= delta.x / state.time_view.px_per_sec;
            let dv = delta.y / plot.area.height().max(1.0)
                * (state.value_view.max - state.value_view.min);
            state.value_view.min += dv;
            state.value_view.max += dv;
        }
    }

    fn hit(&self, state: &EditorState, plot: &Plot, p: Pos2) -> Option<Hit> {
        let ti = state.active_track?;
        let track = state.clip.tracks.get(ti)?;
        let comps = scalar::labels(&track.channel).len();
        let mut best: Option<(f32, Hit)> = None;
        let mut consider = |d: f32, h: Hit| {
            if d <= KEY_HIT && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, h));
            }
        };
        for ki in 0..track.channel.key_count() {
            let k = KeyRef { track: ti, key: ki };
            let Some(time) = track.channel.key_time(ki) else {
                continue;
            };
            for c in 0..comps {
                if let Some(v) = scalar::key_value(&track.channel, ki, c) {
                    consider(
                        pos2(plot.x(state, time), plot.y(state, v)).distance(p),
                        Hit::Key(k, c),
                    );
                }
                if state.selection.contains(&k) {
                    for side in [HandleSide::In, HandleSide::Out] {
                        if let Some((ht, hv)) = scalar::handle_point(&track.channel, ki, c, side) {
                            let hp = handle_screen(
                                plot,
                                state,
                                time,
                                track_value(&track.channel, ki, c),
                                ht,
                                hv,
                            );
                            consider(hp.distance(p) - 0.5, Hit::Handle(k, c, side));
                        }
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    fn area_input(
        &self,
        ui: &egui::Ui,
        state: &mut EditorState,
        plot: &Plot,
        resp: &egui::Response,
    ) {
        let mods = ui.input(|i| i.modifiers);
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = pointer
        {
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
            match self.hit(state, plot, press) {
                Some(Hit::Handle(k, c, side)) => {
                    state.drag = Some(DragOp::Handle {
                        key: k,
                        comp: c,
                        side,
                    })
                }
                Some(Hit::Key(k, c)) => {
                    if !state.selection.contains(&k) {
                        if !mods.shift {
                            state.selection.clear();
                        }
                        state.selection.insert(k);
                    }
                    let originals = state
                        .selection
                        .iter()
                        .filter(|s| s.track == k.track)
                        .filter_map(|s| {
                            let tr = state.clip.tracks.get(s.track)?;
                            Some((
                                *s,
                                tr.channel.key_time(s.key)?,
                                scalar::key_value(&tr.channel, s.key, c)?,
                            ))
                        })
                        .collect();
                    state.drag = Some(DragOp::CurveKeys {
                        comp: c,
                        grab: (plot.time(state, press.x), plot.value(state, press.y)),
                        originals,
                    });
                }
                None => {
                    state.drag = Some(DragOp::BoxSelect {
                        from: press,
                        to: p,
                        additive: mods.shift,
                    })
                }
            }
        }
        match state.drag.clone() {
            Some(DragOp::Handle { key, comp, side }) => {
                if let Some(p) = pointer {
                    let (t, v) = (plot.time(state, p.x), plot.value(state, p.y));
                    let Some(track) = state.clip.tracks.get_mut(key.track) else {
                        return;
                    };
                    scalar::set_handle_point(&mut track.channel, key.key, comp, side, t, v);
                    state.dirty = true;
                }
                if resp.drag_stopped() {
                    state.drag = None;
                }
            }
            Some(DragOp::CurveKeys {
                comp,
                grab,
                originals,
            }) => {
                if let Some(p) = pointer {
                    let dv = plot.value(state, p.y) - grab.1;
                    for (k, _, v0) in &originals {
                        if let Some(track) = state.clip.tracks.get_mut(k.track) {
                            scalar::set_key_value(&mut track.channel, k.key, comp, v0 + dv);
                        }
                    }
                    state.dirty = true;
                }
                if resp.drag_stopped() {
                    state.drag = None;
                    if let Some(p) = pointer {
                        let raw = plot.time(state, p.x) - grab.0;
                        let snap = state.snap && !mods.alt;
                        let first = originals.first().map_or(0.0, |o| o.1);
                        let dt = if snap {
                            state.clip.snap_to_frame(first + raw) - first
                        } else {
                            raw
                        };
                        if dt.abs() > 1e-5 {
                            let s = state.snap;
                            state.snap = snap;
                            state.move_selected(dt);
                            state.snap = s;
                        }
                    }
                }
            }
            Some(DragOp::BoxSelect { from, additive, .. }) => {
                if let Some(p) = pointer {
                    state.drag = Some(DragOp::BoxSelect {
                        from,
                        to: p,
                        additive,
                    });
                }
                if resp.drag_stopped() {
                    let to = pointer.unwrap_or(from);
                    state.drag = None;
                    let r = Rect::from_two_pos(from, to).expand(3.0);
                    if !additive {
                        state.selection.clear();
                    }
                    if let Some(ti) = state.active_track
                        && let Some(track) = state.clip.tracks.get(ti)
                    {
                        let comps = scalar::labels(&track.channel).len();
                        for ki in 0..track.channel.key_count() {
                            let time = track.channel.key_time(ki).unwrap_or(0.0);
                            let inside = (0..comps).any(|c| {
                                scalar::key_value(&track.channel, ki, c).is_some_and(|v| {
                                    r.contains(pos2(plot.x(state, time), plot.y(state, v)))
                                })
                            });
                            if inside {
                                state.selection.insert(KeyRef { track: ti, key: ki });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if resp.clicked()
            && let Some(p) = pointer
        {
            match self.hit(state, plot, p) {
                Some(Hit::Key(k, _)) => {
                    if !mods.shift {
                        state.selection.clear();
                    }
                    state.selection.insert(k);
                }
                Some(Hit::Handle(..)) => {}
                None => {
                    if !mods.shift {
                        state.selection.clear();
                    }
                }
            }
        }
    }

    fn paint(&self, ui: &egui::Ui, state: &EditorState, plot: &Plot, hover: Option<Pos2>) {
        let t = &self.theme;
        let p = ui.painter_at(plot.rect);
        p.rect_filled(plot.area, 0.0, t.deep);
        p.rect_filled(plot.gutter, 0.0, t.panel);
        p.rect_filled(
            Rect::from_min_max(
                pos2(plot.rect.left(), plot.ruler.top()),
                pos2(plot.gutter.right(), plot.ruler.bottom()),
            ),
            0.0,
            t.panel,
        );
        paint::ruler(
            &p,
            t,
            plot.ruler,
            plot.area,
            &state.time_view,
            state.clip.frame_rate,
            state.clip.duration,
        );
        let ap = ui.painter_at(plot.area);

        // Value grid.
        let vv = state.value_view;
        let step = nice_step((vv.max - vv.min) / (plot.area.height() / 48.0).max(1.0));
        let mut v = (vv.min / step).floor() * step;
        while v <= vv.max {
            let y = plot.y(state, v).round() + 0.5;
            let zero = v.abs() < step * 0.01;
            ap.hline(
                plot.area.x_range(),
                y,
                Stroke::new(1.0, if zero { t.line } else { t.grid }),
            );
            if y > plot.area.top() + 6.0 {
                p.text(
                    pos2(plot.gutter.right() - 8.0, y),
                    Align2::RIGHT_CENTER,
                    fmt_value(v, step),
                    FontId::monospace(10.0),
                    t.text_dim,
                );
            }
            v += step;
        }
        p.vline(
            plot.gutter.right() - 0.5,
            plot.gutter.y_range(),
            t.hairline(),
        );

        let Some(ti) = state.active_track else {
            ap.text(
                plot.area.center(),
                Align2::CENTER_CENTER,
                "Select a track to see its curves",
                FontId::proportional(13.0),
                t.text_dim,
            );
            return;
        };
        let Some(track) = state.clip.tracks.get(ti) else {
            return;
        };
        let labels = scalar::labels(&track.channel);
        let comps = labels.len();
        let hovered = hover.and_then(|h| self.hit(state, plot, h));
        let left = plot.area.left();
        let end_x = plot.area.right();

        for c in 0..comps {
            let col = if comps == 1 { t.silver } else { t.axis[c % 3] };
            // Curve, sampled every 2 px; dimmer outside the clip.
            let mut pts = Vec::new();
            let mut x = left;
            while x <= end_x + 2.0 {
                let time = state.time_view.time(left, x);
                if let Some(v) = scalar::sample(&track.channel, c, time.clamp(0.0, f32::MAX)) {
                    pts.push(pos2(x, plot.y(state, v)));
                }
                x += 2.0;
            }
            if pts.len() > 1 {
                ap.add(Shape::line(pts, Stroke::new(1.8, col)));
            }
            // Keys and handles.
            for ki in 0..track.channel.key_count() {
                let k = KeyRef { track: ti, key: ki };
                let (Some(time), Some(v)) = (
                    track.channel.key_time(ki),
                    scalar::key_value(&track.channel, ki, c),
                ) else {
                    continue;
                };
                let kp = pos2(plot.x(state, time), plot.y(state, v));
                let selected = state.selection.contains(&k);
                if selected {
                    for side in [HandleSide::In, HandleSide::Out] {
                        if let Some((ht, hv)) = scalar::handle_point(&track.channel, ki, c, side) {
                            if (ht - time).abs() < 1e-6 {
                                continue;
                            }
                            let hp = handle_screen(plot, state, time, v, ht, hv);
                            ap.line_segment(
                                [kp, hp],
                                Stroke::new(1.0, t.accent.gamma_multiply(0.75)),
                            );
                            let hot = hovered == Some(Hit::Handle(k, c, side));
                            ap.circle(
                                hp,
                                if hot { 4.5 } else { 3.5 },
                                t.deep,
                                Stroke::new(1.5, if hot { t.accent_hover } else { t.accent }),
                            );
                        }
                    }
                }
                let hot = hovered == Some(Hit::Key(k, c));
                let interp = track.channel.key_interpolation(ki).unwrap_or_default();
                let fill = if selected {
                    t.accent
                } else if hot {
                    t.accent_hover
                } else {
                    t.text
                };
                let r = if hot { 5.5 } else { 4.5 };
                let diamond = vec![
                    pos2(kp.x, kp.y - r),
                    pos2(kp.x + r, kp.y),
                    pos2(kp.x, kp.y + r),
                    pos2(kp.x - r, kp.y),
                ];
                match interp {
                    Interpolation::Constant => {
                        ap.rect(
                            Rect::from_center_size(kp, vec2(r * 1.6, r * 1.6)),
                            1.0,
                            fill,
                            Stroke::new(1.0, t.deep),
                            StrokeKind::Middle,
                        );
                    }
                    Interpolation::Linear => {
                        ap.circle(kp, r * 0.9, fill, Stroke::new(1.0, t.deep));
                    }
                    _ => {
                        ap.add(Shape::convex_polygon(
                            diamond,
                            fill,
                            Stroke::new(1.0, t.deep),
                        ));
                    }
                }
            }
        }

        if let Some(DragOp::BoxSelect { from, to, .. }) = &state.drag {
            let r = Rect::from_two_pos(*from, *to);
            ap.rect_filled(r, 2.0, t.accent_soft);
            ap.rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }
        if let Some(DragOp::CurveKeys { grab, .. }) = &state.drag
            && let Some(h) = hover
        {
            let dt = plot.time(state, h.x) - grab.0;
            if dt.abs() * state.clip.frame_rate >= 0.5 {
                ap.text(
                    h + vec2(12.0, -14.0),
                    Align2::LEFT_BOTTOM,
                    format!("{:+.0} frames", dt * state.clip.frame_rate),
                    FontId::proportional(11.0),
                    t.text_dim,
                );
            }
        }
        let x = plot.x(state, state.time);
        paint::playhead(
            &p,
            t,
            x,
            plot.ruler,
            plot.rect.bottom(),
            &paint::format_time(state.time, state.clip.frame_rate),
        );
    }
}

fn track_value(channel: &metallic_anim::Channel, key: usize, comp: usize) -> f32 {
    scalar::key_value(channel, key, comp).unwrap_or(0.0)
}

/// Screen position of a handle point (its true position, like a graph editor).
fn handle_screen(plot: &Plot, state: &EditorState, _kt: f32, _kv: f32, ht: f32, hv: f32) -> Pos2 {
    pos2(plot.x(state, ht), plot.y(state, hv))
}

fn nice_step(raw: f32) -> f32 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let mag = 10f32.powf(raw.log10().floor());
    let n = raw / mag;
    let m = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    m * mag
}

fn fmt_value(v: f32, step: f32) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 4.0) as usize;
    format!("{v:.decimals$}")
}
