//! Insert Object (#352): a window of every class the registry can create, with its icon, in sections by category
//! (#500), and a search, as Roblox Studio's. A class's description shows when the pointer rests on it. The button at
//! the top right switches between a **list** (one column) and a **grid** (the sections flowing in columns across a
//! wide window, every class at once); Studio keeps the choice.
//!
//! It opens from an Explorer row's `+` or its right-click menu, for that row, or with its keybind (Shift+Space, #411)
//! for the selected instance: choosing a class creates one inside it, selected (one undo step), and closes the window.
//! Something with a place in the world goes where the editor camera looks, or at the pivot of what it goes in (#412).
//! Studio draws it over the editor ([`frame`]), so it shows whichever panels are open.

use egui::{Align2, CornerRadius, Id, Rect, Response, Role, ScrollArea, Sense, TextEdit, Ui, WidgetInfo, pos2, vec2};
use metallic_engine::metallic_core::{ClassId, Registry};

use super::{class_icon, matches_words};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts, window_header};

/// The window's title.
pub const TITLE: &str = "Insert Object";
/// A row's icon size.
const ICON: f32 = 16.0;
/// A row's height.
const ROW: f32 = 26.0;
/// A grid column's width.
const COLUMN: f32 = 230.0;

/// The classes the first section offers while nothing is searched, in this order (those the registry has).
const FREQUENT: [&str; 8] = [
    "Part",
    "MeshPart",
    "Model",
    "Folder",
    "Script",
    "LocalScript",
    "ModuleScript",
    "PointLight",
];

/// The registry's categories, in the order their sections come; any other follows by name, then the classes with
/// none.
const ORDER: [&str; 7] = ["Parts", "Containers", "Lights", "Scripts", "Events", "Values", "World"];

/// The section of classes with no category.
const OTHER: &str = "Other";

/// What the window is open for: the instance it inserts into, and the search.
#[derive(Clone, Debug, PartialEq)]
pub struct Inserting {
    /// Where the new instance goes.
    pub parent: metallic_engine::metallic_core::Id,
    /// The search.
    pub query: String,
    /// Where the window's top-left corner goes on its next frame: the pointer when it opens (#504), its corner when
    /// the view switches. `None` leaves it where it is.
    pub place: Option<Place>,
}

/// Where Insert Object's window goes on its next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// At the pointer (where it was clicked open), kept on screen.
    Pointer,
    /// With its top-left corner here.
    At(egui::Pos2),
}

/// One class the window offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassChoice {
    /// The class.
    pub class: ClassId,
    /// Its name.
    pub name: &'static str,
    /// What it is, in a line: the tooltip.
    pub description: &'static str,
    /// Its section (the registry's category; empty for none).
    pub category: &'static str,
}

/// One section of the window: a heading and its classes, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Its heading.
    pub name: &'static str,
    /// Its classes.
    pub choices: Vec<ClassChoice>,
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
            category: info.desc().category,
        })
        .filter(|choice| matches_words(&format!("{} {}", choice.name, choice.description), query.trim()))
        .collect();
    out.sort_by_key(|choice| choice.name);
    out
}

/// The window's sections for `query` (#500): Frequently Used while nothing is searched, then a section per category
/// in [`ORDER`], any others by name, and last the classes with none. Empty sections are left out.
pub fn sections(registry: &Registry, query: &str) -> Vec<Section> {
    let all = choices(registry, query);
    let mut out = Vec::new();
    if query.trim().is_empty() {
        let frequent: Vec<ClassChoice> = FREQUENT
            .iter()
            .filter_map(|name| all.iter().find(|choice| choice.name == *name).cloned())
            .collect();
        if !frequent.is_empty() {
            out.push(Section {
                name: "Frequently Used",
                choices: frequent,
            });
        }
    }
    let mut categories: Vec<&'static str> = all
        .iter()
        .map(|choice| choice.category)
        .filter(|category| !category.is_empty())
        .collect();
    categories.sort_by_key(|category| {
        (
            ORDER.iter().position(|known| known == category).unwrap_or(ORDER.len()),
            *category,
        )
    });
    categories.dedup();
    for category in categories {
        out.push(Section {
            name: category,
            choices: all
                .iter()
                .filter(|choice| choice.category == category)
                .cloned()
                .collect(),
        });
    }
    let other: Vec<ClassChoice> = all
        .iter()
        .filter(|choice| choice.category.is_empty())
        .cloned()
        .collect();
    if !other.is_empty() {
        out.push(Section {
            name: OTHER,
            choices: other,
        });
    }
    out
}

/// Draws the Insert Object window over `ctx` while `editor` has it open (#411: from the editor's frame, whichever
/// panels are open), and inserts the class chosen. `grid` is the view (#500), switched by the window's button.
pub fn frame(ctx: &egui::Context, editor: &mut crate::editor::EditorState, grid: &mut bool) {
    // Where it goes (#412): where the editor camera looks.
    let aim = crate::viewport::aim(&editor.dm, &editor.viewport.camera);
    let mut scene = editor.scene();
    let Some(mut inserting) = scene.explorer.take_inserting() else {
        return;
    };
    let registry = std::sync::Arc::clone(scene.dm.registry());
    let (chosen, open) = show(ctx, &registry, &mut inserting, grid);
    if let Some(class) = chosen
        && let Err(error) = scene.insert(class, inserting.parent, Some(aim))
    {
        log::warn!("insert: could not insert: {error}");
    }
    if open && scene.dm.contains(inserting.parent) {
        scene.explorer.keep_inserting(inserting);
    }
}

/// Draws the window over `ctx`, as a list or with `grid` a grid. Returns the class chosen this frame, and whether the
/// window stays open.
pub fn show(
    ctx: &egui::Context,
    registry: &Registry,
    inserting: &mut Inserting,
    grid: &mut bool,
) -> (Option<ClassId>, bool) {
    let mut open = true;
    let mut chosen = None;
    let mut switched = false;
    // Each view keeps its own size: a narrow list, a wide grid.
    let (id, size, min_width) = if *grid {
        ("metallic.explorer.insert.grid", [980.0, 600.0], 480.0)
    } else {
        ("metallic.explorer.insert", [320.0, 520.0], 260.0)
    };
    let mut window = egui::Window::new(TITLE)
        .id(Id::unique(id))
        .pivot(Align2::LEFT_TOP)
        .default_pos(ctx.content_rect().center() - vec2(size[0], size[1]) / 2.0)
        .constrain(true);
    // At the pointer when it opens (#504), not where it was last; where its corner was when the view switches.
    let place = match inserting.place.take() {
        Some(Place::Pointer) => ctx.pointer_latest_pos(),
        Some(Place::At(at)) => Some(at),
        None => None,
    };
    if let Some(at) = place {
        window = window.current_pos(at);
    }
    let shown = window
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size(size)
        .min_width(min_width)
        .min_height(220.0)
        .show(ctx, |ui| {
            window_header(ui, TITLE, &mut open);
            let p = current_colors(ui.ctx()).palette;
            let search = ui
                .horizontal(|ui| {
                    icons::show(ui, Icon::UiSearch, text::BODY_LG, p.subtle);
                    // What the toggle takes, with the gap the layout puts before it (#520): a field a pixel too wide
                    // pushes the toggle past the window's edge, and a resizable window grows to fit it, every frame.
                    let toggle_width = size::CONTROL + ui.spacing().item_spacing.x;
                    let field = ui.add(
                        TextEdit::singleline(&mut inserting.query)
                            .hint_text("Search classes")
                            .desired_width((ui.available_width() - toggle_width).max(80.0))
                            .margin(vec2(space::S4, space::S3)),
                    );
                    if view_toggle(ui, *grid).clicked() {
                        *grid = !*grid;
                        switched = true;
                    }
                    field
                })
                .inner;
            // Typing starts at once.
            if !search.has_focus() && ui.memory(|memory| memory.focused().is_none()) {
                search.request_focus();
            }
            ui.add_space(space::S3);
            let sections = sections(registry, &inserting.query);
            // Enter takes the first match.
            if search.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                chosen = sections
                    .iter()
                    .flat_map(|section| &section.choices)
                    .next()
                    .map(|choice| choice.class);
            }
            ScrollArea::vertical()
                .id_salt("explorer.insert.list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if sections.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "No class matches \u{201C}{}\u{201D}.",
                                inserting.query.trim()
                            ))
                            .color(p.muted),
                        );
                        return;
                    }
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let picked = if *grid {
                        grid_view(ui, registry, &sections)
                    } else {
                        list_view(ui, registry, &sections)
                    };
                    chosen = chosen.or(picked);
                });
        });
    // The other view's window opens where this one is.
    if switched && let Some(shown) = shown {
        inserting.place = Some(Place::At(shown.response.rect.min));
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    (chosen, open && chosen.is_none())
}

/// The button that switches the view (#500): the grid's icon in the list, the list's in the grid.
fn view_toggle(ui: &mut Ui, grid: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let side = size::CONTROL;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let label = if grid { "Show as a list" } else { "Show as a grid" };
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let ink = if response.hovered() { p.text } else { p.muted };
        let center = rect.center();
        if grid {
            // Three lines: the list.
            for dy in [-4.0, 0.0, 4.0] {
                ui.painter().line_segment(
                    [center + vec2(-6.0, dy), center + vec2(6.0, dy)],
                    egui::Stroke::new(1.5, ink),
                );
            }
        } else {
            // Four squares: the grid.
            for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                ui.painter().rect_filled(
                    Rect::from_center_size(center + vec2(dx, dy), vec2(5.0, 5.0)),
                    CornerRadius::same(1),
                    ink,
                );
            }
        }
    }
    response.on_hover_text(label)
}

/// The list: each section's heading, then its classes, one under the other. Returns the class clicked.
fn list_view(ui: &mut Ui, registry: &Registry, sections: &[Section]) -> Option<ClassId> {
    let mut chosen = None;
    for section in sections {
        heading(ui, section.name, ui.available_width());
        for choice in &section.choices {
            if row(ui, registry, choice, ui.available_width()).clicked() {
                chosen = Some(choice.class);
            }
        }
        ui.add_space(space::S3);
    }
    chosen
}

/// The grid: the sections' headings and classes in one sequence, flowing down columns as wide as [`COLUMN`] across
/// the window, every class at once (as the owner's reference). Returns the class clicked.
fn grid_view(ui: &mut Ui, registry: &Registry, sections: &[Section]) -> Option<ClassId> {
    enum Item<'a> {
        Heading(&'static str),
        Class(&'a ClassChoice),
    }
    let items: Vec<Item<'_>> = sections
        .iter()
        .flat_map(|section| std::iter::once(Item::Heading(section.name)).chain(section.choices.iter().map(Item::Class)))
        .collect();
    // Columns of a fixed width, as many as fit: widths taken from the window would grow a resizable window every
    // frame.
    let columns = ((ui.available_width() / COLUMN).floor() as usize).max(1);
    let per_column = items.len().div_ceil(columns);
    let column_width = COLUMN;
    let mut chosen = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for column in items.chunks(per_column.max(1)) {
            ui.vertical(|ui| {
                ui.set_width(column_width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for item in column {
                    match item {
                        Item::Heading(name) => heading(ui, name, column_width - space::S4),
                        Item::Class(choice) => {
                            if row(ui, registry, choice, column_width - space::S4).clicked() {
                                chosen = Some(choice.class);
                            }
                        }
                    }
                }
            });
        }
    });
    chosen
}

/// A section's heading: its name, muted, and a rule after it to the end of the row.
fn heading(ui: &mut Ui, name: &str, width: f32) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    if ui.is_rect_visible(rect) {
        let galley = ui
            .painter()
            .layout_no_wrap(name.to_owned(), egui::FontId::proportional(text::BODY), p.subtle);
        let x = rect.left() + space::S3;
        let right = x + galley.size().x + space::S4;
        ui.painter()
            .galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, p.subtle);
        if right < rect.right() - space::S3 {
            ui.painter().hline(
                right..=rect.right() - space::S3,
                rect.center().y,
                egui::Stroke::new(size::BORDER, p.border),
            );
        }
    }
}

/// One class: its icon and name, the whole row a button; its description on hover.
fn row(ui: &mut Ui, registry: &Registry, choice: &ClassChoice, width: f32) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Insert {}", choice.name)));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let icon = class_icon(registry, choice.class);
        let at = Rect::from_min_size(
            pos2(rect.left() + space::S3, rect.center().y - ICON / 2.0),
            vec2(ICON, ICON),
        );
        icons::paint(ui.painter(), icon, at, icon.color(&colors));
        let name = ui
            .painter()
            .layout_no_wrap(choice.name.to_owned(), fonts::named(fonts::STRONG, text::BODY), p.text);
        let x = at.right() + space::S3;
        ui.painter()
            .galley(pos2(x, rect.center().y - name.size().y / 2.0), name, p.text);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(choice.description)
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

    /// #500: sections by category, Frequently Used first while nothing is searched; every class in one category
    /// section exactly once.
    #[test]
    fn the_classes_come_in_sections_by_category() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let all = sections(&registry, "");
        let names: Vec<&str> = all.iter().map(|section| section.name).collect();
        assert_eq!(names.first(), Some(&"Frequently Used"), "{names:?}");
        for expected in ["Parts", "Containers", "Lights", "Scripts", "Values"] {
            assert!(names.contains(&expected), "{expected}: {names:?}");
        }
        // The categories in their order.
        let at = |name: &str| names.iter().position(|n| *n == name).unwrap();
        assert!(at("Parts") < at("Containers") && at("Containers") < at("Lights") && at("Lights") < at("Scripts"));
        let section = |name: &str| all.iter().find(|s| s.name == name).unwrap();
        let classes = |name: &str| -> Vec<&str> { section(name).choices.iter().map(|c| c.name).collect() };
        assert!(classes("Parts").contains(&"Part") && classes("Parts").contains(&"MeshPart"));
        assert!(classes("Lights").contains(&"SpotLight"));
        assert!(classes("Containers").contains(&"Folder") && classes("Containers").contains(&"Model"));
        // Past Frequently Used, each class once.
        let rest: Vec<&str> = all[1..].iter().flat_map(|s| s.choices.iter().map(|c| c.name)).collect();
        assert_eq!(rest.len(), choices(&registry, "").len(), "{rest:?}");
        // A search has no Frequently Used and only sections with a match.
        let found = sections(&registry, "light");
        assert!(
            found
                .iter()
                .all(|s| s.name != "Frequently Used" && !s.choices.is_empty())
        );
        assert!(found.iter().any(|s| s.name == "Lights"));
    }
}

#[cfg(test)]
mod view_tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// #500: both views draw every class by section, and the button switches between them.
    #[test]
    fn the_grid_and_the_list_show_every_class_and_switch() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let parent = metallic_engine::metallic_core::Id::new(1, 0);
        for start_grid in [false, true] {
            let mut state = (
                Inserting {
                    parent,
                    query: String::new(),
                    place: None,
                },
                start_grid,
            );
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1400.0, 1000.0))
                .with_context_setup(crate::theme::fonts::install)
                .build_ui_state(
                    |ui, (inserting, grid): &mut (Inserting, bool)| {
                        show(ui.ctx(), &registry, inserting, grid);
                    },
                    state.clone(),
                );
            for _ in 0..8 {
                harness.step();
            }
            let label = if start_grid { "Show as a list" } else { "Show as a grid" };
            harness.get_by_label(label);
            assert_eq!(
                harness.query_all_by_label("Insert Part").count(),
                2,
                "grid {start_grid}"
            );
            harness.get_by_label("Insert SpotLight");
            harness.get_by_label(label).click();
            for _ in 0..8 {
                harness.step();
            }
            assert_eq!(harness.state().1, !start_grid);
            harness.get_by_label("Insert SpotLight");
            state.1 = start_grid;
        }
    }

    /// #520: the window keeps its width while it is open, in both views (it grew a little every frame).
    #[test]
    fn the_window_keeps_its_width() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let parent = metallic_engine::metallic_core::Id::new(1, 0);
        for grid in [false, true] {
            let state = (
                Inserting {
                    parent,
                    query: String::new(),
                    place: None,
                },
                grid,
            );
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1400.0, 1000.0))
                .with_context_setup(crate::theme::fonts::install)
                .build_ui_state(
                    |ui, (inserting, grid): &mut (Inserting, bool)| {
                        show(ui.ctx(), &registry, inserting, grid);
                    },
                    state,
                );
            let width = |harness: &Harness<'_, (Inserting, bool)>| harness.get_by_role(Role::TextInput).rect().width();
            for _ in 0..4 {
                harness.step();
            }
            let first = width(&harness);
            for _ in 0..40 {
                harness.step();
            }
            assert_eq!(width(&harness), first, "grid {grid}");
        }
    }
}
