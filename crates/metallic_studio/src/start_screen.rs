//! The start screen (research doc 4.2): pick a template, name the game and create it, or open /
//! re-open a game.
//!
//! - **Rail** (288 px, `bg-panel`): "New game" and "Open a game folder", then "Recent games" as
//!   rows with an 80 x 45 thumbnail, the name, and "Template, relative time"; a game whose folder
//!   is gone keeps its row with a warning. The first-run empty state replaces the list. The
//!   footer has the version and the release notes link.
//! - **Stage**: one plate with the cut: the template's art (with chips) and the form: title,
//!   facts, description, "Inside", Game name, Folder + Browse, a help line that turns into
//!   inline validation, and the one violet action "Create game".
//! - **Starting points**: the template cards with a segmented filter (All, 3D, 2D, Empty).
//!
//! Keyboard: Tab walks the rail, the form (name, folder, Browse, Create), the filter and the
//! cards in that order; Enter in a field or on Create creates the game; arrows move between
//! cards; Ctrl N is New game, Ctrl O opens a folder; Esc leaves a field, then closes the newest
//! toast, then clears the form's error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Align2, CornerRadius, Key, KeyboardShortcut, Layout, Margin, Modifiers, Rect,
    RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, UiBuilder, vec2,
};

use crate::png_image::{decode_png, png_dimensions};
use crate::recent::{RecentProject, RecentProjects, relative_time, unix_now};
use crate::templates::{
    PROJECT_FILE, TemplateInfo, TemplateKind, create_from_template, validate_project_name,
    with_builtin,
};
use crate::theme::{self, fonts, icons};
use crate::widgets::card::{Art, ThemeArt, paint_art};
use crate::widgets::toast::{ToastEvent, Toasts, Tone};
use crate::widgets::{Button, Card, cut, keycap, segmented, truncated};
use crate::{StudioError, updater};

const RAIL_WIDTH: f32 = 288.0;
const FORM_WIDTH: f32 = 360.0;
const ROW_HEIGHT: f32 = 61.0;
const THUMB: egui::Vec2 = vec2(80.0, 45.0);
const FOOTER_HEIGHT: f32 = 65.0;
const GRID_COLUMNS: usize = 6;
const GRID_GAP: f32 = 12.0;
/// Below this window height the "Inside" line hides, so Create never clips.
const INSIDE_MIN_HEIGHT: f32 = 860.0;

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
    /// Open the Settings dialog (the rail's Settings button, or Ctrl+,).
    OpenSettings,
    /// Open the Third-party licences screen (the rail footer's link).
    OpenLicences,
}

/// The starting-points filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Every template.
    #[default]
    All,
    /// 3D games.
    ThreeD,
    /// 2D games.
    TwoD,
    /// Empty worlds.
    Empty,
}

impl Filter {
    fn admits(self, kind: TemplateKind) -> bool {
        match self {
            Self::All => true,
            Self::ThreeD => kind == TemplateKind::ThreeD,
            Self::TwoD => kind == TemplateKind::TwoD,
            Self::Empty => kind == TemplateKind::Empty,
        }
    }

    fn empty_message(self) -> &'static str {
        match self {
            Self::All => "Studio found no templates. Open a game folder instead.",
            Self::ThreeD => "No 3D starting points yet. Try All or Empty.",
            Self::TwoD => "No 2D starting points yet. 2D templates are on the way.",
            Self::Empty => "No empty worlds yet. Try All.",
        }
    }
}

/// Which form field an error belongs to (it gets the red border).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Folder,
}

/// State of the start screen.
pub struct StartScreen {
    templates: Vec<TemplateInfo>,
    selected: usize,
    name: String,
    name_touched: bool,
    location: String,
    error: Option<(String, Field)>,
    recent: RecentProjects,
    /// Whether each recent game's folder still exists, in list order.
    present: Vec<bool>,
    previews: HashMap<PathBuf, Option<TextureHandle>>,
    /// Set by "New game": the name field takes keyboard focus on the next draw.
    focus_name: bool,
    /// Whether a widget had keyboard focus last frame (Esc then only leaves it).
    had_focus: bool,
    filter: Filter,
    toasts: Toasts,
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

/// A start-screen thumbnail is at most this wide or tall (a GPU texture limit with room to spare).
const PREVIEW_MAX_SIDE: u32 = 4096;
/// ... and at most this many decoded RGBA bytes.
const PREVIEW_MAX_BYTES: usize = 32 * 1024 * 1024;
/// ... and the file at most this large (a PNG compresses well, so this alone is not enough).
const PREVIEW_MAX_FILE: u64 = 16 * 1024 * 1024;

/// Whether an image of this size may be decoded for a thumbnail.
fn preview_fits(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= PREVIEW_MAX_SIDE
        && height <= PREVIEW_MAX_SIDE
        && (width as usize) * (height as usize) * 4 <= PREVIEW_MAX_BYTES
}

/// Reads a preview PNG. `None` (the "no preview" state) when the file is missing, broken or over
/// the caps; the size is checked from the header before anything is decoded.
fn read_preview(path: &Path) -> Option<egui::ColorImage> {
    if std::fs::metadata(path).ok()?.len() > PREVIEW_MAX_FILE {
        log::warn!("preview {} is too large to show", path.display());
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let label = path.to_string_lossy();
    let shown = png_dimensions(&bytes, &label).and_then(|(w, h)| {
        if preview_fits(w, h) {
            decode_png(&bytes, &label)
        } else {
            Err(StudioError::Image {
                path: label.to_string(),
                message: format!("{w} x {h} is too large for a preview"),
            })
        }
    });
    match shown {
        Ok(img) => Some(egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba,
        )),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    }
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
        let mut screen = Self {
            templates,
            selected: 0,
            name,
            name_touched: false,
            location: default_location(),
            error: None,
            recent,
            present: Vec::new(),
            previews: HashMap::new(),
            focus_name: false,
            had_focus: false,
            filter: Filter::All,
            toasts: Toasts::default(),
        };
        screen.refresh_present();
        screen
    }

    /// Overrides the parent folder (used by tests and screenshots).
    pub fn set_location(&mut self, location: &Path) {
        self.location = location.to_string_lossy().into_owned();
    }

    /// Shows `message` as the folder's error line (used by the error screenshot scene).
    pub fn set_error(&mut self, message: &str) {
        self.error = Some((message.to_owned(), Field::Folder));
    }

    /// A game could not be opened: an error toast, not a message on the Folder field.
    pub fn open_failed(&mut self, name: &str, error: &StudioError) {
        self.toasts
            .push(Tone::Error, format!("Could not open {name}: {error}"));
    }

    /// Sets the game name as if typed by the creator.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_owned();
        self.name_touched = true;
    }

    /// Selects the template with `id`, if there is one (screenshots).
    pub fn select_id(&mut self, id: &str) {
        if let Some(i) = self.templates.iter().position(|t| t.id == id) {
            self.select(i);
        }
    }

    /// Shows a toast (screenshots and other screens).
    pub fn toast(&mut self, tone: Tone, text: &str) {
        self.toasts.push(tone, text);
    }

    /// The error toast for a recent game whose folder is gone, with "Remove from list".
    pub fn report_missing(&mut self, name: &str, path: &Path) {
        self.toasts.push_with_action(
            Tone::Error,
            format!("{name}: the folder was moved or deleted."),
            "Remove from list",
            path.to_string_lossy(),
        );
    }

    /// The form's error line, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|(m, _)| m.as_str())
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
            self.error = Some(("Choose where to create the game.".to_owned(), Field::Folder));
            return None;
        }
        let target = self.target();
        let result = validate_project_name(&self.name)
            .and_then(|()| create_from_template(template, &target, &self.name));
        match result {
            Ok(()) => {
                self.error = None;
                let name = self.name.trim().to_owned();
                let id = template.id.clone();
                self.recent.record(&name, &target, Some(&id));
                self.refresh_present();
                Some(StartAction::OpenProject { name, path: target })
            }
            Err(e) => {
                let field = match e {
                    StudioError::InvalidName(_) => Field::Name,
                    _ => Field::Folder,
                };
                self.error = Some((e.to_string(), field));
                None
            }
        }
    }

    /// Opens an existing game folder (must contain `metallic.toml`). A failure shows an error
    /// toast and returns `None`.
    pub fn open(&mut self, dir: &Path) -> Option<StartAction> {
        match read_game_name(dir) {
            Ok(name) => {
                self.recent.record(&name, dir, None);
                self.refresh_present();
                Some(StartAction::OpenProject {
                    name,
                    path: dir.to_path_buf(),
                })
            }
            Err(e) => {
                self.toasts.push(Tone::Error, e.to_string());
                None
            }
        }
    }

    /// "New game": selects the first template (Baseplate) and focuses the game name field, so
    /// Enter creates the game.
    fn new_game(&mut self) {
        self.filter = Filter::All;
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

    fn refresh_present(&mut self) {
        self.present = self
            .recent
            .items()
            .iter()
            .map(RecentProject::exists)
            .collect();
    }

    /// Decodes and uploads every template preview now (a splash step), so the first frame of the
    /// start screen does not stall on image files.
    pub fn preload(&mut self, ctx: &egui::Context) {
        let paths: Vec<PathBuf> = self
            .templates
            .iter()
            .map(|t| t.preview_path.clone())
            .collect();
        for path in paths {
            self.preview(ctx, &path);
        }
    }

    /// Texture for a preview image, loaded on first use; `None` if the file is missing, broken or
    /// too large to show.
    fn preview(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        if path.as_os_str().is_empty() {
            return None;
        }
        self.previews
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let color = read_preview(path)?;
                Some(ctx.load_texture(path.to_string_lossy(), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// The render of template `index`, if it has one.
    fn art_for(&mut self, ctx: &egui::Context, index: usize) -> Option<TextureHandle> {
        let path = self.templates.get(index)?.preview_path.clone();
        self.preview(ctx, &path)
    }

    /// Draws the whole start screen into `ui`.
    pub fn show(&mut self, ui: &mut Ui) -> Option<StartAction> {
        let mut action = None;
        self.shortcuts(ui, &mut action);
        egui::Panel::left("recent_games")
            .exact_size(RAIL_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::bg_panel()))
            .show(ui, |ui| {
                if let Some(a) = self.rail(ui) {
                    action = Some(a);
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg_base()))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin {
                                left: 48,
                                right: 48,
                                top: 36,
                                bottom: 28,
                            })
                            .show(ui, |ui| {
                                if let Some(a) = self.main_column(ui) {
                                    action = Some(a);
                                }
                            });
                    });
            });
        if let Some(ToastEvent::Action(key)) = self.toasts.show(ui.ctx()) {
            self.recent.remove(Path::new(&key));
            self.refresh_present();
        }
        self.had_focus = ui.ctx().memory(|m| m.focused().is_some());
        action
    }

    fn shortcuts(&mut self, ui: &mut Ui, action: &mut Option<StartAction>) {
        let (new, open, settings, esc) = ui.input_mut(|i| {
            (
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma)),
                i.key_pressed(Key::Escape),
            )
        });
        if settings {
            *action = Some(StartAction::OpenSettings);
        }
        if new {
            self.new_game();
        }
        if open {
            *action = self.pick_and_open();
        }
        // Esc first leaves a focused field (egui does that), and only then acts here.
        if esc && !self.had_focus && !self.toasts.dismiss_newest() {
            self.error = None;
        }
    }

    fn pick_and_open(&mut self) -> Option<StartAction> {
        let dir = rfd::FileDialog::new()
            .set_title("Open a Metallic game")
            .pick_folder()?;
        self.open(&dir)
    }

    // ----- Rail -----

    fn rail(&mut self, ui: &mut Ui) -> Option<StartAction> {
        let mut action = None;
        let full = ui.max_rect();
        ui.painter().vline(
            full.right() - 0.5,
            full.y_range(),
            Stroke::new(1.0, theme::line()),
        );
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let width = RAIL_WIDTH - 32.0;
        egui::Frame::new()
            .inner_margin(Margin {
                left: 16,
                right: 16,
                top: 16,
                bottom: 8,
            })
            .show(ui, |ui| {
                let new_game = Button::secondary("New game")
                    .icon(icons::PLUS)
                    .keycap("Ctrl N")
                    .width(width);
                if ui
                    .add_enabled(!self.templates.is_empty(), new_game)
                    .clicked()
                {
                    self.new_game();
                }
                ui.add_space(8.0);
                let open = Button::secondary("Open a game folder")
                    .icon(icons::FOLDER_OPEN)
                    .keycap("Ctrl O")
                    .width(width);
                if ui.add(open).clicked() {
                    action = self.pick_and_open();
                }
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Recent games").font(fonts::heading()));
                    if !self.recent.items().is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(self.recent.items().len().to_string())
                                .font(fonts::caption())
                                .color(theme::text_3()),
                        );
                    }
                });
            });

        let list_height = (full.bottom() - FOOTER_HEIGHT - ui.cursor().top()).max(0.0);
        if self.recent.items().is_empty() {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(16, 0))
                .show(ui, |ui| {
                    empty_plate(
                        ui,
                        "No games yet",
                        "Games you create or open show up here, newest first. Pick a starting \
                         point to make your first one.",
                    );
                });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("recent-list")
                .max_height(list_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(8, 0))
                        .show(ui, |ui| {
                            if let Some(a) = self.recent_rows(ui) {
                                action = Some(a);
                            }
                        });
                });
        }

        let footer = Rect::from_min_max(
            egui::pos2(full.left(), full.bottom() - FOOTER_HEIGHT),
            egui::pos2(full.right() - 1.0, full.bottom()),
        );
        if let Some(a) = ui
            .scope_builder(UiBuilder::new().max_rect(footer), rail_footer)
            .inner
        {
            action = Some(a);
        }
        action
    }

    fn recent_rows(&mut self, ui: &mut Ui) -> Option<StartAction> {
        let mut action = None;
        let items = self.recent.items().to_vec();
        let now = unix_now();
        for (index, item) in items.iter().enumerate() {
            let present = self.present.get(index).copied().unwrap_or(false);
            let template = item
                .template
                .as_deref()
                .and_then(|id| self.templates.iter().position(|t| t.id == id));
            let thumb = match template {
                Some(i) if present => self.art_for(ui.ctx(), i),
                _ => None,
            };
            let width = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
            let resp = resp.on_hover_text(item.path.to_string_lossy());
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ListItem, true, item.name.as_str())
            });
            let hover = ui.ctx().animate_bool_with_time(
                resp.id.with("hover"),
                resp.hovered(),
                theme::t_fast(),
            );
            let radius = CornerRadius::same(theme::R_MD);
            if hover > 0.0 {
                ui.painter()
                    .rect_filled(rect, radius, theme::bg_hover().gamma_multiply(hover));
            }
            let thumb_rect = Rect::from_min_size(rect.min + vec2(8.0, 8.0), THUMB);
            let thumb_radius = CornerRadius::same(theme::R_XS);
            if present {
                let art = match &thumb {
                    Some(tex) => Art::Image(tex),
                    None => Art::Theme(ThemeArt::NEUTRAL, icons::CUBE),
                };
                paint_art(ui, thumb_rect, thumb_radius, 0.0, art, 16.0);
            } else {
                ui.painter().rect_stroke(
                    thumb_rect,
                    thumb_radius,
                    Stroke::new(1.0, theme::line_strong()),
                    StrokeKind::Inside,
                );
                ui.painter().text(
                    thumb_rect.center(),
                    Align2::CENTER_CENTER,
                    icons::FOLDER_OPEN,
                    fonts::icon(16.0),
                    theme::text_3(),
                );
            }
            let painter = ui.painter();
            let text_left = thumb_rect.right() + 12.0;
            let text_w = rect.right() - 8.0 - text_left;
            let top = rect.top() + (ROW_HEIGHT - 34.0) / 2.0;
            let name = truncated(
                painter,
                &item.name,
                fonts::strong(),
                theme::text_1(),
                text_w,
            );
            painter.galley(
                egui::pos2(text_left, top + 9.0 - name.size().y / 2.0),
                name,
                theme::text_1(),
            );
            let meta_y = top + 18.0 + 8.0;
            if present {
                let template_name = template
                    .and_then(|i| self.templates.get(i))
                    .map(|t| &t.name);
                let when = item.opened.map(|t| relative_time(t, now));
                let meta = match (template_name, when) {
                    (Some(t), Some(w)) => format!("{t}, {w}"),
                    (Some(t), None) => t.clone(),
                    (None, Some(w)) => w,
                    (None, None) => String::new(),
                };
                let meta = truncated(painter, &meta, fonts::small(), theme::text_3(), text_w);
                painter.galley(
                    egui::pos2(text_left, meta_y - meta.size().y / 2.0),
                    meta,
                    theme::text_3(),
                );
            } else {
                painter.text(
                    egui::pos2(text_left, meta_y),
                    Align2::LEFT_CENTER,
                    icons::WARNING,
                    fonts::icon(14.0),
                    theme::warning(),
                );
                let warn = truncated(
                    painter,
                    "Folder moved or deleted",
                    fonts::small(),
                    theme::warning(),
                    text_w - 18.0,
                );
                painter.galley(
                    egui::pos2(text_left + 18.0, meta_y - warn.size().y / 2.0),
                    warn,
                    theme::warning(),
                );
            }
            if resp.has_focus() {
                cut::frame(painter, rect, radius, 0.0, Stroke::new(2.0, theme::focus()));
            }
            if resp.clicked() {
                if present {
                    action = self.open(&item.path);
                } else {
                    self.report_missing(&item.name, &item.path);
                }
            }
        }
        action
    }

    // ----- Main column -----

    fn main_column(&mut self, ui: &mut Ui) -> Option<StartAction> {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.label(RichText::new("What do you want to make?").font(fonts::display()));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let text = |ui: &mut Ui, s: &str| {
                ui.label(
                    RichText::new(s)
                        .font(fonts::body_lg())
                        .color(theme::text_2()),
                );
            };
            text(ui, "Every template opens ready to play. Press ");
            keycap(ui, "F5");
            text(ui, " to try it, then change anything you like.");
        });
        ui.add_space(24.0);

        let action = self.stage(ui);
        ui.add_space(32.0);
        self.starting_points(ui);
        action
    }

    fn stage(&mut self, ui: &mut Ui) -> Option<StartAction> {
        let window_h = ui.ctx().content_rect().height();
        let height = (window_h - 400.0).clamp(380.0, 456.0);
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let template = self.templates.get(self.selected)?.clone();
        let texture = self.art_for(ui.ctx(), self.selected);

        let painter = ui.painter();
        cut::fill(
            painter,
            rect,
            CornerRadius::same(theme::R_LG),
            theme::CUT_LG,
            theme::bg_panel(),
        );
        let art_rect = Rect::from_min_size(rect.min, vec2(width - FORM_WIDTH, height));
        painter.hline(
            art_rect.right()..=rect.right() - theme::CUT_LG,
            rect.top() + 0.5,
            Stroke::new(1.0, theme::highlight()),
        );
        let art = match &texture {
            Some(tex) => Art::Image(tex),
            None => Art::Theme(ThemeArt::NEUTRAL, kind_glyph(template.kind)),
        };
        let art_radius = CornerRadius {
            nw: theme::R_LG,
            sw: theme::R_LG,
            ..CornerRadius::ZERO
        };
        paint_art(ui, art_rect, art_radius, 0.0, art, 96.0);
        chips(ui, art_rect, &template);

        let form_rect = Rect::from_min_max(
            egui::pos2(art_rect.right() + 28.0, rect.top() + 26.0),
            egui::pos2(rect.right() - 28.0, rect.bottom() - 24.0),
        );
        let mut about = ui.new_child(
            UiBuilder::new()
                .max_rect(form_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        about.spacing_mut().item_spacing = egui::Vec2::ZERO;
        about.label(RichText::new(&template.name).font(fonts::title()));
        if !template.facts.is_empty() {
            about.add_space(2.0);
            about.label(
                RichText::new(&template.facts)
                    .font(fonts::small())
                    .color(theme::text_3()),
            );
        }
        about.add_space(16.0);
        about.add(
            egui::Label::new(
                RichText::new(&template.description)
                    .font(fonts::body_lg())
                    .line_height(Some(21.0))
                    .color(theme::text_1()),
            )
            .wrap(),
        );
        if window_h >= INSIDE_MIN_HEIGHT && !template.inside.is_empty() {
            about.add_space(10.0);
            let mut job = egui::text::LayoutJob::default();
            let strong = egui::FontId::new(fonts::small().size, fonts::strong().family);
            job.append("Inside: ", 0.0, text_format(strong, theme::text_1()));
            job.append(
                &template.inside,
                0.0,
                text_format(fonts::small(), theme::text_2()),
            );
            job.wrap.max_width = form_rect.width();
            about.label(job);
        }
        self.form(ui, form_rect)
    }

    /// Game name, Folder + Browse, help or error, and Create, pinned to the bottom of `rect`.
    fn form(&mut self, ui: &mut Ui, rect: Rect) -> Option<StartAction> {
        let help_w = rect.width();
        let (help_text, help_colour, indent) = match &self.error {
            Some((message, _)) => (message.clone(), theme::error(), 20.0),
            None => (
                format!(
                    "Studio creates {}. It must be new or empty.",
                    self.target().display()
                ),
                theme::text_3(),
                0.0,
            ),
        };
        let mut job = egui::text::LayoutJob::single_section(
            help_text,
            text_format(fonts::small(), help_colour),
        );
        job.wrap.max_width = help_w - indent;
        let help = ui.painter().layout_job(job);
        let label_h = 16.0 + 6.0;
        let height = label_h + 32.0 + 14.0 + label_h + 32.0 + 6.0 + help.size().y + 18.0 + 36.0;
        let fields = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - height), rect.max);
        let mut ui = ui.new_child(
            UiBuilder::new()
                .max_rect(fields)
                .layout(Layout::top_down(Align::Min)),
        );
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

        field_label(&mut ui, "Game name");
        let name_resp = ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .margin(Margin::symmetric(10, 8))
                .min_size(vec2(rect.width(), 32.0))
                .desired_width(rect.width()),
        );
        if std::mem::take(&mut self.focus_name) {
            name_resp.request_focus();
        }
        if name_resp.changed() {
            self.name_touched = true;
            // Live check of the name; folder problems show when Create is pressed.
            self.error = validate_project_name(&self.name)
                .err()
                .map(|e| (e.to_string(), Field::Name));
        }
        self.error_border(&ui, &name_resp, Field::Name);
        ui.add_space(14.0);

        field_label(&mut ui, "Folder");
        let (folder_resp, browse) = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let browse_w = 72.0;
                let edit_w = rect.width() - browse_w - 8.0;
                let folder = ui.add(
                    egui::TextEdit::singleline(&mut self.location)
                        .margin(Margin::symmetric(10, 8))
                        .min_size(vec2(edit_w, 32.0))
                        .desired_width(edit_w),
                );
                let browse = ui
                    .add(Button::secondary("Browse").width(browse_w))
                    .clicked();
                (folder, browse)
            })
            .inner;
        if folder_resp.changed() && self.error.as_ref().is_some_and(|e| e.1 == Field::Folder) {
            self.error = None;
        }
        self.error_border(&ui, &folder_resp, Field::Folder);
        if browse
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose where to create the game")
                .pick_folder()
        {
            self.location = dir.to_string_lossy().into_owned();
            self.error = None;
        }

        ui.add_space(6.0);
        let (help_rect, _) = ui.allocate_exact_size(vec2(help_w, help.size().y), Sense::hover());
        if indent > 0.0 {
            ui.painter().text(
                help_rect.left_top() + vec2(0.0, 1.0),
                Align2::LEFT_TOP,
                icons::WARNING_CIRCLE,
                fonts::icon_fill(14.0),
                theme::error(),
            );
        }
        ui.painter()
            .galley(help_rect.left_top() + vec2(indent, 0.0), help, help_colour);
        ui.add_space(18.0);
        let create = ui.add(Button::primary("Create game").keycap("Enter"));

        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let submitted = |r: &egui::Response| r.lost_focus() && enter;
        if create.clicked() || submitted(&name_resp) || submitted(&folder_resp) {
            return self.create();
        }
        None
    }

    fn error_border(&self, ui: &Ui, resp: &egui::Response, field: Field) {
        if self.error.as_ref().is_some_and(|e| e.1 == field) {
            ui.painter().rect_stroke(
                resp.rect,
                theme::R_SM,
                Stroke::new(1.0, theme::error()),
                StrokeKind::Inside,
            );
        }
    }

    fn starting_points(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(width, 30.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(width);
                ui.label(RichText::new("Starting points").font(fonts::heading()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    segmented(
                        ui,
                        &mut self.filter,
                        &[
                            (Filter::All, "All"),
                            (Filter::ThreeD, "3D"),
                            (Filter::TwoD, "2D"),
                            (Filter::Empty, "Empty"),
                        ],
                    );
                });
            },
        );
        ui.add_space(12.0);

        let visible: Vec<usize> = (0..self.templates.len())
            .filter(|&i| self.filter.admits(self.templates[i].kind))
            .collect();
        if visible.is_empty() {
            empty_plate(ui, "Nothing here yet", self.filter.empty_message());
            return;
        }
        let card_w =
            ((width - GRID_GAP * (GRID_COLUMNS as f32 - 1.0)) / GRID_COLUMNS as f32).floor();
        let (left, right) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
            )
        });
        let mut picked = None;
        for (row_index, row) in visible.chunks(GRID_COLUMNS).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GRID_GAP;
                for (slot, &index) in row.iter().enumerate() {
                    let texture = self.art_for(ui.ctx(), index);
                    let t = &self.templates[index];
                    let art = match &texture {
                        Some(tex) => Art::Image(tex),
                        None => Art::Theme(ThemeArt::NEUTRAL, kind_glyph(t.kind)),
                    };
                    let facts = if t.facts.is_empty() {
                        t.description.as_str()
                    } else {
                        t.facts.as_str()
                    };
                    let resp = ui.add(
                        Card::new(card_w, &t.name, facts, art).selected(index == self.selected),
                    );
                    if resp.clicked() {
                        picked = Some(index);
                    }
                    // Arrows on a focused card select its neighbour (egui moves the focus).
                    if resp.has_focus() {
                        let pos = row_index * GRID_COLUMNS + slot;
                        if right && pos + 1 < visible.len() {
                            picked = Some(visible[pos + 1]);
                        } else if left && pos > 0 {
                            picked = Some(visible[pos - 1]);
                        }
                    }
                }
            });
            ui.add_space(GRID_GAP);
        }
        if let Some(i) = picked {
            self.select(i);
        }
    }
}

fn text_format(font: egui::FontId, color: egui::Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font,
        color,
        line_height: Some(16.0),
        ..Default::default()
    }
}

/// `field-label`: 12/16 in the strong voice, `text-2`, 6 px above its field.
fn field_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(egui::FontId::new(12.0, fonts::strong().family))
            .color(theme::text_2()),
    );
    ui.add_space(6.0);
}

/// The kind's glyph for Theme art.
fn kind_glyph(kind: TemplateKind) -> char {
    match kind {
        TemplateKind::TwoD => icons::SQUARES_FOUR,
        TemplateKind::ThreeD | TemplateKind::Empty => icons::CUBE,
    }
}

/// The chips on the stage art: dimension, and the look when the template names one.
fn chips(ui: &Ui, art: Rect, template: &TemplateInfo) {
    let painter = ui.painter();
    let mut x = art.left() + 14.0;
    let bottom = art.bottom() - 14.0;
    let mut items = vec![(kind_glyph(template.kind), template.kind.dimension())];
    if !template.look.is_empty() {
        items.push((icons::SUN, template.look.as_str()));
    }
    for (glyph, label) in items {
        let text = painter.layout_no_wrap(
            label.to_owned(),
            egui::FontId::new(12.0, fonts::strong().family),
            theme::ART_TEXT,
        );
        let w = 9.0 + 14.0 + 6.0 + text.size().x + 9.0;
        let chip = Rect::from_min_max(egui::pos2(x, bottom - 26.0), egui::pos2(x + w, bottom));
        painter.rect_filled(chip, theme::R_SM, theme::art_scrim());
        painter.text(
            egui::pos2(chip.left() + 9.0, chip.center().y),
            Align2::LEFT_CENTER,
            glyph,
            fonts::icon(14.0),
            theme::ART_ICON,
        );
        painter.galley(
            egui::pos2(chip.left() + 29.0, chip.center().y - text.size().y / 2.0),
            text,
            theme::ART_TEXT,
        );
        x = chip.right() + 6.0;
    }
}

/// An empty-state plate in place of a list: the fact, then the cause and what fills it.
fn empty_plate(ui: &mut Ui, title: &str, body: &str) {
    let shown = egui::Frame::new()
        .fill(theme::bg_raised())
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(fonts::strong()));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(body)
                        .line_height(Some(18.0))
                        .color(theme::text_2()),
                )
                .wrap(),
            );
        });
    let r = shown.response.rect;
    let inset = f32::from(theme::R_MD);
    ui.painter().hline(
        r.left() + inset..=r.right() - inset,
        r.top() + 0.5,
        Stroke::new(1.0, theme::highlight()),
    );
}

/// A small underlined-on-hover text link; true when clicked.
fn footer_link(ui: &mut Ui, text: &str) -> bool {
    let link = ui.add(
        egui::Label::new(
            RichText::new(text)
                .font(fonts::small())
                .color(theme::text_3()),
        )
        .sense(Sense::click()),
    );
    if link.hovered() || link.has_focus() {
        let r = link.rect;
        ui.painter().hline(
            r.x_range(),
            r.bottom() + 1.0,
            Stroke::new(1.0, theme::text_1()),
        );
    }
    link.clicked()
}

/// Draws the rail's footer; returns the action its Settings button or licences link asked for.
fn rail_footer(ui: &mut Ui) -> Option<StartAction> {
    let mut action = None;
    let rect = ui.max_rect();
    ui.painter().hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, theme::line()),
    );
    egui::Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 13,
            bottom: 14,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(14.0, 6.0);
            ui.label(
                RichText::new(format!(
                    "Metallic Studio {}, {} channel",
                    env!("CARGO_PKG_VERSION"),
                    updater::Channel::default().as_str()
                ))
                .font(fonts::small())
                .color(theme::text_2()),
            );
            ui.horizontal(|ui| {
                if footer_link(ui, "Release notes") {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                        "https://github.com/{}/releases",
                        updater::STUDIO_FEED.repo
                    )));
                }
                if footer_link(ui, "Licences") {
                    action = Some(StartAction::OpenLicences);
                }
            });
        });
    // Bottom right, beside the "Release notes" line, clear of the version line above it.
    let button = Rect::from_min_max(
        egui::pos2(rect.left() + 16.0, rect.top() + 31.0),
        egui::pos2(rect.right() - 16.0, rect.top() + 59.0),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(button)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.add(Button::ghost("Settings").icon(icons::GEAR_SIX).compact())
                .clicked()
        },
    )
    .inner
    .then_some(StartAction::OpenSettings)
    .or(action)
}

/// The default game name offered for a template.
fn default_name(template_name: &str) -> String {
    format!("My {template_name}")
}

/// A demo list for the start screen capture: games made from the shipped templates (their
/// folders stand in for game folders and are only read) and one whose folder is gone. Never saved.
pub fn demo_recent(templates_dir: &Path) -> RecentProjects {
    let now = unix_now();
    let game = |name: &str, folder: &str, template: &str, ago: u64| RecentProject {
        name: name.to_owned(),
        path: templates_dir.join(folder),
        template: Some(template.to_owned()),
        opened: Some(now.saturating_sub(ago)),
    };
    RecentProjects::in_memory(vec![
        game("Lava Run", "parkour", "parkour", 12 * 60),
        game("Rooftop Run", "parkour", "parkour", 30 * 3600),
        game("Arena Prototype", "arena-prototype", "parkour", 3 * 86_400),
        game("Flat World", "baseplate", "baseplate", 9 * 86_400),
    ])
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
            kind: TemplateKind::ThreeD,
            facts: String::new(),
            inside: String::new(),
            look: String::new(),
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
        assert_eq!(screen.error.as_ref().map(|e| e.1), Some(Field::Folder));
    }

    #[test]
    fn a_bad_name_is_reported_on_the_name_field() {
        let tpl = tempfile::tempdir().expect("tmp");
        let out = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(vec![template(tpl.path())], RecentProjects::load(None));
        screen.set_location(out.path());
        screen.set_name("a:b");
        assert!(screen.create().is_none());
        assert_eq!(screen.error.as_ref().map(|e| e.1), Some(Field::Name));
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
        assert_eq!(screen.recent.items()[0].template.as_deref(), Some("t"));
        assert_eq!(screen.present, [true]);
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
        screen.filter = Filter::TwoD;
        assert_eq!(screen.name, "My Parkour");
        screen.new_game();
        assert_eq!(screen.selected, 0);
        assert_eq!(screen.templates[0].id, "baseplate");
        assert_eq!(screen.name, "My Baseplate");
        assert!(screen.focus_name);
        assert_eq!(screen.filter, Filter::All);
    }

    #[test]
    fn open_rejects_a_folder_without_metallic_toml() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        assert!(screen.open(dir.path()).is_none());
        assert_eq!(
            screen.toasts.texts(),
            ["This folder isn't a Metallic game (it has no metallic.toml)."]
        );
    }

    #[test]
    fn preview_caps_are_per_side_and_per_byte() {
        assert!(preview_fits(1280, 720));
        assert!(preview_fits(4096, 2048));
        assert!(!preview_fits(4097, 10), "too wide");
        assert!(!preview_fits(10, 8192), "too tall");
        assert!(!preview_fits(4096, 4096), "64 MB decoded");
        assert!(!preview_fits(0, 10));
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        let file = std::fs::File::create(path).expect("create");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("header");
        writer
            .write_image_data(&vec![0x80; (width * height) as usize])
            .expect("data");
    }

    #[test]
    fn an_oversized_preview_shows_no_preview() {
        let dir = tempfile::tempdir().expect("tmp");
        let ok = dir.path().join("ok.png");
        write_png(&ok, 64, 36);
        assert_eq!(read_preview(&ok).expect("small image").size, [64, 36]);
        // 8192 px wide is under the 64 MB decoder limit but over the cap.
        let wide = dir.path().join("wide.png");
        write_png(&wide, 8192, 4);
        assert!(read_preview(&wide).is_none());
        assert!(read_preview(&dir.path().join("missing.png")).is_none());
    }

    #[test]
    fn a_failed_open_is_a_toast_and_leaves_the_folder_field_alone() {
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        let error = StudioError::FolderNotEmpty(PathBuf::new());
        screen.open_failed("Lava Run", &error);
        assert_eq!(screen.error(), None);
        assert_eq!(
            screen.toasts.texts(),
            [format!("Could not open Lava Run: {error}")]
        );
    }

    #[test]
    fn filters_admit_their_kind() {
        assert!(Filter::All.admits(TemplateKind::TwoD));
        assert!(Filter::ThreeD.admits(TemplateKind::ThreeD));
        assert!(!Filter::ThreeD.admits(TemplateKind::Empty));
        assert!(Filter::Empty.admits(TemplateKind::Empty));
        assert!(!Filter::TwoD.admits(TemplateKind::ThreeD));
    }

    #[test]
    fn the_demo_list_marks_the_missing_folder() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let screen = StartScreen::new(Vec::new(), demo_recent(&dir));
        assert_eq!(screen.present, [true, true, false, true]);
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0))),
            events,
            ..Default::default()
        }
    }

    /// Runs one frame of the whole screen with `events` and returns its action.
    fn frame(
        ctx: &egui::Context,
        screen: &mut StartScreen,
        events: Vec<egui::Event>,
    ) -> Option<StartAction> {
        let mut action = None;
        ctx.run_ui(input(events), |ui| action = screen.show(ui))
            .textures_delta
            .clear();
        action
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn focused(ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.focused().is_some())
    }

    #[test]
    fn keyboard_path_new_game_then_enter_creates_and_esc_clears() {
        let out = tempfile::tempdir().expect("tmp");
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme::ThemeId::Graphite);
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        screen.set_location(out.path());
        frame(&ctx, &mut screen, vec![]);
        // Ctrl N focuses the name field.
        frame(&ctx, &mut screen, vec![key(Key::N, Modifiers::COMMAND)]);
        frame(&ctx, &mut screen, vec![]);
        assert!(focused(&ctx));
        // A bad name shows inline at once; the first Esc leaves the field, the second clears it.
        frame(&ctx, &mut screen, vec![egui::Event::Text(":".into())]);
        assert_eq!(screen.error.as_ref().map(|e| e.1), Some(Field::Name));
        frame(&ctx, &mut screen, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!focused(&ctx));
        assert!(screen.error().is_some());
        frame(&ctx, &mut screen, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(screen.error().is_none());
        // Back in the field with a good name, Enter creates the game.
        screen.set_name("My Baseplate");
        screen.focus_name = true;
        frame(&ctx, &mut screen, vec![]);
        frame(&ctx, &mut screen, vec![]);
        let action = frame(&ctx, &mut screen, vec![key(Key::Enter, Modifiers::NONE)]);
        assert_eq!(
            action,
            Some(StartAction::OpenProject {
                name: "My Baseplate".into(),
                path: out.path().join("My Baseplate"),
            })
        );
    }

    #[test]
    fn ctrl_comma_opens_settings() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme::ThemeId::Graphite);
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        frame(&ctx, &mut screen, vec![]);
        let action = frame(&ctx, &mut screen, vec![key(Key::Comma, Modifiers::COMMAND)]);
        assert_eq!(action, Some(StartAction::OpenSettings));
    }

    #[test]
    fn the_rail_settings_button_opens_settings() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        let mut harness = Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            // The name and folder fields have no accessible names yet (not part of this test).
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, state: &mut (StartScreen, Option<StartAction>)| {
                    theme::apply(&ui.ctx().clone(), theme::ThemeId::Graphite);
                    if let Some(a) = state.0.show(ui) {
                        state.1 = Some(a);
                    }
                },
                (screen, None),
            );
        harness.run_steps(3);
        assert!(harness.state().1.is_none());
        harness.get_by_label("Settings").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1, Some(StartAction::OpenSettings));
    }

    #[test]
    fn the_footer_licences_link_opens_licences() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        let mut harness = Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, state: &mut (StartScreen, Option<StartAction>)| {
                    theme::apply(&ui.ctx().clone(), theme::ThemeId::Graphite);
                    if let Some(a) = state.0.show(ui) {
                        state.1 = Some(a);
                    }
                },
                (screen, None),
            );
        harness.run_steps(3);
        harness.get_by_label("Licences").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1, Some(StartAction::OpenLicences));
    }

    #[test]
    fn esc_closes_the_newest_toast_before_the_error() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme::ThemeId::Graphite);
        let mut screen = StartScreen::new(Vec::new(), RecentProjects::load(None));
        screen.set_error("Folder problem");
        screen.toast(Tone::Info, "note");
        frame(&ctx, &mut screen, vec![]);
        frame(&ctx, &mut screen, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(screen.toasts.texts().is_empty());
        assert_eq!(screen.error(), Some("Folder problem"));
        frame(&ctx, &mut screen, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(screen.error().is_none());
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;

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
            kind: TemplateKind::ThreeD,
            facts: String::new(),
            inside: String::new(),
            look: String::new(),
        };
        let mut screen = StartScreen::new(vec![info], RecentProjects::load(None));
        screen.set_location(Path::new(""));
        assert!(screen.create().is_none());
        assert_eq!(screen.error(), Some("Choose where to create the game."));
    }
}
