//! The projects page's Team Create section (ADR 0011): the projects the user hosts or joined, each as a card that
//! says, from a real check of its host ([`super::probe`]), whether it can be joined now.
//!
//! - **Distinct cards.** A Team Create card has the Team Create icon on its thumbnail and an outline: green while
//!   its host answers for it, grey while it does not, with the reason under the pointer.
//! - **Opening.** A participant joins an available project (double-click, or Join); one that is not available cannot
//!   be opened. The owner can always open their own file, and Open and host hosts it again on its port.
//! - **Join by address** joins a project not in the list yet; joining adds it.

use egui::{
    CornerRadius, Frame, Id, Label, Layout, Modal, Rect, RichText, Role, Stroke, StrokeKind, TextEdit, Ui, WidgetInfo,
    pos2, vec2,
};

use super::probe::Reach;
use super::store::Role as Owned;
use super::{PageAction, Session, TeamCreate};
use crate::projects::{THUMBNAIL_ASPECT, card, grid as cards};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{Hue, radius, shadow, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

fn dialog_id() -> Id {
    Id::unique("metallic.team_create.join_dialog")
}

/// Shows the Join by address dialog from the next frame.
pub fn open_join_dialog(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(dialog_id(), true));
}

/// The Team Create projects as cards, probing their hosts as the page shows them.
pub fn grid(ui: &mut Ui, team: &mut TeamCreate) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    team.prober.poll();
    team.prober
        .refresh(&team.store.projects, std::time::Instant::now(), false);
    // The join under way, or why the last one failed.
    if matches!(team.session(), Session::Joining(_)) {
        ui.label(RichText::new(team.status_line()).size(text::BODY).color(p.muted));
        ui.add_space(space::S4);
    } else if let Some(note) = team.note.as_ref().filter(|note| note.error) {
        ui.label(
            RichText::new(&note.text)
                .size(text::BODY)
                .color(crate::panels::output::ERROR),
        );
        ui.add_space(space::S4);
    }
    if team.store.projects.is_empty() {
        ui.add_space(space::S10);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No Team Create projects yet")
                    .font(fonts::named(fonts::HEADING, text::TITLE))
                    .color(p.text),
            );
            ui.label(
                RichText::new(
                    "Host a project from the editor's Team Create button, or join one by its host's address.",
                )
                .size(text::BODY)
                .color(p.muted),
            );
        });
        return;
    }
    let projects = team.store.projects.clone();
    let hosting = team.info().filter(|_| team.is_hosting()).map(|info| info.project);
    let mut actions = Vec::new();
    cards(ui, projects.len(), |ui, index, rect| {
        let project = &projects[index];
        let reach = if hosting == Some(project.id) {
            Reach::Available {
                users: team.users().len() as u32,
                host: project.owner.clone(),
            }
        } else {
            team.prober.reach(project.id).clone()
        };
        let available = reach.is_available();
        let owner = project.role == Owned::Owner;
        let address = format!("{}:{}", project.host, project.port);
        let line = if owner {
            format!("You host it \u{00B7} {address}")
        } else {
            format!("Hosted by {} \u{00B7} {address}", project.owner)
        };
        let response = card(
            ui,
            rect,
            Id::unique(("metallic.team_create.card", project.id)),
            &project.name,
            &line,
            false,
        );
        let status = match &reach {
            Reach::Available { users, .. } => format!("Available \u{00B7} {users} in it"),
            Reach::Checking | Reach::Unknown => "Checking\u{2026}".to_owned(),
            Reach::Unavailable(_) => "Not hosted".to_owned(),
        };
        let label = format!("{} (Team Create, {status})", project.name);
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
        // The thumbnail: the Team Create mark, an outline in the project's state, and its status.
        let thumbnail = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * THUMBNAIL_ASPECT));
        let green = colors.hue(Hue::Green);
        let edge = if available { green } else { p.subtle.gamma_multiply(0.6) };
        ui.painter().rect_stroke(
            thumbnail,
            CornerRadius::same(radius::PANEL),
            Stroke::new(2.0, edge),
            StrokeKind::Inside,
        );
        let mark = Rect::from_center_size(thumbnail.center(), vec2(40.0, 40.0));
        icons::paint(
            ui.painter(),
            Icon::ToolbarTeamCreate,
            mark,
            if available { green } else { p.subtle },
        );
        let pill_text = ui
            .painter()
            .layout_no_wrap(status.clone(), fonts::named(fonts::STRONG, text::SMALL), p.text);
        let pill = Rect::from_min_size(
            thumbnail.min + vec2(space::S4, space::S4),
            vec2(pill_text.size().x + 22.0, pill_text.size().y + 8.0),
        );
        ui.painter()
            .rect_filled(pill, CornerRadius::same(radius::ITEM), p.panel);
        let dot = if available {
            green
        } else if matches!(reach, Reach::Checking | Reach::Unknown) {
            colors.hue(Hue::Yellow)
        } else {
            p.subtle
        };
        ui.painter()
            .circle_filled(pos2(pill.left() + 9.0, pill.center().y), 3.5, dot);
        ui.painter().galley(
            pos2(pill.left() + 17.0, pill.center().y - pill_text.size().y / 2.0),
            pill_text,
            p.text,
        );
        if owner && let Some(path) = &project.path {
            let host = PageAction::OpenOwned {
                path: path.clone(),
                port: project.port,
                host: true,
            };
            if response.double_clicked() {
                actions.push(host.clone());
            }
            response.context_menu(|ui| {
                if ui.button("Open and host").clicked() {
                    actions.push(host.clone());
                    ui.close();
                }
                if ui.button("Open without hosting").clicked() {
                    actions.push(PageAction::OpenOwned {
                        path: path.clone(),
                        port: project.port,
                        host: false,
                    });
                    ui.close();
                }
                if ui.button("Remove from list").clicked() {
                    actions.push(PageAction::Forget(project.id));
                    ui.close();
                }
            });
        } else {
            if response.double_clicked() && available {
                actions.push(PageAction::Join(project.id));
            }
            response.context_menu(|ui| {
                if ui.add_enabled(available, egui::Button::new("Join")).clicked() {
                    actions.push(PageAction::Join(project.id));
                    ui.close();
                }
                if ui.button("Remove from list").clicked() {
                    actions.push(PageAction::Forget(project.id));
                    ui.close();
                }
            });
        }
        let tip = match &reach {
            Reach::Unavailable(why) if !owner => format!("Not available: {why}"),
            Reach::Unavailable(why) => format!("Not hosted now ({why}). Double-click to open and host it."),
            Reach::Available { .. } if owner => "Double-click to open and host it.".to_owned(),
            Reach::Available { .. } => "Double-click to join.".to_owned(),
            _ => "Checking whether it is hosted\u{2026}".to_owned(),
        };
        response.on_hover_text(tip);
    });
    team.page_actions.extend(actions);
}

/// The Join by address dialog, while it is open: the address and the name, and Join.
pub fn join_dialog(ctx: &egui::Context, team: &mut TeamCreate) {
    if !ctx.data(|data| data.get_temp::<bool>(dialog_id())).unwrap_or(false) {
        return;
    }
    let p = current_colors(ctx).palette;
    let (mut join, mut cancel) = (false, false);
    let frame = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::DIALOG))
        .shadow(shadow::MENU)
        .inner_margin(space::S8);
    Modal::new(dialog_id().with("modal")).frame(frame).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(
            RichText::new("Join a Team Create project")
                .font(fonts::named(fonts::HEADING, text::TITLE))
                .color(p.text),
        );
        ui.add(
            Label::new(
                RichText::new("The host's address and port, as their Team Create menu shows them.")
                    .size(text::BODY)
                    .color(p.muted),
            )
            .wrap(),
        );
        ui.add_space(space::S5);
        ui.label(crate::projects::section_label("Address", p.subtle));
        let address = ui.add(
            TextEdit::singleline(&mut team.form.address)
                .hint_text("192.168.1.20:7350")
                .char_limit(300)
                .desired_width(f32::INFINITY),
        );
        address.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Team Create address"));
        ui.add_space(space::S4);
        ui.label(crate::projects::section_label("Your name", p.subtle));
        let name = ui.add(
            TextEdit::singleline(&mut team.form.name)
                .hint_text("Your profile's name")
                .char_limit(40)
                .desired_width(f32::INFINITY),
        );
        name.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Team Create name"));
        ui.add_space(space::S6);
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            join = button(ui, ButtonKind::Primary, "Join").clicked()
                || (address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            cancel = button(ui, ButtonKind::Secondary, "Cancel").clicked();
        });
    });
    if join {
        team.page_actions.push(PageAction::JoinAddress);
    }
    if join || cancel {
        ctx.data_mut(|data| data.remove::<bool>(dialog_id()));
    }
}
