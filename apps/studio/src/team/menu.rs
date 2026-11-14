//! The top bar's Team Create button and its menu (ADR 0011): host the open project, join one by address, or see the
//! session under way.
//!
//! The menu is drawn in Studio's own style (#643), as the profile popup and the Settings window are: uppercase section
//! labels, the segmented control for Host and Join, bordered buttons and fields. egui's menu style, which strips the
//! buttons' fill and border, is not applied.

use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Popup, PopupCloseBehavior, Rect, Response,
    RichText, Sense, Stroke, TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use super::{Session, TeamCreate};
use crate::projects::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{Hue, radius, size, space, text};
use crate::theme::{ButtonKind, current_colors, fonts, wide_button};

/// The menu's width.
const WIDTH: f32 = 300.0;

/// How long a copied address shows a check mark in place of the Copy icon, in seconds.
const COPIED_FOR: f64 = 1.5;

/// What the menu asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamAction {
    /// Host the open project on the port typed.
    Host,
    /// Join the address typed, as the name typed.
    Join,
    /// End the hosted session.
    End,
    /// Leave the joined session (or stop joining).
    Leave,
}

/// Which of Host and Join the menu shows while there is no session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Host,
    Join,
}

/// What the menu needs to know about the editor.
#[derive(Clone, Copy, Debug)]
pub struct EditorFacts<'a> {
    /// The open project's name, if one is open.
    pub project: Option<&'a str>,
    /// Whether the open project has unsaved changes (joining would close it).
    pub modified: bool,
    /// The profile's name and colour, the default identity.
    pub profile: (&'a str, [u8; 3]),
}

pub(crate) fn popup_id() -> egui::Id {
    egui::Id::unique("metallic.team_create.popup")
}

fn tab_id() -> egui::Id {
    egui::Id::unique("metallic.team_create.tab")
}

/// The Team Create button, `side` points square, with a dot in the session's colour while one runs; a click opens
/// the menu. Returns what the menu asked for.
pub fn button(ui: &mut Ui, team: &mut TeamCreate, facts: EditorFacts<'_>, side: f32) -> Option<TeamAction> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, "Team Create"));
    let open = Popup::is_id_open(ui.ctx(), popup_id());
    if open || response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
    }
    let icon = Rect::from_center_size(rect.center(), vec2(text::BODY_LG, text::BODY_LG));
    icons::paint(ui.painter(), Icon::ToolbarTeamCreate, icon, p.text);
    if let Some((dot, _)) = status(team, &colors) {
        let centre = pos2(rect.right() - 6.0, rect.top() + 6.0);
        ui.painter().circle_filled(centre, 4.0, p.header);
        ui.painter().circle_filled(centre, 3.0, dot);
    }
    let action = popup(ui, &response, team, facts);
    let tip = format!("Team Create: {}", team.status_line());
    response.on_hover_text(tip);
    action
}

/// The session's state as a colour and a word: green while hosting or joined, the warning colour while joining, none
/// without a session.
fn status(team: &TeamCreate, colors: &crate::theme::Colors) -> Option<(Color32, &'static str)> {
    match team.session() {
        Session::Idle => None,
        Session::Hosting(_) => Some((colors.hue(Hue::Green), "Hosting")),
        Session::Joined(_) => Some((colors.hue(Hue::Green), "Connected")),
        Session::Joining(_) => Some((colors.hue(Hue::Yellow), "Joining")),
    }
}

fn popup(ui: &Ui, response: &Response, team: &mut TeamCreate, facts: EditorFacts<'_>) -> Option<TeamAction> {
    let mut action = None;
    let colors = current_colors(ui.ctx());
    Popup::menu(response)
        // Studio's style, not egui's menu style (see the module documentation).
        .style(egui::style::StyleModifier::default())
        .frame(
            Frame::new()
                .fill(colors.palette.surface)
                .stroke(Stroke::new(size::BORDER, colors.palette.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .shadow(crate::theme::tokens::shadow::MENU)
                .inner_margin(space::S6),
        )
        .layout(Layout::top_down(Align::Min))
        .id(popup_id())
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = space::S3;
            action = contents(ui, team, facts);
        });
    action
}

/// The menu's contents: the session under way, or Host and Join.
pub fn contents(ui: &mut Ui, team: &mut TeamCreate, facts: EditorFacts<'_>) -> Option<TeamAction> {
    header(ui, team);
    ui.separator();
    let action = match team.session() {
        Session::Idle => idle(ui, team, facts),
        _ => active(ui, team),
    };
    if let Some(note) = &team.note {
        let color = if note.error {
            crate::panels::output::ERROR
        } else {
            current_colors(ui.ctx()).palette.muted
        };
        ui.label(RichText::new(&note.text).size(text::SMALL).color(color));
    }
    action
}

/// The Team Create mark on a tile, the title with "Offline" under it (sessions run without Metallic's servers), and
/// the session's state on the right, as the profile popup's header.
fn header(ui: &mut Ui, team: &TeamCreate) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    ui.horizontal(|ui| {
        let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter()
            .rect_filled(tile, CornerRadius::same(radius::ITEM), p.surface_hover);
        let mark = Rect::from_center_size(tile.center(), vec2(text::TITLE, text::TITLE));
        icons::paint(ui.painter(), Icon::ToolbarTeamCreate, mark, p.text);
        ui.add_space(space::S2);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                RichText::new("Team Create")
                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                    .color(p.text),
            );
            ui.label(RichText::new("Offline").size(text::SMALL).color(p.subtle));
        });
        if let Some((dot, word)) = status(team, &colors) {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| chip(ui, dot, word));
        }
    });
}

/// A pill with a coloured dot and a word: the session's state.
fn chip(ui: &mut Ui, dot: Color32, word: &str) {
    let p = current_colors(ui.ctx()).palette;
    let galley = ui
        .painter()
        .layout_no_wrap(word.to_owned(), fonts::named(fonts::STRONG, text::SMALL), p.text);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, galley.size().y + 8.0), Sense::hover());
    let round = CornerRadius::same((rect.height() / 2.0) as u8);
    ui.painter().rect(
        rect,
        round,
        p.input,
        Stroke::new(size::BORDER, p.border),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .circle_filled(pos2(rect.left() + 10.0, rect.center().y), 3.5, dot);
    ui.painter().galley(
        pos2(rect.left() + 17.0, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );
}

fn idle(ui: &mut Ui, team: &mut TeamCreate, facts: EditorFacts<'_>) -> Option<TeamAction> {
    let p = current_colors(ui.ctx()).palette;
    let mut tab: Tab = ui.ctx().data(|data| data.get_temp(tab_id())).unwrap_or_default();
    tabs(ui, &mut tab);
    ui.ctx().data_mut(|data| data.insert_temp(tab_id(), tab));
    let mut action = None;
    match tab {
        Tab::Host => {
            let hint = match facts.project {
                Some(name) => format!(
                    "Share \u{201C}{name}\u{201D} with people on your network, or over the internet with the port \
                     and the one after it (Team Create Play's) forwarded on your router."
                ),
                None => "Open a project to host it.".to_owned(),
            };
            ui.label(RichText::new(hint).size(text::SMALL).color(p.muted));
            field_label(ui, "Port");
            let field = ui.add(
                TextEdit::singleline(&mut team.form.port)
                    .char_limit(5)
                    .desired_width(f32::INFINITY),
            );
            field.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, "Team Create port"));
            ui.add_space(space::S2);
            let enabled = facts.project.is_some();
            let start = ui
                .add_enabled_ui(enabled, |ui| wide_button(ui, ButtonKind::Primary, "Start hosting"))
                .inner;
            if start.clicked() {
                action = Some(TeamAction::Host);
            }
        }
        Tab::Join => {
            ui.label(
                RichText::new("Join a project someone hosts: their address and port.")
                    .size(text::SMALL)
                    .color(p.muted),
            );
            field_label(ui, "Address");
            let address = ui.add(
                TextEdit::singleline(&mut team.form.address)
                    .hint_text("192.168.1.20:7350")
                    .char_limit(300)
                    .desired_width(f32::INFINITY),
            );
            address.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, "Team Create address"));
            field_label(ui, "Your name");
            let name = ui.add(
                TextEdit::singleline(&mut team.form.name)
                    .hint_text(facts.profile.0)
                    .char_limit(40)
                    .desired_width(f32::INFINITY),
            );
            name.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, "Team Create name"));
            if facts.modified {
                ui.label(
                    RichText::new("Joining closes the open project, which has unsaved changes. Save it first.")
                        .size(text::SMALL)
                        .color(crate::panels::output::WARNING),
                );
            }
            ui.add_space(space::S2);
            let enter = address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let join = ui
                .add_enabled_ui(!facts.modified, |ui| wide_button(ui, ButtonKind::Primary, "Join"))
                .inner;
            if (join.clicked() || enter) && !facts.modified {
                action = Some(TeamAction::Join);
            }
        }
    }
    action
}

/// Host and Join as the Settings window's segmented control, the halves sharing the menu's width.
fn tabs(ui: &mut Ui, tab: &mut Tab) {
    let p = current_colors(ui.ctx()).palette;
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::INPUT))
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let gap = 2.0;
                ui.spacing_mut().item_spacing.x = gap;
                let half = (ui.available_width() - gap) / 2.0;
                for (choice, label) in [(Tab::Host, "Host"), (Tab::Join, "Join")] {
                    let chosen = *tab == choice;
                    let words = RichText::new(label)
                        .font(fonts::named(fonts::STRONG, text::BODY))
                        .color(if chosen { p.text } else { p.muted });
                    let response = ui.add(
                        egui::Button::new(words)
                            .fill(if chosen { p.surface_hover } else { Color32::TRANSPARENT })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(radius::ITEM - 1))
                            .min_size(vec2(half, size::CONTROL - 6.0)),
                    );
                    response.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, chosen, label));
                    if response.clicked() {
                        *tab = choice;
                    }
                }
            });
        });
}

fn active(ui: &mut Ui, team: &mut TeamCreate) -> Option<TeamAction> {
    let p = current_colors(ui.ctx()).palette;
    let mut action = None;
    if let Some(info) = team.info() {
        field_label(ui, "Project");
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = space::S1;
            ui.label(
                RichText::new(&info.project_name)
                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                    .color(p.text),
            );
            ui.label(
                RichText::new(format!("Hosted by {}", info.host_name))
                    .size(text::SMALL)
                    .color(p.muted),
            );
        });
    }
    ui.label(RichText::new(team.status_line()).size(text::SMALL).color(p.subtle));
    if let Session::Joining(joining) = team.session()
        && let Some((received, total)) = joining.progress
        && total > 0
    {
        ui.add(egui::ProgressBar::new(received as f32 / total as f32).desired_height(4.0));
    }
    if let Session::Hosting(hosting) = team.session() {
        field_label(ui, "Others join at");
        let port = hosting.server.port();
        for address in &hosting.addresses {
            let text = match address {
                std::net::IpAddr::V6(v6) => format!("[{v6}]:{port}"),
                std::net::IpAddr::V4(v4) => format!("{v4}:{port}"),
            };
            address_row(ui, &text);
        }
        // These are this machine's addresses on its own network (#639): someone outside it needs the router's public
        // address, with the session's port and Team Create Play's forwarded to this computer.
        let ports = match crate::team::play_port_after(port) {
            Some(play) => format!("TCP ports {port} and {play}"),
            None => format!("TCP port {port}"),
        };
        ui.label(
            RichText::new(format!(
                "On your network. From outside it, people join at your public address, with {ports} forwarded \
                 to this computer."
            ))
            .size(text::SMALL)
            .color(p.subtle),
        );
    }
    let users = team.users();
    if !users.is_empty() {
        field_label(ui, &format!("People \u{00B7} {}", users.len()));
        let me = team.me();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = space::S2;
            for (user, ready) in users {
                let mut name = user.identity.name.clone();
                if Some(user.id) == me {
                    name.push_str(" (you)");
                }
                let role = if user.host {
                    "Host"
                } else if ready {
                    "Editing"
                } else {
                    "Joining\u{2026}"
                };
                person(ui, &name, &user.identity.name, user.identity.color, role);
            }
        });
    }
    ui.separator();
    match team.session() {
        Session::Hosting(_) => {
            if wide_button(ui, ButtonKind::Secondary, "End session")
                .on_hover_text("Saves the project, tells everyone, and stops hosting")
                .clicked()
            {
                action = Some(TeamAction::End);
            }
        }
        Session::Joining(_) => {
            if wide_button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                action = Some(TeamAction::Leave);
            }
        }
        Session::Joined(_) => {
            if wide_button(ui, ButtonKind::Secondary, "Disconnect").clicked() {
                action = Some(TeamAction::Leave);
            }
        }
        Session::Idle => {}
    }
    action
}

/// An address others join at, in a field-like box, with a Copy button that shows a check mark for a moment after a
/// copy.
fn address_row(ui: &mut Ui, address: &str) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let copied_id = egui::Id::unique(("metallic.team_create.copied", address));
    let now = ui.input(|input| input.time);
    let copied = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(copied_id))
        .is_some_and(|at| now - at < COPIED_FOR);
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::INPUT))
        .inner_margin(Margin {
            left: space::S4 as i8,
            right: 3,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(address).font(FontId::monospace(text::BODY)).color(p.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Copy {address}")));
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(radius::ITEM - 2), p.surface_hover);
                    }
                    let (icon, ink) = if copied {
                        (Icon::UiCheck, colors.hue(Hue::Green))
                    } else if response.hovered() {
                        (Icon::UiCopy, p.text)
                    } else {
                        (Icon::UiCopy, p.muted)
                    };
                    icons::paint_at(
                        ui.painter(),
                        rect.center(),
                        Align2::CENTER_CENTER,
                        icon,
                        text::BODY,
                        ink,
                    );
                    if response.clicked() {
                        ui.ctx().copy_text(address.to_owned());
                        ui.ctx().data_mut(|data| data.insert_temp(copied_id, now));
                    }
                    response.on_hover_text(if copied { "Copied" } else { "Copy the address" });
                });
            });
        });
    if copied {
        // Back to the Copy icon when the moment is over, without waiting for the pointer to move.
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

/// One person in the session: their colour as an avatar with their initial, `label` (their name, marked when it is
/// you), and what they are doing.
fn person(ui: &mut Ui, label: &str, name: &str, color: [u8; 3], role: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        let [r, g, b] = color;
        ui.painter()
            .circle_filled(rect.center(), rect.width() / 2.0, Color32::from_rgb(r, g, b));
        if let Some(initial) = name.chars().find(|c| c.is_alphanumeric()) {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                initial.to_uppercase(),
                fonts::named(fonts::STRONG, text::SMALL),
                crate::profile::ink_on(color),
            );
        }
        ui.add_space(space::S1);
        ui.label(RichText::new(label).size(text::BODY).color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(role).size(text::SMALL).color(p.subtle));
        });
    });
}

/// A field's or a section's label: uppercase, as the rest of Studio's.
fn field_label(ui: &mut Ui, label: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.add_space(space::S1);
    ui.label(section_label(label, p.subtle));
}
