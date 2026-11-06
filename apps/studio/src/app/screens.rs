//! Placeholder screens: a section label, a title, a few lines and the button that goes on. The Editor screen is one
//! until the editor issue (#216) replaces it.

use egui::{Align, Layout, RichText};

use crate::theme::tokens::{space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// Draws a placeholder screen filling `ui`. Returns its button's response.
pub(crate) fn placeholder(
    ui: &mut egui::Ui,
    section: &str,
    title: &str,
    lines: &[&str],
    action: &str,
) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    ui.painter().rect_filled(ui.max_rect(), 0.0, p.bg);
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() / 2.0 - 2.0 * space::S11).max(space::S9));
        ui.label(
            RichText::new(section.to_uppercase())
                .font(fonts::named(fonts::STRONG, text::LABEL))
                .extra_letter_spacing(text::LABEL_TRACKING)
                .color(p.subtle),
        );
        ui.add_space(space::S2);
        ui.label(
            RichText::new(title)
                .font(fonts::named(fonts::HEADING, text::DISPLAY))
                .extra_letter_spacing(text::DISPLAY_TRACKING)
                .color(p.text),
        );
        ui.add_space(space::S3);
        for line in lines {
            ui.label(RichText::new(*line).size(text::BODY_LG).color(p.muted));
        }
        ui.add_space(space::S8);
        button(ui, ButtonKind::Primary, action)
    })
    .inner
}
