//! The Settings dialog: theme, interface size and reduced motion.
//!
//! It floats over whatever is on screen (the start screen or the editor) on a dimmed backdrop.
//! Changes take effect at once and are saved by the caller ([`crate::app`]) when [`Outcome::changed`]
//! is set. Esc, the close button and a click on the backdrop all close it.

use eframe::egui::{
    self, Align, Align2, Area, CornerRadius, Frame, Id, Key, Layout, Margin, Order, RichText,
    Sense, Stroke, Ui, vec2,
};

use crate::settings::{SIZE_MAX, SIZE_MIN, SIZE_STEP, Settings, ThemeChoice};
use crate::theme::{self, fonts, icons};
use crate::widgets::button::Button;
use crate::widgets::segmented::segmented;

/// Width of the dialog.
const WIDTH: f32 = 520.0;

/// What happened this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// A value changed; apply and save the settings.
    pub changed: bool,
    /// The creator closed the dialog.
    pub close: bool,
    /// The creator asked for the Third-party licences screen.
    pub licences: bool,
}

/// The interface sizes Settings offers, with their labels.
fn size_options() -> Vec<(u16, String)> {
    (SIZE_MIN..=SIZE_MAX)
        .step_by(usize::from(SIZE_STEP))
        .map(|p| (p, format!("{p}%")))
        .collect()
}

/// Draws the dialog over the whole window and edits `settings`.
pub fn show(ctx: &egui::Context, settings: &mut Settings) -> Outcome {
    let mut out = Outcome::default();
    let screen = ctx.content_rect();

    // The scrim also catches clicks, so nothing underneath reacts while the dialog is open.
    let backdrop = Area::new(Id::unique("settings-backdrop"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let response = ui.allocate_rect(screen, Sense::click());
            ui.painter().rect_filled(screen, 0.0, theme::backdrop());
            response
        });
    if backdrop.inner.clicked() {
        out.close = true;
    }

    Area::new(Id::unique("settings-dialog"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(theme::bg_panel())
                .stroke(Stroke::new(1.0, theme::line_strong()))
                .corner_radius(CornerRadius::same(theme::R_LG))
                .shadow(theme::SHADOW_MODAL)
                .inner_margin(Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    body(ui, settings, &mut out);
                });
        });

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        out.close = true;
    }
    out
}

fn body(ui: &mut Ui, settings: &mut Settings, out: &mut Outcome) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Settings")
                .font(fonts::title())
                .color(theme::text_1()),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(
                    Button::ghost("")
                        .icon(icons::X)
                        .alt("Close settings")
                        .compact(),
                )
                .clicked()
            {
                out.close = true;
            }
        });
    });
    ui.add_space(20.0);

    section(
        ui,
        "Theme",
        "System follows the Windows light or dark setting.",
    );
    ui.add_space(8.0);
    let options: Vec<(ThemeChoice, &str)> = ThemeChoice::ALL
        .into_iter()
        .map(|t| (t, t.label()))
        .collect();
    if segmented(ui, &mut settings.theme, &options).changed() {
        out.changed = true;
    }
    ui.add_space(24.0);

    section(
        ui,
        "Interface size",
        "Makes text and controls bigger or smaller. 100% is the standard size.",
    );
    ui.add_space(8.0);
    let sizes = size_options();
    let options: Vec<(u16, &str)> = sizes.iter().map(|(p, l)| (*p, l.as_str())).collect();
    if segmented(ui, &mut settings.interface_size, &options).changed() {
        out.changed = true;
    }
    ui.add_space(24.0);

    section(
        ui,
        "Motion",
        "Turns off sliding and fading, and shortens the rest.",
    );
    ui.add_space(8.0);
    if ui
        .checkbox(&mut settings.reduce_motion, "Reduce motion")
        .changed()
    {
        out.changed = true;
    }
    ui.add_space(24.0);

    section(
        ui,
        "About",
        "The fonts, icons and libraries Metallic Studio is built with, and their licences.",
    );
    ui.add_space(8.0);
    if ui.add(Button::secondary("Third-party licences")).clicked() {
        out.licences = true;
    }
    ui.add_space(24.0);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Changes are saved as you make them.")
                .font(fonts::small())
                .color(theme::text_3()),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(Button::primary("Done")).clicked() {
                out.close = true;
            }
        });
    });
}

fn section(ui: &mut Ui, title: &str, help: &str) {
    ui.label(
        RichText::new(title)
            .font(fonts::heading())
            .color(theme::text_1()),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new(help)
            .font(fonts::small())
            .color(theme::text_2()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_run_from_90_to_150_in_steps_of_10() {
        let sizes: Vec<u16> = size_options().into_iter().map(|(p, _)| p).collect();
        assert_eq!(sizes, [90, 100, 110, 120, 130, 140, 150]);
    }

    #[test]
    fn the_dialog_reports_a_theme_change_and_a_close() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let mut harness = Harness::builder()
            .with_size(vec2(900.0, 700.0))
            .build_ui_state(
                |ui, state: &mut (Settings, Outcome)| {
                    let ctx = ui.ctx().clone();
                    theme::apply(&ctx, theme::ThemeId::Graphite);
                    let out = show(&ctx, &mut state.0);
                    state.1.changed |= out.changed;
                    state.1.close |= out.close;
                    state.1.licences |= out.licences;
                },
                (Settings::default(), Outcome::default()),
            );
        harness.run();
        harness.get_by_label("Light").click();
        harness.run();
        assert_eq!(harness.state().0.theme, ThemeChoice::Light);
        assert!(harness.state().1.changed);
        assert!(!harness.state().1.close);
        harness.get_by_label("130%").click();
        harness.get_by_label("Reduce motion").click();
        harness.run();
        assert_eq!(harness.state().0.interface_size, 130);
        assert!(harness.state().0.reduce_motion);
        harness.get_by_label("Third-party licences").click();
        harness.run();
        assert!(harness.state().1.licences);
        harness.get_by_label("Done").click();
        harness.run();
        assert!(harness.state().1.close);
    }
}
