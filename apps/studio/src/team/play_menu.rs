//! The arrow beside Play (ADR 0011 §8): how Play runs. Play Solo (the default), Team Create Play (the host's shared
//! server, everyone a player) or Multiple Clients (a local server and N Player windows, the count remembered).
//!
//! Drawn in Studio's own style (#643), as the Team Create menu is: each mode a row with a radio mark, its name and
//! what it does, and the number of clients a stepper.

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Layout, Popup, PopupCloseBehavior, Rect, RichText, Sense, Stroke, Ui,
    WidgetInfo, pos2, vec2,
};

use super::store::MAX_CLIENTS;
use super::{PlayMode, TeamCreate};
use crate::projects::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// The menu's width.
const WIDTH: f32 = 280.0;

/// What a mode is called.
pub fn mode_name(mode: PlayMode) -> &'static str {
    match mode {
        PlayMode::Solo => "Play Solo",
        PlayMode::TeamCreate => "Team Create Play",
        PlayMode::Clients => "Multiple Clients",
    }
}

pub(crate) fn popup_id() -> egui::Id {
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
        // Studio's style, not egui's menu style, which strips buttons' fill and border.
        .style(egui::style::StyleModifier::default())
        .frame(
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(size::BORDER, p.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .shadow(crate::theme::tokens::shadow::MENU)
                .inner_margin(space::S4),
        )
        .layout(Layout::top_down(Align::Min))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = space::S1;
            ui.add_space(space::S2);
            ui.horizontal(|ui| {
                ui.add_space(space::S4);
                ui.label(section_label("Play mode", p.subtle));
            });
            ui.add_space(space::S1);
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
                if mode_row(ui, mode, line, team.play_mode == mode, allowed) {
                    team.play_mode = mode;
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add_space(space::S4);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new("Clients")
                            .font(fonts::named(fonts::STRONG, text::BODY))
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new("Player windows to open")
                            .size(text::SMALL)
                            .color(p.subtle),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let clients = &mut team.store.clients;
                    let before = *clients;
                    stepper(ui, clients);
                    if *clients != before {
                        team.play_mode = PlayMode::Clients;
                        team.store.save_or_log();
                    }
                });
            });
            ui.add_space(space::S2);
        });
    // A mode that cannot run any more (hosting ended) falls back to the default.
    if team.play_mode == PlayMode::TeamCreate && !team.is_hosting() {
        team.play_mode = PlayMode::Solo;
    }
}

/// One mode as a row: a radio mark, its name, and `line` under it. Raised when chosen or under the pointer; dimmed
/// when it cannot run. Returns whether it was clicked.
fn mode_row(ui: &mut Ui, mode: PlayMode, line: &str, chosen: bool, allowed: bool) -> bool {
    let p = current_colors(ui.ctx()).palette;
    let name = mode_name(mode);
    let indent = 34.0;
    let width = ui.available_width();
    let fade = |color: Color32| if allowed { color } else { color.gamma_multiply(0.48) };
    let title = ui
        .painter()
        .layout_no_wrap(name.to_owned(), fonts::named(fonts::STRONG, text::BODY), fade(p.text));
    let words = ui.painter().layout(
        line.to_owned(),
        egui::FontId::proportional(text::SMALL),
        fade(p.subtle),
        width - indent - space::S4,
    );
    let height = space::S4 + title.size().y + space::S1 + words.size().y + space::S4;
    let sense = if allowed { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    response.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, allowed, chosen, name));
    if ui.is_rect_visible(rect) {
        let fill = if chosen {
            p.surface_hover
        } else if allowed && response.hovered() {
            p.surface_hover.gamma_multiply(0.6)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
        // The radio mark: a ring, filled in the primary button's colour when chosen.
        let centre = pos2(
            rect.left() + space::S4 + 8.0,
            rect.top() + space::S4 + title.size().y / 2.0,
        );
        if chosen {
            ui.painter().circle_filled(centre, 8.0, p.action);
            ui.painter().circle_filled(centre, 3.0, p.action_text);
        } else {
            ui.painter()
                .circle_stroke(centre, 7.5, Stroke::new(size::BORDER, fade(p.subtle)));
        }
        let left = rect.left() + indent;
        let top = rect.top() + space::S4;
        let title_height = title.size().y;
        ui.painter().galley(pos2(left, top), title, p.text);
        ui.painter()
            .galley(pos2(left, top + title_height + space::S1), words, p.subtle);
    }
    allowed && response.clicked()
}

/// The number of clients, with a button either side to take one off or add one (1 to [`MAX_CLIENTS`]), in one
/// bordered control. Laid out by a right-to-left row.
fn stepper(ui: &mut Ui, clients: &mut u8) {
    let p = current_colors(ui.ctx()).palette;
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::INPUT))
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if step(ui, "+", "One more client", *clients < MAX_CLIENTS) {
                *clients += 1;
            }
            let (rect, _) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                clients.to_string(),
                fonts::named(fonts::STRONG, text::BODY),
                p.text,
            );
            if step(ui, "\u{2212}", "One client fewer", *clients > 1) {
                *clients -= 1;
            }
        });
}

/// One of the stepper's buttons, `glyph` on it; returns whether it was clicked.
fn step(ui: &mut Ui, glyph: &str, label: &str, enabled: bool) -> bool {
    let p = current_colors(ui.ctx()).palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), sense);
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM - 1), p.surface_hover);
    }
    let ink = if enabled { p.text } else { p.subtle.gamma_multiply(0.6) };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        fonts::named(fonts::STRONG, text::BODY_LG),
        ink,
    );
    enabled && response.clicked()
}
