//! The app: [`Studio`], the one window's screens as a state machine (ADR 0009, Screens), and the shell that runs it
//! on Studio's own winit loop with the engine's render device ([`run`]).
//!
//! [`Studio`] knows nothing of winit or wgpu: it draws into an `egui::Ui` and asks the shell for window changes
//! through [`WindowRequest`]s, so the UI tests drive it without a window or a GPU.

mod native;
mod screens;
mod shell;
pub mod splash;
pub mod startup;

use std::time::Instant;

use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, Frame, Id, Key, KeyboardShortcut, Label, Layout, Modifiers,
    RichText, Stroke, TextureHandle, TextureOptions, pos2, vec2,
};

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

/// Opens and closes the Settings window. TODO(#214): the keymap's command for it; until the keymap lands this is the
/// one shortcut read directly. `COMMAND` is "Primary": Cmd on macOS, Ctrl elsewhere.
const SETTINGS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);

/// Studio's interface state: the screen shown, the theme, the settings and what startup loaded.
pub struct Studio {
    screen: Screen,
    splash: Splash,
    startup: Startup,
    /// What the startup steps produced, once the splash is done: the renderer the viewport will use (#214) and the
    /// recent projects (#215).
    loaded: Option<Loaded>,
    theme: ThemeState,
    /// The settings: in memory until the splash has read them, then the file's.
    settings: SettingsStore,
    settings_window: SettingsWindow,
    /// What to tell the user about the settings file, until dismissed.
    notice: Option<String>,
    /// Whether the system asks for reduced motion (the Appearance setting can follow it).
    system_reduces_motion: bool,
    /// The interface scale last applied.
    applied_scale: Option<f32>,
    /// The emblem, until the first frame uploads it.
    emblem_image: Option<ColorImage>,
    emblem: Option<TextureHandle>,
    requests: Vec<WindowRequest>,
}

impl std::fmt::Debug for Studio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Studio")
            .field("screen", &self.screen)
            .field("startup", &self.startup)
            .field("loaded", &self.loaded)
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
            // The splash's look: the default dark mode and accent, before the settings are read.
            theme: ThemeState::new(Look {
                reduce_motion,
                ..Look::default()
            }),
            settings: SettingsStore::in_memory(),
            settings_window: SettingsWindow::default(),
            notice: None,
            system_reduces_motion: reduce_motion,
            applied_scale: None,
            emblem_image: emblem,
            emblem: None,
            requests: Vec::new(),
        }
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

    /// What Studio is telling the user about the settings file, until it is dismissed.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Draws one frame into `ui`, the whole window, at `now`.
    pub fn ui(&mut self, ui: &mut egui::Ui, now: Instant) {
        let ctx = ui.ctx().clone();
        // The splash keeps its own look; the settings apply from the first screen after it.
        if self.screen != Screen::Splash {
            self.apply_settings(&ctx);
        }
        self.theme.apply(&ctx);
        match self.screen {
            Screen::Splash => self.splash(ui, now),
            Screen::SignIn => {
                if screens::placeholder(
                    ui,
                    "Welcome",
                    "Sign in",
                    "Online services aren't available yet.",
                    "Continue offline",
                ) {
                    self.screen = Screen::Projects;
                }
            }
            Screen::Projects => {
                if screens::placeholder(
                    ui,
                    "Studio",
                    "Projects",
                    "Your projects will be listed here.",
                    "Open the editor",
                ) {
                    self.screen = Screen::Editor;
                }
            }
            Screen::Editor => {
                if screens::placeholder(
                    ui,
                    "Project",
                    "Editor",
                    "The editor's panels will be here.",
                    "Close the project",
                ) {
                    self.screen = Screen::Projects;
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
        let shortcut = ui.ctx().format_shortcut(&SETTINGS_SHORTCUT);
        if ui
            .put(at, gear)
            .on_hover_text(format!("Settings ({shortcut})"))
            .clicked()
        {
            let open = self.settings_window.is_open();
            self.settings_window.set_open(!open);
        }
    }

    /// The Settings window and its shortcut, the settings file's notice, and saving what changed.
    fn settings_ui(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_shortcut(&SETTINGS_SHORTCUT)) {
            let open = self.settings_window.is_open();
            self.settings_window.set_open(!open);
        }
        let before = self.settings.settings().clone();
        self.settings_window.show(ctx, self.settings.settings_mut());
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

    /// The settings file's notice, if any, at the bottom of the window until dismissed.
    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.notice.as_deref() else {
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
                                RichText::new("Settings were reset")
                                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                                    .color(p.text),
                            );
                        });
                        ui.add(Label::new(RichText::new(notice).color(p.muted)).wrap());
                        ui.add_space(space::S4);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            dismissed = button(ui, ButtonKind::Secondary, "Dismiss").clicked();
                        });
                    });
            });
        if dismissed {
            self.notice = None;
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
            self.notice = self.settings.take_notice();
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

    use egui::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::startup::{Step, settings_step, wait};
    use super::*;
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

    /// The real settings step over `path`, and a quick second step.
    fn steps_reading(path: PathBuf) -> Vec<Step> {
        vec![settings_step(Some(path)), Step::new("Reading recent projects", |_| {})]
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
        harness.step();
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
        harness.get_by_label("Projects");

        click(&mut harness, "Open the editor");
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        harness.get_by_label("Editor");

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

        // Picking a category clears the search.
        harness.get_by_role_and_label(Role::Tab, "Keybinds").click();
        settle(&mut harness);
        assert_eq!(harness.state_mut().studio.settings_window().query(), "");
        harness.get_by_label_contains("coming in #214");
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
}
