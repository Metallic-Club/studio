//! Other users in the viewport (ADR 0011): their selections outlined in their colour, and their cameras as small
//! gizmos with their name above. Editor-only: drawn over the scene, never in it, never saved.

use std::time::Instant;

use egui::{Align2, Color32, FontId, Painter, Pos2, Stroke};
use metallic_engine::metallic_core::DataModel;
use metallic_engine::metallic_core::glam::Vec3;

use super::presence::Shown;
use crate::viewport::View;
use crate::viewport::lines::Lines;
use crate::viewport::pick::{self, EDGES};

/// How thick another user's outline is, in points.
const OUTLINE: f32 = 1.5;
/// How much bigger each further user's outline is than the one inside it, as a share of the box: several users on
/// one instance give nested boxes, each colour visible.
const NEST: f32 = 0.04;

fn colour([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Adds the outlines of what the others have selected to `lines`: one box per user and instance, a little larger for
/// each user already outlining it.
pub fn outlines(lines: &mut Lines, view: &View, dm: &DataModel, shown: &Shown) {
    let mut drawn: Vec<(metallic_engine::metallic_core::Id, usize)> = Vec::new();
    for user in &shown.users {
        let stroke = Stroke::new(OUTLINE, colour(user.color));
        for &id in &user.selection {
            if !dm.contains(id) || !pick::is_outlined(dm, id) {
                continue;
            }
            let Some(mut bounds) = pick::outline_box(dm, id) else {
                continue;
            };
            let nth = match drawn.iter_mut().find(|(i, _)| *i == id) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    drawn.push((id, 1));
                    1
                }
            };
            // The first user's box sits just outside the part; each further one a little further out.
            let grow = 1.0 + NEST * nth as f32;
            bounds.half = bounds.half * grow + Vec3::splat(0.02 * nth as f32);
            let corners = bounds.corners();
            for (a, b) in EDGES {
                if let Some(line) = view.segment(corners[a], corners[b]) {
                    lines.segment(line, stroke);
                }
            }
        }
    }
}

/// Paints the others' cameras: a small frustum in their colour where each looks from, and their name over it.
pub fn cameras(painter: &Painter, view: &View, shown: &Shown, now: Instant) {
    for user in &shown.users {
        let Some(camera) = user.camera_at(now) else {
            continue;
        };
        let color = colour(user.color);
        let eye = camera.position();
        let rot = camera.rotation();
        // A camera body and the pyramid of its view, about a metre long.
        let corner = |x: f32, y: f32| eye + rot * Vec3::new(x * 0.45, y * 0.3, -0.9);
        let corners = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
        ];
        let stroke = Stroke::new(1.5, color);
        for (i, &c) in corners.iter().enumerate() {
            if let Some([a, b]) = view.segment(eye, c) {
                painter.line_segment([a, b], stroke);
            }
            if let Some([a, b]) = view.segment(c, corners[(i + 1) % 4]) {
                painter.line_segment([a, b], stroke);
            }
        }
        // The top edge marked, so which way is up reads.
        if let Some(top) = view.project(eye + rot * Vec3::new(0.0, 0.5, -0.9)) {
            painter.circle_filled(top, 2.5, color);
        }
        let Some(at) = view.project(eye) else {
            continue;
        };
        painter.circle_filled(at, 4.0, color);
        let label = Pos2::new(at.x, at.y - 14.0);
        let font = FontId::proportional(12.0);
        let galley = painter.layout_no_wrap(user.name.clone(), font, Color32::WHITE);
        let rect = Align2::CENTER_BOTTOM
            .anchor_size(label, galley.size())
            .expand2(egui::vec2(5.0, 2.0));
        painter.rect_filled(rect, 4.0, color.gamma_multiply(0.9));
        painter.galley(rect.min + egui::vec2(5.0, 2.0), galley, Color32::WHITE);
    }
}
