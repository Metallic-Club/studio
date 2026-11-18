//! The Game Settings window (#782): the open game's own settings, in the Settings window's style (a list of pages on
//! the left, rows of a name, its description and its control on the right). Shift+Alt+S, or File > Game Settings,
//! opens and closes it.
//!
//! What it shows follows how the game ships (`mode` in `metallic.toml`):
//!
//! - **Offline** (standalone): the **Game** page (its name, the author, its version and the picture its executable's
//!   icon is made from) and a **Graphics** page: the renderer's settings the exported game starts with, kept in the
//!   project's [`GAME_GRAPHICS_FILE`] and shipped beside its executable. A settings screen a game draws itself, through
//!   an API, comes later; until then a player changes them in that file.
//! - **Online** (platform): the Metallic platform is not built yet, so the page says so, with the game's name and the
//!   switch back to offline.
//!
//! Every change is made at once, as an unsaved edit of the project that Save writes.

use egui::{Id, RichText, ScrollArea, TextEdit, Ui, vec2};
use metallic_engine::metallic_project::{GAME_GRAPHICS_FILE, Project, ProjectMode};
use metallic_engine::metallic_render::settings::GraphicsSettings;

use super::window::{CATEGORY_WIDTH, CONTROL_WIDTH, ENTRIES, category_item, row, segmented, setting_row};
use super::{Category, StudioSettings};
use crate::editor::EditorState;
use crate::panels::values;
use crate::theme::tokens::{space, text};
use crate::theme::{current_colors, fonts};

/// The window's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    /// The game: its name, author, version and icon.
    #[default]
    Game,
    /// The graphics an offline game starts with.
    Graphics,
}

impl Page {
    /// What the list calls it.
    fn label(self) -> &'static str {
        match self {
            Page::Game => "Game",
            Page::Graphics => "Graphics",
        }
    }

    /// Its lead line, under its title.
    fn lead(self) -> &'static str {
        match self {
            Page::Game => "What the game is called and who made it. An offline game's program shows them.",
            Page::Graphics => {
                "What the exported game draws with until its player changes it. For now these settings ship as a \
                 file beside the game; a settings screen of the game's own comes later."
            }
        }
    }
}

/// The Game Settings window.
#[derive(Debug, Default)]
pub struct GameSettingsWindow {
    open: bool,
    page: Page,
}

impl GameSettingsWindow {
    /// Whether it is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens it, or closes it when it is open.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// The page shown.
    pub fn page(&self) -> Page {
        self.page
    }

    /// Draws the window, if open, over `ctx`, changing `editor`'s project as the user does.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut EditorState) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Game Settings")
            .id(Id::unique("metallic.game_settings"))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(760.0, 520.0))
            .min_width(560.0)
            .min_height(360.0)
            .show(ctx, |ui| {
                crate::theme::window_header(ui, "Game Settings", &mut open);
                self.contents(ui, editor);
            });
        if !open {
            self.open = false;
        }
    }

    /// The list of pages and the page, filling `ui`.
    fn contents(&mut self, ui: &mut Ui, editor: &mut EditorState) {
        let offline = editor.project.manifest().mode == ProjectMode::Standalone;
        let pages: &[Page] = if offline {
            &[Page::Game, Page::Graphics]
        } else {
            &[Page::Game]
        };
        if !pages.contains(&self.page) {
            self.page = Page::Game;
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(CATEGORY_WIDTH);
                for &page in pages {
                    if category_item(ui, page.label(), page == self.page, false).clicked() {
                        self.page = page;
                    }
                }
            });
            ui.separator();
            ScrollArea::vertical()
                .id_salt("game_settings.page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        title(ui, self.page);
                        match self.page {
                            Page::Game => game_page(ui, editor),
                            Page::Graphics => graphics_page(ui, &mut editor.project),
                        }
                    });
                });
        });
    }
}

/// A page's title and lead line.
fn title(ui: &mut Ui, page: Page) {
    let p = current_colors(ui.ctx()).palette;
    ui.label(
        RichText::new(page.label())
            .font(fonts::named(fonts::HEADING, text::TITLE))
            .color(p.text),
    );
    ui.label(RichText::new(page.lead()).size(text::BODY).color(p.muted));
    ui.add_space(space::S5);
}

/// The Game page: how it ships, its name, and for an offline game its author, version and icon.
fn game_page(ui: &mut Ui, editor: &mut EditorState) {
    let files = values::Files::with_pictures(ui.ctx(), editor);
    let mut manifest = editor.project.manifest().clone();
    let before = manifest.clone();
    let mut offline = manifest.mode == ProjectMode::Standalone;
    setting_row(
        ui,
        "Mode",
        "Offline games are exported as a program of their own. Online games run on the Metallic platform.",
        |ui| segmented(ui, &mut offline, &[(true, "Offline"), (false, "Online")]),
    );
    manifest.mode = if offline {
        ProjectMode::Standalone
    } else {
        ProjectMode::Platform
    };
    text_row(
        ui,
        "Name",
        "The game's name, its program's and its window's.",
        &mut manifest.name,
    );
    if offline {
        text_row(ui, "Author", "Who made the game.", &mut manifest.author);
        text_row(
            ui,
            "Version",
            "The game's version, such as 1.0.2.",
            &mut manifest.version,
        );
        setting_row(
            ui,
            "Icon",
            "A picture in the project, made into the program's icon. None keeps Metallic's.",
            |ui| {
                if let Some(chosen) = values::picture_field(ui, &files, "Icon", &manifest.icon, CONTROL_WIDTH) {
                    manifest.icon = chosen;
                }
            },
        );
    } else {
        let p = current_colors(ui.ctx()).palette;
        ui.add_space(space::S4);
        ui.label(
            RichText::new(
                "Online games are not available yet: the Metallic platform they run on is still to come. Choose \
                 Offline to export the game as a program of its own.",
            )
            .size(text::BODY)
            .color(p.muted),
        );
    }
    if manifest.name.trim().is_empty() {
        manifest.name = before.name.clone();
    }
    if manifest != before
        && let Err(error) = editor.project.set_manifest(manifest)
    {
        log::warn!("game settings: {error}");
    }
}

/// A row with a one-line text field `value` in its control's column, labelled `name` for screen readers.
fn text_row(ui: &mut Ui, name: &str, description: &str, value: &mut String) {
    setting_row(ui, name, description, |ui| {
        let field = ui.add(TextEdit::singleline(value).desired_width(CONTROL_WIDTH));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
    });
}

/// The graphics an offline game starts with, from `project`'s [`GAME_GRAPHICS_FILE`]: the Auto preset when it has
/// none, or one that cannot be read.
pub fn game_graphics(project: &mut Project) -> GraphicsSettings {
    project
        .read_text(GAME_GRAPHICS_FILE)
        .ok()
        .and_then(|text| GraphicsSettings::from_text(&text).ok())
        .unwrap_or_default()
}

/// The Graphics page: the Settings window's Rendering rows, for the game's settings.
fn graphics_page(ui: &mut Ui, project: &mut Project) {
    let mut settings = StudioSettings {
        rendering: game_graphics(project),
        ..StudioSettings::default()
    };
    let before = settings.rendering.clone();
    for entry in ENTRIES.iter().filter(|entry| entry.category == Category::Rendering) {
        row(ui, entry, &mut settings);
    }
    if settings.rendering != before
        && let Err(error) = project.write(GAME_GRAPHICS_FILE, settings.rendering.to_text().into_bytes())
    {
        log::warn!("game settings: {error}");
    }
}
