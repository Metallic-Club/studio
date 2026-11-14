//! The arrow beside Play (ADR 0011 §8): how Play runs. Play Solo (the default), Team Create Play (the host's shared
//! server, everyone a player) or Multiple Clients (a local server and N Player windows, the count remembered).

use egui::{CornerRadius, Popup, PopupCloseBehavior, Rect, RichText, Sense, Stroke, Ui, WidgetInfo, vec2};

use super::store::MAX_CLIENTS;
use super::{PlayMode, TeamCreate};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// What a mode is called.
pub fn mode_name(mode: PlayMode) -> &'static str {
    match mode {
        PlayMode::Solo => "Play Solo",
        PlayMode::TeamCreate => "Team Create Play",
        PlayMode::Clients => "Multiple Clients",
    }
}

fn popup_id() -> egui::Id {
    egui::Id::unique("metallic.play_mode.popup")
}

/// The arrow, `enabled` while Play can be pressed; a click opens the modes.
pub fn arrow(ui: &mut Ui, team: &mut TeamCreate, enabled: bool) {
    let p = current_colors(ui.ctx()).palette;
    let height = 28.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, height), sense);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, "Play options"));
    if enabled && (response.hovered() || Popup::is_id_open(ui.ctx(), popup_id())) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
    }
    let ink = if enabled { p.text } else { p.text.gamma_multiply(0.48) };
    let icon = Rect::from_center_size(rect.center(), vec2(text::SMALL, text::SMALL));
    icons::paint(ui.painter(), Icon::UiCollapse, icon, ink);
    let response = response.on_hover_text(format!("How Play runs: {}", mode_name(team.play_mode)));
    Popup::menu(&response)
        .id(popup_id())
        .frame(
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(size::BORDER, p.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .shadow(crate::theme::tokens::shadow::MENU)
                .inner_margin(space::S4),
        )
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(260.0);
            ui.spacing_mut().item_spacing.y = space::S2;
            ui.label(
                RichText::new("Play")
                    .font(fonts::named(fonts::STRONG, text::BODY))
                    .color(p.text),
            );
            let hosting = team.is_hosting();
            for (mode, line) in [
                (PlayMode::Solo, "The game in Studio alone."),
                (
                    PlayMode::TeamCreate,
                    if hosting {
                        "One server here; everyone in the session plays in it."
                    } else {
                        "Host the project in Team Create first."
                    },
                ),
                (PlayMode::Clients, "A server here and Player windows that join it."),
            ] {
                let allowed = mode != PlayMode::TeamCreate || hosting;
                ui.add_enabled_ui(allowed, |ui| {
                    let chosen = team.play_mode == mode;
                    let response = ui.radio(chosen, mode_name(mode));
                    if response.clicked() {
                        team.play_mode = mode;
                    }
                    ui.label(RichText::new(line).size(text::SMALL).color(p.subtle));
                });
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Clients").size(text::SMALL).color(p.subtle));
                let clients = &mut team.store.clients;
                let before = *clients;
                let less = ui.add_enabled(*clients > 1, egui::Button::new("−").small());
                if less.clicked() {
                    *clients -= 1;
                }
                ui.label(RichText::new(clients.to_string()).size(text::BODY).color(p.text));
                let more = ui.add_enabled(*clients < MAX_CLIENTS, egui::Button::new("+").small());
                if more.clicked() {
                    *clients += 1;
                }
                if *clients != before {
                    team.play_mode = PlayMode::Clients;
                    team.store.save_or_log();
                }
            });
        });
    // A mode that cannot run any more (hosting ended) falls back to the default.
    if team.play_mode == PlayMode::TeamCreate && !team.is_hosting() {
        team.play_mode = PlayMode::Solo;
    }
}
