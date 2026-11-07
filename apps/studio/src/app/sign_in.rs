//! The sign-in screen (ADR 0009, Screens): the website's sign-in card with the Metallic mark. There are no servers
//! yet, so "Sign in" is disabled and says so, and "Continue offline" goes on to the projects. "Don't ask again" is
//! `general.skip_sign_in` in the settings, which skips this screen from the next start.
//!
//! This is the seam real sign-in will use: the screen's outcome is one [`SignIn`] value.

use egui::{Align, CornerRadius, Frame, Label, Layout, RichText, Stroke, TextureHandle, Ui, UiBuilder, Vec2, vec2};

use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{ButtonKind, current_colors, fonts, wide_button};

/// The card's width.
const CARD_WIDTH: f32 = 400.0;
/// The mark's size on the card.
pub const MARK: f32 = 48.0;

/// What the user chose on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignIn {
    /// Nothing yet.
    Waiting,
    /// Go on without an account.
    Offline,
}

/// Draws the sign-in screen filling `ui`, with `mark` on the card. `skip_sign_in` is the settings' "Don't ask
/// again", changed in place.
pub fn show(ui: &mut Ui, mark: Option<&TextureHandle>, skip_sign_in: &mut bool) -> SignIn {
    let p = current_colors(ui.ctx()).palette;
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, p.bg);
    let mut outcome = SignIn::Waiting;
    // Centred by last frame's height; the first frame starts a little high, which the second corrects.
    let height_id = egui::Id::unique("metallic.sign_in.height");
    let height = ui.ctx().data(|data| data.get_temp::<f32>(height_id)).unwrap_or(420.0);
    let top = (screen.center().y - height / 2.0).max(screen.top() + space::S8);
    let area = egui::Rect::from_min_size(
        egui::pos2(screen.center().x - CARD_WIDTH / 2.0, top),
        vec2(CARD_WIDTH, screen.bottom() - top),
    );
    let card = ui.scope_builder(
        UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)),
        |ui| {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(size::BORDER, p.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .inner_margin(space::S9)
                .show(ui, |ui| {
                    ui.set_width(CARD_WIDTH - 2.0 * space::S9);
                    if let Some(mark) = mark {
                        ui.add(egui::Image::new((mark.id(), Vec2::splat(MARK))));
                        ui.add_space(space::S5);
                    }
                    ui.label(
                        RichText::new("WELCOME")
                            .font(fonts::named(fonts::STRONG, text::LABEL))
                            .extra_letter_spacing(text::LABEL_TRACKING)
                            .color(p.subtle),
                    );
                    ui.label(
                        RichText::new("Sign in to Metallic")
                            .font(fonts::named(fonts::HEADING, text::DISPLAY))
                            .extra_letter_spacing(text::DISPLAY_TRACKING)
                            .color(p.text),
                    );
                    ui.add(
                        Label::new(
                            RichText::new("Your account will publish your games and keep your projects in sync.")
                                .size(text::BODY)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                    ui.add_space(space::S6);
                    ui.add_enabled_ui(false, |ui| wide_button(ui, ButtonKind::Primary, "Sign in"));
                    ui.label(
                        RichText::new("Online services aren't available yet.")
                            .size(text::SMALL)
                            .color(p.subtle),
                    );
                    ui.add_space(space::S3);
                    if wide_button(ui, ButtonKind::Secondary, "Continue offline").clicked() {
                        outcome = SignIn::Offline;
                    }
                    ui.add_space(space::S5);
                    ui.separator();
                    ui.add_space(space::S2);
                    crate::theme::checkbox(ui, skip_sign_in, "Don't ask again");
                    ui.label(
                        RichText::new("Studio will start on your projects. Change it in Settings, General.")
                            .size(text::SMALL)
                            .color(p.subtle),
                    );
                })
                .response
                .rect
                .height()
        },
    );
    if (card.inner - height).abs() > 0.5 {
        ui.ctx().data_mut(|data| data.insert_temp(height_id, card.inner));
        ui.ctx().request_repaint();
    }
    outcome
}
