//! What a [`Player`](crate::Player) runs and how.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use metallic_engine::metallic_platform::WindowConfig;

use crate::scene::Scene;

/// The frame cap a player runs with unless told otherwise: a backstop for when vsync does not pace the loop (a
/// minimised or occluded window presents nothing, and the loop would spin). #84's frame cap and vsync settings
/// replace it.
pub const DEFAULT_FRAME_CAP: u32 = 240;

/// The frames `--bench` runs before it starts measuring: pipelines are built and the scene is uploaded in the first
/// frames, which would otherwise dominate the slowest percentiles.
pub const DEFAULT_WARMUP_FRAMES: u64 = 30;

/// A graphics preset (ADR 0008 §8): how much a frame may cost, never what the game looks like.
///
/// Parsed and kept by the Player; nothing applies it until #84 adds the graphics settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// Integrated GPUs and phones.
    Low,
    /// The mid-range desktop budget (60 fps on a GTX 1650 class GPU).
    Medium,
    /// Higher-end desktops.
    High,
    /// Everything at its most expensive.
    Ultra,
    /// Calibrated on the machine, with dynamic resolution holding the frame time.
    Auto,
}

impl Preset {
    /// Every preset, in the order `--help` lists them.
    pub const ALL: &[Preset] = &[Preset::Low, Preset::Medium, Preset::High, Preset::Ultra, Preset::Auto];

    /// The name `--preset` takes.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Low => "low",
            Preset::Medium => "medium",
            Preset::High => "high",
            Preset::Ultra => "ultra",
            Preset::Auto => "auto",
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A `--preset` name that is not a preset.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown preset `{0}` (presets: {names})", names = preset_names())]
pub struct UnknownPreset(pub String);

fn preset_names() -> String {
    Preset::ALL
        .iter()
        .map(|preset| preset.name())
        .collect::<Vec<_>>()
        .join(", ")
}

impl FromStr for Preset {
    type Err = UnknownPreset;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Preset::ALL
            .iter()
            .copied()
            .find(|preset| preset.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownPreset(name.to_owned()))
    }
}

/// How a [`Player`](crate::Player) is made: the game, the window and when to stop.
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
    /// The graphics preset asked for. Kept, and reported by `--bench`, until #84 applies it.
    pub preset: Option<Preset>,
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
            preset: None,
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
