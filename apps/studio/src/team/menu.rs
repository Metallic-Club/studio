//! The top bar's Team Create button and its menu (ADR 0011): host the open project, join one by address, or see the
//! session under way.

use egui::{
    Color32, CornerRadius, FontId, Popup, PopupCloseBehavior, Rect, Response, RichText, Sense, Stroke, TextEdit, Ui,
    WidgetInfo, pos2, vec2,
};

use super::{Session, TeamCreate};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{Hue, radius, size, space, text};
use crate::theme::{ButtonKind, current_colors, fonts, wide_button};

/// The menu's width.
const WIDTH: f32 = 300.0;

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
    if let Some(dot) = status_color(team, &colors) {
        let centre = pos2(rect.right() - 6.0, rect.top() + 6.0);
        ui.painter().circle_filled(centre, 4.0, p.header);
        ui.painter().circle_filled(centre, 3.0, dot);
    }
    let action = popup(ui, &response, team, facts);
    let tip = format!("Team Create: {}", team.status_line());
    response.on_hover_text(tip);
    action
}

/// The dot's colour: green while hosting or joined, the warning colour while joining, none without a session.
fn status_color(team: &TeamCreate, colors: &crate::theme::Colors) -> Option<Color32> {
    match team.session() {
        Session::Idle => None,
        Session::Hosting(_) | Session::Joined(_) => Some(colors.hue(Hue::Green)),
        Session::Joining(_) => Some(colors.hue(Hue::Yellow)),
    }
}

fn popup(ui: &Ui, response: &Response, team: &mut TeamCreate, facts: EditorFacts<'_>) -> Option<TeamAction> {
    let mut action = None;
    let colors = current_colors(ui.ctx());
    Popup::menu(response)
        .frame(
            egui::Frame::new()
                .fill(colors.palette.surface)
                .stroke(Stroke::new(size::BORDER, colors.palette.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .shadow(crate::theme::tokens::shadow::MENU)
                .inner_margin(space::S5),
        )
        .layout(egui::Layout::top_down(egui::Align::Min))
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
    let p = current_colors(ui.ctx()).palette;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Team Create")
                .font(fonts::named(fonts::STRONG, text::BODY_LG))
                .color(p.text),
        );
        ui.label(RichText::new("Offline").size(text::SMALL).color(p.subtle));
    });
    let action = match team.session() {
        Session::Idle => idle(ui, team, facts),
        _ => active(ui, team),
    };
    if let Some(note) = &team.note {
        let color = if note.error {
            crate::panels::output::ERROR
        } else {
            p.muted
        };
        ui.label(RichText::new(&note.text).size(text::SMALL).color(color));
    }
    action
}

fn idle(ui: &mut Ui, team: &mut TeamCreate, facts: EditorFacts<'_>) -> Option<TeamAction> {
    let p = current_colors(ui.ctx()).palette;
    let mut tab: Tab = ui.ctx().data(|data| data.get_temp(tab_id())).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
        for (choice, label) in [(Tab::Host, "Host"), (Tab::Join, "Join")] {
            let chosen = tab == choice;
            let response = ui.add(
                egui::Button::selectable(chosen, RichText::new(label).size(text::BODY))
                    .corner_radius(CornerRadius::same(radius::ITEM)),
            );
            response.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, chosen, label));
            if response.clicked() {
                tab = choice;
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(tab_id(), tab));
    let mut action = None;
    match tab {
        Tab::Host => {
            let hint = match facts.project {
                Some(name) => format!(
                    "Share \u{201C}{name}\u{201D} with people on your network, or over the internet with the port \
                     forwarded on your router."
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

fn active(ui: &mut Ui, team: &mut TeamCreate) -> Option<TeamAction> {
    let p = current_colors(ui.ctx()).palette;
    let mut action = None;
    if let Some(info) = team.info() {
        ui.label(
            RichText::new(&info.project_name)
                .font(fonts::named(fonts::STRONG, text::BODY))
                .color(p.text),
        );
        ui.label(
            RichText::new(format!("Hosted by {}", info.host_name))
                .size(text::SMALL)
                .color(p.muted),
        );
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
            ui.horizontal(|ui| {
                ui.label(RichText::new(&text).font(FontId::monospace(text::BODY)).color(p.text));
                if ui.small_button("Copy").on_hover_text("Copy the address").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
            });
        }
    }
    let users = team.users();
    if !users.is_empty() {
        field_label(ui, &format!("People ({})", users.len()));
        let me = team.me();
        for (user, ready) in users {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                let [r, g, b] = user.identity.color;
                ui.painter()
                    .circle_filled(dot.center(), 5.0, Color32::from_rgb(r, g, b));
                let mut name = user.identity.name.clone();
                if Some(user.id) == me {
                    name.push_str(" (you)");
                }
                ui.label(RichText::new(name).size(text::BODY).color(p.text));
                let role = if user.host {
                    "host"
                } else if ready {
                    "editing"
                } else {
                    "joining…"
                };
                ui.label(RichText::new(role).size(text::SMALL).color(p.subtle));
            });
        }
    }
    ui.add_space(space::S1);
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

fn field_label(ui: &mut Ui, label: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.label(RichText::new(label).size(text::SMALL).color(p.subtle));
}
