//! Insert Object (#352): a small window listing every class the registry can create, with its icon and what it is,
//! and a search, as Roblox Studio's. It opens from an Explorer row's `+` or its right-click menu, for that row:
//! choosing a class creates one inside it, selected (one undo step), and closes the window. A part is placed in front
//! of the editor camera.

use egui::{Align2, CornerRadius, Id, Rect, Response, Role, ScrollArea, Sense, TextEdit, Ui, WidgetInfo, pos2, vec2};
use metallic_engine::metallic_core::{ClassId, Registry};

use super::{class_icon, matches_words};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, space, text};
use crate::theme::{current_colors, fonts, window_header};

/// The window's title.
pub const TITLE: &str = "Insert Object";
/// A row's icon size.
const ICON: f32 = 18.0;

/// What the window is open for: the instance it inserts into, and the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inserting {
    /// Where the new instance goes.
    pub parent: metallic_engine::metallic_core::Id,
    /// The search.
    pub query: String,
}

/// One class the window offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassChoice {
    /// The class.
    pub class: ClassId,
    /// Its name.
    pub name: &'static str,
    /// What it is, in a line.
    pub description: &'static str,
}

/// Every class Studio can insert, by name, that matches `query` (every word, in its name or description): creatable,
/// not a service, and shown in the Explorer.
pub fn choices(registry: &Registry, query: &str) -> Vec<ClassChoice> {
    let mut out: Vec<ClassChoice> = registry
        .classes()
        .iter()
        .filter(|info| info.creatable() && info.service_order().is_none() && !info.hidden_in_explorer())
        .map(|info| ClassChoice {
            class: info.id(),
            name: info.name(),
            description: info.desc().description,
        })
        .filter(|choice| matches_words(&format!("{} {}", choice.name, choice.description), query.trim()))
        .collect();
    out.sort_by_key(|choice| choice.name);
    out
}

/// Draws the window over `ctx`. Returns the class chosen this frame, and whether the window stays open.
pub fn show(ctx: &egui::Context, registry: &Registry, inserting: &mut Inserting) -> (Option<ClassId>, bool) {
    let mut open = true;
    let mut chosen = None;
    egui::Window::new(TITLE)
        .id(Id::unique("metallic.explorer.insert"))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size([380.0, 480.0])
        .min_width(300.0)
        .min_height(220.0)
        .show(ctx, |ui| {
            window_header(ui, TITLE, &mut open);
            let p = current_colors(ui.ctx()).palette;
            let search = ui
                .horizontal(|ui| {
                    icons::show(ui, Icon::UiSearch, text::BODY_LG, p.subtle);
                    ui.add(
                        TextEdit::singleline(&mut inserting.query)
                            .hint_text("Search classes")
                            .desired_width(f32::INFINITY)
                            .margin(vec2(space::S4, space::S3)),
                    )
                })
                .inner;
            // Typing starts at once.
            if !search.has_focus() && ui.memory(|memory| memory.focused().is_none()) {
                search.request_focus();
            }
            ui.add_space(space::S4);
            let list = choices(registry, &inserting.query);
            // Enter takes the first match.
            if search.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                chosen = list.first().map(|choice| choice.class);
            }
            ScrollArea::vertical()
                .id_salt("explorer.insert.list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if list.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "No class matches \u{201C}{}\u{201D}.",
                                inserting.query.trim()
                            ))
                            .color(p.muted),
                        );
                    }
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for choice in &list {
                        if row(ui, registry, choice).clicked() {
                            chosen = Some(choice.class);
                        }
                    }
                });
        });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    (chosen, open && chosen.is_none())
}

/// One class: its icon, its name over what it is, the whole row a button.
fn row(ui: &mut Ui, registry: &Registry, choice: &ClassChoice) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let width = ui.available_width();
    let left = ICON + 2.0 * space::S4;
    let words = (width - left - space::S4).max(40.0);
    let name = ui
        .painter()
        .layout_no_wrap(choice.name.to_owned(), fonts::named(fonts::STRONG, text::BODY), p.text);
    let line = ui.painter().layout(
        choice.description.to_owned(),
        egui::FontId::proportional(text::SMALL),
        p.muted,
        words,
    );
    let height = space::S3 + name.size().y + space::S1 + line.size().y + space::S3;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Insert {}", choice.name)));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let icon = class_icon(registry, choice.class);
        let at = Rect::from_min_size(pos2(rect.left() + space::S4, rect.top() + space::S3), vec2(ICON, ICON));
        icons::paint(ui.painter(), icon, at, icon.color(&colors));
        let x = rect.left() + left;
        let name_top = rect.top() + space::S3;
        ui.painter().galley(pos2(x, name_top), name, p.text);
        ui.painter()
            .galley(pos2(x, name_top + space::S1 + text::BODY + 2.0), line, p.muted);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_what_can_be_created_by_name_and_searches_words() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let all = choices(&registry, "");
        let names: Vec<&str> = all.iter().map(|choice| choice.name).collect();
        for name in ["Part", "Folder", "Model", "Script"] {
            assert!(names.contains(&name), "{name}: {names:?}");
        }
        for name in ["Workspace", "Lighting", "Instance", "BasePart"] {
            assert!(!names.contains(&name), "{name} cannot be inserted");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert!(
            all.iter().all(|choice| !choice.description.is_empty()),
            "every class says what it is"
        );
        let found: Vec<&str> = choices(&registry, "part").iter().map(|choice| choice.name).collect();
        assert!(found.contains(&"Part") && !found.contains(&"Folder"), "{found:?}");
        assert!(choices(&registry, "teleporter").is_empty());
    }
}
