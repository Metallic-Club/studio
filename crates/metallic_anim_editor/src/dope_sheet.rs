//! The dope sheet: one row per track, keys as glyphs on a shared time axis.
//!
//! Mouse: click a key to select (Shift adds, Ctrl toggles), drag keys to move them (snapped to
//! frames; hold Alt to move freely), drag on empty space to box-select, drag the grips above a
//! multi-key selection to scale it in time, click or drag the ruler to scrub, double-click a row
//! to add a key there, right-click for interpolation. Wheel pans, Ctrl+wheel zooms, middle-drag
//! pans. Keys: Delete removes keys, F frames the clip, Ctrl+A selects all, I keys the active
//! track at the playhead. Record keys (K) lives on the transport bar.

use crate::paint::{self, key_glyph};
use crate::state::{DragOp, EditorState, KeyRef};
use crate::theme::Theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use metallic_anim::{Channel, EasingDirection, EasingStyle, Interpolation, TrackTarget};

/// Width of the track name column.
pub const LABEL_WIDTH: f32 = 210.0;
/// Height of the time ruler.
pub const RULER_HEIGHT: f32 = 26.0;
/// Height of the marker / selection band under the ruler.
pub const BAND_HEIGHT: f32 = 18.0;
/// Height of one track row.
pub const ROW_HEIGHT: f32 = 26.0;
const KEY_RADIUS: f32 = 6.0;

/// The dope sheet widget.
#[derive(Debug, Clone, Copy)]
pub struct DopeSheet {
    theme: Theme,
}

impl Default for DopeSheet {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

/// Screen layout of one frame of the dope sheet (also handy for tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DopeSheetLayout {
    /// Whole widget.
    pub rect: Rect,
    /// Ruler.
    pub ruler: Rect,
    /// Marker and selection band.
    pub band: Rect,
    /// Key lanes (all rows, right of the labels).
    pub lanes: Rect,
    /// Track label column.
    pub labels: Rect,
}

impl DopeSheetLayout {
    /// Layout for a widget occupying `rect`.
    pub fn new(rect: Rect) -> Self {
        let labels = Rect::from_min_max(
            pos2(rect.left(), rect.top() + RULER_HEIGHT + BAND_HEIGHT),
            pos2(rect.left() + LABEL_WIDTH, rect.bottom()),
        );
        let ruler = Rect::from_min_max(
            pos2(rect.left() + LABEL_WIDTH, rect.top()),
            pos2(rect.right(), rect.top() + RULER_HEIGHT),
        );
        let band = Rect::from_min_max(
            pos2(ruler.left(), ruler.bottom()),
            pos2(rect.right(), ruler.bottom() + BAND_HEIGHT),
        );
        let lanes = Rect::from_min_max(pos2(ruler.left(), band.bottom()), rect.max);
        Self {
            rect,
            ruler,
            band,
            lanes,
            labels,
        }
    }

    /// Top of row `row` (row 0 is the summary row; track `i` is row `i + 1`).
    pub fn row_top(&self, row: usize) -> f32 {
        self.lanes.top() + row as f32 * ROW_HEIGHT
    }

    /// Screen position of a key.
    pub fn key_pos(&self, state: &EditorState, k: KeyRef) -> Option<Pos2> {
        let t = state.key_time(k)?;
        Some(pos2(
            state.time_view.x(self.lanes.left(), t),
            self.row_top(k.track + 1) + ROW_HEIGHT / 2.0,
        ))
    }
}

/// Short name for a track's row and its secondary label.
pub fn track_labels(target: &TrackTarget) -> (String, String) {
    match target {
        TrackTarget::Joint { joint, channel } => (joint.clone(), channel.label().to_owned()),
        TrackTarget::Property { path, property } => (path.clone(), property.to_lowercase()),
    }
}

fn type_color(theme: &Theme, ch: &Channel) -> Color32 {
    match ch {
        Channel::F32(_) => theme.silver,
        Channel::Vec2(_) | Channel::Vec3(_) => theme.axis[2],
        Channel::Quat(_) => theme.axis[1],
        Channel::Color3(_) => theme.axis[0],
        Channel::Bool(_) => theme.warning,
    }
}

impl DopeSheet {
    /// A dope sheet with these colours.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Shows the dope sheet in all available space.
    pub fn show(self, ui: &mut egui::Ui, state: &mut EditorState) -> egui::Response {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::hover());
        let layout = DopeSheetLayout::new(rect);
        let id = ui.scope_id().with("dope_sheet");
        self.handle_view_input(ui, state, &layout);
        let lanes_resp = ui.interact(layout.lanes, id.with("lanes"), Sense::click_and_drag());
        let ruler_resp = ui.interact(
            Rect::from_min_max(layout.ruler.min, layout.band.max),
            id.with("ruler"),
            Sense::click_and_drag(),
        );
        let label_resp = ui.interact(layout.labels, id.with("labels"), Sense::click());
        self.handle_ruler(ui, state, &layout, &ruler_resp);
        self.handle_lanes(ui, state, &layout, &lanes_resp);
        if label_resp.clicked()
            && let Some(p) = label_resp.interact_pointer_pos()
        {
            let row = ((p.y - layout.lanes.top()) / ROW_HEIGHT).floor() as isize - 1;
            if row >= 0 && (row as usize) < state.clip.tracks.len() {
                state.active_track = Some(row as usize);
            }
        }
        self.handle_keys(ui, state, &resp, &lanes_resp);
        lanes_resp.context_menu(|ui| self.context_menu(ui, state));
        self.paint(ui, state, &layout, lanes_resp.hover_pos());
        resp
    }

    fn handle_view_input(&self, ui: &egui::Ui, state: &mut EditorState, layout: &DopeSheetLayout) {
        let hovered = ui.rect_contains_pointer(layout.rect);
        if !hovered {
            return;
        }
        let (scroll, zoom, modifiers, pointer) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = pointer.map_or(layout.lanes.center().x, |p| p.x);
        if zoom != 1.0 {
            state.time_view.zoom_at(layout.lanes.left(), anchor, zoom);
        } else if modifiers.command && scroll.y != 0.0 {
            state
                .time_view
                .zoom_at(layout.lanes.left(), anchor, (scroll.y / 200.0).exp());
        } else if scroll != egui::Vec2::ZERO {
            let dx = if scroll.x != 0.0 { scroll.x } else { scroll.y };
            state.time_view.start -= dx / state.time_view.px_per_sec;
        }
        let (middle, delta) = ui.input(|i| (i.pointer.middle_down(), i.pointer.delta()));
        if middle {
            state.time_view.start -= delta.x / state.time_view.px_per_sec;
        }
    }

    fn handle_ruler(
        &self,
        ui: &egui::Ui,
        state: &mut EditorState,
        layout: &DopeSheetLayout,
        resp: &egui::Response,
    ) {
        let free = ui.input(|i| i.modifiers.alt);
        // Scale grips live in the band; they take precedence over scrubbing.
        if resp.drag_started()
            && let Some(p) = resp.interact_pointer_pos()
            && layout.band.contains(p)
            && let Some((t0, t1)) = state.selection_span()
            && t1 > t0
        {
            let x0 = state.time_view.x(layout.lanes.left(), t0);
            let x1 = state.time_view.x(layout.lanes.left(), t1);
            if (p.x - x0).abs() < 8.0 {
                state.drag = Some(DragOp::ScaleKeys {
                    pivot: t1,
                    grabbed: t0,
                    factor: 1.0,
                });
            } else if (p.x - x1).abs() < 8.0 {
                state.drag = Some(DragOp::ScaleKeys {
                    pivot: t0,
                    grabbed: t1,
                    factor: 1.0,
                });
            }
        }
        if let Some(DragOp::ScaleKeys {
            pivot,
            grabbed,
            factor,
        }) = &mut state.drag
        {
            if let Some(p) = resp.interact_pointer_pos() {
                let t = state.time_view.time(layout.lanes.left(), p.x);
                let t = if state.snap && !free {
                    state.clip.snap_to_frame(t)
                } else {
                    t
                };
                let span = *grabbed - *pivot;
                if span.abs() > 1e-6 {
                    *factor = ((t - *pivot) / span).max(0.0);
                }
            }
            if resp.drag_stopped() {
                let (pivot, factor) = (*pivot, *factor);
                state.drag = None;
                state.scale_selected(pivot, factor);
            }
            return;
        }
        if (resp.clicked() || resp.dragged())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let t = state.time_view.time(layout.lanes.left(), p.x);
            let t = if state.snap && !free {
                state.clip.snap_to_frame(t)
            } else {
                t
            };
            state.time = t.clamp(0.0, state.clip.duration);
            state.drag = if resp.dragged() {
                Some(DragOp::Scrub)
            } else {
                None
            };
        }
        if resp.drag_stopped() && state.drag == Some(DragOp::Scrub) {
            state.drag = None;
        }
    }

    fn key_at(&self, state: &EditorState, layout: &DopeSheetLayout, p: Pos2) -> Option<KeyRef> {
        let mut best: Option<(f32, KeyRef)> = None;
        for k in state.all_keys() {
            if let Some(kp) = layout.key_pos(state, k) {
                let d = kp.distance(p);
                if d <= KEY_RADIUS + 3.0 && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, k));
                }
            }
        }
        best.map(|b| b.1)
    }

    fn handle_lanes(
        &self,
        ui: &egui::Ui,
        state: &mut EditorState,
        layout: &DopeSheetLayout,
        resp: &egui::Response,
    ) {
        let modifiers = ui.input(|i| i.modifiers);
        let pointer = resp.interact_pointer_pos();
        let left = layout.lanes.left();
        if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = pointer
        {
            // The press position, not the position after the drag threshold.
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
            if let Some(k) = self.key_at(state, layout, press) {
                if !state.selection.contains(&k) {
                    if !modifiers.shift {
                        state.selection.clear();
                    }
                    state.selection.insert(k);
                }
                state.active_track = Some(k.track);
                let grab = state.key_time(k).unwrap_or(0.0);
                state.drag = Some(DragOp::MoveKeys {
                    grab_time: grab,
                    delta: 0.0,
                });
            } else {
                state.drag = Some(DragOp::BoxSelect {
                    from: press,
                    to: p,
                    additive: modifiers.shift,
                });
            }
        }
        match &mut state.drag {
            Some(DragOp::MoveKeys { grab_time, delta }) => {
                if let Some(p) = pointer {
                    let t = state.time_view.time(left, p.x);
                    let target = if state.snap && !modifiers.alt {
                        state.clip.snap_to_frame(t)
                    } else {
                        t
                    };
                    *delta = target - *grab_time;
                }
                if resp.drag_stopped() {
                    let d = *delta;
                    state.drag = None;
                    let snap = state.snap;
                    state.snap = snap && !modifiers.alt;
                    if d != 0.0 {
                        state.move_selected(d);
                    }
                    state.snap = snap;
                }
            }
            Some(DragOp::BoxSelect { to, .. }) => {
                if let Some(p) = pointer {
                    *to = p;
                }
                if resp.drag_stopped()
                    && let Some(DragOp::BoxSelect { from, to, additive }) = state.drag.take()
                {
                    let r = Rect::from_two_pos(from, to);
                    if !additive {
                        state.selection.clear();
                    }
                    for k in state.all_keys() {
                        if layout
                            .key_pos(state, k)
                            .is_some_and(|kp| r.expand(2.0).contains(kp))
                        {
                            state.selection.insert(k);
                        }
                    }
                }
            }
            _ => {}
        }
        if resp.clicked()
            && let Some(p) = pointer
        {
            match self.key_at(state, layout, p) {
                Some(k) => {
                    if modifiers.command {
                        if !state.selection.remove(&k) {
                            state.selection.insert(k);
                        }
                    } else if modifiers.shift {
                        state.selection.insert(k);
                    } else {
                        state.selection.clear();
                        state.selection.insert(k);
                    }
                    state.active_track = Some(k.track);
                    if let Some(t) = state.key_time(k) {
                        state.time = t;
                    }
                }
                None => {
                    if !modifiers.shift {
                        state.selection.clear();
                    }
                    let row = ((p.y - layout.lanes.top()) / ROW_HEIGHT).floor() as isize - 1;
                    if row >= 0 && (row as usize) < state.clip.tracks.len() {
                        state.active_track = Some(row as usize);
                    }
                }
            }
        }
        if resp.double_clicked()
            && let Some(p) = pointer
            && self.key_at(state, layout, p).is_none()
        {
            let row = ((p.y - layout.lanes.top()) / ROW_HEIGHT).floor() as isize - 1;
            if row >= 0 && (row as usize) < state.clip.tracks.len() {
                let t = state
                    .time_view
                    .time(left, p.x)
                    .clamp(0.0, state.clip.duration);
                state.time = state.snap_time(t);
                state.key_track_at_playhead(row as usize);
            }
        }
    }

    fn handle_keys(
        &self,
        ui: &egui::Ui,
        state: &mut EditorState,
        area: &egui::Response,
        lanes: &egui::Response,
    ) {
        if !(area.hovered() || lanes.hovered() || lanes.has_focus())
            || ui.ctx().egui_wants_keyboard_input()
        {
            return;
        }
        let (del, f, all, k) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::F),
                i.modifiers.command && i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::I),
            )
        });
        if del {
            state.delete_selected();
        }
        if f {
            let w = (area.rect.width() - LABEL_WIDTH).max(50.0);
            state.time_view.fit(0.0, state.clip.duration, w);
        }
        if all {
            state.selection = state.all_keys().into_iter().collect();
        }
        if k && let Some(t) = state.active_track {
            state.key_track_at_playhead(t);
        }
    }

    fn context_menu(&self, ui: &mut egui::Ui, state: &mut EditorState) {
        if state.selection.is_empty() {
            if let Some(t) = state.active_track
                && ui.button("Add key at playhead").clicked()
            {
                state.key_track_at_playhead(t);
                ui.close();
            }
            ui.label(egui::RichText::new("Select keys to change their interpolation.").weak());
            return;
        }
        ui.label(egui::RichText::new(format!("{} keys selected", state.selection.len())).weak());
        ui.separator();
        for (label, interp) in [
            ("Bezier (smooth)", Interpolation::Bezier),
            ("Linear", Interpolation::Linear),
            ("Constant (step)", Interpolation::Constant),
        ] {
            if ui.button(label).clicked() {
                state.set_selected_interpolation(interp);
                ui.close();
            }
        }
        ui.menu_button("Ease", |ui| {
            for style in EasingStyle::ALL {
                ui.menu_button(style.label(), |ui| {
                    for dir in EasingDirection::ALL {
                        if ui.button(dir.label()).clicked() {
                            state.set_selected_interpolation(Interpolation::Eased(style, dir));
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.separator();
        if ui.button("Delete keys").clicked() {
            state.delete_selected();
            ui.close();
        }
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        state: &EditorState,
        layout: &DopeSheetLayout,
        hover: Option<Pos2>,
    ) {
        let t = &self.theme;
        let p = ui.painter_at(layout.rect);
        let left = layout.lanes.left();
        let view = state.time_view;
        p.rect_filled(layout.rect, 0.0, t.deep);

        // Rows.
        let rows = state.clip.tracks.len() + 1;
        for row in 0..rows {
            let top = layout.row_top(row);
            if top > layout.rect.bottom() {
                break;
            }
            let r = Rect::from_min_max(
                pos2(layout.rect.left(), top),
                pos2(layout.rect.right(), top + ROW_HEIGHT),
            );
            let active = row > 0 && state.active_track == Some(row - 1);
            let bg = if row == 0 {
                t.base
            } else if active {
                t.select_weak
            } else if row % 2 == 0 {
                Color32::from_rgb(0x14, 0x13, 0x19)
            } else {
                t.deep
            };
            p.rect_filled(r, 0.0, bg);
            p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, t.grid));
        }

        paint::ruler(
            &p,
            t,
            layout.ruler,
            layout.lanes,
            &view,
            state.clip.frame_rate,
            state.clip.duration,
        );
        p.rect_filled(layout.band, 0.0, t.base);
        p.hline(
            layout.band.x_range(),
            layout.band.bottom() - 0.5,
            t.hairline(),
        );

        // Label column.
        let header = Rect::from_min_max(
            layout.rect.min,
            pos2(layout.labels.right(), layout.band.bottom()),
        );
        p.rect_filled(header, 0.0, t.panel);
        p.text(
            pos2(header.left() + 12.0, header.top() + 12.0),
            Align2::LEFT_CENTER,
            &state.clip.name,
            FontId::proportional(13.0),
            t.text,
        );
        p.text(
            pos2(header.left() + 12.0, header.top() + 30.0),
            Align2::LEFT_CENTER,
            format!(
                "{} tracks  ·  {} keys  ·  {:.0} fps",
                state.clip.tracks.len(),
                state.key_count(),
                state.clip.frame_rate
            ),
            FontId::proportional(10.5),
            t.text_dim,
        );
        p.rect_filled(layout.labels, 0.0, t.panel);
        let label_font = FontId::proportional(12.5);
        let sub_font = FontId::proportional(10.5);
        {
            let top = layout.row_top(0);
            p.text(
                pos2(layout.labels.left() + 12.0, top + ROW_HEIGHT / 2.0),
                Align2::LEFT_CENTER,
                "Summary",
                label_font.clone(),
                t.text_dim,
            );
        }
        for (i, track) in state.clip.tracks.iter().enumerate() {
            let top = layout.row_top(i + 1);
            if top > layout.rect.bottom() {
                break;
            }
            let cy = top + ROW_HEIGHT / 2.0;
            let active = state.active_track == Some(i);
            if active {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(layout.labels.left(), top + 3.0),
                        vec2(2.0, ROW_HEIGHT - 6.0),
                    ),
                    1.0,
                    t.accent,
                );
                p.rect_filled(
                    Rect::from_min_max(
                        pos2(layout.labels.left(), top),
                        pos2(layout.labels.right(), top + ROW_HEIGHT),
                    ),
                    0.0,
                    t.select_weak,
                );
            }
            p.circle_filled(
                pos2(layout.labels.left() + 16.0, cy),
                3.0,
                type_color(t, &track.channel),
            );
            let (name, sub) = track_labels(&track.target);
            let name_rect = p.text(
                pos2(layout.labels.left() + 27.0, cy),
                Align2::LEFT_CENTER,
                name,
                label_font.clone(),
                if active { t.text } else { t.silver },
            );
            p.text(
                pos2(name_rect.right() + 6.0, cy + 0.5),
                Align2::LEFT_CENTER,
                sub,
                sub_font.clone(),
                t.text_dim,
            );
            p.text(
                pos2(layout.labels.right() - 10.0, cy),
                Align2::RIGHT_CENTER,
                track.channel.type_label(),
                sub_font.clone(),
                t.text_meta.gamma_multiply(0.8),
            );
            p.hline(
                layout.labels.x_range(),
                top + ROW_HEIGHT - 0.5,
                Stroke::new(1.0, t.grid),
            );
        }
        p.vline(
            layout.labels.right() - 0.5,
            layout.rect.y_range(),
            t.hairline(),
        );

        let lanes_p = ui.painter_at(layout.lanes);

        // Markers.
        let band_p = ui.painter_at(layout.band);
        for m in &state.clip.markers {
            let x = view.x(left, m.time);
            let base = layout.band.bottom() - 3.0;
            let flag = vec![
                pos2(x, base - 11.0),
                pos2(x + 8.0, base - 8.0),
                pos2(x, base - 5.0),
            ];
            band_p.vline(x, (base - 12.0)..=base, Stroke::new(1.0, t.warning));
            band_p.add(Shape::convex_polygon(flag, t.warning, Stroke::NONE));
            band_p.text(
                pos2(x + 11.0, base - 7.5),
                Align2::LEFT_CENTER,
                &m.name,
                FontId::proportional(10.0),
                t.warning.gamma_multiply(0.85),
            );
            lanes_p.vline(
                x,
                layout.lanes.y_range(),
                Stroke::new(1.0, t.warning.gamma_multiply(0.18)),
            );
        }

        // Selection span with scale grips.
        let drag_delta = match &state.drag {
            Some(DragOp::MoveKeys { delta, .. }) => *delta,
            _ => 0.0,
        };
        let scale = match &state.drag {
            Some(DragOp::ScaleKeys { pivot, factor, .. }) => Some((*pivot, *factor)),
            _ => None,
        };
        let shown_time = |time: f32| -> f32 {
            match scale {
                Some((pivot, f)) => pivot + (time - pivot) * f,
                None => time + drag_delta,
            }
        };
        if let Some((t0, t1)) = state.selection_span()
            && t1 > t0
            && state.selection.len() > 1
        {
            let x0 = view.x(left, shown_time(t0));
            let x1 = view.x(left, shown_time(t1));
            let y = layout.band.center().y + 1.0;
            band_p.rect_filled(
                Rect::from_min_max(pos2(x0, y - 2.0), pos2(x1, y + 2.0)),
                2.0,
                t.accent.gamma_multiply(0.35),
            );
            for x in [x0, x1] {
                let g = Rect::from_center_size(pos2(x, y), vec2(6.0, 12.0));
                let hot = hover.is_some_and(|h| (h.x - x).abs() < 8.0) || scale.is_some();
                band_p.rect(
                    g,
                    2.0,
                    if hot { t.accent } else { t.silver },
                    Stroke::new(1.0, t.deep),
                    StrokeKind::Middle,
                );
            }
        }

        // Summary keys (union of times).
        let mut times: Vec<f32> = state
            .clip
            .tracks
            .iter()
            .flat_map(|tr| tr.channel.key_times())
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        let sy = layout.row_top(0) + ROW_HEIGHT / 2.0;
        for time in times {
            let x = view.x(left, time);
            let pts = vec![
                pos2(x, sy - 4.5),
                pos2(x + 4.5, sy),
                pos2(x, sy + 4.5),
                pos2(x - 4.5, sy),
            ];
            lanes_p.add(Shape::convex_polygon(pts, t.text_meta, Stroke::NONE));
        }

        // Keys, with "hold" bars between equal neighbours.
        let hovered_key = hover.and_then(|h| self.key_at(state, layout, h));
        for (ti, track) in state.clip.tracks.iter().enumerate() {
            let cy = layout.row_top(ti + 1) + ROW_HEIGHT / 2.0;
            let key_times = track.channel.key_times();
            for (ki, pair) in key_times.windows(2).enumerate() {
                if track.channel.key_interpolation(ki) == Some(Interpolation::Constant)
                    || equal_values(&track.channel, ki)
                {
                    let r = Rect::from_min_max(
                        pos2(view.x(left, pair[0]), cy - 2.0),
                        pos2(view.x(left, pair[1]), cy + 2.0),
                    );
                    lanes_p.rect_filled(r, 1.0, t.text_meta.gamma_multiply(0.35));
                }
            }
            for (ki, &time) in key_times.iter().enumerate() {
                let k = KeyRef { track: ti, key: ki };
                let selected = state.selection.contains(&k);
                let shown = if selected { shown_time(time) } else { time };
                if selected && (scale.is_some() || drag_delta != 0.0) {
                    key_glyph(
                        &lanes_p,
                        t,
                        pos2(view.x(left, time), cy),
                        KEY_RADIUS - 1.0,
                        Interpolation::Bezier,
                        false,
                        false,
                    );
                }
                let interp = track.channel.key_interpolation(ki).unwrap_or_default();
                key_glyph(
                    &lanes_p,
                    t,
                    pos2(view.x(left, shown), cy),
                    KEY_RADIUS,
                    interp,
                    selected,
                    hovered_key == Some(k),
                );
            }
        }

        // Box select.
        if let Some(DragOp::BoxSelect { from, to, .. }) = &state.drag {
            let r = Rect::from_two_pos(*from, *to);
            lanes_p.rect_filled(r, 2.0, t.accent_soft);
            lanes_p.rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }

        // Playhead.
        let x = view.x(left, state.time);
        if x >= left {
            paint::playhead(
                &p,
                t,
                x,
                layout.ruler,
                layout.rect.bottom(),
                &paint::format_time(state.time, state.clip.frame_rate),
            );
        }
    }
}

/// True when key `i` and key `i + 1` hold the same value (drawn as a hold bar).
fn equal_values(channel: &Channel, i: usize) -> bool {
    metallic_anim::with_curve!(channel, c => {
        let k = c.keys();
        match (k.get(i), k.get(i + 1)) {
            (Some(a), Some(b)) => a.value == b.value,
            _ => false,
        }
    })
}
