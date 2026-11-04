//! [`Player`] and [`PlayerApp`]: a game in the engine, and the event loop handler that runs it in a window.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use metallic_engine::graphics::save_graphics_settings;
use metallic_engine::metallic_core::{DataModel, DmError};
use metallic_engine::metallic_platform::{
    AppHandler, ButtonState, DeviceEvent, EventLoopContext, GamepadEvent, KeyCode, Platform, PlatformError,
    WindowEvent, WindowId,
};
use metallic_engine::metallic_render::ContextOptions;
use metallic_engine::metallic_script::OutputSink;
use metallic_engine::perf::PerfSummary;
use metallic_engine::stress::{StressError, StressScene};
use metallic_engine::{
    Engine, EngineConfig, EngineError, EngineMode, FrameStats, RunContext, SceneRenderer, StdOutput, WindowedApp,
    WindowedOptions,
};

use crate::config::PlayerConfig;
use crate::stats::Report;

/// Why a [`Player`] could not be made or stopped early.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    /// The engine refused: its settings, a script host error, the window or the renderer.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The platform's event loop could not start.
    #[error(transparent)]
    Platform(#[from] PlatformError),
    /// The scene's instances could not be made.
    #[error(transparent)]
    Game(#[from] DmError),
    /// A stress scene could not be built.
    #[error(transparent)]
    Stress(#[from] StressError),
    /// The class registry lacks a class the scene needs.
    #[error("the game has no {0} class")]
    MissingClass(&'static str),
}

/// Seconds of a stress scene's motion per frame: its moving parts move by frame number, as in the engine's
/// `stress` example, so every run moves them the same way whatever its frame rate.
const STRESS_MOTION_FPS: f64 = 60.0;

/// Moves a stress scene's moving parts to where they are in frame `frame` (from 1).
fn animate(stress: Option<&StressScene>, dm: &mut DataModel, frame: u64) {
    if let Some(stress) = stress
        && let Err(error) = stress.animate(dm, frame as f64 / STRESS_MOTION_FPS)
    {
        log::warn!("could not move the stress scene's parts: {error}");
    }
}

/// Keeps a calibration the Auto preset just finished in `config`'s graphics settings, and saves them to
/// [`PlayerConfig::graphics_file`] when there is one, so the next run starts calibrated.
fn save_calibration(renderer: &RefCell<SceneRenderer>, config: &mut PlayerConfig) {
    let Some(calibration) = renderer.borrow_mut().take_calibration() else {
        return;
    };
    config.graphics.calibration = Some(calibration);
    if let Some(path) = config.graphics_file.as_deref() {
        match save_graphics_settings(path, &config.graphics) {
            Ok(()) => log::info!("graphics: saved the calibration to {}", path.display()),
            Err(error) => log::warn!("graphics: could not save {}: {error}", path.display()),
        }
    }
}

/// A game running in the engine: the embeddable Player.
///
/// [`Player::new`] makes the engine (a standalone [`RunContext::Local`] game, windowed), loads the scene, installs
/// the renderer and starts the game, so the scene's scripts have run their top-level code when it returns. What
/// drives the frames is up to the host; see the crate docs for the three ways.
pub struct Player<O: OutputSink = StdOutput> {
    engine: Engine<O>,
    config: PlayerConfig,
    renderer: Rc<RefCell<SceneRenderer>>,
    stress: Option<StressScene>,
}

impl Player<StdOutput> {
    /// A player for `config` whose scripts print to the standard streams.
    pub fn new(config: PlayerConfig) -> Result<Self, PlayerError> {
        Self::with_output(config, StdOutput)
    }
}

impl<O: OutputSink> Player<O> {
    /// A player for `config` whose scripts print to `output` (Studio's Output panel, a test's collector).
    pub fn with_output(config: PlayerConfig, output: O) -> Result<Self, PlayerError> {
        let engine_config = EngineConfig::new(RunContext::Local)
            .with_mode(EngineMode::Windowed)
            .with_sources(config.scene.sources());
        let mut engine = Engine::with_output(engine_config, output)?;
        let stress = config.scene.install(engine.data_model_mut())?;
        let mut config = config;
        let mut renderer = SceneRenderer::new(ContextOptions {
            pipeline_cache_dir: config.pipeline_cache.clone(),
            ..ContextOptions::default()
        });
        if let Some(frames) = config.max_frames {
            // The whole measured run, so the report summarises every measured frame.
            renderer.set_perf_window(usize::try_from(frames).unwrap_or(usize::MAX).min(1 << 20));
        }
        renderer.set_overlay(config.overlay);
        let graphics = config.effective_graphics();
        // A benchmark keeps its own pacing (no cap); otherwise the settings' cap, 0 for none, replaces the default.
        if config.max_frames.is_none()
            && let Some(cap) = graphics.frame_cap
        {
            config.frame_cap = (cap > 0).then_some(cap);
        }
        log::info!("graphics: the {} preset", graphics.preset);
        renderer.set_graphics(graphics);
        renderer.set_frame_cap(config.frame_cap);
        // `UserGameSettings` holds the settings before the scripts' first lines run.
        renderer.write_user_settings(engine.data_model_mut());
        let renderer = Rc::new(RefCell::new(renderer));
        engine.set_render_hook(Box::new(Rc::clone(&renderer)))?;
        engine.start()?;
        Ok(Self {
            engine,
            config,
            renderer,
            stress,
        })
    }

    /// The settings it was made with.
    pub fn config(&self) -> &PlayerConfig {
        &self.config
    }

    /// The engine: the game, its state and its clock.
    pub fn engine(&self) -> &Engine<O> {
        &self.engine
    }

    /// The engine, for a host that drives it: attach its own window, push input, pause or stop the game.
    pub fn engine_mut(&mut self) -> &mut Engine<O> {
        &mut self.engine
    }

    /// The renderer the engine draws with: its frame statistics ([`SceneRenderer::perf`]), the performance
    /// overlay ([`SceneRenderer::set_overlay`]) and its settings.
    pub fn renderer(&self) -> &Rc<RefCell<SceneRenderer>> {
        &self.renderer
    }

    /// Runs one frame of `real_dt` of wall time, for a host that owns both the window and the loop (the engine's
    /// window comes from [`Engine::attach_window`] then). A stress scene's moving parts move first.
    pub fn frame(&mut self, real_dt: Duration) -> FrameStats {
        let next = self.engine.frame_count() + 1;
        animate(self.stress.as_ref(), self.engine.data_model_mut(), next);
        self.renderer
            .borrow_mut()
            .write_user_settings(self.engine.data_model_mut());
        let stats = self.engine.frame(real_dt);
        save_calibration(&self.renderer, &mut self.config);
        stats
    }

    /// The summary of the frames recorded so far (frame times, CPU and GPU times, draws): empty until the renderer
    /// has drawn into a window.
    pub fn summary(&self) -> PerfSummary {
        self.renderer.borrow_mut().perf_mut().summary()
    }

    /// An event loop handler that opens [`PlayerConfig::window`] and runs this game in it, for a host that owns the
    /// event loop but lets the Player own its window (the Launcher).
    pub fn into_app(self) -> Result<PlayerApp<O>, PlayerError> {
        let mut options = WindowedOptions::new(self.config.window.clone());
        if let Some(limit) = self.config.exit_after {
            options = options.with_exit_after(limit);
        }
        // The cap is applied by `PlayerApp`, after the frame, so `WindowedApp` runs uncapped.
        Ok(PlayerApp {
            app: WindowedApp::new(self.engine, options)?,
            config: self.config,
            renderer: self.renderer,
            stress: self.stress,
            first_frame: None,
            last_frame_end: None,
        })
    }

    /// Opens the window and runs the game until the window is closed (or [`PlayerConfig::max_frames`] or
    /// [`PlayerConfig::exit_after`] is reached), then reports what it measured. Call it on the main thread.
    pub fn run(self) -> Result<Report, PlayerError> {
        let mut app = self.into_app()?;
        Platform::new()?.run(&mut app)?;
        app.finish()
    }
}

impl<O: OutputSink> std::fmt::Debug for Player<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("config", &self.config)
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

/// Runs a [`Player`] in the platform's event loop: an [`AppHandler`].
///
/// It is the engine's [`WindowedApp`] (window, input, frames) plus what the Player adds: a stress scene's motion,
/// starting the measurement after [`PlayerConfig::warmup_frames`], the frame cap, stopping after
/// [`PlayerConfig::max_frames`], and F3 to show or hide the performance overlay. [`Player::run`] hands it to
/// [`Platform::run`]; a host with its own event loop forwards its callbacks here instead.
pub struct PlayerApp<O: OutputSink = StdOutput> {
    app: WindowedApp<O>,
    config: PlayerConfig,
    renderer: Rc<RefCell<SceneRenderer>>,
    stress: Option<StressScene>,
    first_frame: Option<Instant>,
    last_frame_end: Option<Instant>,
}

impl<O: OutputSink> PlayerApp<O> {
    /// The engine.
    pub fn engine(&self) -> &Engine<O> {
        self.app.engine()
    }

    /// The engine, to change it between frames.
    pub fn engine_mut(&mut self) -> &mut Engine<O> {
        self.app.engine_mut()
    }

    /// What the run has measured so far.
    pub fn report(&self) -> Report {
        let engine = self.app.engine();
        let wall_time = match (self.first_frame, self.last_frame_end) {
            (Some(first), Some(last)) => last.duration_since(first),
            _ => Duration::ZERO,
        };
        let mut renderer = self.renderer.borrow_mut();
        let surface = renderer.renderer().map(|r| (r.settings().width, r.settings().height));
        Report {
            scene: self.config.scene,
            preset: Some(self.config.effective_graphics().preset),
            graphics: renderer.graphics_status(),
            warmup_frames: self.config.warmup_frames,
            total_frames: engine.frame_count(),
            wall_time,
            game_time: engine.game_time(),
            surface,
            summary: renderer.perf_mut().summary(),
        }
    }

    /// Ends the run: the error that stopped the loop, if one did, or what it measured.
    pub fn finish(mut self) -> Result<Report, PlayerError> {
        match self.app.take_error() {
            Some(error) => Err(error.into()),
            None => Ok(self.report()),
        }
    }

    /// The engine back, once the loop has exited (the game is stopped then).
    pub fn into_engine(self) -> Engine<O> {
        self.app.into_engine()
    }
}

impl<O: OutputSink> AppHandler for PlayerApp<O> {
    fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
        self.app.resumed(cx);
    }

    fn suspended(&mut self, cx: &mut dyn EventLoopContext) {
        self.app.suspended(cx);
    }

    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
        if let WindowEvent::Key(key) = &event
            && key.key_code == KeyCode::F3
            && key.state == ButtonState::Pressed
            && !key.repeat
        {
            let mut renderer = self.renderer.borrow_mut();
            let on = !renderer.overlay();
            renderer.set_overlay(on);
        }
        self.app.window_event(cx, window, event);
    }

    fn device_event(&mut self, cx: &mut dyn EventLoopContext, event: DeviceEvent) {
        self.app.device_event(cx, event);
    }

    fn gamepad_event(&mut self, cx: &mut dyn EventLoopContext, event: GamepadEvent) {
        self.app.gamepad_event(cx, event);
    }

    fn about_to_wait(&mut self, cx: &mut dyn EventLoopContext) {
        let before = self.app.engine().frame_count();
        if let Some(frames) = self.config.max_frames
            && before >= self.config.warmup_frames.saturating_add(frames)
        {
            // The loop may come round once more after asking to exit; that iteration must not run a frame.
            cx.exit();
            return;
        }
        let start = Instant::now();
        animate(self.stress.as_ref(), self.app.engine_mut().data_model_mut(), before + 1);
        self.renderer
            .borrow_mut()
            .write_user_settings(self.app.engine_mut().data_model_mut());
        self.app.about_to_wait(cx);
        let frame = self.app.engine().frame_count();
        if frame == before {
            // No frame ran: the first iteration only reads the clock, or the app is suspended or exiting.
            return;
        }
        save_calibration(&self.renderer, &mut self.config);
        self.first_frame.get_or_insert(start);
        self.last_frame_end = Some(Instant::now());
        if frame == self.config.warmup_frames {
            // The renderer has recorded every frame so far; the measurement starts after the warm-up.
            self.renderer.borrow_mut().perf_mut().reset();
        }

        if let Some(frames) = self.config.max_frames
            && frame >= self.config.warmup_frames.saturating_add(frames)
        {
            cx.exit();
            return;
        }
        if let Some(fps) = self.config.frame_cap.filter(|&fps| fps > 0) {
            let budget = Duration::from_secs(1) / fps;
            let spent = start.elapsed();
            if spent < budget {
                std::thread::sleep(budget - spent);
            }
        }
    }

    fn exiting(&mut self) {
        self.app.exiting();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use metallic_engine::metallic_platform::raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use metallic_engine::metallic_platform::{
        CursorGrab, LoopMode, Size, Window, WindowBackend, WindowConfig, WindowMode,
    };
    use metallic_engine::metallic_script::CollectOutput;
    use metallic_engine::perf::FrameSample;
    use metallic_engine::{RenderFrame, RenderHook};

    use super::*;
    use crate::scene::Scene;

    /// A window with no OS behind it.
    struct FakeWindow;

    impl HasWindowHandle for FakeWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl HasDisplayHandle for FakeWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    impl WindowBackend for FakeWindow {
        fn id(&self) -> WindowId {
            WindowId::from_raw(1)
        }
        fn inner_size(&self) -> Size {
            Size::new(640, 360)
        }
        fn scale_factor(&self) -> f64 {
            1.0
        }
        fn set_title(&self, _title: &str) {}
        fn set_mode(&self, _mode: WindowMode) -> Result<(), PlatformError> {
            Ok(())
        }
        fn set_cursor_grab(&self, grab: CursorGrab) -> Result<CursorGrab, PlatformError> {
            Ok(grab)
        }
        fn set_cursor_visible(&self, _visible: bool) {}
        fn request_redraw(&self) {}
    }

    /// An event loop with no OS behind it.
    #[derive(Default)]
    struct FakeLoop {
        exit: bool,
    }

    impl EventLoopContext for FakeLoop {
        fn create_window(&mut self, _config: &WindowConfig) -> Result<Window, PlatformError> {
            Ok(Window::from_backend(Arc::new(FakeWindow)))
        }
        fn set_loop_mode(&mut self, _mode: LoopMode) {}
        fn exit(&mut self) {
            self.exit = true;
        }
    }

    /// A render hook that draws nothing (a fake window has no surface for the real renderer) but records a frame
    /// into the Player's statistics as the renderer would, with the frame number as its frame time.
    struct NoDrawing(Rc<RefCell<SceneRenderer>>);

    impl RenderHook for NoDrawing {
        fn render(&mut self, frame: &mut RenderFrame<'_>) {
            let sample = FrameSample {
                frame_ms: frame.stats.frame as f32,
                ..FrameSample::default()
            };
            self.0.borrow_mut().perf_mut().record(sample, None, Default::default());
        }
    }

    #[test]
    fn a_bench_run_measures_its_frames_after_the_warmup_then_exits() {
        let config = PlayerConfig::bench(Scene::Demo, 5).with_warmup_frames(3);
        let mut player = Player::with_output(config, CollectOutput::new()).expect("player");
        let renderer = Rc::clone(player.renderer());
        player
            .engine_mut()
            .set_render_hook(Box::new(NoDrawing(renderer)))
            .expect("hook");
        let mut app = player.into_app().expect("app");
        let mut cx = FakeLoop::default();
        app.resumed(&mut cx);
        let mut iterations = 0;
        while !cx.exit {
            app.about_to_wait(&mut cx);
            iterations += 1;
            assert!(iterations < 100, "the run never ended");
        }
        // The loop may come round once more after the exit was asked for: no frame runs then.
        app.about_to_wait(&mut cx);
        app.exiting();

        let report = app.finish().expect("no error");
        assert_eq!(report.total_frames, 8, "3 warm-up frames and 5 measured ones");
        assert_eq!(report.summary.frames, 5, "the warm-up frames are dropped");
        assert_eq!(report.summary.frame_ms.max, 8.0, "the last frame recorded");
        assert_eq!(report.summary.frame_ms.p50, 6.0);
        assert_eq!(report.surface, None, "no device, no drawn size");
        assert!(report.wall_time > Duration::ZERO);
        assert!(report.game_time > 0.0);
    }

    #[test]
    fn a_stress_scene_is_built_and_its_parts_move_every_frame() {
        use metallic_engine::metallic_core::Value;
        use metallic_engine::stress::StressKind;

        let config = PlayerConfig::new(Scene::Stress(StressKind::Mobile));
        assert_eq!(config.window.size, Size::new(1280, 720));
        let mut player = Player::with_output(config, CollectOutput::new()).expect("player");
        let dm = player.engine().data_model();
        let workspace = dm
            .service(dm.registry().class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let cframes = |dm: &DataModel| -> Vec<Value> {
            dm.descendants(workspace)
                .expect("descendants")
                .filter_map(|id| dm.get_by_name(id, "CFrame").ok())
                .collect()
        };
        let before = cframes(player.engine().data_model());
        assert!(before.len() >= 5_000, "{}", before.len());
        player.frame(Duration::from_millis(16));
        let after = cframes(player.engine().data_model());
        let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        // The workspace's camera is a descendant too; only the 1% moving parts change.
        assert_eq!(moved, 50);
    }
}
