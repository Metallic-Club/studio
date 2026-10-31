//! Segmented filter tabs (research doc 3.7): a `bg-deep` track with a 1 px `line` and 2 px padding,
//! 26 px segments; the selected one is `bg-active` with the top highlight. Each segment is a
//! focusable button, so Tab and Enter/Space work.

use eframe::egui::{self, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::theme::{self, fonts};
use crate::widgets::cut;

const PAD: f32 = 2.0;
const HEIGHT: f32 = 26.0;
const PAD_X: f32 = 10.0;

/// Draws the segments for `options` and updates `current` when one is picked. The returned
/// response covers the whole track and is `changed()` when the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    current: &mut T,
    options: &[(T, &str)],
) -> Response {
    let font = egui::FontId::new(12.0, fonts::strong().family);
    let painter = ui.painter();
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| {
            painter.layout_no_wrap((*label).to_owned(), font.clone(), theme::text_2())
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|g| (g.size().x + 2.0 * PAD_X).ceil())
        .collect();
    let size = vec2(widths.iter().sum::<f32>() + 2.0 * PAD, HEIGHT + 2.0 * PAD);
    let (track, mut response) = ui.allocate_exact_size(size, Sense::hover());
    let radius = CornerRadius::same(theme::R_SM);
    ui.painter().rect(
        track,
        radius,
        theme::bg_deep(),
        Stroke::new(1.0, theme::line()),
        StrokeKind::Inside,
    );

    let mut x = track.left() + PAD;
    for (((value, label), galley), width) in options.iter().zip(galleys).zip(widths) {
        let rect = Rect::from_min_size(egui::pos2(x, track.top() + PAD), vec2(width, HEIGHT));
        x += width;
        let id = response.id.with(*label);
        let seg = ui.interact(rect, id, Sense::click());
        let selected = *current == *value;
        seg.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, *label)
        });
        if seg.clicked() && !selected {
            *current = *value;
            response.mark_changed();
        }
        let hover =
            ui.ctx()
                .animate_bool_with_time(id.with("hover"), seg.hovered(), theme::t_fast());
        let on = ui
            .ctx()
            .animate_bool_with_time(id.with("on"), selected, theme::t_fast());
        let painter = ui.painter();
        let seg_radius = CornerRadius::same(theme::R_XS);
        if on > 0.0 {
            painter.rect_filled(rect, seg_radius, theme::bg_active().gamma_multiply(on));
            painter.hline(
                rect.left() + 3.0..=rect.right() - 3.0,
                rect.top() + 0.5,
                Stroke::new(1.0, theme::highlight().gamma_multiply(on)),
            );
        }
        let ink = theme::text_2().lerp_to_gamma(theme::text_1(), hover.max(on));
        let pos = rect.center() - galley.size() / 2.0;
        painter.galley_with_override_text_color(pos, galley, ink);
        if seg.has_focus() {
            cut::frame(
                painter,
                rect,
                seg_radius,
                0.0,
                Stroke::new(2.0, theme::focus()),
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_a_segment_selects_it() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::ThemeId::Graphite);
        let mut value = 0;
        let run = |ctx: &egui::Context, events, value: &mut i32| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = None;
            ctx.run_ui(input, |ui| {
                out = Some(segmented(ui, value, &[(0, "All"), (1, "3D"), (2, "2D")]));
            })
            .textures_delta
            .clear();
            out.expect("ran")
        };
        let r = run(&ctx, vec![], &mut value);
        assert_eq!(r.rect.height(), 30.0);
        // Click near the right end of the track: the "2D" segment.
        let at = egui::pos2(r.rect.right() - 8.0, r.rect.center().y);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&ctx, vec![egui::Event::PointerMoved(at)], &mut value);
        run(&ctx, vec![press(true)], &mut value);
        let r = run(&ctx, vec![press(false)], &mut value);
        assert_eq!(value, 2);
        assert!(r.changed());
    }
}
