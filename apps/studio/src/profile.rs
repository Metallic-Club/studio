//! The profile button (#602): a round avatar beside the editor's Settings gear and at the foot of the projects page's
//! rail. A click opens a popup about the account. There is no sign-in yet, so it says so and lets the user choose the
//! name and colour the avatar shows ([`crate::settings::Profile`], saved in Studio's settings).

use egui::{
    Align2, Color32, CornerRadius, FontId, Popup, PopupCloseBehavior, Rect, Response, RichText, Sense, Stroke,
    TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use crate::settings::Profile;
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};

/// The avatar colours offered; the first is the default.
pub const COLORS: [[u8; 3]; 8] = [
    [59, 130, 246],
    [16, 163, 127],
    [234, 179, 8],
    [239, 68, 68],
    [168, 85, 247],
    [236, 72, 153],
    [249, 115, 22],
    [100, 116, 139],
];

/// The popup's width.
const WIDTH: f32 = 280.0;

/// Draws `profile`'s avatar filling `rect`: its colour, and the name's initial on it, or a head and shoulders while
/// there is no name.
pub fn paint_avatar(ui: &Ui, rect: Rect, profile: &Profile) {
    let [r, g, b] = profile.color;
    let fill = Color32::from_rgb(r, g, b);
    let ink = ink_on(profile.color);
    let centre = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let painter = ui.painter();
    painter.circle_filled(centre, radius, fill);
    match profile.initial() {
        Some(initial) => {
            painter.text(
                centre,
                Align2::CENTER_CENTER,
                initial,
                fonts::named(fonts::STRONG, radius * 1.05),
                ink,
            );
        }
        None => {
            // A head, and shoulders cut off by the circle's box.
            painter.circle_filled(centre - vec2(0.0, radius * 0.22), radius * 0.3, ink);
            painter.with_clip_rect(rect.shrink(radius * 0.12)).circle_filled(
                centre + vec2(0.0, radius * 0.78),
                radius * 0.52,
                ink,
            );
        }
    }
}

/// White or near-black, whichever reads better on `color`.
fn ink_on([r, g, b]: [u8; 3]) -> Color32 {
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 165.0 {
        Color32::from_rgb(24, 24, 27)
    } else {
        Color32::WHITE
    }
}

/// The profile button as an avatar `side` points across, raised under the pointer and while its popup is open; a
/// click opens the popup, which edits `profile`. Returns the button's response.
pub fn button(ui: &mut Ui, profile: &mut Profile, side: f32) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, "Profile"));
    let open = Popup::is_id_open(ui.ctx(), popup_id());
    if open || response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
    }
    paint_avatar(ui, rect.shrink(side * 0.15), profile);
    popup(ui, &response, profile);
    response.on_hover_text(format!("{} (offline)", profile.shown_name()))
}

/// The profile button at the foot of the projects page's rail: the avatar with the name and "Working offline" beside
/// it, `width` points wide; a click opens the same popup.
pub fn rail_button(ui: &mut Ui, profile: &mut Profile, width: f32) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let height = 40.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, "Profile"));
    let open = Popup::is_id_open(ui.ctx(), popup_id());
    if open || response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
    }
    let avatar = Rect::from_center_size(pos2(rect.left() + space::S2 + 14.0, rect.center().y), vec2(28.0, 28.0));
    paint_avatar(ui, avatar, profile);
    let left = avatar.right() + space::S3;
    let painter = ui.painter().with_clip_rect(rect);
    painter.text(
        pos2(left, rect.center().y - 1.0),
        Align2::LEFT_BOTTOM,
        profile.shown_name(),
        fonts::named(fonts::STRONG, text::BODY),
        p.text,
    );
    painter.text(
        pos2(left, rect.center().y + 1.0),
        Align2::LEFT_TOP,
        "Working offline",
        FontId::proportional(text::SMALL),
        p.subtle,
    );
    popup(ui, &response, profile);
    response
}

/// The popup's id: one for Studio, whichever button opened it.
fn popup_id() -> egui::Id {
    egui::Id::unique("metallic.profile.popup")
}

/// The name field's text while it is typed into: trimmed only as it is kept, so spaces can be typed.
fn draft_id() -> egui::Id {
    egui::Id::unique("metallic.profile.name")
}

/// The popup under `response`: who is signed in (no one, offline), the name and the colour.
fn popup(ui: &Ui, response: &Response, profile: &mut Profile) {
    let id = popup_id();
    let was_open = Popup::is_id_open(ui.ctx(), id);
    Popup::menu(response)
        .frame(
            egui::Frame::new()
                .fill(current_colors(ui.ctx()).palette.surface)
                .stroke(Stroke::new(size::BORDER, current_colors(ui.ctx()).palette.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .shadow(crate::theme::tokens::shadow::MENU)
                .inner_margin(space::S5),
        )
        .layout(egui::Layout::top_down(egui::Align::Min))
        .id(id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let colors = current_colors(ui.ctx());
            let p = colors.palette;
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = space::S3;
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                paint_avatar(ui, rect, profile);
                ui.add_space(space::S2);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(profile.shown_name())
                            .font(fonts::named(fonts::STRONG, text::BODY_LG))
                            .color(p.text),
                    );
                    ui.label(RichText::new("Offline").size(text::SMALL).color(p.subtle));
                });
            });
            ui.separator();
            ui.label(
                RichText::new(
                    "You're not signed in. Accounts come with Metallic's servers; until then, your name and colour \
                     are kept on this computer.",
                )
                .size(text::SMALL)
                .color(p.muted),
            );
            ui.label(RichText::new("Name").size(text::SMALL).color(p.subtle));
            let mut draft = ui.ctx().data_mut(|data| {
                if !was_open {
                    data.remove::<String>(draft_id());
                }
                data.get_temp::<String>(draft_id())
                    .unwrap_or_else(|| profile.name.clone())
            });
            let field = ui.add(
                TextEdit::singleline(&mut draft)
                    .hint_text("Guest")
                    .char_limit(Profile::MAX_NAME + 8)
                    .desired_width(f32::INFINITY),
            );
            field.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, "Profile name"));
            if field.changed() {
                profile.name = Profile::clean_name(&draft);
            }
            ui.ctx().data_mut(|data| data.insert_temp(draft_id(), draft));
            ui.label(RichText::new("Colour").size(text::SMALL).color(p.subtle));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = space::S2;
                for choice in COLORS {
                    swatch(ui, profile, choice);
                }
                custom(ui, profile, id);
            });
        });
}

/// One colour to choose, a round swatch; the chosen one ringed.
fn swatch(ui: &mut Ui, profile: &mut Profile, choice: [u8; 3]) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let chosen = profile.color == choice;
    let [r, g, b] = choice;
    let name = format!("Colour #{r:02x}{g:02x}{b:02x}");
    response.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, chosen, &name));
    if response.clicked() {
        profile.color = choice;
    }
    let centre = rect.center();
    ui.painter().circle_filled(centre, 8.0, Color32::from_rgb(r, g, b));
    let ring = if chosen {
        Some(p.text)
    } else if response.hovered() {
        Some(p.subtle)
    } else {
        None
    };
    if let Some(ring) = ring {
        ui.painter().circle_stroke(centre, 11.0, Stroke::new(2.0, ring));
    }
}

/// A colour of the user's own: a swatch with the palette mark that opens Properties' colour picker (#416).
fn custom(ui: &mut Ui, profile: &mut Profile, popup: egui::Id) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let own = !COLORS.contains(&profile.color);
    response.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, own, "Custom colour"));
    let centre = rect.center();
    if own {
        let [r, g, b] = profile.color;
        ui.painter().circle_filled(centre, 8.0, Color32::from_rgb(r, g, b));
        ui.painter().circle_stroke(centre, 11.0, Stroke::new(2.0, p.text));
    } else {
        ui.painter().circle(
            centre,
            10.0,
            p.input,
            Stroke::new(size::BORDER, if response.hovered() { p.text } else { p.subtle }),
        );
        crate::theme::icons::paint_at(
            ui.painter(),
            centre,
            Align2::CENTER_CENTER,
            crate::theme::icons::Icon::UiPalette,
            text::SMALL,
            p.text,
        );
    }
    let id = popup.with("picker");
    Popup::menu(&response)
        .id(id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut rgb = profile.color;
            if crate::panels::color_picker::picker(ui, id.with("colour"), &mut rgb) {
                profile.color = rgb;
            }
        });
    let _ = response.on_hover_text("A colour of your own");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_avatar_shows_the_name_s_initial_on_ink_that_reads() {
        let mut profile = Profile::default();
        assert_eq!(profile.initial(), None);
        assert_eq!(profile.shown_name(), "Guest");
        profile.name = Profile::clean_name("  ada lovelace  ");
        assert_eq!(profile.name, "ada lovelace");
        assert_eq!(profile.initial(), Some('A'));
        assert_eq!(Profile::clean_name(&"x".repeat(40)).len(), Profile::MAX_NAME);
        assert_eq!(ink_on([250, 250, 250]), Color32::from_rgb(24, 24, 27));
        assert_eq!(ink_on(COLORS[0]), Color32::WHITE);
    }
}
