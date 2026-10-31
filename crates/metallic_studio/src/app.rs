//! The Studio application: splash, start screen, and the placeholder for the opened game.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, FontId, RichText, TextureHandle, Vec2};

use crate::colour::demo::ColourDemo;
use crate::png_image::{RgbaImage, decode_png};
use crate::recent::RecentProjects;
use crate::screenshot::ScreenshotJob;
use crate::start_screen::{StartAction, StartScreen};
use crate::templates::{list_templates, templates_dir};
use crate::{StudioError, theme};

/// The brand icon (a placeholder until the final brand files arrive).
const ICON_PNG: &[u8] = include_bytes!("../../../branding/placeholder-icon.png");
/// Window title.
pub const WINDOW_TITLE: &str = "Metallic Studio";
/// How long the splash shows at least.
const SPLASH_TIME: Duration = Duration::from_millis(1200);

/// The folder shown in screenshots (never touched).
const DEMO_LOCATION: &str = r"C:\Games";

/// Which screen a capture should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scene {
    /// The splash.
    Splash,
    /// The start screen (default).
    #[default]
    Start,
    /// The start screen after pressing Create on a non-empty folder.
    Error,
    /// The colour popover demo (stand-in until the editor's Properties panel exists).
    Colour,
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
}

impl Options {
    /// Parses `--screenshot <path>`, `--scene splash|start|error|colour`, `--location <dir>`.
    /// Unknown arguments are reported in the returned message.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut opts = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--screenshot" => opts.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--location" => opts.location = Some(PathBuf::from(value("--location")?)),
                "--scene" => {
                    opts.scene = match value("--scene")?.as_str() {
                        "splash" => Scene::Splash,
                        "start" => Scene::Start,
                        "error" => Scene::Error,
                        "colour" | "color" => Scene::Colour,
                        other => return Err(format!("unknown scene {other}")),
                    }
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(opts)
    }
}

enum Phase {
    Splash { since: Instant },
    Start,
    Opened { name: String, path: PathBuf },
    Colour(Box<ColourDemo>),
}

/// The eframe application.
pub struct StudioApp {
    phase: Phase,
    start: StartScreen,
    icon: Option<TextureHandle>,
    screenshot: Option<ScreenshotJob>,
    pending_error_scene: bool,
}

/// Decodes the embedded brand icon.
pub fn icon_image() -> Result<RgbaImage, StudioError> {
    decode_png(ICON_PNG, "branding/placeholder-icon.png")
}

impl StudioApp {
    /// Builds the app: loads templates and recent games, applies the theme.
    pub fn new(ctx: &egui::Context, options: Options) -> Self {
        theme::apply(ctx);
        let icon = icon_image().ok().map(|img| {
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [img.width as usize, img.height as usize],
                &img.rgba,
            );
            ctx.load_texture("brand-icon", color, egui::TextureOptions::LINEAR)
        });
        let mut start = StartScreen::new(
            list_templates(&templates_dir()),
            recent_projects(options.screenshot.is_some()),
        );
        if options.screenshot.is_some() {
            // Display only: nothing is written, and no real path or user name can end up in a picture.
            start.set_location(Path::new(DEMO_LOCATION));
        } else if let Some(location) = &options.location {
            start.set_location(location);
        }
        let phase = match (options.screenshot.is_some(), options.scene) {
            (shot, Scene::Colour) => {
                let mut demo = ColourDemo::new(!shot);
                if shot {
                    demo.open_at(egui::pos2(420.0, 120.0));
                }
                Phase::Colour(Box::new(demo))
            }
            (_, Scene::Splash) => Phase::Splash {
                since: Instant::now() + Duration::from_secs(3600),
            },
            (true, _) => Phase::Start,
            (false, _) => Phase::Splash {
                since: Instant::now(),
            },
        };
        Self {
            phase,
            start,
            icon,
            screenshot: options.screenshot.map(ScreenshotJob::new),
            pending_error_scene: options.scene == Scene::Error,
        }
    }

    fn splash(&self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, theme::DEEP);
        let center = rect.center();
        if let Some(icon) = &self.icon {
            let r = egui::Rect::from_center_size(center - Vec2::new(0.0, 36.0), Vec2::splat(128.0));
            egui::Image::new((icon.id(), Vec2::splat(128.0))).paint_at(ui, r);
        }
        ui.painter().text(
            center + Vec2::new(0.0, 56.0),
            Align2::CENTER_CENTER,
            WINDOW_TITLE,
            theme::fonts::display(),
            theme::TEXT,
        );
        ui.painter().text(
            center + Vec2::new(0.0, 88.0),
            Align2::CENTER_CENTER,
            "Starting...",
            FontId::proportional(13.0),
            theme::TEXT_DIM,
        );
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Phase::Colour(demo) = &mut self.phase {
            if demo.show(ui) {
                self.phase = Phase::Start;
                ctx.request_repaint();
            }
        } else {
            match &self.phase {
                Phase::Splash { since } => {
                    self.splash(ui);
                    if since.elapsed() >= SPLASH_TIME {
                        self.phase = Phase::Start;
                    }
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
                Phase::Start => {
                    if self.pending_error_scene {
                        self.pending_error_scene = false;
                        self.start
                            .set_error(&StudioError::FolderNotEmpty(PathBuf::new()).to_string());
                    }
                    if let Some(StartAction::OpenProject { name, path }) = self.start.show(ui) {
                        self.phase = Phase::Opened { name, path };
                    }
                }
                // Drawn above, where it can borrow the phase mutably.
                Phase::Colour(_) => {}
                Phase::Opened { name, path } => {
                    let (name, path) = (name.clone(), path.clone());
                    let mut back = false;
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.label(RichText::new(name).size(28.0).strong());
                        ui.label(RichText::new(path.to_string_lossy()).color(theme::TEXT_DIM));
                        ui.add_space(12.0);
                        ui.label("The editor opens here in the next step of the roadmap.");
                        back = ui.button("Back to start").clicked();
                    });
                    if back {
                        self.phase = Phase::Start;
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
    fn screenshot_mode_never_uses_the_real_config_dir() {
        assert!(!recent_projects(true).is_persistent());
    }
}
