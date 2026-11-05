//! How an [`Engine`](crate::Engine) runs: its context, its clock and where script sources come from.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use metallic_script::{LoadError, MemorySources, RunContext, SourceLoader};

/// The simulation rate a game runs at unless it asks for another.
pub const DEFAULT_TICK_RATE: u32 = 60;

/// The most simulation steps one frame runs unless configured otherwise. Past it, owed steps are dropped and the
/// game runs slower than real time instead of falling further behind (the spiral of death).
pub const DEFAULT_MAX_CATCH_UP: u32 = 8;

/// The longest real time step one frame takes into account unless configured otherwise. A longer stall (a
/// breakpoint, a window being dragged, a laptop waking) moves game time by this much only.
pub const DEFAULT_MAX_FRAME_TIME: Duration = Duration::from_millis(250);

/// Whether the engine draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EngineMode {
    /// No window and no rendering: dedicated servers, tests and tools. Frames are driven by
    /// [`Engine::tick`](crate::Engine::tick) or [`Engine::frame`](crate::Engine::frame).
    #[default]
    Headless,
    /// A window and a render hook, driven by the platform's event loop (`render` feature,
    /// [`run_windowed`](crate::run_windowed)).
    Windowed,
}

/// The settings an [`Engine`](crate::Engine) is made with.
///
/// ```
/// use metallic_engine::{EngineConfig, EngineMode};
/// use metallic_script::{MemorySources, RunContext};
///
/// let config = EngineConfig::new(RunContext::Server)
///     .with_tick_rate(30)
///     .with_sources(MemorySources::new().with("Main.luau", "print('hi')"));
/// assert_eq!(config.mode, EngineMode::Headless);
/// ```
#[derive(Clone)]
pub struct EngineConfig {
    /// Which side this engine runs: it decides which scripts start and whether `RenderStepped` fires (clients and
    /// standalone games only).
    pub context: RunContext,
    /// Simulation steps per second (60 by default). Must be at least 1.
    pub tick_rate: u32,
    /// The most simulation steps one frame runs ([`DEFAULT_MAX_CATCH_UP`]). Must be at least 1.
    pub max_catch_up: u32,
    /// The longest real time step one frame takes into account ([`DEFAULT_MAX_FRAME_TIME`]). Must not be zero.
    pub max_frame_time: Duration,
    /// Headless (the default) or windowed.
    pub mode: EngineMode,
    /// The game runs inside Studio: `RunService:IsStudio()` returns true.
    pub studio: bool,
    /// Where scripts' `Source` paths are read from. Empty [`MemorySources`] by default; a project's scripts come
    /// from [`project_sources`](crate::project_sources).
    pub sources: Rc<dyn SourceLoader>,
}

impl EngineConfig {
    /// The defaults for `context`: 60 Hz, headless, no sources.
    pub fn new(context: RunContext) -> Self {
        Self {
            context,
            tick_rate: DEFAULT_TICK_RATE,
            max_catch_up: DEFAULT_MAX_CATCH_UP,
            max_frame_time: DEFAULT_MAX_FRAME_TIME,
            mode: EngineMode::Headless,
            studio: false,
            sources: Rc::new(MemorySources::new()),
        }
    }

    /// Sets [`EngineConfig::tick_rate`].
    pub fn with_tick_rate(mut self, tick_rate: u32) -> Self {
        self.tick_rate = tick_rate;
        self
    }

    /// Sets [`EngineConfig::max_catch_up`].
    pub fn with_max_catch_up(mut self, steps: u32) -> Self {
        self.max_catch_up = steps;
        self
    }

    /// Sets [`EngineConfig::max_frame_time`].
    pub fn with_max_frame_time(mut self, max: Duration) -> Self {
        self.max_frame_time = max;
        self
    }

    /// Sets [`EngineConfig::mode`].
    pub fn with_mode(mut self, mode: EngineMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets [`EngineConfig::studio`].
    pub fn with_studio(mut self, studio: bool) -> Self {
        self.studio = studio;
        self
    }

    /// Sets [`EngineConfig::sources`].
    pub fn with_sources(mut self, sources: impl SourceLoader + 'static) -> Self {
        self.sources = Rc::new(sources);
        self
    }

    /// Why the settings cannot run, if they cannot.
    pub(crate) fn invalid(&self) -> Option<&'static str> {
        if self.tick_rate == 0 {
            Some("the tick rate must be at least 1 step per second")
        } else if self.max_catch_up == 0 {
            Some("the catch-up cap must allow at least 1 step per frame")
        } else if self.max_frame_time.is_zero() {
            Some("the longest frame time must not be zero")
        } else {
            None
        }
    }
}

impl Default for EngineConfig {
    /// A standalone game ([`RunContext::Local`]) with the defaults of [`EngineConfig::new`].
    fn default() -> Self {
        Self::new(RunContext::Local)
    }
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineConfig")
            .field("context", &self.context)
            .field("tick_rate", &self.tick_rate)
            .field("max_catch_up", &self.max_catch_up)
            .field("max_frame_time", &self.max_frame_time)
            .field("mode", &self.mode)
            .field("studio", &self.studio)
            .finish_non_exhaustive()
    }
}

/// The configured sources, handed to the script host each time the game starts (the host takes ownership of its
/// loader, and a game can be stopped and started again).
pub(crate) struct SharedSources(pub(crate) Rc<dyn SourceLoader>);

impl SourceLoader for SharedSources {
    fn load(&self, path: &str) -> Result<Arc<str>, LoadError> {
        self.0.load(path)
    }
}
