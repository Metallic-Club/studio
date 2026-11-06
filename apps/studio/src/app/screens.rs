//! The screens after the splash. Placeholders for now: a section label, a title, a line and the button that goes on.
//! Their own issues replace each one (sign-in, the projects page, the editor).

use egui::{Align, Layout, RichText};

use crate::theme::tokens::{space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// Draws a placeholder screen filling `ui`. Returns whether its button was clicked.
pub(crate) fn placeholder(ui: &mut egui::Ui, section: &str, title: &str, line: &str, action: &str) -> bool {
    let p = current_colors(ui.ctx()).palette;
    ui.painter().rect_filled(ui.max_rect(), 0.0, p.bg);
    let mut clicked = false;
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
        ui.label(RichText::new(line).size(text::BODY_LG).color(p.muted));
        ui.add_space(space::S8);
        clicked = button(ui, ButtonKind::Primary, action).clicked();
    });
    clicked
}
