//! Studio's menus (#276, #660): items as wide as the menu, 28 points tall with no gap between them, a check mark
//! where an item is on, and the shortcut on the right; the menu bar's menus and the Explorer's row menu draw with
//! them.

use egui::{Align2, CornerRadius, Response, Role, Sense, Ui, WidgetInfo, pos2, vec2};

use super::current_colors;
use super::icons::{self, Icon};
use super::tokens::{radius, space, text};

/// The narrowest a menu is: otherwise as wide as its longest item and shortcut need.
pub const MENU_MIN_WIDTH: f32 = 150.0;
/// The space between an item's label and its shortcut.
pub const SHORTCUT_GAP: f32 = space::S8;
/// A menu item's height.
pub const ITEM_HEIGHT: f32 = 28.0;
/// The space between a menu's items.
pub const ITEM_GAP: f32 = 2.0;

/// The width a menu of `lines` (each a label, its shortcut, and whether it has room for a check mark) takes: its
/// longest line, with the item's padding on both sides; never less than [`MENU_MIN_WIDTH`].
pub fn menu_width<'a>(ui: &Ui, lines: impl IntoIterator<Item = (&'a str, Option<&'a str>, bool)>) -> f32 {
    let painter = ui.painter();
    let measure = |text: &str, size: f32| {
        painter
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), egui::Color32::WHITE)
            .size()
            .x
    };
    let widest = lines
        .into_iter()
        .map(|(label, shortcut, check)| {
            let check = if check { text::BODY + space::S3 } else { 0.0 };
            let keys = shortcut.map_or(0.0, |keys| SHORTCUT_GAP + measure(keys, text::SMALL));
            check + measure(label, text::BODY) + keys
        })
        .fold(0.0, f32::max);
    (widest + 2.0 * space::S4).ceil().max(MENU_MIN_WIDTH)
}

/// A menu item `width` wide: a check mark when `checked` is `Some(true)`, the label, and the shortcut on the right. A click closes
/// the menu. Not `enabled`, it is drawn at the theme's disabled strength and does nothing.
pub fn menu_item(
    ui: &mut Ui,
    width: f32,
    label: &str,
    shortcut: Option<String>,
    checked: Option<bool>,
    enabled: bool,
) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    // The menu's width, measured from its items: a popup's available width is not its own, and items sized from it
    // widen the menu every frame.
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ITEM_HEIGHT), sense);
    response.widget_info(|| match checked {
        Some(on) => WidgetInfo::selected(Role::MenuItemCheckBox, enabled, on, label),
        None => WidgetInfo::labeled(Role::MenuItem, enabled, label),
    });
    // The website's disabled strength: 48 %.
    let ink = if enabled { p.text } else { p.text.gamma_multiply(0.48) };
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let mut x = rect.left() + space::S4;
        if checked.is_some() {
            if checked == Some(true) {
                icons::paint_at(
                    ui.painter(),
                    pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    Icon::UiCheck,
                    text::BODY,
                    colors.accent,
                );
            }
            x += text::BODY + space::S3;
        }
        ui.painter().text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(text::BODY),
            ink,
        );
        if let Some(keys) = shortcut {
            ui.painter().text(
                pos2(rect.right() - space::S4, rect.center().y),
                Align2::RIGHT_CENTER,
                keys,
                egui::FontId::proportional(text::SMALL),
                p.subtle,
            );
        }
    }
    if response.clicked() {
        ui.close();
    }
    response
}

/// A menu item `width` wide that opens a submenu to its side (#702): the label, and a chevron on the right. Unlike
/// [`menu_item`], a click does not close the menu; the caller shows the submenu from the response
/// (`egui::containers::menu::SubMenu`).
pub fn submenu_item(ui: &mut Ui, width: f32, label: &str, enabled: bool) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ITEM_HEIGHT), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::MenuItem, enabled, label));
    let ink = if enabled { p.text } else { p.text.gamma_multiply(0.48) };
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        ui.painter().text(
            pos2(rect.left() + space::S4, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(text::BODY),
            ink,
        );
        icons::paint_at(
            ui.painter(),
            pos2(rect.right() - space::S3, rect.center().y),
            Align2::RIGHT_CENTER,
            Icon::UiExpand,
            text::SMALL,
            p.subtle,
        );
    }
    response
}
