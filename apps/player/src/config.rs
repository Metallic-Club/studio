//! What a [`Player`](crate::Player) runs and how.

use std::path::PathBuf;
use std::time::Duration;

use metallic_engine::metallic_platform::WindowConfig;
pub use metallic_engine::metallic_render::settings::{GraphicsSettings, Preset, UnknownPreset};

use crate::scene::Scene;

/// The Player's name for its per-user folders ([`metallic_engine::metallic_platform::AppId`]), where its graphics
/// settings live ([`metallic_engine::graphics`]).
pub const PLAYER_APP_ID: &str = "dev.metallic.Player";

/// The frame cap a player runs with unless told otherwise: a backstop for when vsync does not pace the loop (a
/// minimised or occluded window presents nothing, and the loop would spin). The graphics settings' frame cap
/// replaces it ([`GraphicsSettings::frame_cap`]).
pub const DEFAULT_FRAME_CAP: u32 = 240;

/// The frames `--bench` runs before it starts measuring: pipelines are built and the scene is uploaded in the first
/// frames, which would otherwise dominate the slowest percentiles.
pub const DEFAULT_WARMUP_FRAMES: u64 = 30;

/// How a [`Player`](crate::Player) is made: the game, the window, the graphics settings and when to stop.
///
/// ```
/// use metallic_player::{PlayerConfig, Preset, Scene};
///
/// let config = PlayerConfig::new(Scene::Demo).with_preset(Preset::High).with_max_frames(600);
/// assert_eq!(config.max_frames, Some(600));
/// ```
#[derive(Clone, Debug)]
pub struct PlayerConfig {
    /// The game to run. Built-in scenes only until projects load from disk (M4).
    pub scene: Scene,
    /// The player's graphics settings (ADR 0008 §8): the preset and its overrides, and Auto's calibration. The
    /// `metallic_player` binary loads them from the user's settings file; the Auto preset by default.
    pub graphics: GraphicsSettings,
    /// A preset for this run only (`--preset`), in place of [`PlayerConfig::graphics`]'s; not saved.
    pub preset: Option<Preset>,
    /// Where [`PlayerConfig::graphics`] came from: the Player saves Auto's calibration there. `None` saves nothing.
    pub graphics_file: Option<PathBuf>,
    /// The folder compiled pipelines are kept in between runs (on Vulkan; the user's cache folder for the
    /// `metallic_player` binary). `None` keeps nothing.
    pub pipeline_cache: Option<PathBuf>,
    /// The window [`Player::run`](crate::Player::run) and [`PlayerApp`](crate::PlayerApp) open. A host that
    /// brings its own window ignores it.
    pub window: WindowConfig,
    /// The most frames per second, or `None` to let vsync alone pace the loop ([`DEFAULT_FRAME_CAP`]).
    pub frame_cap: Option<u32>,
    /// Stop after this many measured frames (after [`PlayerConfig::warmup_frames`]): `--bench`. `None` runs until
    /// the window is closed.
    pub max_frames: Option<u64>,
    /// Frames run before frame times are recorded ([`DEFAULT_WARMUP_FRAMES`] with `--bench`, otherwise 0).
    pub warmup_frames: u64,
    /// Stop after this much wall time, for smoke tests. `None` runs until the window is closed.
    pub exit_after: Option<Duration>,
    /// Start with the performance overlay shown (F3 toggles it while running).
    pub overlay: bool,
}

impl PlayerConfig {
    /// `scene` in a window titled "Metallic Player" of the scene's size ([`Scene::window_size`]: 1280 by 720 for
    /// the demo), capped at [`DEFAULT_FRAME_CAP`], until the window is closed.
    pub fn new(scene: Scene) -> Self {
        let (width, height) = scene.window_size();
        Self {
            scene,
            graphics: GraphicsSettings::default(),
            preset: None,
            graphics_file: None,
            pipeline_cache: None,
            window: WindowConfig::new("Metallic Player").with_size(width, height),
            frame_cap: Some(DEFAULT_FRAME_CAP),
            max_frames: None,
            warmup_frames: 0,
            exit_after: None,
            overlay: false,
        }
    }

    /// Sets [`PlayerConfig::overlay`].
    pub fn with_overlay(mut self, overlay: bool) -> Self {
        self.overlay = overlay;
        self
    }

    /// Sets [`PlayerConfig::preset`].
    pub fn with_preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Sets [`PlayerConfig::graphics`].
    pub fn with_graphics(mut self, graphics: GraphicsSettings) -> Self {
        self.graphics = graphics;
        self
    }

    /// The graphics settings this run applies: [`PlayerConfig::graphics`] with [`PlayerConfig::preset`] in place of
    /// its preset when there is one.
    pub fn effective_graphics(&self) -> GraphicsSettings {
        let mut graphics = self.graphics.clone();
        if let Some(preset) = self.preset {
            graphics.preset = preset;
        }
        graphics
    }

    /// Sets [`PlayerConfig::window`].
    pub fn with_window(mut self, window: WindowConfig) -> Self {
        self.window = window;
        self
    }

    /// Sets [`PlayerConfig::frame_cap`].
    pub fn with_frame_cap(mut self, fps: Option<u32>) -> Self {
        self.frame_cap = fps;
        self
    }

    /// Sets [`PlayerConfig::max_frames`].
    pub fn with_max_frames(mut self, frames: u64) -> Self {
        self.max_frames = Some(frames);
        self
    }

    /// Sets [`PlayerConfig::warmup_frames`].
    pub fn with_warmup_frames(mut self, frames: u64) -> Self {
        self.warmup_frames = frames;
        self
    }

    /// Sets [`PlayerConfig::exit_after`].
    pub fn with_exit_after(mut self, wall_time: Duration) -> Self {
        self.exit_after = Some(wall_time);
        self
    }

    /// The settings `--bench <frames>` runs with: `frames` measured frames after [`DEFAULT_WARMUP_FRAMES`], with
    /// no frame cap so only vsync paces the loop.
    pub fn bench(scene: Scene, frames: u64) -> Self {
        Self::new(scene)
            .with_frame_cap(None)
            .with_warmup_frames(DEFAULT_WARMUP_FRAMES)
            .with_max_frames(frames)
    }
}

impl Default for PlayerConfig {
    /// The demo ([`Scene::Demo`]) with the defaults of [`PlayerConfig::new`].
    fn default() -> Self {
        Self::new(Scene::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse_by_name_whatever_the_case() {
        assert_eq!("HIGH".parse(), Ok(Preset::High));
        for preset in Preset::ALL {
            assert_eq!(preset.name().parse(), Ok(*preset));
            assert_eq!(preset.to_string(), preset.name());
        }
        let error = "epic".parse::<Preset>().unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown preset `epic` (presets: low, medium, high, ultra, auto)"
        );
    }

    #[test]
    fn a_bench_measures_after_a_warmup_without_a_frame_cap() {
        let config = PlayerConfig::bench(Scene::Demo, 100);
        assert_eq!(config.max_frames, Some(100));
        assert_eq!(config.warmup_frames, DEFAULT_WARMUP_FRAMES);
        assert_eq!(config.frame_cap, None);
        assert_eq!(PlayerConfig::default().frame_cap, Some(DEFAULT_FRAME_CAP));
    }
}
