//! The Editor screen's own pieces: the progress overlay while a project opens, and the top bar with its menus.
//!
//! The menus act through [`MenuAction`]s the app carries out, the same as the keymap's commands, and show every
//! binding of each command from the keymap.

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Layout, Rect, Response, RichText, Role, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo, pos2, vec2,
};

use crate::dock::Dock;
use crate::editor::{EditorState, STEPS};
use crate::keymap::{Keymap, ids};
use crate::panels;
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{current_colors, fonts};

/// The top bar's height.
pub(crate) const BAR_HEIGHT: f32 = 40.0;
/// A menu's width.
const MENU_WIDTH: f32 = 240.0;
/// The width the File and View menus take in the bar.
const MENUS_WIDTH: f32 = 96.0;
/// The progress card's width.
const CARD_WIDTH: f32 = 420.0;

/// What a menu item asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    /// File, Save.
    Save,
    /// File, Close project.
    Close,
    /// View: open or close a panel.
    Toggle(&'static str),
    /// View, Reset layout.
    ResetLayout,
    /// The gear: open or close the Settings window.
    Settings,
}

/// The progress overlay while the project `name` opens, at step `step` (from 0) of [`STEPS`].
pub(crate) fn opening_overlay(ui: &mut Ui, name: &str, step: usize) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, p.bg);
    let card = Rect::from_center_size(screen.center(), vec2(CARD_WIDTH, 168.0));
    ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(size::BORDER, p.border))
            .corner_radius(CornerRadius::same(radius::PANEL))
            .shadow(shadow::MENU)
            .inner_margin(space::S8)
            .show(ui, |ui| {
                ui.set_width(CARD_WIDTH - 2.0 * space::S8);
                ui.label(
                    RichText::new("OPENING")
                        .font(fonts::named(fonts::STRONG, text::LABEL))
                        .extra_letter_spacing(text::LABEL_TRACKING)
                        .color(p.subtle),
                );
                ui.add_space(space::S2);
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .font(fonts::named(fonts::HEADING, text::TITLE + 4.0))
                            .extra_letter_spacing(text::DISPLAY_TRACKING / 2.0)
                            .color(p.text),
                    )
                    .truncate(),
                );
                ui.add_space(space::S5);
                let step = step.min(STEPS.len() - 1);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(STEPS[step]).size(text::BODY).color(p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} of {}", step + 1, STEPS.len()))
                                .size(text::SMALL)
                                .color(p.subtle),
                        );
                    });
                });
                ui.add_space(space::S4);
                // The 2 px accent bar, filled to the steps done.
                let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(1), p.border);
                let done = (step + 1) as f32 / STEPS.len() as f32;
                let filled = Rect::from_min_size(bar.min, vec2(bar.width() * done, bar.height()));
                ui.painter().rect_filled(filled, CornerRadius::same(1), colors.accent);
            });
    });
    // Polls the worker until it is done.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
}

/// The top bar: the mark, the File and View menus, the project's name, and the gear (raised while the Settings
/// window is open, `settings_open`). Returns what was asked for.
pub(crate) fn top_bar(
    ui: &mut Ui,
    mark: Option<&egui::TextureHandle>,
    editor: &EditorState,
    dock: &Dock,
    keymap: &Keymap,
    settings_open: bool,
) -> Option<MenuAction> {
    let p = current_colors(ui.ctx()).palette;
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, p.header);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - size::BORDER / 2.0,
        Stroke::new(size::BORDER, p.border),
    );
    let mut action = None;
    let inner = rect.shrink2(vec2(space::S5, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if let Some(mark) = mark {
                ui.add(egui::Image::new((mark.id(), egui::Vec2::splat(20.0))));
                ui.add_space(space::S4);
            }
            // A menu bar takes all the width it is given; it gets the menus' only, so the rest of the bar follows it.
            ui.allocate_ui_with_layout(
                vec2(MENUS_WIDTH, BAR_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.menu_button("File", |ui| {
                            ui.set_min_width(MENU_WIDTH);
                            if menu_item(ui, "Save", keymap.shortcut_text(ids::FILE_SAVE), None).clicked() {
                                action = Some(MenuAction::Save);
                            }
                            ui.separator();
                            if menu_item(ui, "Close project", keymap.shortcut_text(ids::PROJECT_CLOSE), None).clicked()
                            {
                                action = Some(MenuAction::Close);
                            }
                        });
                        ui.menu_button("View", |ui| {
                            ui.set_min_width(MENU_WIDTH);
                            for panel in dock.panels() {
                                let open = dock.is_open(panel.id());
                                if menu_item(ui, panel.title(), None, Some(open)).clicked() {
                                    action = Some(MenuAction::Toggle(panel.id()));
                                }
                            }
                            ui.separator();
                            if menu_item(ui, "Reset layout", keymap.shortcut_text(ids::VIEW_RESET_LAYOUT), None)
                                .clicked()
                            {
                                action = Some(MenuAction::ResetLayout);
                            }
                        });
                    });
                },
            );
            ui.add_space(space::S5);
            ui.label(
                RichText::new(editor.name())
                    .font(fonts::named(fonts::STRONG, text::BODY))
                    .color(p.text),
            )
            .on_hover_text(editor.path().display().to_string());
            if editor.project.is_modified() {
                ui.label(RichText::new("Unsaved changes").size(text::SMALL).color(p.subtle));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let tip = match keymap.shortcut_text(ids::SETTINGS_OPEN) {
                    Some(keys) => format!("Settings ({keys})"),
                    None => "Settings".to_owned(),
                };
                if gear(ui, settings_open).on_hover_text(tip).clicked() {
                    action = Some(MenuAction::Settings);
                }
            });
        },
    );
    action
}

/// The gear that opens Settings; raised while Settings is open.
fn gear(ui: &mut Ui, open: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, open, "Settings"));
    let fill = if open || response.hovered() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        fonts::icon::GEAR,
        fonts::named(fonts::ICONS, text::BODY_LG + 2.0),
        if open || response.hovered() { p.text } else { p.muted },
    );
    response
}

/// A menu item: a check mark when `checked` is `Some(true)`, the label, and the shortcut on the right. A click closes
/// the menu.
fn menu_item(ui: &mut Ui, label: &str, shortcut: Option<String>, checked: Option<bool>) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let width = ui.available_width().max(MENU_WIDTH);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    response.widget_info(|| match checked {
        Some(on) => WidgetInfo::selected(Role::MenuItemCheckBox, true, on, label),
        None => WidgetInfo::labeled(Role::MenuItem, true, label),
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let mut x = rect.left() + space::S4;
        if checked.is_some() {
            if checked == Some(true) {
                ui.painter().text(
                    pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    panels::icon::CHECK,
                    fonts::named(fonts::ICONS, text::BODY),
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
            p.text,
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
