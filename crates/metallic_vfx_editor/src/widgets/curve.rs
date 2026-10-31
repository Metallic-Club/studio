//! Inline curve editor for [`Curve`] (over-life sizes, transparency, trail widths).
//!
//! Drag a key to move it, double-click empty space to add a key, right-click a key to remove it. The
//! value range auto-fits the keys and freezes while dragging so the graph does not jump. Shared-widget
//! note: `metallic_anim` (#40) builds the full curve editor with Bezier handles; this inline version
//! should become a thin wrapper over it when both land.

use crate::theme::{self, color};
use egui::{Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use metallic_vfx::{Curve, Interp, Key};

#[derive(Clone, Copy, Default)]
struct DragState {
    key: Option<usize>,
    range: (f32, f32),
}

/// Shows the curve editor; returns true when the curve changed.
pub fn curve_edit(
    ui: &mut Ui,
    curve: &mut Curve,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    height: f32,
) -> bool {
    let id = Id::unique(("curve", id_salt));
    let width = ui.available_width().max(120.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect, theme::R_SM, color::bg_deep());

    let mut state: DragState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let fitted = fit_range(curve);
    let (lo, hi) = if state.key.is_some() {
        state.range
    } else {
        fitted
    };
    let inner = rect.shrink2(Vec2::new(8.0, 8.0));
    let to_screen = |t: f32, v: f32| {
        Pos2::new(
            inner.min.x + t * inner.width(),
            inner.max.y - (v - lo) / (hi - lo) * inner.height(),
        )
    };
    let from_screen = |p: Pos2| {
        (
            ((p.x - inner.min.x) / inner.width()).clamp(0.0, 1.0),
            lo + (inner.max.y - p.y) / inner.height() * (hi - lo),
        )
    };
    // Grid: quarters in time, zero line if visible.
    for i in 1..4 {
        let x = inner.min.x + inner.width() * i as f32 / 4.0;
        painter.line_segment(
            [
                Pos2::new(x, rect.min.y + 2.0),
                Pos2::new(x, rect.max.y - 2.0),
            ],
            Stroke::new(1.0, color::line()),
        );
    }
    if lo < 0.0 && hi > 0.0 {
        let y = to_screen(0.0, 0.0).y;
        painter.line_segment(
            [
                Pos2::new(rect.min.x + 2.0, y),
                Pos2::new(rect.max.x - 2.0, y),
            ],
            Stroke::new(1.0, color::line_strong()),
        );
    }
    let n = 96;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            to_screen(t, curve.sample(t))
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color::text())));
    let label = |v: f32| format!("{v:.2}");
    painter.text(
        rect.min + Vec2::new(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        label(hi),
        egui::FontId::proportional(10.0),
        color::text_3(),
    );
    painter.text(
        Pos2::new(rect.min.x + 4.0, rect.max.y - 2.0),
        egui::Align2::LEFT_BOTTOM,
        label(lo),
        egui::FontId::proportional(10.0),
        color::text_3(),
    );

    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let near = |p: Pos2| {
        curve
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| (i, to_screen(k.t, k.v).distance(p)))
            .filter(|(_, d)| *d < 8.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let hovered_key = pointer.and_then(near);
    let mut changed = false;

    if response.drag_started() {
        state = DragState {
            key: pointer.and_then(near),
            range: fitted,
        };
    }
    if response.dragged()
        && let (Some(k), Some(p)) = (state.key, pointer)
    {
        let (t, v) = from_screen(p);
        let keys = &mut curve.keys;
        let t_min = if k == 0 { 0.0 } else { keys[k - 1].t };
        let t_max = if k + 1 == keys.len() {
            1.0
        } else {
            keys[k + 1].t
        };
        keys[k].t = t.clamp(t_min, t_max);
        keys[k].v = v;
        changed = true;
    }
    if response.drag_stopped() {
        state.key = None;
    }
    if response.double_clicked()
        && hovered_key.is_none()
        && let Some(p) = pointer
    {
        let (t, _) = from_screen(p);
        let v = curve.sample(t);
        curve.keys.push(Key { t, v, env: 0.0 });
        curve.sort();
        changed = true;
    }
    if response.secondary_clicked()
        && let Some(k) = hovered_key
        && curve.keys.len() > 1
    {
        curve.keys.remove(k);
        changed = true;
    }
    for (i, k) in curve.keys.iter().enumerate() {
        let p = to_screen(k.t, k.v);
        let active = Some(i) == state.key || Some(i) == hovered_key;
        let r = Rect::from_center_size(p, Vec2::splat(if active { 8.0 } else { 6.0 }));
        let fill = if active {
            color::accent()
        } else {
            color::text()
        };
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(r.center().x, r.min.y),
                Pos2::new(r.max.x, r.center().y),
                Pos2::new(r.center().x, r.max.y),
                Pos2::new(r.min.x, r.center().y),
            ],
            fill,
            Stroke::NONE,
        ));
    }
    if response.hovered() {
        painter.rect_stroke(
            rect,
            theme::R_SM,
            Stroke::new(1.0, color::line_strong()),
            StrokeKind::Inside,
        );
    }
    ui.data_mut(|d| d.insert_temp(id, state));
    response.on_hover_text("Drag keys · double-click to add · right-click to remove");
    changed
}

/// Interpolation picker shown next to curves and gradients.
pub fn interp_combo(
    ui: &mut Ui,
    interp: &mut Interp,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let before = *interp;
    egui::ComboBox::from_id_salt(("interp", id_salt))
        .width(84.0)
        .selected_text(interp_label(*interp))
        .show_ui(ui, |ui| {
            for i in [Interp::Linear, Interp::Smooth, Interp::Step] {
                ui.selectable_value(interp, i, interp_label(i));
            }
        });
    before != *interp
}

fn interp_label(i: Interp) -> &'static str {
    match i {
        Interp::Linear => "Linear",
        Interp::Smooth => "Smooth",
        Interp::Step => "Stepped",
    }
}

fn fit_range(curve: &Curve) -> (f32, f32) {
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for k in &curve.keys {
        lo = lo.min(k.v - k.env.abs());
        hi = hi.max(k.v + k.env.abs());
    }
    let pad = (hi - lo) * 0.08;
    (lo - if lo < 0.0 { pad } else { 0.0 }, hi + pad)
}
