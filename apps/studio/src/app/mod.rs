//! The app: [`Studio`], the one window's screens as a state machine (ADR 0009, Screens), and the shell that runs it
//! on Studio's own winit loop with the engine's render device ([`run`]).
//!
//! [`Studio`] knows nothing of winit or wgpu: it draws into an `egui::Ui` and asks the shell for window changes
//! through [`WindowRequest`]s, so the UI tests drive it without a window or a GPU.

mod native;
mod screens;
mod shell;
pub mod sign_in;
pub mod splash;
pub mod startup;

use std::path::{Path, PathBuf};
use std::time::Instant;

use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, Frame, Id, Label, Layout, RichText, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};

use crate::emblem;
use crate::keymap::{ActivationKind, Context, Focus, Keymap, Overrides, ids};
use crate::projects::{FileDialog, Projects, RAIL_MARK, name_from_path};
use crate::settings::{SettingsStore, SettingsWindow, StudioSettings};
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{ButtonKind, Look, ThemeState, button, current_colors, fonts};
pub use shell::run;
use splash::{Splash, Status};
use startup::{Loaded, Startup};

/// Which screen the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The loading splash, over the startup steps.
    Splash,
    /// Sign in, or continue offline.
    SignIn,
    /// The projects page.
    Projects,
    /// The editor, with a project open.
    Editor,
}

/// A change only the shell can make to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// The splash is done: the window becomes Studio's main window, centred where the splash was.
    BecomeMainWindow,
}

/// Something Studio tells the user at the bottom of the window until it is dismissed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    title: &'static str,
    body: String,
}

/// Studio's interface state: the screen shown, the theme, the settings and what startup loaded.
pub struct Studio {
    screen: Screen,
    splash: Splash,
    startup: Startup,
    /// What the startup steps produced, once the splash is done: the renderer the viewport will use (#217). The
    /// projects list and the templates move from it to `projects`.
    loaded: Option<Loaded>,
    /// The projects page and its list.
    projects: Projects,
    /// The project the Editor screen shows.
    open_project: Option<PathBuf>,
    theme: ThemeState,
    /// The settings: in memory until the splash has read them, then the file's.
    settings: SettingsStore,
    settings_window: SettingsWindow,
    /// Every shortcut's command and bindings: the settings' overrides over the defaults.
    keymap: Keymap,
    /// What to tell the user (about the settings file, or a new default shortcut left out), first shown first.
    notices: Vec<Notice>,
    /// Whether the system asks for reduced motion (the Appearance setting can follow it).
    system_reduces_motion: bool,
    /// The interface scale last applied.
    applied_scale: Option<f32>,
    /// The emblem, until the first frame uploads it.
    emblem_image: Option<ColorImage>,
    emblem: Option<TextureHandle>,
    /// The mark at the sizes the screens after the splash show it, in pixels: drawn for each, never scaled.
    marks: Vec<(u32, TextureHandle)>,
    requests: Vec<WindowRequest>,
}

impl std::fmt::Debug for Studio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Studio")
            .field("screen", &self.screen)
            .field("startup", &self.startup)
            .field("loaded", &self.loaded)
            .field("projects", &self.projects)
            .field("open_project", &self.open_project)
            .finish_non_exhaustive()
    }
}

impl Studio {
    /// Studio on the splash, over `startup`'s steps. `emblem` is the splash's emblem, and `reduce_motion` the
    /// system's preference until the settings say otherwise.
    pub fn new(startup: Startup, emblem: Option<ColorImage>, reduce_motion: bool) -> Self {
        Self {
            screen: Screen::Splash,
            splash: Splash::new(),
            startup,
            loaded: None,
            projects: Projects::new(),
            open_project: None,
            // The splash's look: the default dark mode and accent, before the settings are read.
            theme: ThemeState::new(Look {
                reduce_motion,
                ..Look::default()
            }),
            settings: SettingsStore::in_memory(),
            settings_window: SettingsWindow::default(),
            keymap: Keymap::studio(&Overrides::new()),
            notices: Vec::new(),
            system_reduces_motion: reduce_motion,
            applied_scale: None,
            emblem_image: emblem,
            emblem: None,
            marks: Vec::new(),
            requests: Vec::new(),
        }
    }

    /// Asks `dialog` for files and folders (the shell gives the system's; until then every dialog is cancelled).
    pub fn set_file_dialog(&mut self, dialog: Box<dyn FileDialog>) {
        self.projects.set_file_dialog(dialog);
    }

    /// The projects page and its list.
    pub fn projects(&self) -> &Projects {
        &self.projects
    }

    /// The projects page and its list, to change.
    pub fn projects_mut(&mut self) -> &mut Projects {
        &mut self.projects
    }

    /// The project the Editor screen shows.
    pub fn open_project(&self) -> Option<&Path> {
        self.open_project.as_deref()
    }

    /// Goes to the projects page, looking at the projects' files again.
    fn show_projects(&mut self) {
        self.projects.refresh();
        self.screen = Screen::Projects;
    }

    /// Goes to the Editor screen with the project at `path`.
    fn open(&mut self, path: PathBuf) {
        log::info!("projects: opening {}", path.display());
        self.open_project = Some(path);
        self.screen = Screen::Editor;
    }

    /// The mark, `points` square at `ctx`'s scale, drawn for that size the first time it is asked for.
    fn mark(&mut self, ctx: &egui::Context, points: f32) -> TextureHandle {
        let pixels = (points * ctx.pixels_per_point()).round().max(1.0) as u32;
        if let Some((_, texture)) = self.marks.iter().find(|(size, _)| *size == pixels) {
            return texture.clone();
        }
        let mark = emblem::render(pixels);
        let image = ColorImage::from_rgba_unmultiplied([mark.width as usize, mark.height as usize], &mark.rgba);
        let texture = ctx.load_texture(format!("mark-{pixels}"), image, TextureOptions::LINEAR);
        self.marks.push((pixels, texture.clone()));
        texture
    }

    /// The screen shown.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// What startup loaded, once the splash is done.
    pub fn loaded(&self) -> Option<&Loaded> {
        self.loaded.as_ref()
    }

    /// The window changes asked for since the last call.
    pub fn take_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// What the window is cleared to under the interface: nothing around the splash's shape, the theme's ground
    /// after it.
    pub fn clear_color(&self) -> Color32 {
        match self.screen {
            Screen::Splash => Color32::TRANSPARENT,
            _ => self.theme.colors().palette.bg,
        }
    }

    /// The settings now.
    pub fn settings(&self) -> &StudioSettings {
        self.settings.settings()
    }

    /// The Settings window's state.
    pub fn settings_window(&mut self) -> &mut SettingsWindow {
        &mut self.settings_window
    }

    /// What Studio is telling the user (about the settings file, or the keymap), until it is dismissed.
    pub fn notice(&self) -> Option<&str> {
        self.notices.first().map(|notice| notice.body.as_str())
    }

    /// The keymap.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Draws one frame into `ui`, the whole window, at `now`.
    pub fn ui(&mut self, ui: &mut egui::Ui, now: Instant) {
        let ctx = ui.ctx().clone();
        // The splash keeps its own look; the settings apply from the first screen after it.
        if self.screen != Screen::Splash {
            self.apply_settings(&ctx);
        }
        self.theme.apply(&ctx);
        if self.screen != Screen::Splash {
            // Before anything is drawn, so a key a command used reaches nothing else.
            self.run_keymap(&ctx);
        }
        match self.screen {
            Screen::Splash => self.splash(ui, now),
            Screen::SignIn => {
                let mark = self.mark(&ctx, sign_in::MARK);
                let skip = &mut self.settings.settings_mut().general.skip_sign_in;
                if sign_in::show(ui, Some(&mark), skip) == sign_in::SignIn::Offline {
                    self.show_projects();
                }
            }
            Screen::Projects => {
                let mark = self.mark(&ctx, RAIL_MARK);
                if let Some(path) = self.projects.show(ui, Some(&mark), &self.keymap) {
                    self.open(path);
                }
            }
            Screen::Editor => {
                let path = self.open_project.clone().unwrap_or_default();
                let name = self
                    .projects
                    .list()
                    .get(&path)
                    .map_or_else(|| name_from_path(&path), |entry| entry.name.clone());
                let location = path.display().to_string();
                let close = screens::placeholder(
                    ui,
                    "Project",
                    &name,
                    &[&location, "The editor's panels will be here."],
                    "Close the project",
                );
                let close = match self.keymap.shortcut_text(ids::PROJECT_CLOSE) {
                    Some(keys) => close.on_hover_text(format!("Close project ({keys})")),
                    None => close,
                };
                if close.clicked() {
                    self.show_projects();
                }
            }
        }
        if self.screen != Screen::Splash {
            self.gear(ui);
            self.settings_ui(&ctx);
        }
    }

    /// Gives the interface the settings' look and scale. Each is applied only when it changed.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        let appearance = self.settings.settings().appearance;
        self.theme.set(appearance.look(self.system_reduces_motion));
        if self.applied_scale != Some(appearance.ui_scale) {
            // The scale is the setting's alone: egui's own Ctrl +/- zoom would drift from it unseen.
            ctx.options_mut(|options| options.zoom_with_keyboard = false);
            ctx.set_zoom_factor(appearance.ui_scale);
            self.applied_scale = Some(appearance.ui_scale);
        }
    }

    /// The gear in the screen's top-right corner, which opens and closes Settings.
    fn gear(&mut self, ui: &mut egui::Ui) {
        let p = current_colors(ui.ctx()).palette;
        let screen = ui.max_rect();
        let at = egui::Rect::from_min_size(
            pos2(screen.right() - space::S5 - size::CONTROL, screen.top() + space::S5),
            vec2(size::CONTROL, size::CONTROL),
        );
        let hovered = ui.rect_contains_pointer(at);
        let icon = RichText::new(fonts::icon::GEAR.to_string())
            .font(fonts::named(fonts::ICONS, text::TITLE))
            .color(if hovered { p.text } else { p.muted });
        let gear = egui::Button::new(icon)
            .fill(if hovered { p.surface_hover } else { Color32::TRANSPARENT })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(radius::ITEM));
        // Every binding of the command, not just the first.
        let tip = match self.keymap.shortcut_text(ids::SETTINGS_OPEN) {
            Some(keys) => format!("Settings ({keys})"),
            None => "Settings".to_owned(),
        };
        if ui.put(at, gear).on_hover_text(tip).clicked() {
            self.toggle_settings();
        }
    }

    fn toggle_settings(&mut self) {
        let open = self.settings_window.is_open();
        self.settings_window.set_open(!open);
    }

    /// Runs the frame's input through the keymap and does what the commands ask. Every Studio shortcut is here; no
    /// screen or panel reads keys itself.
    fn run_keymap(&mut self, ctx: &egui::Context) {
        if self.settings_window.is_capturing() {
            // The capture dialog is recording a key: it must not also run a command.
            return;
        }
        // Panels arrive with docking (#216) and set the focused panel's context; until then the editor's own. The
        // projects page has the focus unless a dialog or the Settings window is over it.
        let context = match self.screen {
            Screen::Editor => Context::Editor,
            Screen::Projects if !self.projects.is_creating() && !self.settings_window.is_open() => Context::Projects,
            _ => Context::Global,
        };
        let focus = Focus {
            context,
            hovered: None,
            typing: ctx.egui_wants_keyboard_input(),
        };
        for activation in self.keymap.dispatch(ctx, focus) {
            if activation.kind != ActivationKind::Press {
                continue;
            }
            match activation.command {
                ids::SETTINGS_OPEN => self.toggle_settings(),
                ids::PROJECT_CLOSE if self.screen == Screen::Editor => self.show_projects(),
                id @ (ids::PROJECTS_OPEN | ids::PROJECTS_NEW | ids::PROJECTS_ADD) if context == Context::Projects => {
                    if let Some(path) = self.projects.run(id) {
                        self.open(path);
                    }
                }
                // The viewport's navigation, for #217.
                _ => {}
            }
        }
    }

    /// The Settings window, the notices, and saving what changed.
    fn settings_ui(&mut self, ctx: &egui::Context) {
        let before = self.settings.settings().clone();
        self.settings_window
            .show(ctx, self.settings.settings_mut(), &mut self.keymap);
        if self.keymap.take_changed() {
            self.settings.settings_mut().keybinds.overrides = self.keymap.overrides().clone();
        }
        if *self.settings.settings() != before {
            // Restyled at the start of the next pass, which this asks for at once.
            ctx.request_repaint();
        }
        self.show_notice(ctx);
        // Not while a value is dragged, so a drag writes once when it ends rather than every frame.
        if !ctx.input(|input| input.pointer.any_down())
            && let Err(error) = self.settings.save_if_changed()
        {
            log::warn!("settings: could not save them: {error}");
        }
    }

    /// The first notice, if any, at the bottom of the window until dismissed.
    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some(Notice { title, body: notice }) = self.notices.first() else {
            return;
        };
        let colors = current_colors(ctx);
        let p = colors.palette;
        let mut dismissed = false;
        egui::Area::new(Id::unique("metallic.settings.notice"))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -space::S8])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(size::BORDER, p.border))
                    .corner_radius(CornerRadius::same(radius::PANEL))
                    .shadow(shadow::MENU)
                    .inner_margin(space::S6)
                    .show(ui, |ui| {
                        ui.set_width(520.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(fonts::icon::WARNING.to_string())
                                    .font(fonts::named(fonts::ICONS, text::TITLE))
                                    .color(colors.accent),
                            );
                            ui.label(
                                RichText::new(*title)
                                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                                    .color(p.text),
                            );
                        });
                        ui.add(Label::new(RichText::new(notice.as_str()).color(p.muted)).wrap());
                        ui.add_space(space::S4);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            dismissed = button(ui, ButtonKind::Secondary, "Dismiss").clicked();
                        });
                    });
            });
        if dismissed {
            self.notices.remove(0);
        }
    }

    /// Builds the keymap from the settings' overrides. A new default that would take one of the user's keys is left
    /// out: the user is told, and the choice is saved so it is told once.
    fn load_keymap(&mut self) {
        self.keymap = Keymap::studio(&self.settings.settings().keybinds.overrides);
        for body in self.keymap.take_notices() {
            self.notices.push(Notice {
                title: "A new shortcut was left out",
                body,
            });
        }
        if self.keymap.take_changed() {
            self.settings.settings_mut().keybinds.overrides = self.keymap.overrides().clone();
        }
    }

    fn splash(&mut self, ui: &mut egui::Ui, now: Instant) {
        self.startup.poll();
        if let Some(image) = self.emblem_image.take() {
            self.emblem = Some(ui.ctx().load_texture("emblem", image, TextureOptions::LINEAR));
        }
        let status = Status {
            steps: self.startup.names(),
            done: self.startup.done(),
        };
        let motion = self.theme.look().motion();
        self.splash.show(ui, now, status, self.emblem.as_ref(), motion);

        if self.splash.finished(self.startup.is_finished(), now) {
            let mut loaded = self.startup.take_loaded().unwrap_or_default();
            self.settings = std::mem::take(&mut loaded.settings);
            if let Some(body) = self.settings.take_notice() {
                self.notices.push(Notice {
                    title: "Settings were reset",
                    body,
                });
            }
            self.load_keymap();
            let mut list = std::mem::take(&mut loaded.projects);
            if let Some(body) = list.take_notice() {
                self.notices.push(Notice {
                    title: "The projects list was reset",
                    body,
                });
            }
            self.projects.set_list(list);
            self.projects.set_templates(std::mem::take(&mut loaded.templates));
            self.loaded = Some(loaded);
            self.screen = if self.settings.settings().general.skip_sign_in {
                Screen::Projects
            } else {
                Screen::SignIn
            };
            self.requests.push(WindowRequest::BecomeMainWindow);
        }
        // Polls the steps and runs the minimum time out; vsync paces it.
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use std::path::PathBuf;

    use egui::{Key, Modifiers, Role};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::startup::{Step, projects_step, settings_step, wait};
    use super::*;
    use crate::projects::dialog::ScriptedDialog;
    use crate::projects::{PROJECTS_FILE, View, templates_dir};
    use crate::settings::{Category, SETTINGS_FILE};
    use crate::theme::tokens::{self, Accent};
    use crate::theme::{ThemeMode, fonts};

    /// A settings file of its own for `test`, holding `text` (none when `None`).
    fn settings_file(test: &str, text: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        path
    }

    /// Studio in a test harness at a settable clock, starting at `start`.
    struct Test {
        studio: Studio,
        now: Instant,
    }

    fn harness(steps: Vec<Step>) -> Harness<'static, Test> {
        let studio = Studio::new(Startup::spawn(steps).unwrap(), None, false);
        Harness::builder()
            .with_size(splash::SIZE)
            .with_context_setup(fonts::install)
            .build_ui_state(
                |ui, test: &mut Test| test.studio.ui(ui, test.now),
                Test {
                    studio,
                    now: Instant::now(),
                },
            )
    }

    /// The real settings step over `path`, and the real projects step over `projects.toml` beside it and the
    /// repository's templates.
    fn steps_reading(path: PathBuf) -> Vec<Step> {
        let list = path.with_file_name(PROJECTS_FILE);
        vec![settings_step(Some(path)), projects_step(Some(list), templates_dir())]
    }

    /// Steps whose settings file says light and mint.
    fn quick_steps() -> Vec<Step> {
        let path = settings_file(
            &format!("quick-{:?}", std::thread::current().id()),
            Some("[appearance]\nmode = \"light\"\naccent = \"mint\"\n"),
        );
        steps_reading(path)
    }

    /// Studio past its splash on the settings file at `path`, at the main window's size.
    fn past_the_splash(path: PathBuf) -> Harness<'static, Test> {
        let mut harness = harness(steps_reading(path));
        finish_steps(&mut harness, splash::MIN_TIME);
        harness.set_size(egui::vec2(1280.0, 800.0));
        // The sign-in card centres itself by its height in the frame before.
        settle(&mut harness);
        harness
    }

    /// Runs the frames that show what a click did: the one that handles it and the one that draws the result.
    fn settle(harness: &mut Harness<'_, Test>) {
        harness.step();
        harness.step();
    }

    /// Runs the steps out and moves the clock `by` past the splash's first frame.
    fn finish_steps(harness: &mut Harness<'_, Test>, by: Duration) {
        harness.step();
        wait(&mut harness.state_mut().studio.startup);
        harness.state_mut().now += by;
        harness.step();
    }

    #[test]
    fn the_splash_shows_the_running_step() {
        let (go, gate) = mpsc::channel::<()>();
        let mut harness = harness(vec![
            Step::new("Reading settings", |_| {}),
            Step::new("Starting the renderer", move |_| gate.recv().unwrap()),
            Step::new("Reading recent projects", |_| {}),
        ]);
        // Until the second step reports, the first is named.
        for _ in 0..2000 {
            harness.step();
            if harness.query_by_label("Starting the renderer").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        harness.get_by_label("Starting the renderer");
        harness.get_by_label("2 of 3");
        harness.get_by_label("Metallic");
        harness.get_by_label("Studio");
        harness.get_by_label(&format!("Version {}", crate::VERSION));
        // Blocked on a step, the splash stays however long it takes.
        harness.state_mut().now += Duration::from_secs(5);
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::Splash);

        go.send(()).unwrap();
        wait(&mut harness.state_mut().studio.startup);
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn the_splash_never_finishes_before_its_minimum_time() {
        let mut harness = harness(quick_steps());
        finish_steps(&mut harness, Duration::from_millis(100));
        assert!(harness.state().studio.startup.is_finished());
        assert_eq!(
            harness.state().studio.screen(),
            Screen::Splash,
            "every step done, but only 100 ms shown"
        );
        assert!(harness.state_mut().studio.take_requests().is_empty());
        harness.get_by_label("Reading recent projects");
        harness.get_by_label("2 of 2");

        harness.state_mut().now += splash::MIN_TIME;
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn the_screens_go_splash_sign_in_projects_editor() {
        let mut harness = harness(quick_steps());
        assert_eq!(harness.state().studio.clear_color(), Color32::TRANSPARENT);
        finish_steps(&mut harness, splash::MIN_TIME);
        let studio = &mut harness.state_mut().studio;
        assert_eq!(studio.screen(), Screen::SignIn);
        assert_eq!(studio.take_requests(), [WindowRequest::BecomeMainWindow]);
        assert!(studio.take_requests().is_empty(), "asked once");
        assert!(studio.loaded().is_some());

        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.step();
        // The settings' look replaces the splash's dark one.
        let studio = &harness.state().studio;
        assert_eq!(studio.theme.look().mode, ThemeMode::Light);
        assert_eq!(studio.theme.colors().accent, Accent::Mint.colors().light);
        assert_eq!(studio.clear_color(), tokens::LIGHT.bg);
        harness.get_by_label("Sign in");
        harness.get_by_label("WELCOME");
        harness.get_by_label("Online services aren't available yet.");
        // A click switches the screen in its frame; the next frame draws the new one.
        let click = |harness: &mut Harness<'_, Test>, label: &str| {
            harness.get_by_label(label).click();
            harness.step();
            harness.step();
        };
        click(&mut harness, "Continue offline");
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_role_and_label(Role::Tab, "Recent");
        harness.get_by_label("No projects yet");

        let path = std::env::temp_dir().join("A Game.mtproj");
        harness.state_mut().studio.open(path.clone());
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        harness.get_by_label("A Game");
        harness.get_by_label(&path.display().to_string());

        click(&mut harness, "Close the project");
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state_mut().studio.take_requests().is_empty());
    }

    #[test]
    fn skipping_sign_in_goes_straight_to_the_projects() {
        let path = settings_file("skip", Some("[general]\nskip_sign_in = true\n"));
        let harness = past_the_splash(path);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state().studio.notice().is_none());
    }

    #[test]
    fn switching_the_theme_in_settings_restyles_at_once_and_saves() {
        let path = settings_file("live", None);
        let mut harness = past_the_splash(path.clone());
        let style = |harness: &Harness<'_, Test>| harness.ctx.style_of(harness.ctx.theme());
        assert_eq!(style(&harness).visuals.panel_fill, tokens::DARK.bg);
        assert!(!path.exists(), "nothing changed yet, so nothing written");

        harness.get_by_label(&fonts::icon::GEAR.to_string()).click();
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_open());
        harness.get_by_role_and_label(Role::Tab, "Appearance").click();
        settle(&mut harness);
        assert_eq!(
            harness.state_mut().studio.settings_window().category(),
            Category::Appearance
        );
        // The window keeps its size: no row asks for more width than it has.
        for _ in 0..5 {
            harness.step();
        }
        let window = harness.get_by_label("Settings").rect();
        assert!(window.width() < 900.0 && window.right() <= 1280.0, "{window:?}");

        harness.get_by_label("Light").click();
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.settings().appearance.mode, ThemeMode::Light);
        assert!(!studio.theme.colors().dark);
        assert_eq!(studio.clear_color(), tokens::LIGHT.bg);
        assert_eq!(style(&harness).visuals.panel_fill, tokens::LIGHT.bg);
        assert!(!style(&harness).visuals.dark_mode);

        harness
            .get_by_role_and_label(Role::RadioButton, Accent::Blue.label())
            .click();
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.theme.colors().accent,
            Accent::Blue.colors().light
        );

        harness.get_by_role_and_label(Role::CheckBox, "High contrast").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.theme.colors().palette.muted, tokens::LIGHT.text);

        // Each change was written as it was made.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mode = \"light\""), "{text}");
        assert!(text.contains("accent = \"blue\""), "{text}");
        assert!(text.contains("high_contrast = true"), "{text}");
    }

    #[test]
    fn the_interface_scale_and_reduced_motion_apply_live() {
        let mut harness = past_the_splash(settings_file("scale", None));
        assert_eq!(harness.ctx.zoom_factor(), 1.0);
        assert!(harness.ctx.style_of(harness.ctx.theme()).animation_time > 0.0);
        let appearance = &mut harness.state_mut().studio.settings.settings_mut().appearance;
        appearance.ui_scale = 1.5;
        appearance.motion = crate::settings::MotionPreference::Reduced;
        settle(&mut harness);
        assert_eq!(harness.ctx.zoom_factor(), 1.5);
        assert_eq!(harness.ctx.style_of(harness.ctx.theme()).animation_time, 0.0);
    }

    #[test]
    fn primary_comma_opens_and_closes_settings() {
        let mut harness = past_the_splash(settings_file("shortcut", None));
        assert!(!harness.state_mut().studio.settings_window().is_open());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_open());
        harness.get_by_role_and_label(Role::Tab, "General");
        // The screen stays under the window.
        harness.get_by_label("Continue offline");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_open());
        assert!(harness.query_by_role_and_label(Role::Tab, "General").is_none());
    }

    #[test]
    fn the_search_box_filters_every_page() {
        let mut harness = past_the_splash(settings_file("search", None));
        let window = harness.state_mut().studio.settings_window();
        window.set_open(true);
        window.set_query("contrast");
        settle(&mut harness);
        harness.get_by_role_and_label(Role::CheckBox, "High contrast");
        harness.get_by_label("APPEARANCE");
        assert!(harness.query_by_label("Quality preset").is_none());
        assert!(harness.query_by_label("Skip sign-in").is_none());

        // By description too: none of these is named "frames".
        harness.state_mut().studio.settings_window().set_query("frames");
        settle(&mut harness);
        harness.get_by_label("RENDERING");
        harness.get_by_label("Frame cap");
        harness.get_by_label("Dynamic resolution");
        assert!(harness.query_by_label("APPEARANCE").is_none());
        assert!(harness.query_by_label("High contrast").is_none());

        harness.state_mut().studio.settings_window().set_query("teleport");
        settle(&mut harness);
        harness.get_by_label_contains("No settings match");

        // Keybinds are found too: by key, by name, by description.
        harness.state_mut().studio.settings_window().set_query("wheel");
        settle(&mut harness);
        harness.get_by_label("KEYBINDS");
        harness.get_by_label("Zoom");
        harness.get_by_label("Fly speed");
        assert!(harness.query_by_label("Look around").is_none());

        // Picking a category clears the search.
        harness.get_by_role_and_label(Role::Tab, "Keybinds").click();
        settle(&mut harness);
        assert_eq!(harness.state_mut().studio.settings_window().query(), "");
        harness.get_by_label("Fly forward");
        harness.get_by_label("VIEWPORT NAVIGATION");
    }

    /// Studio past the splash, on the Editor screen, with Settings open on the Keybinds page.
    fn on_the_keybinds_page(path: PathBuf) -> Harness<'static, Test> {
        let mut harness = past_the_splash(path);
        harness.state_mut().studio.screen = Screen::Editor;
        let window = harness.state_mut().studio.settings_window();
        window.set_open(true);
        window.show_category(Category::Keybinds);
        settle(&mut harness);
        harness
    }

    /// The label of command `id`'s first binding, as this platform shows it.
    fn label_of(harness: &Harness<'_, Test>, id: &str) -> String {
        let keymap = harness.state().studio.keymap();
        keymap.label(&keymap.bindings(id)[0])
    }

    #[test]
    fn a_focused_text_field_keeps_its_keys_from_commands() {
        let mut harness = on_the_keybinds_page(settings_file("typing", None));
        harness.get_by_label("Close the project");
        // In the search box, Primary+W is text editing, not Close project.
        harness.get_all_by_role(Role::TextInput).next().unwrap().click();
        settle(&mut harness);
        assert!(harness.ctx.egui_wants_keyboard_input());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        // Settings is marked "while typing": it still closes.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_open());
        // Nothing focused now: Primary+W closes the project.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
    }

    #[test]
    fn a_shortcut_is_captured_and_saved() {
        let path = settings_file("capture", None);
        let mut harness = on_the_keybinds_page(path.clone());
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        harness.get_by_label(&settings).click();
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_capturing());
        harness.get_by_label("Change a shortcut for Settings");
        // While capturing, the keymap does not run: Primary+, is recorded (the default again), not obeyed.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert!(harness.state_mut().studio.settings_window().is_open());
        assert!(harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN));

        harness.get_by_label(&settings).click();
        settle(&mut harness);
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        let studio = &harness.state().studio;
        assert_eq!(
            studio.keymap().shortcut_text(ids::SETTINGS_OPEN).as_deref(),
            Some("F10")
        );
        assert_eq!(studio.settings().keybinds.overrides[ids::SETTINGS_OPEN], ["F10"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"settings.open\" = [\"F10\"]"), "{text}");
        harness.get_by_label("Reset");

        // The new key works, and the old one no longer does.
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_open());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_open());
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_open());

        // Esc cancels a capture and changes nothing.
        harness.get_by_label("F10").click();
        settle(&mut harness);
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["F10"]
        );

        // A two-step sequence, in place of F10.
        harness.get_by_label("F10").click();
        settle(&mut harness);
        harness
            .get_by_role_and_label(Role::CheckBox, "Two-step sequence (such as Ctrl+K, then S)")
            .click();
        settle(&mut harness);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
        settle(&mut harness);
        assert!(
            harness.state_mut().studio.settings_window().is_capturing(),
            "waits for the second key"
        );
        harness.key_press(Key::S);
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["Primary+K, S"]
        );

        // Reset gives the default back and removes the override from the file.
        harness.get_by_label("Reset").click();
        settle(&mut harness);
        assert!(harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("settings.open"), "{text}");
    }

    #[test]
    fn a_conflict_offers_replace_swap_and_cancel() {
        let mut harness = on_the_keybinds_page(settings_file("conflict", None));
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        let close = label_of(&harness, ids::PROJECT_CLOSE);
        // Clicks the chip labelled `chip`, presses Primary+W, and expects it to clash with `owner`.
        let capture = |harness: &mut Harness<'_, Test>, chip: &str, owner: &str| {
            harness.get_by_label(chip).click();
            settle(harness);
            harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
            settle(harness);
            harness.get_by_label(&format!("{close} is already used by"));
            harness.get_by_label(owner);
        };
        capture(&mut harness, &settings, "Close project (Editor)");
        harness.get_by_label("Cancel").click();
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert!(
            harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN),
            "cancelled"
        );

        // Swap: Settings takes Primary+W, Close project gets Primary+, .
        capture(&mut harness, &settings, "Close project (Editor)");
        harness.get_by_label("Swap").click();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert_eq!(keymap.shortcut_text(ids::SETTINGS_OPEN), Some(close.clone()));
        assert_eq!(keymap.shortcut_text(ids::PROJECT_CLOSE), Some(settings.clone()));

        // Replace: Close project takes Primary+W back from Settings, which is left unbound.
        capture(&mut harness, &settings, "Settings (Global)");
        harness.get_by_label("Replace").click();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert_eq!(keymap.shortcut_text(ids::PROJECT_CLOSE), Some(close));
        assert!(keymap.bindings(ids::SETTINGS_OPEN).is_empty());
        harness.get_by_label("Unbound");
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_new_default_that_would_take_a_users_key_is_left_out_and_told_once() {
        let path = settings_file("stolen", Some("[keybinds]\n\"settings.open\" = [\"Primary+W\"]\n"));
        let harness = past_the_splash(path.clone());
        harness.get_by_label("A new shortcut was left out");
        harness.get_by_label_contains("Close project");
        assert!(harness.state().studio.keymap().bindings(ids::PROJECT_CLOSE).is_empty());
        // Saved, so the next start is quiet.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"project.close\" = []"), "{text}");
        let again = past_the_splash(path);
        assert!(again.state().studio.notice().is_none());
        assert!(again.state().studio.keymap().bindings(ids::PROJECT_CLOSE).is_empty());
    }

    #[test]
    fn a_corrupt_settings_file_is_kept_and_reported_once() {
        let path = settings_file("corrupt", Some("[appearance\nmode = "));
        let mut harness = past_the_splash(path.clone());
        assert!(path.with_extension("toml.bak").is_file());
        assert_eq!(harness.state().studio.settings(), &StudioSettings::default());
        harness.get_by_label("Settings were reset");
        harness.get_by_label_contains("settings.toml.bak");

        harness.get_by_label("Dismiss").click();
        settle(&mut harness);
        assert!(harness.state().studio.notice().is_none());
        assert!(harness.query_by_label("Settings were reset").is_none());
    }

    // The sign-in screen and the projects page (#215).

    /// A folder of its own for `test`, holding `settings.toml` (`settings`) and `projects.toml` (`projects`) when
    /// given. Returns the settings file's path; the projects list is beside it.
    fn config(test: &str, settings: Option<&str>, projects: Option<&str>) -> PathBuf {
        let path = settings_file(test, settings);
        if let Some(projects) = projects {
            std::fs::write(path.with_file_name(PROJECTS_FILE), projects).unwrap();
        }
        path
    }

    /// A real project called `name` in `dir`, made from the repository's Baseplate template.
    fn make_project(dir: &Path, name: &str) -> PathBuf {
        let template =
            metallic_engine::metallic_project::Template::from_dir(templates_dir().unwrap().join("baseplate-3d"))
                .unwrap();
        let path = dir.join(format!("{name}.mtproj"));
        metallic_engine::metallic_project::Project::create(&path, &template, name).unwrap();
        path
    }

    /// `projects.toml` text listing `projects` (path, name), most recent first.
    fn list_of(projects: &[(&Path, &str)]) -> String {
        projects
            .iter()
            .enumerate()
            .map(|(n, (path, name))| {
                format!(
                    "[[project]]\npath = '{}'\nname = \"{name}\"\nlast_opened = {}\n",
                    path.display(),
                    1_700_000_000 - n * 3600
                )
            })
            .collect()
    }

    /// Clicks the node labelled `label` twice, a frame apart: a double-click.
    fn double_click(harness: &mut Harness<'_, Test>, label: &str) {
        harness.get_by_label(label).click();
        harness.step();
        // The harness's clock jumps by whole repaint delays between frames; a double-click is two clicks 50 ms
        // apart.
        let time = harness.ctx.input(|input| input.time);
        harness.input_mut().time = Some(time + 0.05);
        harness.get_by_label(label).click();
        settle(harness);
        harness.input_mut().time = None;
    }

    /// Studio past the splash on the projects page, asking `dialog` for files.
    fn on_the_projects_page(path: PathBuf, dialog: &ScriptedDialog) -> Harness<'static, Test> {
        let mut harness = past_the_splash(path);
        harness.state_mut().studio.set_file_dialog(Box::new(dialog.clone()));
        if harness.state().studio.screen() == Screen::SignIn {
            harness.get_by_label("Continue offline").click();
            settle(&mut harness);
        }
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness
    }

    #[test]
    fn continuing_offline_goes_to_the_projects_and_saves_nothing() {
        let path = config("offline", None, None);
        let mut harness = past_the_splash(path.clone());
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
        harness.get_by_label("Sign in to Metallic");
        let sign_in = harness.get_by_role_and_label(Role::Button, "Sign in");
        assert!(
            egui_kittest::kittest::NodeT::accesskit_node(&sign_in).is_disabled(),
            "there are no servers yet"
        );
        harness.get_by_label("Online services aren't available yet.");

        harness.get_by_label("Continue offline").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(!harness.state().studio.settings().general.skip_sign_in);
        assert!(!path.exists(), "nothing changed, nothing written");
        // Opened again, it asks again.
        assert_eq!(past_the_splash(path).state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn dont_ask_again_is_saved_and_skips_sign_in_from_then_on() {
        let path = config("dont-ask", None, None);
        let mut harness = past_the_splash(path.clone());
        harness.get_by_role_and_label(Role::CheckBox, "Don't ask again").click();
        settle(&mut harness);
        assert!(harness.state().studio.settings().general.skip_sign_in);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("skip_sign_in = true"), "{text}");
        harness.get_by_label("Continue offline").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);

        assert_eq!(past_the_splash(path).state().studio.screen(), Screen::Projects);
    }

    #[test]
    fn a_new_project_is_made_from_a_template_listed_and_opened() {
        let path = config("new", Some("[general]\nskip_sign_in = true\n"), None);
        let folder = path.with_file_name("Made here");
        let dialog = ScriptedDialog::new([Some(folder.clone())]);
        let mut harness = on_the_projects_page(path.clone(), &dialog);

        // Primary+N opens the dialog, Cancel closes it.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
        settle(&mut harness);
        assert!(harness.state().studio.projects().is_creating());
        harness.get_by_label("Create a project");
        harness.get_by_label("Cancel").click();
        settle(&mut harness);
        assert!(!harness.state().studio.projects().is_creating());

        harness.get_by_label("New project").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::RadioButton, "Baseplate").click();
        let name = harness.get_all_by_role(Role::TextInput).next().unwrap();
        name.set_text_value_accesskit("Space Race");
        settle(&mut harness);
        harness.get_by_label("Browse\u{2026}").click();
        settle(&mut harness);
        assert_eq!(dialog.asked(), ["folder"]);
        let made = folder.join("Space Race.mtproj");
        harness.get_by_label(&format!("Makes {}", made.display()));

        harness.get_by_label("Create").click();
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        let opened = studio.open_project().unwrap().to_path_buf();
        assert_eq!(opened, std::path::absolute(&made).unwrap());
        let project = metallic_engine::metallic_project::Project::open(&opened).unwrap();
        assert_eq!(project.manifest().name, "Space Race");
        assert_eq!(studio.projects().list().entries()[0].path, opened);
        harness.get_by_label("Space Race");
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(list.contains("name = \"Space Race\""), "{list}");

        // Making it again refuses to replace the file, and says so in the dialog.
        harness.get_by_label("Close the project").click();
        settle(&mut harness);
        harness.get_by_label("New project").click();
        settle(&mut harness);
        harness
            .state_mut()
            .studio
            .projects_mut()
            .set_new_project("Space Race", &folder);
        harness.get_by_label("Create").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("a file is already there");
    }

    #[test]
    fn adding_a_project_lists_and_opens_it_and_a_non_project_is_refused() {
        let path = config("add", Some("[general]\nskip_sign_in = true\n"), None);
        let dir = path.parent().unwrap().to_path_buf();
        let project = make_project(&dir, "Added Game");
        let not_a_project = dir.join("notes.mtproj");
        std::fs::write(&not_a_project, "just text").unwrap();
        let dialog = ScriptedDialog::new([None, Some(not_a_project), Some(project.clone())]);
        let mut harness = on_the_projects_page(path.clone(), &dialog);

        // Cancelled: nothing happens.
        harness.get_by_label("Add project").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state().studio.projects().list().entries().is_empty());

        // Not a project: refused, and said why.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("Could not add that project");
        assert!(harness.state().studio.projects().list().entries().is_empty());

        harness.get_by_label("Add project").click();
        settle(&mut harness);
        assert_eq!(dialog.asked(), ["project", "project", "project"]);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert_eq!(studio.open_project(), Some(project.as_path()));
        let entry = &studio.projects().list().entries()[0];
        assert_eq!(entry.name, "Added Game");
        assert!(entry.last_opened.is_some());
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(list.contains("Added Game"), "{list}");
    }

    #[test]
    fn a_missing_project_is_marked_and_does_not_open() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-missing-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let here = make_project_once(&dir, "Still Here");
        let gone = dir.join("Gone Game.mtproj");
        let path = config(
            "missing",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&gone, "Gone Game"), (&here, "Still Here")])),
        );
        let mut harness = on_the_projects_page(path, &ScriptedDialog::default());
        harness.get_by_label("Gone Game");
        harness.get_by_label("Still Here");
        assert_eq!(harness.query_all_by_label("File missing").count(), 1);

        double_click(&mut harness, "Gone Game");
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("is not at");
        harness.get_by_label("Dismiss").click();
        settle(&mut harness);
        assert!(harness.state().studio.projects().error().is_none());
    }

    /// A real project called `name` in `dir`, made the first time it is asked for.
    fn make_project_once(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.mtproj"));
        if path.is_file() { path } else { make_project(dir, name) }
    }

    #[test]
    fn removing_a_project_takes_it_off_the_list_and_keeps_its_file() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-remove-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let keep = make_project_once(&dir, "Keep Me");
        let other = make_project_once(&dir, "Other");
        let path = config(
            "remove",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&keep, "Keep Me"), (&other, "Other")])),
        );
        let mut harness = on_the_projects_page(path.clone(), &ScriptedDialog::default());
        harness.get_by_label("Keep Me").click_secondary();
        settle(&mut harness);
        harness.get_by_label("Remove from list").click();
        settle(&mut harness);

        assert!(harness.query_by_label("Keep Me").is_none());
        harness.get_by_label("Other");
        assert!(keep.is_file(), "the file is left alone");
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(!list.contains("Keep Me") && list.contains("Other"), "{list}");
    }

    #[test]
    fn opening_a_project_moves_to_the_editor_with_its_path() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-open-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = make_project_once(&dir, "First");
        let second = make_project_once(&dir, "Second");
        let path = config(
            "open",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&first, "First"), (&second, "Second")])),
        );
        let mut harness = on_the_projects_page(path.clone(), &ScriptedDialog::default());

        // A click selects.
        harness.get_by_label("Second").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().selected(), Some(second.as_path()));

        // A double-click opens.
        double_click(&mut harness, "Second");
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert_eq!(studio.open_project(), Some(second.as_path()));
        harness.get_by_label(&second.display().to_string());
        // It moved to the front of the list, with the time.
        let entries = harness.state().studio.projects().list().entries();
        assert_eq!(entries[0].path, second);
        assert!(entries[0].last_opened.unwrap() > 1_700_000_000);

        // Enter opens the selected card.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label("First").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().selected(), Some(first.as_path()));
        harness.key_press(Key::Enter);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        assert_eq!(harness.state().studio.open_project(), Some(first.as_path()));
    }

    #[test]
    fn the_templates_view_starts_a_new_project() {
        let path = config("templates", Some("[general]\nskip_sign_in = true\n"), None);
        let mut harness = on_the_projects_page(path, &ScriptedDialog::default());
        harness.get_by_role_and_label(Role::Tab, "Templates").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().view(), View::Templates);
        harness.get_by_label("3D template").click();
        settle(&mut harness);
        assert!(harness.state().studio.projects().is_creating());
        harness.get_by_role_and_label(Role::RadioButton, "Baseplate");
    }

    #[test]
    fn a_corrupt_projects_list_is_kept_as_bak_and_the_list_starts_empty() {
        let path = config(
            "corrupt-list",
            Some("[general]\nskip_sign_in = true\n"),
            Some("[[project]\npath = "),
        );
        let list = path.with_file_name(PROJECTS_FILE);
        let harness = past_the_splash(path);
        assert!(list.with_extension("toml.bak").is_file());
        assert!(!list.exists());
        assert!(harness.state().studio.projects().list().entries().is_empty());
        harness.get_by_label("The projects list was reset");
        harness.get_by_label_contains("projects.toml.bak");
        harness.get_by_label("No projects yet");
    }

    /// Runs `studio` for `frames` frames of `size` points at `pixels_per_point`, drawn by egui-wgpu on the engine's
    /// offscreen device as the shell draws them, calling `before(studio, frame)` before each. Returns the last frame
    /// as RGBA rows.
    fn render_offscreen(
        studio: &mut Studio,
        size: egui::Vec2,
        pixels_per_point: f32,
        frames: usize,
        mut before: impl FnMut(&mut Studio, usize),
    ) -> (u32, u32, Vec<u8>) {
        use metallic_engine::metallic_render::{ContextOptions, RenderContext, wgpu};

        let context = RenderContext::new_offscreen(&ContextOptions::default()).expect("an offscreen device");
        let (device, queue) = (context.device(), context.queue());
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut painter = egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        let (width, height) = (
            (size.x * pixels_per_point).round() as u32,
            (size.y * pixels_per_point).round() as u32,
        );
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let start = Instant::now();
        for frame in 0..frames {
            before(studio, frame);
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(pixels_per_point);
            // The splash shows in the first frame, and its minimum time is out by the second.
            let now = match frame {
                0 => start,
                _ => start + splash::MIN_TIME + Duration::from_millis(16 * frame as u64),
            };
            let mut output = ctx.run_ui(input, |ui| studio.ui(ui, now));
            let mut textures = std::mem::take(&mut output.textures_delta);
            for (id, deltas) in textures.set.drain() {
                for delta in &deltas {
                    painter.update_texture(device, queue, id, delta);
                }
            }
            let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: output.pixels_per_point,
            };
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let mut buffers = painter.update_buffers(device, queue, &mut encoder, &jobs, &screen);
            let clear = egui::Rgba::from(studio.clear_color());
            {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("studio screenshot"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: f64::from(clear.r()),
                                    g: f64::from(clear.g()),
                                    b: f64::from(clear.b()),
                                    a: f64::from(clear.a()),
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    })
                    .forget_lifetime();
                painter.render(&mut pass, &jobs, &screen);
            }
            buffers.push(encoder.finish());
            context.submit(buffers);
            for id in textures.free.drain() {
                painter.free_texture(&id);
            }
        }

        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio screenshot readback"),
            size: u64::from(padded * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            target.size(),
        );
        context.submit(Some(encoder.finish()));
        buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the readback"));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("wait for the readback");
        let data = buffer.get_mapped_range(..).expect("mapped");
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in data.chunks_exact(padded as usize) {
            pixels.extend_from_slice(&row[..(width * 4) as usize]);
        }
        (width, height, pixels)
    }

    /// Writes RGBA rows to `path` as a PNG.
    fn write_png(path: &Path, (width, height, pixels): (u32, u32, Vec<u8>)) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }

    /// Studio over `steps`, the steps done, so its second frame is past the splash.
    fn studio_after_startup(steps: Vec<Step>) -> Studio {
        let mut studio = Studio::new(Startup::spawn(steps).unwrap(), None, false);
        wait(&mut studio.startup);
        studio
    }

    /// The screenshots in `docs/screenshots` (`cargo test -p metallic_studio -- --ignored screenshots`). Offscreen:
    /// no window, no real input, and the scripted file dialog.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshots_of_the_sign_in_and_projects_screens() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let size = egui::vec2(1280.0, 800.0);

        let path = config("shot-sign-in", None, None);
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, size, 1.25, 40, |_, _| {});
        assert_eq!(studio.screen(), Screen::SignIn);
        write_png(&out.join("215-sign-in.png"), shot);

        let dir = std::env::temp_dir().join(format!("metallic-studio-shot-projects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let samples = [
            ("Space Race", 2 * 3600, true),
            ("Obby Tower", 26 * 3600, true),
            ("Kart Prototype", 4 * 86_400, true),
            ("Old Jam Game", 70 * 86_400, false),
            ("Lighting Tests", 200 * 86_400, true),
        ];
        let mut list = String::new();
        for (name, ago, exists) in samples {
            let file = if exists {
                make_project_once(&dir, name)
            } else {
                dir.join(format!("{name}.mtproj"))
            };
            list.push_str(&format!(
                "[[project]]\npath = '{}'\nname = \"{name}\"\nlast_opened = {}\n",
                file.display(),
                now - ago
            ));
        }
        let path = config("shot-projects", Some("[general]\nskip_sign_in = true\n"), Some(&list));
        let mut studio = studio_after_startup(steps_reading(path));
        let selected = dir.join("Obby Tower.mtproj");
        let shot = render_offscreen(&mut studio, size, 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.projects_mut().select(selected.clone());
            }
        });
        assert_eq!(studio.screen(), Screen::Projects);
        write_png(&out.join("215-projects.png"), shot);

        let shot = render_offscreen(&mut studio, size, 1.25, 40, |studio, frame| {
            if frame == 2 {
                let projects = studio.projects_mut();
                projects.new_project(None);
                projects.set_new_project("Space Race 2", Path::new("C:/Users/me/Documents/Metallic"));
            }
        });
        assert!(studio.projects().is_creating());
        write_png(&out.join("215-new-project.png"), shot);
    }
}
