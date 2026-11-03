//! [`Engine`]: the game, its scripts, its clock and the frame loop.

use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use metallic_classes::GameError;
use metallic_classes::services::run_service::{RunFlags, set_run_flags};
use metallic_core::{DataModel, Id, ScriptValue, SignalId};
use metallic_script::{OutputSink, Phase, RunContext, ScriptHost, SourceLoader};

use crate::clock::{Advance, Clock};
use crate::config::{EngineConfig, EngineMode, SharedSources};
use crate::error::EngineError;
use crate::hooks::{FrameStats, NoSimulation, Simulation};
use crate::output::StdOutput;

/// Whether the game runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RunState {
    /// Not started, or stopped: no scripts run and game time stands still. Frames still take input and draw
    /// (Studio's edit mode).
    #[default]
    Stopped,
    /// Started: every frame runs the script phases and the simulation.
    Running,
    /// Started but paused (Studio, M6): frames take input and draw, game time stands still, nothing runs until
    /// [`Engine::resume`] or [`Engine::step_frame`].
    Paused,
}

/// `RunService` and the ids of its frame events, looked up once.
#[derive(Clone, Copy, Debug)]
struct RunService {
    id: Id,
    render_stepped: SignalId,
    pre_simulation: SignalId,
    post_simulation: SignalId,
    heartbeat: SignalId,
}

impl RunService {
    fn find(dm: &DataModel) -> Result<Self, GameError> {
        let class = dm
            .registry()
            .class_id("RunService")
            .ok_or(GameError::MissingClass("RunService"))?;
        let id = dm.service(class).ok_or(GameError::MissingClass("RunService"))?;
        Ok(Self {
            id,
            render_stepped: dm.signal_id(id, Phase::RenderStepped.name())?,
            pre_simulation: dm.signal_id(id, Phase::PreSimulation.name())?,
            post_simulation: dm.signal_id(id, Phase::PostSimulation.name())?,
            heartbeat: dm.signal_id(id, Phase::Heartbeat.name())?,
        })
    }
}

/// The engine: one game (its [`DataModel`]), the [`ScriptHost`] that runs its scripts (with the Luau backend under
/// the `luau` feature), the game clock, and the frame loop that ties them to the simulation and the renderer.
///
/// `O` is where script output goes: [`StdOutput`] by default.
///
/// # Driving it
///
/// - **Headless** (servers, tests, tools): [`Engine::tick`] runs one frame of exactly one simulation step,
///   [`Engine::run_headless`] runs many, [`Engine::frame`] runs one frame of a real time step.
/// - **Windowed** (`render` feature): [`run_windowed`](crate::run_windowed) drives [`Engine::frame`] from the
///   platform's event loop with the wall clock.
///
/// The crate docs give the frame's order.
pub struct Engine<O: OutputSink = StdOutput> {
    dm: DataModel,
    host: ScriptHost<O>,
    context: RunContext,
    mode: EngineMode,
    studio: bool,
    sources: Rc<dyn SourceLoader>,
    clock: Clock,
    state: RunState,
    frames: u64,
    last: FrameStats,
    run_service: RunService,
    user_input_service: Id,
    simulation: Box<dyn Simulation>,
    #[cfg(feature = "render")]
    input: crate::input::InputQueue,
    #[cfg(feature = "render")]
    user_input: crate::user_input::UserInput,
    #[cfg(feature = "render")]
    window: Option<metallic_platform::Window>,
    #[cfg(feature = "render")]
    render_hook: Option<Box<dyn crate::hooks::RenderHook>>,
}

impl Engine<StdOutput> {
    /// A new game ([`metallic_classes::new_game`]) whose script output goes to the standard streams.
    pub fn new(config: EngineConfig) -> Result<Self, EngineError> {
        Self::with_output(config, StdOutput)
    }
}

impl<O: OutputSink> Engine<O> {
    /// A new game ([`metallic_classes::new_game`]) whose script output goes to `output`.
    pub fn with_output(config: EngineConfig, output: O) -> Result<Self, EngineError> {
        Self::with_data_model(metallic_classes::new_game()?, config, output)
    }

    /// An engine for an existing game (a loaded scene, a Studio copy), whose script output goes to `output`. The
    /// game gets a `RunService` if it lacks one.
    pub fn with_data_model(mut dm: DataModel, config: EngineConfig, output: O) -> Result<Self, EngineError> {
        if let Some(why) = config.invalid() {
            return Err(EngineError::Config(why));
        }
        #[cfg(not(feature = "render"))]
        if config.mode == EngineMode::Windowed {
            return Err(EngineError::Mode(
                "a windowed engine needs the `render` feature; this build is headless",
            ));
        }
        #[allow(unused_mut, reason = "only the `luau` feature registers a backend")]
        let mut host = ScriptHost::with_output(output);
        #[cfg(feature = "luau")]
        host.register_language(Box::new(metallic_luau::LuauLanguage::new(std::sync::Arc::clone(
            dm.registry(),
        ))?))?;
        set_run_flags(&mut dm, run_flags(config.context, config.studio, false))?;
        let run_service = RunService::find(&dm)?;
        // Made whatever the build: it creates `UserInputService` in a game that lacks one. Only a build with a window
        // has input to feed it.
        #[cfg(feature = "render")]
        let user_input = crate::user_input::UserInput::new(&mut dm)?;
        #[cfg(feature = "render")]
        let user_input_service = user_input.service();
        #[cfg(not(feature = "render"))]
        let user_input_service = metallic_classes::input::feed::InputFeed::new(&mut dm)?.service();
        Ok(Self {
            dm,
            host,
            context: config.context,
            mode: config.mode,
            studio: config.studio,
            sources: config.sources,
            clock: Clock::new(config.tick_rate, config.max_catch_up, config.max_frame_time),
            state: RunState::Stopped,
            frames: 0,
            last: FrameStats::default(),
            run_service,
            user_input_service,
            simulation: Box::new(NoSimulation),
            #[cfg(feature = "render")]
            input: crate::input::InputQueue::default(),
            #[cfg(feature = "render")]
            user_input,
            #[cfg(feature = "render")]
            window: None,
            #[cfg(feature = "render")]
            render_hook: None,
        })
    }

    /// Starts the game: `RunService:IsRunning()` becomes true, then every script of the context starts
    /// ([`ScriptHost::start_game`]: their top-level code runs now, before the first frame). Fails when the game
    /// already runs, and when the host refuses; a script that fails is only reported to the output.
    pub fn start(&mut self) -> Result<(), EngineError> {
        if self.state != RunState::Stopped {
            return Err(EngineError::AlreadyRunning);
        }
        self.write_flags(true)?;
        let sources = SharedSources(Rc::clone(&self.sources));
        if let Err(error) = self.host.start_game(&mut self.dm, self.context, sources) {
            self.write_flags(false)?;
            return Err(error.into());
        }
        self.state = RunState::Running;
        Ok(())
    }

    /// Stops the game: every script is reaped and `RunService:IsRunning()` becomes false. Game time keeps its value
    /// (it never goes back). Returns `false` when the game was not running.
    pub fn stop(&mut self) -> bool {
        if self.state == RunState::Stopped {
            return false;
        }
        self.host.stop_game(&mut self.dm);
        self.state = RunState::Stopped;
        if let Err(error) = self.write_flags(false) {
            log::warn!("could not clear RunService's running flag: {error}");
        }
        true
    }

    /// Pauses a running game: from the next frame, game time stands still and no script or simulation step runs.
    /// Returns `false` when the game was not running (stopped, or already paused).
    pub fn pause(&mut self) -> bool {
        let pausing = self.state == RunState::Running;
        if pausing {
            self.state = RunState::Paused;
        }
        pausing
    }

    /// Resumes a paused game. Returns `false` when it was not paused.
    pub fn resume(&mut self) -> bool {
        let resuming = self.state == RunState::Paused;
        if resuming {
            self.state = RunState::Running;
        }
        resuming
    }

    /// Runs one whole frame of one fixed step now, even while paused (Studio's "step" button); the game stays in
    /// its state. Returns `None`, and runs nothing, when the game is stopped.
    pub fn step_frame(&mut self) -> Option<FrameStats> {
        if self.state == RunState::Stopped {
            return None;
        }
        let advance = self.clock.advance(self.clock.fixed_step());
        Some(self.run_frame(Some(advance)))
    }

    /// Runs one frame for `real_dt` of real time (the time since the previous frame): input, then, while the game
    /// runs, the script phases and as many fixed simulation steps as are due, then rendering. Game time moves by
    /// `real_dt`, clamped to [`EngineConfig::max_frame_time`]; at most [`EngineConfig::max_catch_up`] steps run.
    pub fn frame(&mut self, real_dt: Duration) -> FrameStats {
        let advance = (self.state == RunState::Running).then(|| self.clock.advance(real_dt));
        self.run_frame(advance)
    }

    /// Runs one frame of exactly one fixed step: the headless loop of servers and tests, where game time follows
    /// the tick count rather than the wall clock.
    pub fn tick(&mut self) -> FrameStats {
        self.frame(self.clock.fixed_step())
    }

    /// Runs `ticks` frames of one fixed step each, as fast as they run, with no window and no platform loop.
    /// Returns the last frame's numbers.
    pub fn run_headless(&mut self, ticks: u64) -> FrameStats {
        for _ in 0..ticks {
            self.tick();
        }
        self.last
    }

    /// The frame, in the order `metallic_script`'s crate docs set (see this crate's docs for the table).
    fn run_frame(&mut self, advance: Option<Advance>) -> FrameStats {
        profiling::scope!("Engine::frame");
        self.frames += 1;
        {
            profiling::scope!("input");
            #[cfg(feature = "render")]
            self.input_step(advance.is_some());
        }
        let mut stats = FrameStats {
            frame: self.frames,
            ..FrameStats::default()
        };
        if let Some(advance) = advance {
            let dt = advance.dt.as_secs_f64();
            let now = self.clock.game_time().as_secs_f64();
            let rs = self.run_service;
            let mut resumed = 0;
            if self.context != RunContext::Server {
                profiling::scope!("RenderStepped");
                resumed += self.phase(Phase::RenderStepped, rs.render_stepped, dt, now);
            }
            {
                profiling::scope!("PreSimulation");
                resumed += self.phase(Phase::PreSimulation, rs.pre_simulation, dt, now);
            }
            {
                profiling::scope!("Simulation");
                let fixed = self.clock.fixed_step();
                for _ in 0..advance.steps {
                    self.simulation.step(&mut self.dm, fixed);
                }
            }
            {
                profiling::scope!("PostSimulation");
                resumed += self.phase(Phase::PostSimulation, rs.post_simulation, dt, now);
            }
            {
                profiling::scope!("Heartbeat");
                resumed += self.phase(Phase::Heartbeat, rs.heartbeat, dt, now);
            }
            stats.dt = dt;
            stats.sim_steps = advance.steps;
            stats.dropped_steps = advance.dropped;
            stats.ran = true;
            stats.resumed = resumed;
        }
        stats.game_time = self.clock.game_time().as_secs_f64();
        stats.alpha = self.clock.alpha();
        {
            profiling::scope!("render");
            #[cfg(feature = "render")]
            self.render(stats);
        }
        self.last = stats;
        profiling::finish_frame!();
        stats
    }

    /// One script phase: fires RunService's event of that name (only when something listens to `RunService`, so
    /// an unwatched frame builds no argument list), then runs the host's step, where its listeners run.
    fn phase(&mut self, phase: Phase, signal: SignalId, dt: f64, now: f64) -> usize {
        let id = self.run_service.id;
        if self.dm.is_watched(id)
            && let Err(error) = self.dm.fire(id, signal, vec![ScriptValue::Number(dt)])
        {
            log::warn!("could not fire RunService.{}: {error}", phase.name());
        }
        self.host.step(&mut self.dm, phase, now).resumed
    }

    fn write_flags(&mut self, running: bool) -> Result<(), GameError> {
        set_run_flags(&mut self.dm, run_flags(self.context, self.studio, running))
    }

    /// Replaces the fixed-step simulation (physics, M7). The engine starts with [`NoSimulation`].
    pub fn set_simulation(&mut self, simulation: Box<dyn Simulation>) {
        self.simulation = simulation;
    }

    /// The game.
    pub fn data_model(&self) -> &DataModel {
        &self.dm
    }

    /// The game, to change it (build a scene, add scripts before [`Engine::start`]).
    pub fn data_model_mut(&mut self) -> &mut DataModel {
        &mut self.dm
    }

    /// The script host (its output, thread counts, budgets).
    pub fn host(&self) -> &ScriptHost<O> {
        &self.host
    }

    /// The script host, to change its budgets or read its output.
    pub fn host_mut(&mut self) -> &mut ScriptHost<O> {
        &mut self.host
    }

    /// The game and the script host at once, for code that runs scripts by hand.
    pub fn parts_mut(&mut self) -> (&mut DataModel, &mut ScriptHost<O>) {
        (&mut self.dm, &mut self.host)
    }

    /// The game's `RunService`.
    pub fn run_service(&self) -> Id {
        self.run_service.id
    }

    /// The game's `UserInputService`.
    pub fn user_input_service(&self) -> Id {
        self.user_input_service
    }

    /// The side this engine runs.
    pub fn context(&self) -> RunContext {
        self.context
    }

    /// Headless or windowed.
    pub fn mode(&self) -> EngineMode {
        self.mode
    }

    /// Whether the game runs, is paused or is stopped.
    pub fn state(&self) -> RunState {
        self.state
    }

    /// Game time in seconds: what `time()` returns in scripts and the clock `task.wait` uses. It moves only while
    /// the game runs, and never goes back.
    pub fn game_time(&self) -> f64 {
        self.clock.game_time().as_secs_f64()
    }

    /// One simulation step: `1 / tick_rate` seconds, rounded up to whole nanoseconds.
    pub fn fixed_step(&self) -> Duration {
        self.clock.fixed_step()
    }

    /// How many frames have run.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// The numbers of the last frame.
    pub fn last_frame(&self) -> FrameStats {
        self.last
    }
}

#[cfg(feature = "render")]
impl<O: OutputSink> Engine<O> {
    /// Installs the renderer, replacing (and detaching) any previous one. If a window is attached, the new hook
    /// attaches to it at once.
    pub fn set_render_hook(&mut self, mut hook: Box<dyn crate::hooks::RenderHook>) -> Result<(), EngineError> {
        if let Some(mut old) = self.render_hook.take() {
            old.detach();
        }
        if let Some(window) = &self.window {
            hook.attach(window).map_err(EngineError::Render)?;
        }
        self.render_hook = Some(hook);
        Ok(())
    }

    /// The window to draw into: the render hook attaches to it. Replaces (and detaches from) any previous window.
    pub fn attach_window(&mut self, window: metallic_platform::Window) -> Result<(), EngineError> {
        self.detach_window();
        if let Some(hook) = self.render_hook.as_mut() {
            hook.attach(&window).map_err(EngineError::Render)?;
        }
        self.window = Some(window);
        self.user_input.forget_window();
        Ok(())
    }

    /// Lets go of the window (the app was suspended, or is exiting): the render hook drops its surface and nothing
    /// is drawn until a window is attached again. Returns the window.
    pub fn detach_window(&mut self) -> Option<metallic_platform::Window> {
        let window = self.window.take();
        self.user_input.forget_window();
        if window.is_some()
            && let Some(hook) = self.render_hook.as_mut()
        {
            hook.detach();
        }
        window
    }

    /// The window being drawn into, if one is attached.
    pub fn window(&self) -> Option<&metallic_platform::Window> {
        self.window.as_ref()
    }

    /// The attached window's drawable area changed: tells the render hook.
    pub fn resize(&mut self, size: metallic_platform::Size, scale_factor: f64) {
        if let Some(hook) = self.render_hook.as_mut() {
            hook.resize(size, scale_factor);
        }
    }

    /// Queues an input event for the next frame's input step.
    pub fn push_input(&mut self, event: crate::input::InputEvent) {
        self.input.push(event);
    }

    /// The input the current frame took in its input step, in arrival order. Valid from the frame's input step
    /// until the next frame starts; `UserInputService` has already taken it by then.
    pub fn input_events(&self) -> &[crate::input::InputEvent] {
        self.input.frame_events()
    }

    /// The frame's input step: the queued events become the frame's, `UserInputService` takes them (firing its
    /// events only when the game runs this frame, `firing`), and the cursor state scripts asked for reaches the
    /// window.
    fn input_step(&mut self, firing: bool) {
        self.input.begin_frame();
        self.user_input.process(&mut self.dm, self.input.frame_events(), firing);
        self.user_input.apply_cursor(&self.dm, self.window.as_ref());
    }

    /// The frame's last step: the render hook draws, when the engine is windowed and has a window.
    fn render(&mut self, stats: FrameStats) {
        if self.mode != EngineMode::Windowed || self.window.is_none() {
            return;
        }
        if let Some(hook) = self.render_hook.as_mut() {
            hook.render(&mut crate::hooks::RenderFrame {
                dm: &mut self.dm,
                stats,
            });
        }
    }
}

impl<O: OutputSink> fmt::Debug for Engine<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("context", &self.context)
            .field("mode", &self.mode)
            .field("state", &self.state)
            .field("frames", &self.frames)
            .field("game_time", &self.clock.game_time())
            .finish_non_exhaustive()
    }
}

/// What `RunService` reports for `context`: a standalone game is both client and server.
fn run_flags(context: RunContext, studio: bool, running: bool) -> RunFlags {
    RunFlags {
        studio,
        running,
        client: matches!(context, RunContext::Client | RunContext::Local),
        server: matches!(context, RunContext::Server | RunContext::Local),
    }
}
