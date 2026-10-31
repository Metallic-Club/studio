//! The Studio application: splash, start screen, and the editor for the opened game.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, TextureHandle};

use crate::colour::demo::ColourDemo;
use crate::editor_ui::{EditorExit, EditorUi};
use crate::png_image::{RgbaImage, decode_png};
use crate::recent::RecentProjects;
use crate::screenshot::ScreenshotJob;
use crate::splash::{self, Splash};
use crate::start_screen::{self, StartAction, StartScreen};
use crate::templates::{TemplateInfo, list_templates, templates_dir};
use crate::{StudioError, theme, widgets};

/// The brand icon (a placeholder until the final brand files arrive).
const ICON_PNG: &[u8] = include_bytes!("../../../branding/placeholder-icon.png");
/// Window title.
pub const WINDOW_TITLE: &str = "Metallic Studio";
/// The folder shown in screenshots (never touched).
const DEMO_LOCATION: &str = r"C:\Games";

/// Which screen a capture should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scene {
    /// The splash.
    Splash,
    /// The start screen with a few recent games (default).
    #[default]
    Start,
    /// The start screen on a first run: no recent games.
    FirstRun,
    /// The start screen after pressing Create on a non-empty folder, with an error toast.
    Error,
    /// The editor with the bundled Parkour template open (nothing is saved).
    Editor,
    /// The colour popover demo (stand-in until the editor's Properties panel exists).
    Colour,
    /// Every Studio widget in every state (design review).
    Components,
}

/// Command-line options (all for design review; creators start Studio without any).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Save one frame to this PNG and exit.
    pub screenshot: Option<PathBuf>,
    /// Which screen to capture.
    pub scene: Scene,
    /// Parent folder for new games (shown in the start screen).
    pub location: Option<PathBuf>,
    /// Window size in points (`--size 1440x900`), to compare captures with the mockups.
    pub size: Option<[f32; 2]>,
}

impl Options {
    /// Parses `--screenshot <path>`, `--scene splash|start|firstrun|error|editor|colour|components`,
    /// `--location <dir>` and `--size <width>x<height>`.
    /// Unknown arguments are reported in the returned message.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut opts = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--screenshot" => opts.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--location" => opts.location = Some(PathBuf::from(value("--location")?)),
                "--size" => opts.size = Some(parse_size(&value("--size")?)?),
                "--scene" => {
                    opts.scene = match value("--scene")?.as_str() {
                        "splash" => Scene::Splash,
                        "start" => Scene::Start,
                        "firstrun" => Scene::FirstRun,
                        "error" => Scene::Error,
                        "editor" => Scene::Editor,
                        "colour" | "color" => Scene::Colour,
                        "components" => Scene::Components,
                        other => return Err(format!("unknown scene {other}")),
                    }
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(opts)
    }
}

/// Parses `1440x900` into a size in points.
fn parse_size(text: &str) -> Result<[f32; 2], String> {
    let bad = || format!("--size wants <width>x<height>, got {text}");
    let (w, h) = text.split_once(['x', 'X']).ok_or_else(bad)?;
    let w: f32 = w.trim().parse().map_err(|_| bad())?;
    let h: f32 = h.trim().parse().map_err(|_| bad())?;
    if w < 320.0 || h < 240.0 {
        return Err(bad());
    }
    Ok([w, h])
}

enum Phase {
    Splash(Splash),
    Start,
    Opened(Box<EditorUi>),
    Colour(Box<ColourDemo>),
    Components,
}

/// What the startup steps have produced so far.
#[derive(Default)]
struct Boot {
    recent: Option<RecentProjects>,
    templates: Option<Vec<TemplateInfo>>,
}

/// The eframe application.
pub struct StudioApp {
    phase: Phase,
    boot: Boot,
    start: Option<StartScreen>,
    options: Options,
    icon: Option<TextureHandle>,
    screenshot: Option<ScreenshotJob>,
    /// The window still has the splash's borderless, see-through look.
    splash_window: bool,
    /// The user agreed to quit from the unsaved-changes prompt; the next close is let through.
    quit_confirmed: bool,
}

/// Decodes the embedded brand icon.
pub fn icon_image() -> Result<RgbaImage, StudioError> {
    decode_png(ICON_PNG, "branding/placeholder-icon.png")
}

impl Options {
    /// True when Studio opens with the splash (a normal start, or the splash capture), so the
    /// first window takes the splash's size and look.
    pub fn shows_splash(&self) -> bool {
        self.screenshot.is_none() || self.scene == Scene::Splash
    }
}

impl StudioApp {
    /// Builds the app and applies the theme. On a normal start the rest of the startup work runs
    /// one step per frame behind the splash; captures run it all here.
    pub fn new(ctx: &egui::Context, options: Options) -> Self {
        theme::apply(ctx);
        let icon = icon_image().ok().map(|img| {
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [img.width as usize, img.height as usize],
                &img.rgba,
            );
            ctx.load_texture("brand-icon", color, egui::TextureOptions::LINEAR)
        });
        let shot = options.screenshot.is_some();
        let phase = match options.scene {
            Scene::Colour => {
                let mut demo = ColourDemo::new(!shot);
                if shot {
                    demo.open_at(egui::pos2(420.0, 120.0));
                }
                Phase::Colour(Box::new(demo))
            }
            Scene::Components => Phase::Components,
            Scene::Editor if shot => EditorUi::demo(&templates_dir().join("parkour"))
                .map_or(Phase::Start, |e| Phase::Opened(Box::new(e))),
            // The capture shows the second step running, like the mockup.
            Scene::Splash if shot => Phase::Splash(Splash::held_at(1)),
            _ if shot => Phase::Start,
            _ => Phase::Splash(Splash::new()),
        };
        let mut app = Self {
            phase,
            boot: Boot::default(),
            start: None,
            splash_window: options.shows_splash(),
            quit_confirmed: false,
            screenshot: options.screenshot.clone().map(ScreenshotJob::new),
            options,
            icon,
        };
        if !matches!(app.phase, Phase::Splash(_)) {
            for step in 0..splash::STEPS.len() {
                app.run_step(ctx, step);
            }
        }
        app
    }

    /// Does the work behind one splash step (`splash::STEPS[step]`).
    fn run_step(&mut self, ctx: &egui::Context, step: usize) {
        match step {
            // Fonts were installed in `new` and are built for the first frame; read settings.
            0 => self.boot.recent = Some(recent_projects(self.options.screenshot.is_some())),
            // Nothing to call: this step ends once the first frame has been drawn and presented,
            // which is when it runs (one step per frame, after painting).
            1 => {}
            2 => self.boot.templates = Some(list_templates(&templates_dir())),
            _ => {
                let templates = self.boot.templates.take().unwrap_or_default();
                let recent = self.boot.recent.take().unwrap_or_default();
                let shot = self.options.screenshot.is_some();
                let scene = self.options.scene;
                let recent = if shot && matches!(scene, Scene::Start | Scene::Error) {
                    start_screen::demo_recent(&templates_dir())
                } else {
                    recent
                };
                let mut start = StartScreen::new(templates, recent);
                if shot {
                    // Display only: nothing is written, and no real path or user name can end up
                    // in a picture.
                    start.set_location(Path::new(DEMO_LOCATION));
                    if scene != Scene::FirstRun {
                        start.select_id("parkour");
                    }
                    if scene == Scene::Error {
                        start.set_error(&StudioError::FolderNotEmpty(PathBuf::new()).to_string());
                        let gone = templates_dir().join("arena-prototype");
                        start.report_missing("Arena Prototype", &gone);
                    }
                } else if let Some(location) = &self.options.location {
                    start.set_location(location);
                }
                start.preload(ctx);
                self.start = Some(start);
            }
        }
    }
}

impl eframe::App for StudioApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.splash_window {
            [0.0; 4]
        } else {
            theme::BG_BASE.to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match &mut self.phase {
            Phase::Colour(demo) => {
                if demo.show(ui) {
                    self.phase = Phase::Start;
                    ctx.request_repaint();
                }
            }
            Phase::Splash(splash) => {
                splash.show(ui, self.icon.as_ref());
                // Paint first, then work: the step named on screen is the one running.
                if let Some(step) = splash.next_step() {
                    self.run_step(&ctx, step);
                    if let Phase::Splash(splash) = &mut self.phase {
                        splash.step_done();
                    }
                    ctx.request_repaint();
                } else if splash.finished() {
                    splash::become_main_window(&ctx);
                    self.splash_window = false;
                    self.phase = Phase::Start;
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(Duration::from_millis(30));
                }
            }
            Phase::Start => {
                if let Some(start) = &mut self.start
                    && let Some(StartAction::OpenProject { name, path }) = start.show(ui)
                {
                    match EditorUi::open(&path) {
                        Ok(editor) => self.phase = Phase::Opened(Box::new(editor)),
                        Err(e) => {
                            log::warn!("could not open {name}: {e}");
                            start.set_error(&e.to_string());
                        }
                    }
                }
            }
            Phase::Components => widgets::components::show(ui, self.icon.as_ref()),
            Phase::Opened(editor) => {
                // Closing the window with unsaved changes asks first (#319 review).
                if ctx.input(|i| i.viewport().close_requested())
                    && !self.quit_confirmed
                    && !editor.request_quit()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                match editor.ui(ui, frame.wgpu_render_state()) {
                    EditorExit::Stay => {}
                    EditorExit::Close => self.phase = Phase::Start,
                    EditorExit::Quit => {
                        self.quit_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        if let Some(job) = &mut self.screenshot {
            job.tick(&ctx);
        }
    }
}

/// Screenshot mode keeps the list in memory so it never reads or writes the real config folder.
fn recent_projects(screenshot: bool) -> RecentProjects {
    if screenshot {
        RecentProjects::load(None)
    } else {
        RecentProjects::load_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_scene_is_parsed() {
        let opts = Options::parse(["--scene".to_owned(), "colour".to_owned()]).unwrap();
        assert_eq!(opts.scene, Scene::Colour);
    }

    #[test]
    fn size_and_components_scene_are_parsed() {
        let args = ["--scene", "components", "--size", "1440x900"].map(str::to_owned);
        let opts = Options::parse(args).unwrap();
        assert_eq!(opts.scene, Scene::Components);
        assert_eq!(opts.size, Some([1440.0, 900.0]));
        assert!(Options::parse(["--size", "big"].map(str::to_owned)).is_err());
        assert!(Options::parse(["--size", "10x10"].map(str::to_owned)).is_err());
    }

    #[test]
    fn screenshot_mode_never_uses_the_real_config_dir() {
        assert!(!recent_projects(true).is_persistent());
    }
}
