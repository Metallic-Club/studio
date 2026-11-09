//! The Tabs window (#329): right-click a toolbar tab for a small window listing every tab, with its name, whether it
//! is shown, whether it loads a layout of its own, buttons to move and delete it, a button for a new tab, and where
//! the tabs sit in their row (left, center or right).

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Role, Sense, TextEdit, Ui, WidgetInfo, vec2};

use super::model::{TabAlign, ToolbarConfig};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, space, text};
use crate::theme::{ButtonKind, button, checkbox_only, current_colors, window_header};

/// The window's title.
pub const TITLE: &str = "Toolbar tabs";
/// The name column's width.
const NAME_WIDTH: f32 = 180.0;
/// A small icon button's side.
const BUTTON: f32 = 24.0;

/// What a row's buttons asked for, done once the list is drawn.
enum Edit {
    Move(usize, usize),
    Delete(usize),
    New,
}

/// Draws the window over `ctx`, changing `config` as the user does. Returns whether it stays open.
pub(super) fn show(ctx: &egui::Context, config: &mut ToolbarConfig) -> bool {
    let mut open = true;
    let mut edit = None;
    egui::Window::new(TITLE)
        .id(Id::unique("metallic.toolbar.tabs"))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            window_header(ui, TITLE, &mut open);
            let p = current_colors(ui.ctx()).palette;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Tabs on the").color(p.muted));
                let shown = egui::ComboBox::from_id_salt("toolbar.tabs.align")
                    .icon(crate::theme::combo_icon)
                    .selected_text(config.align.label())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for align in TabAlign::ALL {
                            ui.selectable_value(&mut config.align, align, align.label());
                        }
                    });
                // Named for screen readers (#464), as the words round it say.
                let response = &shown.response;
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Tab alignment"));
                ui.label(egui::RichText::new("of the row").color(p.muted));
            });
            ui.add_space(space::S4);
            let count = config.tabs.len();
            let shown = config.tabs.iter().filter(|tab| tab.shown).count();
            egui::Grid::new("toolbar.tabs.list")
                .num_columns(5)
                .spacing(vec2(space::S4, space::S3))
                .show(ui, |ui| {
                    let heading = |ui: &mut Ui, label: &str| {
                        ui.label(egui::RichText::new(label).size(text::SMALL).color(p.subtle));
                    };
                    heading(ui, "");
                    heading(ui, "Name");
                    heading(ui, "Show");
                    heading(ui, "Own layout");
                    heading(ui, "");
                    ui.end_row();
                    for index in 0..count {
                        let tab = &mut config.tabs[index];
                        let id = tab.id;
                        ui.push_id(id, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = space::S1;
                                let name = tab.name.clone();
                                if icon_button(ui, Icon::UiMoveUp, &format!("Move {name} up"), index > 0).clicked() {
                                    edit = Some(Edit::Move(index, index - 1));
                                }
                                if icon_button(ui, Icon::UiMoveDown, &format!("Move {name} down"), index + 1 < count)
                                    .clicked()
                                {
                                    edit = Some(Edit::Move(index, index + 1));
                                }
                            });
                        });
                        let name = tab.name.clone();
                        let field = ui.add_sized(
                            vec2(NAME_WIDTH, BUTTON + space::S2),
                            TextEdit::singleline(&mut tab.name)
                                .id_salt(("toolbar.tabs.name", id))
                                .margin(vec2(space::S4, space::S2)),
                        );
                        if field.lost_focus() && tab.name.trim().is_empty() {
                            tab.name = name.trim().to_owned();
                            if tab.name.is_empty() {
                                "Tab".clone_into(&mut tab.name);
                            }
                        }
                        // The last shown tab stays shown: the row is never empty.
                        let can_hide = !tab.shown || shown > 1;
                        ui.add_enabled_ui(can_hide, |ui| {
                            checkbox_only(ui, &mut tab.shown, &format!("Show {name}"));
                        });
                        checkbox_only(ui, &mut tab.own_layout, &format!("Own layout for {name}"));
                        if icon_button(ui, Icon::UiDelete, &format!("Delete {name}"), count > 1).clicked() {
                            edit = Some(Edit::Delete(index));
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(space::S4);
            ui.horizontal(|ui| {
                if button(ui, ButtonKind::Secondary, "New tab").clicked() {
                    edit = Some(Edit::New);
                }
                ui.label(
                    egui::RichText::new("Double-click a tab to rename it where it is.")
                        .size(text::SMALL)
                        .color(p.muted),
                );
            });
        });
    match edit {
        Some(Edit::Move(from, to)) => config.move_tab(from, to),
        Some(Edit::Delete(index)) => {
            config.remove_tab(index);
        }
        Some(Edit::New) => {
            config.add_tab("New tab");
        }
        None => {}
    }
    // Keeps a shown tab in front, whatever was hidden.
    config.show_a_tab();
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    open
}

/// A small square button holding `icon`, labelled `label` for screen readers and its tooltip; greyed out and inert
/// unless `enabled`.
fn icon_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let ink: Color32 = if !enabled {
            p.text.gamma_multiply(0.3)
        } else if response.hovered() {
            p.text
        } else {
            p.muted
        };
        icons::paint(
            ui.painter(),
            icon,
            Rect::from_center_size(rect.center(), vec2(text::BODY, text::BODY)),
            ink,
        );
    }
    response.on_hover_text(label)
}
