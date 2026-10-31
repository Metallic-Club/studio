//! Painting helpers for the tool overlays: projected boxes, circles on surfaces, labels.
//! Colours come from the theme ([`crate::theme`]) and the axis tokens only.
use eframe::egui::{self, Align2, Color32, Pos2, Stroke};
use glam::{DQuat, DVec3};

use super::ViewContext;
use crate::theme;
use crate::tokens::ui_tokens;

const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// The twelve edges of a box (corners in [`crate::tools::geom::box_corners`] order).
pub fn box_edges(
    painter: &egui::Painter,
    cx: &ViewContext<'_>,
    corners: &[DVec3; 8],
    stroke: Stroke,
) {
    let pts: [Option<Pos2>; 8] = corners.map(|c| cx.project(c));
    for (a, b) in EDGES {
        if let (Some(a), Some(b)) = (pts[a], pts[b]) {
            painter.line_segment([a, b], stroke);
        }
    }
}

/// A ghost box: its edges at 60% of the accent (array and mirror previews).
pub fn ghost_box(
    painter: &egui::Painter,
    cx: &ViewContext<'_>,
    centre: DVec3,
    rot: DQuat,
    size: DVec3,
) {
    let corners = crate::tools::geom::box_corners(centre, rot, size);
    box_edges(
        painter,
        cx,
        &corners,
        Stroke::new(1.5, theme::accent().gamma_multiply(0.6)),
    );
}

/// A circle of `radius` studs lying on the surface through `centre` with `normal`.
pub fn surface_circle(
    painter: &egui::Painter,
    cx: &ViewContext<'_>,
    centre: DVec3,
    normal: DVec3,
    radius: f64,
    stroke: Stroke,
) {
    let (u, v) = crate::tools::scatter::tangent_basis(normal.normalize_or(DVec3::Y));
    let n = 64;
    let pts: Vec<Pos2> = (0..=n)
        .filter_map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            cx.project(centre + (u * a.cos() + v * a.sin()) * radius)
        })
        .collect();
    if pts.len() > 2 {
        painter.add(egui::Shape::line(pts, stroke));
    }
}

/// A label on a small plate, so it reads over any scene.
pub fn label(painter: &egui::Painter, at: Pos2, text: &str, colour: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), ui_tokens().small(), colour);
    let rect = Align2::CENTER_BOTTOM
        .anchor_size(at, galley.size())
        .expand(3.0);
    painter.rect_filled(rect, 3.0, theme::bg_raised().gamma_multiply(0.92));
    painter.galley(rect.min + egui::vec2(3.0, 3.0), galley, colour);
}

/// The axis colours (X, Y, Z).
pub fn axis_colour(axis: usize) -> Color32 {
    ui_tokens().axis[axis.min(2)]
}
