//! The start screen: pick a template, name the game and create it, or open / re-open a game.
//!
//! Layout follows the Studio UI research mockup: a "Recent games" sidebar on the left, and on the
//! right the question "What do you want to make?", a large panel for the selected template with
//! the game name and folder, and a row of all starting points.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind, TextureHandle, Vec2,
};

use crate::png_image::decode_png;
use crate::recent::RecentProjects;
use crate::templates::{
    PROJECT_FILE, TemplateInfo, create_from_template, validate_project_name, with_builtin,
};
use crate::{StudioError, theme};

/// What the creator asked the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartAction {
    /// A game was created or chosen; open it.
    OpenProject {
        /// Display name.
        name: String,
        /// The game folder.
        path: PathBuf,
    },
}

/// State of the start screen.
pub struct StartScreen {
    templates: Vec<TemplateInfo>,
    selected: usize,
    name: String,
    name_touched: bool,
    location: String,
    error: Option<String>,
    recent: RecentProjects,
    previews: HashMap<PathBuf, Option<TextureHandle>>,
    /// Set by "+ New game": the name field takes keyboard focus on the next draw.
    focus_name: bool,
}

/// Default parent folder for new games: `<Documents>/Metallic Games`.
fn default_location() -> String {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    home.map(|h| h.join("Documents").join("Metallic Games"))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

impl StartScreen {
    /// Creates the screen for `templates` (already sorted) and the remembered `recent` games.
    /// The compiled-in Baseplate is added when `templates` has none, so a game can always be made.
    pub fn new(templates: Vec<TemplateInfo>, recent: RecentProjects) -> Self {
        let templates = with_builtin(templates);
        let name = templates
            .first()
            .map(|t| default_name(&t.name))
            .unwrap_or_default();
        Self {
            templates,
            selected: 0,
            name,
            name_touched: false,
            location: default_location(),
            error: None,
            recent,
            previews: HashMap::new(),
            focus_name: false,
        }
    }

    /// Overrides the parent folder (used by tests and screenshots).
    pub fn set_location(&mut self, location: &Path) {
        self.location = location.to_string_lossy().into_owned();
    }

    /// Shows `message` as the error line (used by the error screenshot scene).
    pub fn set_error(&mut self, message: &str) {
        self.error = Some(message.to_owned());
    }

    /// Sets the game name as if typed by the creator.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_owned();
        self.name_touched = true;
    }

    /// The error line currently shown, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Where the new game would be created: `<location>/<name>`.
    pub fn target(&self) -> PathBuf {
        Path::new(&self.location).join(self.name.trim())
    }

    /// Creates the game from the selected template. Returns the action on success; on failure the
    /// message is kept in [`Self::error`].
    pub fn create(&mut self) -> Option<StartAction> {
        let template = self.templates.get(self.selected)?;
        if self.location.trim().is_empty() {
            self.error = Some("Choose where to create the game.".to_owned());
            return None;
        }
        let target = self.target();
        let result = validate_project_name(&self.name)
            .and_then(|()| create_from_template(template, &target, &self.name));
        match result {
            Ok(()) => {
                self.error = None;
                let name = self.name.trim().to_owned();
                self.recent.record(&name, &target);
                Some(StartAction::OpenProject { name, path: target })
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// Opens an existing game folder (must contain `metallic.toml`).
    pub fn open(&mut self, dir: &Path) -> Option<StartAction> {
        match read_game_name(dir) {
            Ok(name) => {
                self.error = None;
                self.recent.record(&name, dir);
                Some(StartAction::OpenProject {
                    name,
                    path: dir.to_path_buf(),
                })
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// "+ New game": selects the first template (Baseplate) and focuses the game name field, so
    /// Enter creates the game.
    fn new_game(&mut self) {
        self.select(0);
        self.focus_name = true;
    }

    fn select(&mut self, index: usize) {
        if index != self.selected {
            self.selected = index;
            self.error = None;
            if !self.name_touched {
                self.name = self
                    .templates
                    .get(index)
                    .map(|t| default_name(&t.name))
                    .unwrap_or_default();
            }
        }
    }

    /// Texture for a preview image, loaded on first use; `None` if the file is missing or broken.
    fn preview(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        self.previews
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let bytes = std::fs::read(path).ok()?;
                match decode_png(&bytes, &path.to_string_lossy()) {
                    Ok(img) => {
                        let size = [img.width as usize, img.height as usize];
                        let color = egui::ColorImage::from_rgba_unmultiplied(size, &img.rgba);
                        Some(ctx.load_texture(
                            path.to_string_lossy(),
                            color,
                            egui::TextureOptions::LINEAR,
                        ))
                    }
                    Err(e) => {
                        log::warn!("{e}");
                        None
                    }
                }
            })
            .clone()
    }

    /// Draws the whole start screen into `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<StartAction> {
        let mut action = None;
        egui::Panel::left("recent_games")
            .exact_size(320.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::DEEP).inner_margin(24.0))
            .show(ui, |ui| action = self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BASE)
                    .inner_margin(egui::Margin::symmetric(44, 36)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(a) = self.main_column(ui) {
                            action = Some(a);
                        }
                    });
            });
        action
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) -> Option<StartAction> {
        let mut action = None;
        // The grow atoms on both sides centre the label in the full-width button.
        let new_game = egui::Button::new((
            egui::Atom::grow(),
            RichText::new("+ New game")
                .font(theme::fonts::strong())
                .size(15.0)
                .color(theme::ON_ACCENT),
            egui::Atom::grow(),
        ))
        .fill(theme::ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(ui.available_width(), 44.0));
        if ui
            .add_enabled(!self.templates.is_empty(), new_game)
            .clicked()
        {
            self.new_game();
        }
        ui.add_space(20.0);
        ui.label(RichText::new("Recent games").font(theme::fonts::heading()));
        ui.add_space(4.0);
        if self.recent.items().is_empty() {
            egui::Frame::new()
                .fill(theme::PANEL)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("No games yet").font(theme::fonts::strong()));
                    ui.label(
                        RichText::new("Games you create or open show up here, newest first.")
                            .color(theme::TEXT_DIM),
                    );
                });
        } else {
            let items = self.recent.items().to_vec();
            for item in items {
                let resp = ui
                    .add(
                        egui::Button::new(RichText::new(&item.name).font(theme::fonts::strong()))
                            .min_size(Vec2::new(ui.available_width(), 0.0))
                            .frame(true),
                    )
                    .on_hover_text(item.path.to_string_lossy());
                ui.label(
                    RichText::new(item.path.to_string_lossy())
                        .small()
                        .color(theme::TEXT_DIM),
                );
                if resp.clicked() {
                    action = self.open(&item.path);
                }
            }
        }
        ui.add_space(8.0);
        if ui.button("Open a game folder").clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Open a Metallic game")
                .pick_folder()
        {
            action = self.open(&dir);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(concat!("Metallic Studio ", env!("CARGO_PKG_VERSION")))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });
        action
    }

    fn main_column(&mut self, ui: &mut egui::Ui) -> Option<StartAction> {
        let mut action = None;
        ui.label(
            RichText::new("What do you want to make?")
                .font(theme::fonts::display())
                .strong(),
        );
        ui.label(
            RichText::new("Every template opens ready to play. Press F5 to try it, then change anything you like.")
                .color(theme::TEXT_DIM),
        );
        ui.add_space(14.0);

        if self.templates.is_empty() {
            let dir = crate::templates::templates_dir();
            egui::Frame::new()
                .fill(theme::PANEL)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("No templates found").font(theme::fonts::strong()));
                    ui.label(
                        RichText::new(format!(
                            "Studio looked in {}. Reinstall Studio or open a game folder instead.",
                            dir.display()
                        ))
                        .color(theme::TEXT_DIM),
                    );
                });
            return action;
        }

        action = self.selected_panel(ui);
        ui.add_space(22.0);
        ui.label(RichText::new("All starting points").font(theme::fonts::heading()));
        self.tiles(ui);
        action
    }

    fn selected_panel(&mut self, ui: &mut egui::Ui) -> Option<StartAction> {
        let mut action = None;
        let template = self.templates[self.selected].clone();
        let texture = self.preview(ui.ctx(), &template.preview_path);
        let width = ui.available_width();
        let preview_w = (width * 0.58).floor();
        let height = (preview_w * 9.0 / 16.0).clamp(220.0, 380.0);
        egui::Frame::new()
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(8))
            .show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal_top(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(preview_w, height), Sense::hover());
                    paint_preview(
                        ui,
                        rect,
                        texture.as_ref(),
                        CornerRadius {
                            nw: 8,
                            sw: 8,
                            ne: 0,
                            se: 0,
                        },
                    );
                    let form_w = width - preview_w - 40.0;
                    ui.add_space(20.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(form_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                            ui.add_space(20.0);
                            ui.label(
                                RichText::new(&template.name)
                                    .font(theme::fonts::title())
                                    .strong(),
                            );
                            ui.label(RichText::new(&template.description).color(theme::TEXT));
                            ui.add_space(8.0);
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                                ui.add_space(16.0);
                                action = self.form(ui);
                            });
                        },
                    );
                });
            });
        action
    }

    /// Name, folder and Create button; drawn bottom-up so the button sits at the bottom.
    fn form(&mut self, ui: &mut egui::Ui) -> Option<StartAction> {
        let mut action = None;
        // One top-down group, so the bottom-up parent places it as a single block.
        let (name_resp, create_clicked) = ui.vertical(|ui| self.form_rows(ui)).inner;
        if name_resp.changed() {
            self.name_touched = true;
            self.error = None;
        }
        let enter = name_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if create_clicked || enter {
            action = self.create();
        }
        action
    }

    /// The form rows, top to bottom. Returns the name field's response and whether Create was clicked.
    fn form_rows(&mut self, ui: &mut egui::Ui) -> (egui::Response, bool) {
        let label_w = 80.0;
        let input_w = (ui.available_width() - label_w - 8.0).max(60.0);
        let mut name_resp = None;
        egui::Grid::new("create-form")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .min_col_width(label_w)
            .show(ui, |ui| {
                ui.label(RichText::new("Game name").color(theme::TEXT_DIM));
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(input_w));
                if std::mem::take(&mut self.focus_name) {
                    resp.request_focus();
                }
                name_resp = Some(resp);
                ui.end_row();
                ui.label(RichText::new("Folder").color(theme::TEXT_DIM));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let edit = egui::TextEdit::singleline(&mut self.location)
                        .desired_width((input_w - 90.0).max(40.0));
                    if ui.add(edit).changed() {
                        self.error = None;
                    }
                    if ui.button("Browse").clicked()
                        && let Some(dir) = rfd::FileDialog::new()
                            .set_title("Choose where to create the game")
                            .pick_folder()
                    {
                        self.location = dir.to_string_lossy().into_owned();
                        self.error = None;
                    }
                });
                ui.end_row();
            });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(theme::ERROR));
        } else {
            ui.label(
                RichText::new(format!(
                    "Studio creates {}. It must be new or empty.",
                    self.target().display()
                ))
                .small()
                .color(theme::TEXT_DIM),
            );
        }
        let create = egui::Button::new(
            RichText::new("Create game")
                .font(theme::fonts::strong())
                .color(theme::ON_ACCENT),
        )
        .fill(theme::ACCENT)
        .stroke(Stroke::NONE);
        let create_clicked = ui.add(create).clicked();
        (name_resp.expect("grid ran"), create_clicked)
    }

    fn tiles(&mut self, ui: &mut egui::Ui) {
        let gap = 14.0;
        let columns = ((ui.available_width() + gap) / (204.0 + gap))
            .floor()
            .max(1.0);
        let tile = Vec2::new(
            ((ui.available_width() - gap * (columns - 1.0)) / columns).floor(),
            150.0,
        );
        let mut clicked = None;
        let templates = self.templates.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            for (index, t) in templates.iter().enumerate() {
                let texture = self.preview(ui.ctx(), &t.preview_path);
                let (rect, resp) = ui.allocate_exact_size(tile, Sense::click());
                let selected = index == self.selected;
                let fill = if resp.hovered() {
                    theme::HOVER
                } else {
                    theme::PANEL
                };
                let painter = ui.painter();
                painter.rect_filled(rect, CornerRadius::same(8), fill);
                let image_rect = egui::Rect::from_min_size(
                    rect.min + Vec2::splat(6.0),
                    Vec2::new(tile.x - 12.0, 90.0),
                );
                paint_preview(ui, image_rect, texture.as_ref(), CornerRadius::same(5));
                let text_clip = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 8.0, image_rect.bottom()),
                    egui::pos2(rect.right() - 8.0, rect.bottom()),
                );
                let p = ui.painter().with_clip_rect(text_clip);
                p.text(
                    egui::pos2(rect.left() + 10.0, image_rect.bottom() + 8.0),
                    Align2::LEFT_TOP,
                    &t.name,
                    theme::fonts::strong(),
                    theme::TEXT,
                );
                let desc_font = FontId::proportional(12.0);
                let description = fit_words(&t.description, tile.x - 24.0, |s| {
                    p.layout_no_wrap(s.to_owned(), desc_font.clone(), theme::TEXT_DIM)
                        .size()
                        .x
                });
                p.text(
                    egui::pos2(rect.left() + 10.0, image_rect.bottom() + 28.0),
                    Align2::LEFT_TOP,
                    &description,
                    FontId::proportional(12.0),
                    theme::TEXT_DIM,
                );
                let stroke = if selected {
                    Stroke::new(2.0, theme::ACCENT)
                } else {
                    Stroke::new(1.0, theme::BORDER)
                };
                ui.painter()
                    .rect_stroke(rect, CornerRadius::same(8), stroke, StrokeKind::Inside);
                if resp.clicked() {
                    clicked = Some(index);
                }
            }
        });
        if let Some(i) = clicked {
            self.select(i);
        }
    }
}

/// Shortens `text` at a word boundary and adds "…" so it is at most `max_w` wide (as measured by
/// `measure`). Text that already fits is returned unchanged.
fn fit_words(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_owned();
    }
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while words.len() > 1 {
        words.pop();
        let candidate = format!("{}…", words.join(" "));
        if measure(&candidate) <= max_w {
            return candidate;
        }
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.pop().is_some() {
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if measure(&candidate) <= max_w {
            return candidate;
        }
    }
    "…".to_owned()
}

/// Paints a preview image in `rect` (cropped to fill), or a calm placeholder when there is none.
fn paint_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    texture: Option<&TextureHandle>,
    radius: CornerRadius,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::DEEP);
    match texture {
        Some(tex) => {
            let tex_size = tex.size_vec2();
            let scale = (rect.width() / tex_size.x).max(rect.height() / tex_size.y);
            let visible = rect.size() / (tex_size * scale);
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), visible);
            egui::Image::new((tex.id(), tex_size))
                .uv(uv)
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No preview",
                FontId::proportional(13.0),
                theme::TEXT_DIM,
            );
        }
    }
}

/// The default game name offered for a template.
fn default_name(template_name: &str) -> String {
    format!("My {template_name}")
}

/// Reads the display name from `<dir>/metallic.toml`, falling back to the folder name.
fn read_game_name(dir: &Path) -> Result<String, StudioError> {
    let file = dir.join(PROJECT_FILE);
    let text =
        std::fs::read_to_string(&file).map_err(|_| StudioError::NotAGame(dir.to_path_buf()))?;
    let from_file = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("name").and_then(|n| n.as_str().map(str::to_owned)));
    Ok(from_file
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Game".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(dir: &Path) -> TemplateInfo {
        std::fs::write(dir.join("metallic.toml"), "name = \"T\"\n").expect("write");
        TemplateInfo {
            id: "t".into(),
            name: "Parkour".into(),
            description: "d".into(),
            preview_path: dir.join("p.png"),
            dir: dir.to_path_buf(),
            order: 1,
            embedded: None,
        }
    }

    #[test]
    fn create_reports_non_empty_folder_with_the_agreed_copy() {
        let tpl = tempfile::tempdir().expect("tmp");
        let out = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(out.path().join("My Parkour")).expect("mkdir");
        std::fs::write(out.path().join("My Parkour").join("x.txt"), "x").expect("write");
        let mut screen = StartScreen::new(vec![template(tpl.path())], RecentProjects::load(None));
        screen.select(1);
        screen.set_location(out.path());
        assert!(screen.create().is_none());
        assert_eq!(
            screen.error(),
            Some("This folder isn't empty. Choose an empty folder or a new one.")
        );
    }

    #[test]
    fn create_records_a_recent_game_and_opens_it() {
        let tpl = tempfile::tempdir().expect("tmp");
        let out = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(vec![template(tpl.path())], RecentProjects::load(None));
        screen.select(1);
        screen.set_location(out.path());
        let action = screen.create().expect("created");
        assert_eq!(
            action,
            StartAction::OpenProject {
                name: "My Parkour".into(),
                path: out.path().join("My Parkour")
            }
        );
        assert_eq!(screen.recent.items().len(), 1);
    }

    #[test]
    fn without_templates_the_screen_offers_baseplate_and_creates_it() {
        let out = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        let ids: Vec<&str> = screen.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["baseplate"]);
        screen.set_location(out.path());
        let action = screen.create().expect("created");
        let target = out.path().join("My Baseplate");
        assert_eq!(
            action,
            StartAction::OpenProject {
                name: "My Baseplate".into(),
                path: target.clone()
            }
        );
        assert!(target.join("metallic.toml").is_file());
        assert!(target.join("main.mscene").is_file());
    }

    #[test]
    fn with_the_disk_templates_baseplate_comes_before_parkour() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let screen = StartScreen::new(
            crate::templates::list_templates(&dir),
            RecentProjects::load(None),
        );
        let ids: Vec<&str> = screen.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(&ids[..2], ["baseplate", "parkour"]);
        assert_eq!(screen.name, "My Baseplate");
    }

    #[test]
    fn new_game_selects_baseplate_and_focuses_the_name_field() {
        let tpl = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(vec![template(tpl.path())], RecentProjects::load(None));
        screen.select(1);
        assert_eq!(screen.name, "My Parkour");
        screen.new_game();
        assert_eq!(screen.selected, 0);
        assert_eq!(screen.templates[0].id, "baseplate");
        assert_eq!(screen.name, "My Baseplate");
        assert!(screen.focus_name);
    }

    #[test]
    fn open_rejects_a_folder_without_metallic_toml() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        assert!(screen.open(dir.path()).is_none());
        assert!(screen.error().is_some());
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;

    #[test]
    fn fit_words_cuts_at_a_word_boundary() {
        let width = |s: &str| s.chars().count() as f32;
        assert_eq!(fit_words("short text", 20.0, width), "short text");
        assert_eq!(
            fit_words(
                "A ready-to-play Parkour starter you can change.",
                30.0,
                width
            ),
            "A ready-to-play Parkour…"
        );
        assert_eq!(fit_words("Supercalifragilistic", 8.0, width), "Superca…");
    }

    #[test]
    fn create_with_empty_location_gives_a_clear_message() {
        let tpl = tempfile::tempdir().expect("tmp");
        std::fs::write(tpl.path().join("metallic.toml"), "name = \"T\"\n").expect("write");
        let info = TemplateInfo {
            id: "t".into(),
            name: "Parkour".into(),
            description: "d".into(),
            preview_path: tpl.path().join("p.png"),
            dir: tpl.path().to_path_buf(),
            order: 1,
            embedded: None,
        };
        let mut screen = StartScreen::new(vec![info], RecentProjects::load(None));
        screen.set_location(Path::new(""));
        assert!(screen.create().is_none());
        assert_eq!(screen.error(), Some("Choose where to create the game."));
    }
}
