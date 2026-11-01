//! Measure (Shift+M) and the Alt-hover gap readouts over the viewport (spec 10.2).
//!
//! A click picks the surface point under the cursor, snapped to the nearest corner of the hit
//! part within 10 screen points ([`snap_to_vertex`] over [`geom::snapshot`] corners). Two clicks
//! make a measurement; up to eight stay until Esc or a tool change. Nothing is saved.
use eframe::egui::{self, Stroke};
use glam::DVec3;

use super::{ViewContext, overlay};
use crate::editor::EditorState;
use crate::theme;
use crate::tools::geom::{self, axis_gaps, gap_distance};
use crate::tools::measure::{MeasureTool, Measurement, snap_to_vertex};

/// How far (screen points) a click snaps to a part corner.
const SNAP_PX: f64 = 10.0;

/// The world point a click at the pointer would measure from, snapped to a corner.
pub fn point_under(state: &EditorState, cx: &ViewContext<'_>) -> Option<DVec3> {
    let p = cx.pointer?;
    let ray = cx.ray(p);
    let hit = cx.picks.first(&ray, &[])?;
    let point = ray.at(hit.distance);
    let snap = geom::snapshot(&state.world, &[hit.id]);
    let limit = cx.view.studs_per_point(point) * SNAP_PX;
    let corners = snap.first().map(|s| s.corners()).unwrap_or_default();
    Some(snap_to_vertex(point, corners, limit).unwrap_or(point))
}

fn fmt(v: f64) -> String {
    format!("{:.2}", (v * 100.0).round() / 100.0)
}

fn draw(painter: &egui::Painter, cx: &ViewContext<'_>, m: &Measurement) {
    let (Some(a), Some(b)) = (cx.project(m.a), cx.project(m.b)) else {
        return;
    };
    painter.line_segment([a, b], Stroke::new(1.5, theme::text_1()));
    for end in [a, b] {
        painter.circle_stroke(end, 3.5, Stroke::new(1.5, theme::text_1()));
    }
    let mid = a + (b - a) * 0.5;
    overlay::label(
        painter,
        mid - egui::vec2(0.0, 18.0),
        &format!(
            "{} studs · {}°",
            fmt(m.distance()),
            fmt(m.angle_to_ground_deg())
        ),
        theme::text_1(),
    );
    let d = m.delta();
    for (i, (name, v)) in [("X", d.x), ("Y", d.y), ("Z", d.z)].into_iter().enumerate() {
        overlay::label(
            painter,
            mid + egui::vec2((i as f32 - 1.0) * 58.0, 2.0),
            &format!("{name} {}", fmt(v)),
            overlay::axis_colour(i),
        );
    }
}

/// Clicks and painting while Measure is on.
pub fn viewport(
    ui: &egui::Ui,
    state: &mut EditorState,
    cx: &ViewContext<'_>,
    painter: &egui::Painter,
    tool: &mut MeasureTool,
) {
    let clicked = cx.pointer.is_some()
        && ui.input(|i| i.pointer.primary_released() && !i.pointer.is_decidedly_dragging());
    let here = point_under(state, cx);
    if clicked
        && let Some(p) = here
        && let Some(m) = tool.click(p)
    {
        state.note(format!("{} studs", fmt(m.distance())));
    }
    for m in tool.measurements() {
        draw(painter, cx, m);
    }
    if let Some(a) = tool.pending() {
        if let Some(b) = here {
            draw(painter, cx, &Measurement { a, b });
        } else if let Some(s) = cx.project(a) {
            painter.circle_stroke(s, 3.5, Stroke::new(1.5, theme::text_1()));
        }
    }
    if let Some(p) = here.and_then(|p| cx.project(p)) {
        painter.circle_filled(p, 2.5, theme::accent());
    }
}

/// Alt held over another part with something selected: the gaps between the bounding boxes.
pub fn gap_readout(
    ui: &egui::Ui,
    state: &EditorState,
    cx: &ViewContext<'_>,
    painter: &egui::Painter,
) {
    let (alt, any_down) = ui.input(|i| (i.modifiers.alt, i.pointer.any_down()));
    let (Some(p), Some(h)) = (cx.pointer, cx.hovered) else {
        return;
    };
    if !alt || any_down || state.selection.is_empty() || state.selection.contains(h) {
        return;
    }
    let parts = state.selection.parts(&state.world);
    let Some(sel) = geom::snapshot(&state.world, &parts)
        .iter()
        .map(|s| s.aabb())
        .reduce(|a, b| a.union(&b))
    else {
        return;
    };
    let Some(other) = geom::snapshot(&state.world, &[h]).first().map(|s| s.aabb()) else {
        return;
    };
    let g = axis_gaps(&sel, &other);
    overlay::label(
        painter,
        p - egui::vec2(0.0, 30.0),
        &format!("Gap {} studs", fmt(gap_distance(&sel, &other))),
        theme::text_1(),
    );
    for (i, v) in [g.x, g.y, g.z].into_iter().enumerate() {
        overlay::label(
            painter,
            p + egui::vec2((i as f32 - 1.0) * 58.0, -12.0),
            &format!("{} {}", ["X", "Y", "Z"][i], fmt(v)),
            overlay::axis_colour(i),
        );
    }
}
