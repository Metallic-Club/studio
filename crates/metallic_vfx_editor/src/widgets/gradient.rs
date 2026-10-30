//! Gradient editor for [`Gradient`] (colour over life, trail and beam colours).
//!
//! The bar shows the gradient as it blends (sRGB or Oklab, smooth or banded). Click a key to select it and
//! edit its colour with the swatch, drag it to move, double-click the bar to add a key, right-click a key to
//! remove it.

use crate::theme::{self, color};
use crate::widgets::to_color32;
use egui::{Color32, Id, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use metallic_vfx::{BlendSpace, ColorKey, Gradient, Interp, Rgb};

/// Shows the gradient editor; returns true when the gradient changed.
pub fn gradient_edit(ui: &mut Ui, g: &mut Gradient, id_salt: impl std::hash::Hash + std::fmt::Debug) -> bool {
    let id = Id::unique(("gradient", id_salt));
    let mut selected: usize = ui.data(|d| d.get_temp(id)).unwrap_or(0);
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click_and_drag());
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + 20.0)).shrink2(Vec2::new(6.0, 0.0));
    let painter = ui.painter_at(rect.expand(2.0));
    let mut changed = false;

    // Bar as a mesh of thin vertical strips.
    let mut mesh = Mesh::default();
    let n = 64;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let c = to_color32(g.sample(t));
        let x = bar.min.x + t * bar.width();
        mesh.colored_vertex(Pos2::new(x, bar.min.y), c);
        mesh.colored_vertex(Pos2::new(x, bar.max.y), c);
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(bar, theme::R_SM, Stroke::new(1.0, color::LINE_STRONG), StrokeKind::Outside);

    let handle_y = bar.max.y + 6.0;
    let key_pos = |k: &ColorKey| Pos2::new(bar.min.x + k.t * bar.width(), handle_y);
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let hovered = pointer.and_then(|p| {
        g.keys
            .iter()
            .enumerate()
            .map(|(i, k)| (i, key_pos(k).distance(p)))
            .filter(|(_, d)| *d < 9.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    if (response.clicked() || response.drag_started())
        && let Some(i) = hovered
    {
        selected = i;
    }
    if response.dragged()
        && let Some(p) = pointer
        && selected < g.keys.len()
    {
        let t = ((p.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
        g.keys[selected].t = t;
        // Keep the selection on the dragged key after re-sorting.
        let c = g.keys[selected];
        g.sort();
        selected = g.keys.iter().position(|k| *k == c).unwrap_or(0);
        changed = true;
    }
    if response.double_clicked()
        && hovered.is_none()
        && let Some(p) = pointer
    {
        let t = ((p.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
        let c = quantise(g.sample(t));
        g.keys.push(ColorKey { t, c });
        g.sort();
        selected = g.keys.iter().position(|k| k.t == t).unwrap_or(0);
        changed = true;
    }
    if response.secondary_clicked()
        && let Some(i) = hovered
        && g.keys.len() > 1
    {
        g.keys.remove(i);
        selected = selected.min(g.keys.len() - 1);
        changed = true;
    }
    for (i, k) in g.keys.iter().enumerate() {
        let p = key_pos(k);
        let sel = i == selected;
        let tri = vec![
            Pos2::new(p.x, p.y - 6.0),
            Pos2::new(p.x + 5.5, p.y + 4.0),
            Pos2::new(p.x - 5.5, p.y + 4.0),
        ];
        let outline = if sel { color::ACCENT } else { color::ICON };
        painter.add(egui::Shape::convex_polygon(
            tri,
            to_color32(k.c),
            Stroke::new(if sel { 2.0 } else { 1.0 }, outline),
        ));
    }
    ui.data_mut(|d| d.insert_temp(id, selected));
    response.on_hover_text("Click a key to edit · drag to move · double-click to add · right-click to remove");

    ui.horizontal(|ui| {
        if let Some(k) = g.keys.get_mut(selected) {
            changed |= crate::widgets::color_button(ui, &mut k.c);
            ui.label(egui::RichText::new(format!("{} at {:.0}%", k.c.to_hex_string(), k.t * 100.0)).color(color::TEXT_3));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut banded = g.interp == Interp::Step;
            if ui.selectable_label(banded, "Bands").on_hover_text("Hard colour bands (toon, pixel)").clicked() {
                banded = !banded;
                g.interp = if banded { Interp::Step } else { Interp::Linear };
                changed = true;
            }
            let mut ok = g.space == BlendSpace::Oklab;
            if ui.selectable_label(ok, "Oklab").on_hover_text("Blend in Oklab: even, never muddy").clicked() {
                ok = !ok;
                g.space = if ok { BlendSpace::Oklab } else { BlendSpace::Srgb };
                changed = true;
            }
        });
    });
    let _ = Color32::TRANSPARENT;
    changed
}

fn quantise(c: Rgb) -> Rgb {
    let [r, g, b] = c.to_u8();
    Rgb::from_u8(r, g, b)
}
