//! The Add window (#327): every element a toolbar can hold, in groups, each with its icon, name and what it does.
//! Right-click the toolbar and choose Add..., or press an empty tab's Add tool button (#328); choosing an element puts
//! it where the window was opened for, and the next one after it, so several can be added in a row.

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Response, Role, ScrollArea, Sense, TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use super::model::Element;
use super::tools::{Tool, ToolRegistry, groups};
use crate::panels::matches_words;
use crate::settings::window::section_label;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, space, text};
use crate::theme::{current_colors, fonts, window_header};

/// The window's title.
pub const TITLE: &str = "Add to the toolbar";
/// A row's icon size.
const ICON: f32 = 20.0;

/// One element the window offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// What adding it puts in the toolbar.
    pub element: Element,
    /// Its name.
    pub label: String,
    /// What it does, in a line.
    pub description: String,
    /// Its icon.
    pub icon: Icon,
    /// Whether its icon is drawn in its colour group's colour (a tool's) or the text's (a spacer's, a separator's).
    coloured: bool,
}

impl Choice {
    fn tool(tool: &Tool) -> Self {
        Self {
            element: Element::tool(&tool.id),
            label: tool.label.clone(),
            description: tool.description.clone(),
            icon: tool.icon,
            coloured: true,
        }
    }
}

/// A group of choices under its heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Its heading.
    pub title: String,
    /// Whether it is a plugin's.
    pub plugin: bool,
    /// Its choices, in the order they are registered.
    pub choices: Vec<Choice>,
}

/// Everything the window offers that `query` matches (by name and description; a blank query matches all): Studio's
/// tools by their groups in the order the groups first appear, then the layout's spacer and separator, then each
/// plugin's tools under the plugin's group. Sections with nothing left are left out.
pub fn catalogue(registry: &ToolRegistry, query: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut add = |title: &str, plugin: bool, choice: Choice| {
        let found = matches_words(&choice.label, query) || matches_words(&choice.description, query);
        if !found {
            return;
        }
        match sections
            .iter_mut()
            .find(|section| section.title == title && section.plugin == plugin)
        {
            Some(section) => section.choices.push(choice),
            None => sections.push(Section {
                title: title.to_owned(),
                plugin,
                choices: vec![choice],
            }),
        }
    };
    for tool in registry.tools().iter().filter(|tool| !tool.is_plugin()) {
        add(&tool.group, false, Choice::tool(tool));
    }
    for (element, label, description, icon) in [
        (
            Element::Spacer,
            "Spacer",
            "Empty space between tools.",
            Icon::ToolbarSpacer,
        ),
        (
            Element::Separator,
            "Separator",
            "A thin line between groups of tools.",
            Icon::ToolbarSeparator,
        ),
    ] {
        let choice = Choice {
            element,
            label: label.to_owned(),
            description: description.to_owned(),
            icon,
            coloured: false,
        };
        add(groups::LAYOUT, false, choice);
    }
    for tool in registry.tools().iter().filter(|tool| tool.is_plugin()) {
        add(&tool.group, true, Choice::tool(tool));
    }
    sections
}

/// The window's own state while it is open: the tab it adds to (by id), where, and the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Adding {
    /// The tab, by its id.
    pub(super) tab: u32,
    /// Where the next element goes in it.
    pub(super) at: usize,
    /// The search.
    pub(super) query: String,
}

/// Draws the window over `ctx`. Returns the element chosen this frame, if any, and whether the window stays open.
pub(super) fn show(ctx: &egui::Context, registry: &ToolRegistry, adding: &mut Adding) -> (Option<Element>, bool) {
    let mut open = true;
    let mut chosen = None;
    egui::Window::new(TITLE)
        .id(Id::unique("metallic.toolbar.add"))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size([420.0, 520.0])
        .min_width(340.0)
        .min_height(240.0)
        .show(ctx, |ui| {
            window_header(ui, TITLE, &mut open);
            let p = current_colors(ui.ctx()).palette;
            ui.horizontal(|ui| {
                icons::show(ui, Icon::UiSearch, text::BODY_LG, p.subtle);
                ui.add(
                    TextEdit::singleline(&mut adding.query)
                        .hint_text("Search tools")
                        .desired_width(f32::INFINITY)
                        .margin(vec2(space::S4, space::S3)),
                );
            });
            ui.add_space(space::S4);
            let sections = catalogue(registry, &adding.query);
            ScrollArea::vertical()
                .id_salt("toolbar.add.list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if sections.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("Nothing matches \u{201C}{}\u{201D}.", adding.query.trim()))
                                .color(p.muted),
                        );
                    }
                    for section in &sections {
                        let title = if section.plugin {
                            format!("{} (plugin)", section.title)
                        } else {
                            section.title.clone()
                        };
                        section_label(ui, &title);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for choice in &section.choices {
                            if row(ui, choice).clicked() {
                                chosen = Some(choice.element.clone());
                            }
                        }
                        ui.add_space(space::S5);
                    }
                    if adding.query.trim().is_empty() && !sections.iter().any(|section| section.plugin) {
                        section_label(ui, "Plugins");
                        ui.label(
                            egui::RichText::new("Tools your plugins add will be listed here.")
                                .size(text::SMALL)
                                .color(p.muted),
                        );
                    }
                });
        });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    (chosen, open)
}

/// One choice: its icon, its name over what it does, the whole row a button.
fn row(ui: &mut Ui, choice: &Choice) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let width = ui.available_width();
    let left = ICON + 2.0 * space::S4;
    let words = (width - left - space::S4).max(40.0);
    let name = ui
        .painter()
        .layout_no_wrap(choice.label.clone(), fonts::named(fonts::STRONG, text::BODY), p.text);
    let line = ui.painter().layout(
        choice.description.clone(),
        egui::FontId::proportional(text::SMALL),
        p.muted,
        words,
    );
    let height = space::S4 + name.size().y + space::S1 + line.size().y + space::S4;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Add {}", choice.label)));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let tint: Color32 = if choice.coloured {
            choice.icon.color(&colors)
        } else {
            p.text
        };
        let at = Rect::from_min_size(pos2(rect.left() + space::S4, rect.top() + space::S4), vec2(ICON, ICON));
        icons::paint(ui.painter(), choice.icon, at, tint);
        let x = rect.left() + left;
        let name_top = rect.top() + space::S4;
        let line_top = name_top + name.size().y + space::S1;
        ui.painter().galley(pos2(x, name_top), name, p.text);
        ui.painter().galley(pos2(x, line_top), line, p.muted);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::PANELS;

    #[test]
    fn every_element_is_listed_in_its_group() {
        let mut registry = ToolRegistry::studio(PANELS);
        registry
            .register(Tool::plugin(
                "terrain",
                "paint",
                "Paint terrain",
                "Paints terrain.",
                None,
            ))
            .unwrap();
        let sections = catalogue(&registry, "");
        let titles: Vec<(&str, bool)> = sections
            .iter()
            .map(|section| (section.title.as_str(), section.plugin))
            .collect();
        assert_eq!(
            titles,
            [
                (groups::PROJECT, false),
                (groups::GAME, false),
                (groups::STUDIO, false),
                (groups::PANELS, false),
                (groups::LAYOUT, false),
                ("terrain", true)
            ]
        );
        // Every tool once, each with its description; then the spacer and the separator.
        let listed: Vec<&Choice> = sections.iter().flat_map(|section| &section.choices).collect();
        assert_eq!(listed.len(), registry.tools().len() + 2);
        assert!(listed.iter().all(|choice| !choice.description.is_empty()));
        let game: Vec<&str> = sections[1].choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(game, ["Play", "Pause", "Step", "Stop"]);
        let layout: Vec<&Element> = sections[4].choices.iter().map(|choice| &choice.element).collect();
        assert_eq!(layout, [&Element::Spacer, &Element::Separator]);

        // The search keeps what matches by name or description, and drops empty groups.
        let found = catalogue(&registry, "frame");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].choices[0].label, "Step",
            "\"Runs one frame of the paused game.\""
        );
        assert!(catalogue(&registry, "teleporter").is_empty());
    }
}
