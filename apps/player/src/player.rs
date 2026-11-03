//! [`Player`] and [`PlayerApp`]: a game in the engine, and the event loop handler that runs it in a window.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use metallic_engine::metallic_core::DmError;
use metallic_engine::metallic_platform::{
    AppHandler, DeviceEvent, EventLoopContext, GamepadEvent, Platform, PlatformError, Size, Window, WindowEvent,
    WindowId,
};
use metallic_engine::metallic_script::OutputSink;
use metallic_engine::{
    Engine, EngineConfig, EngineError, EngineMode, FrameStats, RenderFrame, RenderHook, RenderHookError, RunContext,
    SceneRenderer, StdOutput, WindowedApp, WindowedOptions,
};

use crate::config::PlayerConfig;
use crate::stats::{Draws, Recorder, Report};

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
    /// The class registry lacks a class the scene needs.
    #[error("the game has no {0} class")]
    MissingClass(&'static str),
}

/// A game running in the engine: the embeddable Player.
///
/// [`Player::new`] makes the engine (a standalone [`RunContext::Local`] game, windowed), loads the scene, installs
/// the renderer and starts the game, so the scene's scripts have run their top-level code when it returns. What
/// drives the frames is up to the host; see the crate docs for the three ways.
pub struct Player<O: OutputSink = StdOutput> {
    engine: Engine<O>,
    config: PlayerConfig,
    recorder: Rc<RefCell<Recorder>>,
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
        config.scene.install(engine.data_model_mut())?;
        let recorder = Rc::new(RefCell::new(Recorder::new(config.warmup_frames)));
        engine.set_render_hook(Box::new(RecordingRenderer {
            inner: SceneRenderer::default(),
            recorder: Rc::clone(&recorder),
        }))?;
        if let Some(preset) = config.preset {
            log::info!("preset `{preset}` asked for; graphics settings are not applied until #84");
        }
        engine.start()?;
        Ok(Self {
            engine,
            config,
            recorder,
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

    /// Runs one frame of `real_dt` of wall time, for a host that owns both the window and the loop (the engine's
    /// window comes from [`Engine::attach_window`] then). Frame times are the host's to measure.
    pub fn frame(&mut self, real_dt: Duration) -> FrameStats {
        self.engine.frame(real_dt)
    }

    /// What has been recorded so far: the draws of the last frame and, under [`PlayerApp`], the frame times.
    pub fn samples(&self) -> Recorder {
        self.recorder.borrow().clone()
    }

    /// An event loop handler that opens [`PlayerConfig::window`] and runs this game in it, for a host that owns the
    /// event loop but lets the Player own its window (the Launcher).
    pub fn into_app(self) -> Result<PlayerApp<O>, PlayerError> {
        let mut options = WindowedOptions::new(self.config.window.clone());
        if let Some(limit) = self.config.exit_after {
            options = options.with_exit_after(limit);
        }
        // The cap is applied by `PlayerApp`, after it has timed the frame, so the sleep never counts as CPU time.
        Ok(PlayerApp {
            app: WindowedApp::new(self.engine, options)?,
            config: self.config,
            recorder: self.recorder,
            previous_start: None,
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
/// It is the engine's [`WindowedApp`] (window, input, frames) plus what the Player adds: frame timing for
/// `--bench`, the frame cap, and stopping after [`PlayerConfig::max_frames`]. [`Player::run`] hands it to
/// [`Platform::run`]; a host with its own event loop forwards its callbacks here instead.
pub struct PlayerApp<O: OutputSink = StdOutput> {
    app: WindowedApp<O>,
    config: PlayerConfig,
    recorder: Rc<RefCell<Recorder>>,
    /// When the previous frame started, to time the next one from.
    previous_start: Option<Instant>,
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
        Report {
            scene: self.config.scene,
            preset: self.config.preset,
            warmup_frames: self.config.warmup_frames,
            total_frames: engine.frame_count(),
            wall_time,
            game_time: engine.game_time(),
            samples: self.recorder.borrow().clone(),
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
        // Time spent in the background is not a frame.
        self.previous_start = None;
        self.app.resumed(cx);
    }

    fn suspended(&mut self, cx: &mut dyn EventLoopContext) {
        self.app.suspended(cx);
    }

    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
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
        self.app.about_to_wait(cx);
        let frame = self.app.engine().frame_count();
        if frame == before {
            // No frame ran: the first iteration only reads the clock, or the app is suspended or exiting.
            return;
        }
        let end = Instant::now();
        self.first_frame.get_or_insert(start);
        self.last_frame_end = Some(end);
        {
            let mut recorder = self.recorder.borrow_mut();
            if recorder.measures(frame) {
                recorder.cpu_ms.push(millis(end.duration_since(start)));
                if let Some(previous) = self.previous_start {
                    recorder.frame_ms.push(millis(start.duration_since(previous)));
                }
            }
        }
        self.previous_start = Some(start);

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

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The engine's [`SceneRenderer`], recording what each frame drew and how long the GPU took.
struct RecordingRenderer {
    inner: SceneRenderer,
    recorder: Rc<RefCell<Recorder>>,
}

impl RenderHook for RecordingRenderer {
    fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
        self.inner.attach(window)?;
        if let Some(renderer) = self.inner.renderer() {
            log::info!("renderer attached, {:?}", renderer.settings().color_format);
        }
        Ok(())
    }

    fn detach(&mut self) {
        self.inner.detach();
    }

    fn resize(&mut self, size: Size, scale_factor: f64) {
        self.inner.resize(size, scale_factor);
        if size.width > 0 && size.height > 0 {
            self.recorder.borrow_mut().surface = Some((size.width, size.height));
        }
    }

    fn render(&mut self, frame: &mut RenderFrame<'_>) {
        self.inner.render(frame);
        let stats = self.inner.draw_stats();
        let mut recorder = self.recorder.borrow_mut();
        recorder.draws = Draws {
            prepass: stats.prepass_draws,
            opaque: stats.opaque_draws,
            transparent: stats.transparent_draws,
            instances: stats.instances,
            triangles: stats.triangles,
        };
        if !recorder.measures(frame.stats.frame) {
            return;
        }
        let Some(timings) = self.inner.renderer().and_then(|renderer| renderer.gpu_timings()) else {
            return;
        };
        if recorder.last_gpu_frame == Some(timings.frame()) {
            return;
        }
        recorder.last_gpu_frame = Some(timings.frame());
        recorder.gpu_ms.push(f64::from(timings.total_ms()));
        recorder.gpu_passes.clear();
        recorder
            .gpu_passes
            .extend(timings.passes().iter().map(|pass| (pass.pass, f64::from(pass.gpu_ms))));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use metallic_engine::metallic_platform::raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use metallic_engine::metallic_platform::{CursorGrab, LoopMode, WindowBackend, WindowConfig, WindowMode};
    use metallic_engine::metallic_script::CollectOutput;

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

    /// A render hook that draws nothing: a fake window has no surface for the real renderer.
    struct NoDrawing;

    impl RenderHook for NoDrawing {
        fn render(&mut self, _frame: &mut RenderFrame<'_>) {}
    }

    #[test]
    fn a_bench_run_measures_its_frames_after_the_warmup_then_exits() {
        let config = PlayerConfig::bench(Scene::Demo, 5).with_warmup_frames(3);
        let mut player = Player::with_output(config, CollectOutput::new()).expect("player");
        player.engine_mut().set_render_hook(Box::new(NoDrawing)).expect("hook");
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
        assert_eq!(report.samples.cpu_ms.len(), 5);
        assert_eq!(
            report.samples.frame_ms.len(),
            5,
            "the first measured frame is timed from the last warm-up one"
        );
        assert!(report.wall_time > Duration::ZERO);
        assert!(report.game_time > 0.0);
    }
}
