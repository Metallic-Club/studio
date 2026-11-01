//! Timeline strip: a ruler in seconds, one track per emitter with burst markers and spawn spans, and a
//! draggable playhead.
//!
//! Shared-widget note: `metallic_anim` (#40) owns the general Timeline (frames, keys, events). This one
//! only shows what an effect needs (seconds, bursts, spans); when #40 lands it should become a thin
//! adapter over that widget with the same look.

use crate::theme::{self, color};
use egui::{FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// One timeline row.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Row label.
    pub label: String,
    /// Burst times in seconds (diamonds).
    pub markers: Vec<f32>,
    /// Spawning spans in seconds (bars).
    pub spans: Vec<(f32, f32)>,
    /// Dimmed when off.
    pub enabled: bool,
}

/// Width of the label column.
pub const LABEL_COLUMN: f32 = 150.0;

/// Draws the timeline in `rect`. Returns a time when the user clicked or dragged on the ruler or tracks.
pub fn timeline(ui: &mut Ui, rect: Rect, time: f32, length: f32, tracks: &[Track]) -> Option<f32> {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, color::bg_panel());
    let length = length.max(0.1);
    let area = Rect::from_min_max(Pos2::new(rect.min.x + LABEL_COLUMN, rect.min.y), rect.max);
    let lane = Rect::from_min_max(
        area.min + Vec2::new(10.0, 0.0),
        area.max - Vec2::new(16.0, 0.0),
    );
    let x_of = |t: f32| lane.min.x + (t / length).clamp(0.0, 1.0) * lane.width();
    let ruler_h = 22.0;

    // Ruler ticks: pick a step giving roughly 60 px between labels.
    let steps = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];
    let px_per_s = lane.width() / length;
    let step = steps
        .iter()
        .copied()
        .find(|s| s * px_per_s >= 56.0)
        .unwrap_or(10.0);
    let mut t = 0.0;
    while t <= length + 1e-4 {
        let x = x_of(t);
        painter.line_segment(
            [Pos2::new(x, rect.min.y + ruler_h), Pos2::new(x, rect.max.y)],
            Stroke::new(1.0, color::line()),
        );
        painter.text(
            Pos2::new(x, rect.min.y + 4.0),
            egui::Align2::CENTER_TOP,
            format_time(t, step),
            FontId::proportional(11.0),
            color::text_3(),
        );
        t += step;
    }
    painter.text(
        Pos2::new(rect.min.x + 12.0, rect.min.y + 4.0),
        egui::Align2::LEFT_TOP,
        "Tracks",
        FontId::proportional(11.0),
        color::text_3(),
    );

    let row_h = 22.0;
    for (i, tr) in tracks.iter().enumerate() {
        let y = rect.min.y + ruler_h + 4.0 + i as f32 * row_h;
        if y + row_h > rect.max.y {
            break;
        }
        let row = Rect::from_min_size(Pos2::new(rect.min.x, y), Vec2::new(rect.width(), row_h));
        let label_color = if tr.enabled {
            color::text()
        } else {
            color::text_3()
        };
        painter.text(
            Pos2::new(rect.min.x + 12.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            &tr.label,
            FontId::proportional(12.0),
            label_color,
        );
        for &(a, b) in &tr.spans {
            let r = Rect::from_min_max(
                Pos2::new(x_of(a), row.center().y - 3.0),
                Pos2::new(x_of(b), row.center().y + 3.0),
            );
            painter.rect_filled(r, theme::R_SM, color::bg_active());
        }
        for &m in &tr.markers {
            let c = Pos2::new(x_of(m), row.center().y);
            let s = 5.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(0.0, -s),
                    c + Vec2::new(s, 0.0),
                    c + Vec2::new(0.0, s),
                    c + Vec2::new(-s, 0.0),
                ],
                if tr.enabled {
                    color::icon()
                } else {
                    color::text_3()
                },
                Stroke::NONE,
            ));
        }
    }

    // Playhead.
    let x = x_of(time.min(length));
    painter.line_segment(
        [
            Pos2::new(x, rect.min.y + ruler_h - 2.0),
            Pos2::new(x, rect.max.y),
        ],
        Stroke::new(1.5, color::accent()),
    );
    let chip = Rect::from_center_size(Pos2::new(x, rect.min.y + 11.0), Vec2::new(40.0, 18.0));
    painter.rect_filled(chip, theme::R_SM, color::accent());
    painter.text(
        chip.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.2}", time),
        theme::semibold(11.0),
        color::on_accent(),
    );

    let resp = ui.interact(
        area,
        ui.scope_id().with("timeline-scrub"),
        Sense::click_and_drag(),
    );
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        return Some(((p.x - lane.min.x) / lane.width()).clamp(0.0, 1.0) * length);
    }
    None
}

fn format_time(t: f32, step: f32) -> String {
    if step < 0.1 {
        format!("{t:.2}")
    } else if step < 1.0 {
        format!("{t:.1}")
    } else {
        format!("{t:.0} s")
    }
}
