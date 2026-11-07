//! The projects page (ADR 0009, Screens), Roblox-launcher-like in the website's style: a rail (Recent, Templates)
//! and a grid of cards with a blank thumbnail, the name, when it was last opened and a warning when the file is
//! missing.
//!
//! - **New** makes a `.mtproj` from a template in `templates/` ([`templates`]) with `Project::create` (#187), adds it
//!   to the list and opens it.
//! - **Add** picks an existing `.mtproj` with the system's file dialog ([`dialog`]), adds it and opens it.
//! - **Open** is a double-click, or Enter (the `projects.open` command) on the selected card. Right-click, Remove
//!   takes a project off the list and leaves its file alone.
//!
//! The list is `projects.toml` in the config folder ([`list`]). Opening hands the project's path to the app, which
//! moves to the Editor screen.

pub mod dialog;
pub mod list;
pub mod templates;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Label, Layout, Modal, Rect, RichText, Role, ScrollArea,
    Sense, Stroke, StrokeKind, TextEdit, TextureHandle, Ui, UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};
use metallic_engine::metallic_project::{PROJECT_EXTENSION, Project, Template};

use crate::keymap::{Keymap, ids};
use crate::settings::window::category_item;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};
pub use dialog::{FileDialog, NoDialog};
pub use list::{PROJECTS_FILE, ProjectEntry, ProjectList, name_from_path, projects_path};
pub use templates::{TemplateInfo, list_templates, templates_dir};

/// The rail's width: the website's sidebar.
const RAIL_WIDTH: f32 = 208.0;
/// The mark's size at the top of the rail.
pub const RAIL_MARK: f32 = 28.0;
/// The narrowest a card may be; the grid fits as many as it can and widens them to fill the row.
const CARD_MIN_WIDTH: f32 = 200.0;
/// A thumbnail's height to its width.
const THUMBNAIL_ASPECT: f32 = 9.0 / 16.0;
/// The space between rows of cards: the website's game grid.
const ROW_GAP: f32 = 26.0;
/// The space under a thumbnail for the name and the line under it.
const CARD_TEXT: f32 = 46.0;
/// The New project dialog's width.
const DIALOG_WIDTH: f32 = 480.0;

/// Which list the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    /// The projects, most recently opened first.
    #[default]
    Recent,
    /// The templates a new project can start from.
    Templates,
}

/// The New project dialog's fields.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NewProject {
    /// The index of the template chosen.
    template: usize,
    name: String,
    folder: String,
    /// Why the last Create failed.
    error: Option<String>,
}

/// What a card asked for this frame.
enum CardAction {
    Select(PathBuf),
    Open(PathBuf),
    Remove(PathBuf),
    NewFrom(usize),
}

/// The projects page: the list, the templates, the file dialog, and what the page shows.
pub struct Projects {
    list: ProjectList,
    templates: Vec<TemplateInfo>,
    dialog: Box<dyn FileDialog>,
    view: View,
    selected: Option<PathBuf>,
    new_project: Option<NewProject>,
    /// What went wrong last (a missing file, a file that is not a project), until dismissed.
    error: Option<String>,
}

impl std::fmt::Debug for Projects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projects")
            .field("list", &self.list)
            .field("templates", &self.templates)
            .field("view", &self.view)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl Default for Projects {
    fn default() -> Self {
        Self::new()
    }
}

impl Projects {
    /// An empty page: a list in memory, no templates, and a file dialog that always cancels.
    pub fn new() -> Self {
        Self {
            list: ProjectList::in_memory(),
            templates: Vec::new(),
            dialog: Box::new(NoDialog),
            view: View::default(),
            selected: None,
            new_project: None,
            error: None,
        }
    }

    /// Shows `list` (the one startup read).
    pub fn set_list(&mut self, list: ProjectList) {
        self.list = list;
    }

    /// The projects list.
    pub fn list(&self) -> &ProjectList {
        &self.list
    }

    /// The projects list, to change.
    pub fn list_mut(&mut self) -> &mut ProjectList {
        &mut self.list
    }

    /// Offers `templates` for new projects.
    pub fn set_templates(&mut self, templates: Vec<TemplateInfo>) {
        self.templates = templates;
    }

    /// The templates offered.
    pub fn templates(&self) -> &[TemplateInfo] {
        &self.templates
    }

    /// Asks `dialog` for files and folders from now on.
    pub fn set_file_dialog(&mut self, dialog: Box<dyn FileDialog>) {
        self.dialog = dialog;
    }

    /// The list the page shows.
    pub fn view(&self) -> View {
        self.view
    }

    /// Shows `view`.
    pub fn show_view(&mut self, view: View) {
        self.view = view;
    }

    /// The selected project.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    /// Selects the project at `path`.
    pub fn select(&mut self, path: impl Into<PathBuf>) {
        self.selected = Some(path.into());
    }

    /// What went wrong last, until dismissed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Tells the user `error` at the top of the page until it is dismissed (a project that could not be opened).
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    /// Whether the New project dialog is open: the page's own commands wait until it closes.
    pub fn is_creating(&self) -> bool {
        self.new_project.is_some()
    }

    /// Looks at every project's file again, to mark the missing ones. Called whenever the page is shown.
    pub fn refresh(&mut self) {
        self.list.refresh_missing();
    }

    /// Runs the page's command `id`. Returns the project to open, if it opens one.
    pub fn run(&mut self, id: &str) -> Option<PathBuf> {
        match id {
            ids::PROJECTS_OPEN => {
                let selected = self.selected.clone()?;
                self.open(&selected)
            }
            ids::PROJECTS_NEW => {
                self.new_project(None);
                None
            }
            ids::PROJECTS_ADD => self.add(),
            _ => None,
        }
    }

    /// Opens the project at `path`: it moves to the front of the list with the time. Returns its path, or `None`
    /// (and says why on the page) when its file is missing.
    pub fn open(&mut self, path: &Path) -> Option<PathBuf> {
        self.selected = Some(path.to_path_buf());
        let name = self
            .list
            .get(path)
            .map_or_else(|| name_from_path(path), |entry| entry.name.clone());
        if !path.is_file() {
            self.list.set_missing(path, true);
            self.error = Some(format!(
                "\u{201C}{name}\u{201D} is not at {} any more: it was moved, renamed or deleted. Add it again from its \
                 new place, or remove it from the list.",
                path.display()
            ));
            return None;
        }
        self.list.opened(path, &name, SystemTime::now());
        self.list.save_or_log();
        self.error = None;
        Some(path.to_path_buf())
    }

    /// Asks the file dialog for an existing `.mtproj`, adds it to the list and opens it. Returns its path, or `None`
    /// when the dialog was cancelled or the file is not a project (the page says why).
    pub fn add(&mut self) -> Option<PathBuf> {
        let picked = self.dialog.pick_project()?;
        let path = std::path::absolute(&picked).unwrap_or(picked);
        match Project::open(&path) {
            Ok(project) => {
                let name = match project.manifest().name.trim() {
                    "" => name_from_path(&path),
                    name => name.to_owned(),
                };
                self.list.opened(&path, &name, SystemTime::now());
                self.list.save_or_log();
                self.selected = Some(path.clone());
                self.view = View::Recent;
                self.error = None;
                Some(path)
            }
            Err(error) => {
                self.error = Some(format!("Could not add that project. {error}."));
                None
            }
        }
    }

    /// Takes the project at `path` off the list. Its file is left alone.
    pub fn remove(&mut self, path: &Path) {
        if self.list.remove(path) {
            self.list.save_or_log();
        }
        if self.selected.as_deref() == Some(path) {
            self.selected = None;
        }
        self.error = None;
    }

    /// Opens the New project dialog on the template at `template` (the first when `None`), with a name no project
    /// in the default folder has yet.
    pub fn new_project(&mut self, template: Option<usize>) {
        let folder = self.default_folder();
        let mut name = "My Game".to_owned();
        for n in 2..100 {
            if !folder.join(format!("{name}.{PROJECT_EXTENSION}")).exists() {
                break;
            }
            name = format!("My Game {n}");
        }
        self.new_project = Some(NewProject {
            template: template.unwrap_or(0).min(self.templates.len().saturating_sub(1)),
            name,
            folder: folder.display().to_string(),
            error: None,
        });
    }

    /// Sets the New project dialog's name and folder, when it is open.
    pub fn set_new_project(&mut self, name: &str, folder: &Path) {
        if let Some(new) = self.new_project.as_mut() {
            name.clone_into(&mut new.name);
            new.folder = folder.display().to_string();
        }
    }

    /// Where a new project goes unless the user says otherwise: the folder of the latest project, else
    /// `Documents/Metallic` in the user's home.
    fn default_folder(&self) -> PathBuf {
        if let Some(folder) = self
            .list
            .entries()
            .iter()
            .filter(|entry| !entry.missing)
            .find_map(|entry| entry.path.parent().filter(|dir| dir.is_dir()))
        {
            return folder.to_path_buf();
        }
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let documents = home.join("Documents");
        if documents.is_dir() { documents } else { home }.join("Metallic")
    }

    /// Makes the project the New project dialog describes, adds it to the list and closes the dialog. Returns its
    /// path, or `None` with the reason shown in the dialog.
    fn create(&mut self) -> Option<PathBuf> {
        let new = self.new_project.as_mut()?;
        let name = new.name.trim().to_owned();
        let made = (|| -> Result<PathBuf, String> {
            let template = self.templates.get(new.template).ok_or("Pick a template.")?;
            let file = file_name_for(&name).ok_or("Give the project a name.")?;
            let folder = new.folder.trim();
            if folder.is_empty() {
                return Err("Pick a folder for the project.".into());
            }
            let folder = PathBuf::from(folder);
            std::fs::create_dir_all(&folder)
                .map_err(|error| format!("Could not make the folder {}: {error}.", folder.display()))?;
            let path = folder.join(format!("{file}.{PROJECT_EXTENSION}"));
            let path = std::path::absolute(&path).unwrap_or(path);
            let template =
                Template::from_dir(&template.dir).map_err(|error| format!("Could not read the template: {error}."))?;
            Project::create(&path, &template, &name).map_err(|error| format!("{error}."))?;
            Ok(path)
        })();
        match made {
            Ok(path) => {
                self.new_project = None;
                self.list.opened(&path, &name, SystemTime::now());
                self.list.save_or_log();
                self.selected = Some(path.clone());
                self.view = View::Recent;
                self.error = None;
                Some(path)
            }
            Err(error) => {
                new.error = Some(error);
                None
            }
        }
    }

    /// Draws the page filling `ui`, with `mark` at the top of the rail and `keymap`'s shortcuts in the tooltips.
    /// Returns the project to open, if one was opened.
    pub fn show(&mut self, ui: &mut Ui, mark: Option<&TextureHandle>, keymap: &Keymap) -> Option<PathBuf> {
        let p = current_colors(ui.ctx()).palette;
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, p.bg);
        let rail = Rect::from_min_max(full.min, pos2(full.left() + RAIL_WIDTH, full.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(rail), |ui| self.rail(ui, mark));
        let content = Rect::from_min_max(
            pos2(rail.right() + space::S9, full.top() + space::S8),
            pos2(full.right() - space::S9, full.bottom()),
        );
        let mut open = ui
            .scope_builder(
                UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)),
                |ui| self.content(ui, keymap),
            )
            .inner;
        if self.new_project.is_some()
            && let Some(path) = self.new_project_dialog(ui.ctx())
        {
            open = Some(path);
        }
        open
    }

    /// The rail: the mark and wordmark, then Recent and Templates, and the offline line at the bottom.
    fn rail(&mut self, ui: &mut Ui, mark: Option<&TextureHandle>) {
        let p = current_colors(ui.ctx()).palette;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, p.panel);
        ui.painter().vline(
            rect.right() - size::BORDER / 2.0,
            rect.y_range(),
            Stroke::new(size::BORDER, p.border),
        );
        let inner = rect.shrink2(vec2(space::S5, space::GAP));
        ui.scope_builder(
            UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(space::S2);
                    if let Some(mark) = mark {
                        ui.add(egui::Image::new((mark.id(), Vec2::splat(RAIL_MARK))));
                    }
                    ui.add_space(space::S1);
                    ui.add(
                        Label::new(
                            RichText::new("Metallic")
                                .font(fonts::named(fonts::HEADING, text::BODY_LG))
                                .color(p.text),
                        )
                        .selectable(false),
                    );
                    ui.add(
                        Label::new(
                            RichText::new("Studio")
                                .font(fonts::named(fonts::STRONG, text::BODY_LG))
                                .color(p.muted),
                        )
                        .selectable(false),
                    );
                });
                ui.add_space(space::S8);
                ui.add_space(-space::S2);
                ui.horizontal(|ui| {
                    ui.add_space(space::S5);
                    ui.add(Label::new(section_label("Projects", p.subtle)).selectable(false));
                });
                for (view, label) in [(View::Recent, "Recent"), (View::Templates, "Templates")] {
                    if category_item(ui, label, self.view == view).clicked() {
                        self.view = view;
                    }
                }
            },
        );
        ui.painter().text(
            pos2(inner.left() + space::S5, inner.bottom()),
            Align2::LEFT_BOTTOM,
            "Working offline",
            FontId::proportional(text::SMALL),
            p.subtle,
        );
    }

    /// The header, the last error, and the grid of the view.
    fn content(&mut self, ui: &mut Ui, keymap: &Keymap) -> Option<PathBuf> {
        let p = current_colors(ui.ctx()).palette;
        let (title, line) = match self.view {
            View::Recent => ("Recent", "Your projects, the most recently opened first."),
            View::Templates => ("Templates", "Start a new project from one of these."),
        };
        let tip = |label: &str, id: &str| match keymap.shortcut_text(id) {
            Some(keys) => format!("{label} ({keys})"),
            None => label.to_owned(),
        };
        let mut actions = Vec::new();
        let mut open = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(title)
                        .font(fonts::named(fonts::HEADING, text::DISPLAY))
                        .extra_letter_spacing(text::DISPLAY_TRACKING)
                        .color(p.text),
                );
                ui.label(RichText::new(line).size(text::BODY).color(p.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Clear of the settings gear in the window's corner.
                ui.add_space(size::CONTROL + space::S5 + space::S4 - space::S9);
                if button(ui, ButtonKind::Primary, "New project")
                    .on_hover_text(tip("Make a new project from a template", ids::PROJECTS_NEW))
                    .clicked()
                {
                    self.new_project(None);
                }
                if button(ui, ButtonKind::Secondary, "Add project")
                    .on_hover_text(tip("Add a project file you already have", ids::PROJECTS_ADD))
                    .clicked()
                {
                    open = self.add();
                }
            });
        });
        ui.add_space(space::S8);
        if let Some(error) = self.error.clone() {
            if error_banner(ui, &error) {
                self.error = None;
            }
            ui.add_space(space::S6);
        }
        match self.view {
            View::Recent => self.recent_grid(ui, &mut actions),
            View::Templates => self.template_grid(ui, &mut actions),
        }
        for action in actions {
            match action {
                CardAction::Select(path) => self.selected = Some(path),
                CardAction::Open(path) => open = self.open(&path).or(open),
                CardAction::Remove(path) => self.remove(&path),
                CardAction::NewFrom(index) => self.new_project(Some(index)),
            }
        }
        open
    }

    fn recent_grid(&self, ui: &mut Ui, actions: &mut Vec<CardAction>) {
        let p = current_colors(ui.ctx()).palette;
        let entries = self.list.entries();
        if entries.is_empty() {
            ui.add_space(space::S10);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No projects yet")
                        .font(fonts::named(fonts::HEADING, text::TITLE))
                        .color(p.text),
                );
                ui.label(
                    RichText::new("Make a new project from a template, or add a project file you already have.")
                        .size(text::BODY)
                        .color(p.muted),
                );
            });
            return;
        }
        let now = SystemTime::now();
        grid(ui, entries.len(), |ui, index, rect| {
            let entry = &entries[index];
            let selected = self.selected.as_deref() == Some(entry.path.as_path());
            let response = card(
                ui,
                rect,
                Id::unique(("metallic.projects.card", &entry.path)),
                &entry.name,
                &opened_text(entry, now),
                selected,
            );
            if entry.missing {
                missing_badge(ui, rect, &entry.path);
            }
            if response.double_clicked() {
                actions.push(CardAction::Open(entry.path.clone()));
            } else if response.clicked() {
                actions.push(CardAction::Select(entry.path.clone()));
            }
            response.context_menu(|ui| {
                if ui.button("Open").clicked() {
                    actions.push(CardAction::Open(entry.path.clone()));
                    ui.close();
                }
                if ui.button("Remove from list").clicked() {
                    actions.push(CardAction::Remove(entry.path.clone()));
                    ui.close();
                }
            });
            response.on_hover_text(entry.path.display().to_string());
        });
    }

    fn template_grid(&self, ui: &mut Ui, actions: &mut Vec<CardAction>) {
        let p = current_colors(ui.ctx()).palette;
        if self.templates.is_empty() {
            ui.label(RichText::new("No templates were found next to Studio.").color(p.muted));
            return;
        }
        grid(ui, self.templates.len(), |ui, index, rect| {
            let template = &self.templates[index];
            let kind = format!("{} template", template.kind.to_uppercase());
            let response = card(
                ui,
                rect,
                Id::unique(("metallic.projects.template", &template.id)),
                &template.name,
                &kind,
                false,
            );
            if response.clicked() {
                actions.push(CardAction::NewFrom(index));
            }
            response.on_hover_text(format!("New project from {}", template.name));
        });
    }

    /// The New project dialog, over the page. Returns the project made, if Create made one.
    fn new_project_dialog(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let colors = current_colors(ctx);
        let p = colors.palette;
        let templates = &self.templates;
        let new = self.new_project.as_mut()?;
        let (mut create, mut cancel, mut browse) = (false, false, false);
        let frame = Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(size::BORDER, p.border))
            .corner_radius(CornerRadius::same(radius::DIALOG))
            .shadow(shadow::MENU)
            .inner_margin(space::S8);
        Modal::new(Id::unique("metallic.projects.new"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(DIALOG_WIDTH);
                ui.label(
                    RichText::new("Create a project")
                        .font(fonts::named(fonts::HEADING, text::TITLE))
                        .color(p.text),
                );
                ui.add(
                    Label::new(
                        RichText::new("Studio makes one .mtproj file in the folder and opens it.")
                            .size(text::BODY)
                            .color(p.muted),
                    )
                    .wrap(),
                );
                ui.add_space(space::S5);
                ui.label(section_label("Template", p.subtle));
                if templates.is_empty() {
                    ui.label(RichText::new("No templates were found next to Studio.").color(p.muted));
                }
                for (index, template) in templates.iter().enumerate() {
                    ui.radio_value(&mut new.template, index, template.name.as_str());
                }
                ui.add_space(space::S4);
                ui.label(section_label("Name", p.subtle));
                ui.add(
                    TextEdit::singleline(&mut new.name)
                        .id(Id::unique("metallic.projects.new.name"))
                        .hint_text("My Game")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(space::S4);
                ui.label(section_label("Folder", p.subtle));
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        browse = button(ui, ButtonKind::Secondary, "Browse\u{2026}").clicked();
                        ui.add(
                            TextEdit::singleline(&mut new.folder)
                                .id(Id::unique("metallic.projects.new.folder"))
                                .hint_text("Folder")
                                .desired_width(ui.available_width()),
                        );
                    });
                });
                let target = match file_name_for(new.name.trim()) {
                    Some(file) => format!(
                        "Makes {}",
                        Path::new(new.folder.trim())
                            .join(format!("{file}.{PROJECT_EXTENSION}"))
                            .display()
                    ),
                    None => "Give the project a name.".to_owned(),
                };
                ui.add(Label::new(RichText::new(target).size(text::SMALL).color(p.subtle)).wrap());
                if let Some(error) = &new.error {
                    ui.add_space(space::S2);
                    ui.horizontal_top(|ui| {
                        icons::show(ui, Icon::UiWarning, text::BODY_LG, colors.accent);
                        ui.add(Label::new(RichText::new(error.as_str()).color(p.text)).wrap());
                    });
                }
                ui.add_space(space::S6);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    create = button(ui, ButtonKind::Primary, "Create").clicked();
                    cancel = button(ui, ButtonKind::Secondary, "Cancel").clicked();
                });
            });
        if browse {
            let start = PathBuf::from(new.folder.trim());
            let start = start.is_dir().then_some(start.as_path());
            if let Some(folder) = self.dialog.pick_folder(start) {
                new.folder = folder.display().to_string();
            }
        }
        if cancel {
            self.new_project = None;
            return None;
        }
        if create {
            return self.create();
        }
        None
    }
}

/// An uppercase section label in the website's style.
fn section_label(label: &str, color: Color32) -> RichText {
    RichText::new(label.to_uppercase())
        .font(fonts::named(fonts::STRONG, text::LABEL))
        .extra_letter_spacing(text::LABEL_TRACKING)
        .color(color)
}

/// Lays `count` cards out in rows as wide as `ui`, inside a scroll area, and draws each with `draw(ui, index, rect)`.
fn grid(ui: &mut Ui, count: usize, mut draw: impl FnMut(&mut Ui, usize, Rect)) {
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // Room for the selected card's outline, which is drawn outside the card.
        let outline = size::ACCENT_BAR + 2.0;
        let clip = ui.clip_rect();
        ui.set_clip_rect(clip.expand2(vec2(outline, 0.0)));
        ui.add_space(outline);
        let width = ui.available_width();
        let columns = (((width + space::GAP) / (CARD_MIN_WIDTH + space::GAP)).floor() as usize).max(1);
        let card_width = (width - space::GAP * (columns - 1) as f32) / columns as f32;
        let card_height = card_width * THUMBNAIL_ASPECT + CARD_TEXT;
        for row in 0..count.div_ceil(columns) {
            let (row_rect, _) = ui.allocate_exact_size(vec2(width, card_height), Sense::hover());
            for column in 0..columns {
                let index = row * columns + column;
                if index >= count {
                    break;
                }
                let left = row_rect.left() + column as f32 * (card_width + space::GAP);
                draw(
                    ui,
                    index,
                    Rect::from_min_size(pos2(left, row_rect.top()), vec2(card_width, card_height)),
                );
            }
            ui.add_space(ROW_GAP - ui.spacing().item_spacing.y);
        }
    });
}

/// A card in `rect`: a blank thumbnail (the accent's outline when `selected`), `name`, and `line` under it. Returns
/// its response, which clicks anywhere on it reach.
fn card(ui: &mut Ui, rect: Rect, id: Id, name: &str, line: &str, selected: bool) -> egui::Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let response = ui.interact(rect, id, Sense::click());
    let thumbnail = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * THUMBNAIL_ASPECT));
    let edge = if response.hovered() { p.subtle } else { p.border };
    ui.painter().rect(
        thumbnail,
        CornerRadius::same(radius::PANEL),
        p.surface,
        Stroke::new(size::BORDER, edge),
        StrokeKind::Inside,
    );
    if selected {
        ui.painter().rect_stroke(
            thumbnail.expand(size::ACCENT_BAR / 2.0 + 1.0),
            CornerRadius::same(radius::PANEL + 2),
            Stroke::new(size::ACCENT_BAR, colors.accent),
            StrokeKind::Middle,
        );
    }
    let text = Rect::from_min_max(pos2(rect.left(), thumbnail.bottom() + space::S4), rect.max);
    ui.scope_builder(
        UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = space::S1;
            ui.add(
                Label::new(
                    RichText::new(name)
                        .font(fonts::named(fonts::STRONG, text::BODY_LG))
                        .color(p.text),
                )
                .selectable(false)
                .truncate(),
            );
            ui.add(
                Label::new(RichText::new(line).size(text::SMALL).color(p.muted))
                    .selectable(false)
                    .truncate(),
            );
        },
    );
    response
}

/// The "File missing" badge in the top-left corner of the card in `rect`, for the project at `path`.
fn missing_badge(ui: &mut Ui, rect: Rect, path: &Path) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let painter = ui.painter();
    let icon = Vec2::splat(text::BODY);
    let label = painter.layout_no_wrap(
        "File missing".to_owned(),
        fonts::named(fonts::STRONG, text::SMALL),
        p.text,
    );
    let inner = vec2(icon.x + space::S2 + label.size().x, icon.y.max(label.size().y));
    let badge = Rect::from_min_size(
        rect.min + vec2(space::S4, space::S4),
        inner + vec2(2.0 * space::S3, 2.0 * space::S2),
    );
    painter.rect(
        badge,
        CornerRadius::same(radius::ITEM),
        p.bg,
        Stroke::new(size::BORDER, p.border),
        StrokeKind::Inside,
    );
    let left = badge.left() + space::S3;
    icons::paint_at(
        painter,
        pos2(left, badge.center().y),
        egui::Align2::LEFT_CENTER,
        Icon::UiWarning,
        text::BODY,
        colors.accent,
    );
    painter.galley(
        pos2(left + icon.x + space::S2, badge.center().y - label.size().y / 2.0),
        label,
        p.text,
    );
    let response = ui.interact(badge, Id::unique(("metallic.projects.missing", path)), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, "File missing"));
    response.on_hover_text(format!(
        "There is no project at {} any more. Add it again from its new place, or remove it from the list.",
        path.display()
    ));
}

/// The last error, in a bordered strip with Dismiss. Returns whether it was dismissed.
fn error_banner(ui: &mut Ui, error: &str) -> bool {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let mut dismissed = false;
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::PANEL))
        .inner_margin(space::S5)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                dismissed = button(ui, ButtonKind::Secondary, "Dismiss").clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    icons::show(ui, Icon::UiWarning, text::TITLE, colors.accent);
                    ui.add(Label::new(RichText::new(error).color(p.text)).wrap());
                });
            });
        });
    dismissed
}

/// "Opened 3 hours ago", as of `now`.
fn opened_text(entry: &ProjectEntry, now: SystemTime) -> String {
    let Some(at) = entry.last_opened_at() else {
        return "Never opened".to_owned();
    };
    let seconds = now.duration_since(at).map_or(0, |since| since.as_secs());
    let ago = |n: u64, unit: &str| {
        if n == 1 {
            format!("Opened 1 {unit} ago")
        } else {
            format!("Opened {n} {unit}s ago")
        }
    };
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const TWO_DAYS: u64 = 2 * DAY;
    match seconds {
        0..MINUTE => "Opened just now".to_owned(),
        MINUTE..HOUR => ago(seconds / MINUTE, "minute"),
        HOUR..DAY => ago(seconds / HOUR, "hour"),
        DAY..TWO_DAYS => "Opened yesterday".to_owned(),
        _ if seconds < 30 * DAY => ago(seconds / DAY, "day"),
        _ if seconds < 365 * DAY => ago(seconds / (30 * DAY), "month"),
        _ => ago(seconds / (365 * DAY), "year"),
    }
}

/// A file name for a project called `name`: the name without the characters file systems refuse, or `None` when
/// nothing is left.
pub fn file_name_for(name: &str) -> Option<String> {
    let file: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    let file = file.trim().trim_end_matches('.').trim_end();
    (!file.is_empty()).then(|| file.to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn file_names_leave_out_what_file_systems_refuse() {
        assert_eq!(file_name_for("My Game").as_deref(), Some("My Game"));
        assert_eq!(file_name_for("  a/b:c?  ").as_deref(), Some("abc"));
        assert_eq!(file_name_for("Ends with dots...").as_deref(), Some("Ends with dots"));
        assert_eq!(file_name_for("<>:\"/\\|?*"), None);
        assert_eq!(file_name_for("   "), None);
    }

    #[test]
    fn last_opened_reads_as_time_ago() {
        let now = SystemTime::now();
        let opened = |ago: u64| {
            let mut entry = ProjectEntry::new("a.mtproj", "A");
            let at = now - Duration::from_secs(ago);
            entry.last_opened = Some(at.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            opened_text(&entry, now)
        };
        assert_eq!(opened_text(&ProjectEntry::new("a.mtproj", "A"), now), "Never opened");
        assert_eq!(opened(5), "Opened just now");
        assert_eq!(opened(60), "Opened 1 minute ago");
        assert_eq!(opened(3 * 3600 + 5), "Opened 3 hours ago");
        assert_eq!(opened(30 * 3600), "Opened yesterday");
        assert_eq!(opened(5 * 86_400), "Opened 5 days ago");
        assert_eq!(opened(65 * 86_400), "Opened 2 months ago");
        assert_eq!(opened(800 * 86_400), "Opened 2 years ago");
    }
}
