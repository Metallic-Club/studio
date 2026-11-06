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
    Align, Align2, Color32, ColorImage, CornerRadius, Frame, Id, Label, Layout, RichText, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};

use crate::keymap::{ActivationKind, Context, Focus, Keymap, Overrides, ids};
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
    /// What the startup steps produced, once the splash is done: the renderer the viewport will use (#217) and the
    /// recent projects (#215).
    loaded: Option<Loaded>,
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
            keymap: Keymap::studio(&Overrides::new()),
            notices: Vec::new(),
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
                if screens::placeholder(
                    ui,
                    "Welcome",
                    "Sign in",
                    "Online services aren't available yet.",
                    "Continue offline",
                )
                .clicked()
                {
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
                )
                .clicked()
                {
                    self.screen = Screen::Editor;
                }
            }
            Screen::Editor => {
                let close = screens::placeholder(
                    ui,
                    "Project",
                    "Editor",
                    "The editor's panels will be here.",
                    "Close the project",
                );
                let close = match self.keymap.shortcut_text(ids::PROJECT_CLOSE) {
                    Some(keys) => close.on_hover_text(format!("Close project ({keys})")),
                    None => close,
                };
                if close.clicked() {
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
        // Panels arrive with docking (#216) and set the focused panel's context; until then the editor's own.
        let focus = Focus {
            context: if self.screen == Screen::Editor {
                Context::Editor
            } else {
                Context::Global
            },
            hovered: None,
            typing: ctx.egui_wants_keyboard_input(),
        };
        for activation in self.keymap.dispatch(ctx, focus) {
            if activation.kind != ActivationKind::Press {
                continue;
            }
            match activation.command {
                ids::SETTINGS_OPEN => self.toggle_settings(),
                ids::PROJECT_CLOSE if self.screen == Screen::Editor => self.screen = Screen::Projects,
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
}
