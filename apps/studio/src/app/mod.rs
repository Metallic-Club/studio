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

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use crate::theme::{Look, ThemeState};
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

/// Studio's interface state: the screen shown, the theme and what startup loaded.
pub struct Studio {
    screen: Screen,
    splash: Splash,
    startup: Startup,
    /// What the startup steps produced, once the splash is done: the renderer the viewport will use (#214) and the
    /// recent projects (#215).
    loaded: Option<Loaded>,
    theme: ThemeState,
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

    /// Draws one frame into `ui`, the whole window, at `now`.
    pub fn ui(&mut self, ui: &mut egui::Ui, now: Instant) {
        self.theme.apply(ui.ctx());
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
            let loaded = self.startup.take_loaded().unwrap_or_default();
            self.theme.set(loaded.look);
            self.loaded = Some(loaded);
            self.screen = Screen::SignIn;
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

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::startup::{Step, wait};
    use super::*;
    use crate::theme::tokens::{self, Accent};
    use crate::theme::{ThemeMode, fonts};

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

    fn quick_steps() -> Vec<Step> {
        vec![
            Step::new("Reading settings", |loaded| {
                loaded.look.mode = ThemeMode::Light;
                loaded.look.accent = Accent::Mint;
            }),
            Step::new("Reading recent projects", |_| {}),
        ]
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
}
