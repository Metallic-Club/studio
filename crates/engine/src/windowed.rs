//! The windowed driver: an [`AppHandler`] that runs [`Engine`] frames from the platform's event loop.

use std::time::{Duration, Instant};

use metallic_platform::{
    AppHandler, DeviceEvent, EventLoopContext, GamepadEvent, LoopMode, Platform, Window, WindowConfig, WindowEvent,
    WindowId,
};
use metallic_script::OutputSink;

use crate::config::EngineMode;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::input::InputEvent;

/// How [`run_windowed`] runs.
#[derive(Clone, Debug)]
pub struct WindowedOptions {
    /// The window to open.
    pub window: WindowConfig,
    /// The most frames per second, or `None` for no cap. A render hook presenting with vsync paces the loop by
    /// itself; without one (or with vsync off) the loop would spin as fast as it can, so set a cap.
    pub frame_cap: Option<u32>,
    /// Exit on its own after this much wall time, for smoke tests and benchmarks; `None` runs until the window is
    /// closed.
    pub exit_after: Option<Duration>,
}

impl WindowedOptions {
    /// A window from `window`, with no frame cap and no time limit.
    pub fn new(window: WindowConfig) -> Self {
        Self {
            window,
            frame_cap: None,
            exit_after: None,
        }
    }

    /// Sets [`WindowedOptions::frame_cap`].
    pub fn with_frame_cap(mut self, fps: u32) -> Self {
        self.frame_cap = Some(fps);
        self
    }

    /// Sets [`WindowedOptions::exit_after`].
    pub fn with_exit_after(mut self, wall_time: Duration) -> Self {
        self.exit_after = Some(wall_time);
        self
    }
}

/// Opens a window and runs `engine` in the platform's event loop until the window is closed, then hands the engine
/// back (stopped). Call it on the main thread. The engine must be [`EngineMode::Windowed`]; start the game first
/// ([`Engine::start`]) to run scripts, or leave it stopped to only draw.
pub fn run_windowed<O: OutputSink>(engine: Engine<O>, options: WindowedOptions) -> Result<Engine<O>, EngineError> {
    let mut app = WindowedApp::new(engine, options)?;
    Platform::new()?.run(&mut app)?;
    match app.error.take() {
        Some(error) => Err(error),
        None => Ok(app.into_engine()),
    }
}

/// Drives an [`Engine`] from the platform's event loop: an [`AppHandler`].
///
/// - **Frames:** one per loop iteration, in [`AppHandler::about_to_wait`], timed with the wall clock; the loop polls
///   ([`LoopMode::Poll`]) while the app is in the foreground. The first iteration after (re)starting only reads the
///   clock, so time spent suspended or opening the window is not game time.
/// - **Input:** keyboard, mouse, touch and focus events, raw device motion and gamepad events go to the engine's
///   input queue ([`Engine::push_input`]) for the next frame.
/// - **Window:** opened at the first [`AppHandler::resumed`]; resizes and scale changes go to the render hook.
/// - **Suspend and resume** (mobile): the render hook lets go of the surface and the loop waits instead of polling;
///   no frames run until the app is resumed.
/// - **Exit:** closing the window (or [`WindowedOptions::exit_after`]) stops the loop; the engine detaches the window
///   and stops the game.
///
/// [`run_windowed`] is the usual way to use it; apps that need their own handling (Studio's panels, the Player's
/// menus) wrap it and forward the callbacks.
pub struct WindowedApp<O: OutputSink> {
    engine: Engine<O>,
    options: WindowedOptions,
    window: Option<Window>,
    /// When the previous frame started; `None` until the first frame after (re)starting.
    last: Option<Instant>,
    started: Option<Instant>,
    suspended: bool,
    focused: bool,
    error: Option<EngineError>,
}

impl<O: OutputSink> WindowedApp<O> {
    /// A driver for `engine`, which must be [`EngineMode::Windowed`].
    pub fn new(engine: Engine<O>, options: WindowedOptions) -> Result<Self, EngineError> {
        if engine.mode() != EngineMode::Windowed {
            return Err(EngineError::Mode(
                "the windowed driver needs an engine made with EngineMode::Windowed",
            ));
        }
        Ok(Self {
            engine,
            options,
            window: None,
            last: None,
            started: None,
            suspended: false,
            focused: false,
            error: None,
        })
    }

    /// The engine.
    pub fn engine(&self) -> &Engine<O> {
        &self.engine
    }

    /// The engine, to change it between frames.
    pub fn engine_mut(&mut self) -> &mut Engine<O> {
        &mut self.engine
    }

    /// Whether the window has keyboard focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The error that stopped the loop, if one did (the window or the render hook failed).
    pub fn take_error(&mut self) -> Option<EngineError> {
        self.error.take()
    }

    /// The engine back.
    pub fn into_engine(self) -> Engine<O> {
        self.engine
    }

    fn fail(&mut self, cx: &mut dyn EventLoopContext, error: EngineError) {
        log::error!("{error}");
        self.error = Some(error);
        cx.exit();
    }

    fn input(&mut self, event: InputEvent) {
        self.engine.push_input(event);
    }
}

impl<O: OutputSink> AppHandler for WindowedApp<O> {
    fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
        cx.set_loop_mode(LoopMode::Poll);
        self.suspended = false;
        self.last = None;
        let window = match &self.window {
            Some(window) => window.clone(),
            None => match cx.create_window(&self.options.window) {
                Ok(window) => {
                    self.window = Some(window.clone());
                    window
                }
                Err(error) => return self.fail(cx, error.into()),
            },
        };
        let (size, scale) = (window.inner_size(), window.scale_factor());
        if let Err(error) = self.engine.attach_window(window) {
            return self.fail(cx, error);
        }
        self.engine.resize(size, scale);
    }

    fn suspended(&mut self, cx: &mut dyn EventLoopContext) {
        self.suspended = true;
        self.engine.detach_window();
        // Nothing runs in the background, so do not spin.
        cx.set_loop_mode(LoopMode::Wait);
    }

    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => cx.exit(),
            WindowEvent::Resized(size) => {
                let scale = self.window.as_ref().map_or(1.0, Window::scale_factor);
                self.engine.resize(size, scale);
            }
            WindowEvent::ScaleFactorChanged { scale_factor } => {
                if let Some(size) = self.window.as_ref().map(Window::inner_size) {
                    self.engine.resize(size, scale_factor);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                // Input code needs it too: keys held when focus leaves never report their release.
                self.input(InputEvent::Window { window, event });
            }
            // Drawing happens in `about_to_wait`, once per loop iteration; occlusion is the renderer's to read.
            WindowEvent::RedrawRequested | WindowEvent::Occluded(_) => {}
            event => self.input(InputEvent::Window { window, event }),
        }
    }

    fn device_event(&mut self, _cx: &mut dyn EventLoopContext, event: DeviceEvent) {
        self.input(InputEvent::Device(event));
    }

    fn gamepad_event(&mut self, _cx: &mut dyn EventLoopContext, event: GamepadEvent) {
        self.input(InputEvent::Gamepad(event));
    }

    fn about_to_wait(&mut self, cx: &mut dyn EventLoopContext) {
        if self.suspended || self.error.is_some() {
            return;
        }
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        if let Some(limit) = self.options.exit_after
            && now.duration_since(started) >= limit
        {
            cx.exit();
            return;
        }
        let Some(last) = self.last.replace(now) else {
            return;
        };
        self.engine.frame(now.duration_since(last));
        if let Some(fps) = self.options.frame_cap.filter(|&fps| fps > 0) {
            let budget = Duration::from_secs(1) / fps;
            let spent = now.elapsed();
            if spent < budget {
                std::thread::sleep(budget - spent);
            }
        }
    }

    fn exiting(&mut self) {
        self.engine.detach_window();
        self.window = None;
        self.engine.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use metallic_platform::raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };
    use metallic_platform::{CursorGrab, PlatformError, Position, Size, WindowBackend, WindowMode};
    use metallic_script::{CollectOutput, RunContext};

    use super::*;
    use crate::config::EngineConfig;
    use crate::hooks::{RenderFrame, RenderHook, RenderHookError};
    use crate::{FrameStats, RunState};

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
            WindowId::from_raw(7)
        }
        fn inner_size(&self) -> Size {
            Size::new(800, 600)
        }
        fn scale_factor(&self) -> f64 {
            1.5
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

    /// An event loop context with no OS behind it.
    #[derive(Default)]
    struct FakeLoop {
        mode: Option<LoopMode>,
        exit: bool,
        windows: usize,
    }

    impl EventLoopContext for FakeLoop {
        fn create_window(&mut self, _config: &WindowConfig) -> Result<Window, metallic_platform::PlatformError> {
            self.windows += 1;
            Ok(Window::from_backend(Arc::new(FakeWindow)))
        }
        fn set_loop_mode(&mut self, mode: LoopMode) {
            self.mode = Some(mode);
        }
        fn exit(&mut self) {
            self.exit = true;
        }
    }

    /// A render hook that records what it is asked to do.
    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<String>>>);

    impl Recorder {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.borrow_mut())
        }
    }

    impl RenderHook for Recorder {
        fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
            self.0.borrow_mut().push(format!("attach {}", window.id().to_raw()));
            Ok(())
        }
        fn detach(&mut self) {
            self.0.borrow_mut().push("detach".into());
        }
        fn resize(&mut self, size: Size, scale_factor: f64) {
            self.0
                .borrow_mut()
                .push(format!("resize {}x{} @{scale_factor}", size.width, size.height));
        }
        fn render(&mut self, frame: &mut RenderFrame<'_>) {
            let FrameStats { frame: n, ran, .. } = frame.stats;
            self.0.borrow_mut().push(format!("render {n} ran={ran}"));
        }
    }

    fn engine(mode: EngineMode) -> (Engine<CollectOutput>, Recorder) {
        let config = EngineConfig::new(RunContext::Local).with_mode(mode);
        let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
        let recorder = Recorder::default();
        engine.set_render_hook(Box::new(recorder.clone())).expect("hook");
        (engine, recorder)
    }

    fn focus(focused: bool) -> InputEvent {
        InputEvent::Window {
            window: WindowId::from_raw(7),
            event: WindowEvent::Focused(focused),
        }
    }

    #[test]
    fn the_driver_runs_frames_forwards_input_and_exits_cleanly() {
        let (mut engine, hook) = engine(EngineMode::Windowed);
        engine.start().expect("start");
        let mut app = WindowedApp::new(engine, WindowedOptions::new(WindowConfig::default())).expect("windowed");
        let mut cx = FakeLoop::default();
        let id = WindowId::from_raw(7);

        app.resumed(&mut cx);
        assert_eq!(cx.mode, Some(LoopMode::Poll));
        assert_eq!(cx.windows, 1);
        assert_eq!(hook.take(), ["attach 7", "resize 800x600 @1.5"]);

        // The first iteration only reads the clock.
        app.about_to_wait(&mut cx);
        assert_eq!(app.engine().frame_count(), 0);

        app.window_event(&mut cx, id, WindowEvent::Focused(true));
        app.window_event(&mut cx, id, WindowEvent::CursorMoved(Position { x: 1.0, y: 2.0 }));
        app.window_event(&mut cx, id, WindowEvent::RedrawRequested);
        app.about_to_wait(&mut cx);
        assert_eq!(app.engine().frame_count(), 1);
        assert!(app.focused());
        assert_eq!(app.engine().input_events().len(), 2, "focus and cursor, not the redraw");
        assert_eq!(app.engine().input_events()[0], focus(true));
        assert_eq!(hook.take(), ["render 1 ran=true"]);

        app.window_event(&mut cx, id, WindowEvent::Resized(Size::new(1024, 768)));
        assert_eq!(hook.take(), ["resize 1024x768 @1.5"]);

        app.window_event(&mut cx, id, WindowEvent::CloseRequested);
        assert!(cx.exit);
        app.exiting();
        assert_eq!(hook.take(), ["detach"]);
        let engine = app.into_engine();
        assert_eq!(engine.state(), RunState::Stopped);
        assert!(engine.window().is_none());
    }

    #[test]
    fn suspending_drops_the_surface_and_stops_frames_until_resumed() {
        let (engine, hook) = engine(EngineMode::Windowed);
        let mut app = WindowedApp::new(engine, WindowedOptions::new(WindowConfig::default())).expect("windowed");
        let mut cx = FakeLoop::default();
        app.resumed(&mut cx);
        app.about_to_wait(&mut cx);
        app.about_to_wait(&mut cx);
        assert_eq!(app.engine().frame_count(), 1);
        hook.take();

        app.suspended(&mut cx);
        assert_eq!(cx.mode, Some(LoopMode::Wait));
        assert_eq!(hook.take(), ["detach"]);
        app.about_to_wait(&mut cx);
        assert_eq!(app.engine().frame_count(), 1, "no frames in the background");

        app.resumed(&mut cx);
        assert_eq!(cx.windows, 1, "the same window is reused");
        assert_eq!(hook.take(), ["attach 7", "resize 800x600 @1.5"]);
        app.about_to_wait(&mut cx);
        app.about_to_wait(&mut cx);
        assert_eq!(app.engine().frame_count(), 2);
        assert_eq!(hook.take(), ["render 2 ran=false"], "a stopped game still draws");
    }

    #[test]
    fn the_driver_refuses_a_headless_engine() {
        let (engine, _) = engine(EngineMode::Headless);
        assert!(matches!(
            WindowedApp::new(engine, WindowedOptions::new(WindowConfig::default())),
            Err(EngineError::Mode(_))
        ));
    }

    #[test]
    fn a_headless_engine_never_draws() {
        let (mut engine, hook) = engine(EngineMode::Headless);
        engine
            .attach_window(Window::from_backend(Arc::new(FakeWindow)))
            .expect("attach");
        engine.run_headless(3);
        assert_eq!(hook.take(), ["attach 7"]);
    }

    #[test]
    fn render_runs_last_and_input_lasts_one_frame() {
        let (mut engine, hook) = engine(EngineMode::Windowed);
        engine.frame(Duration::from_millis(16));
        assert!(hook.take().is_empty(), "no window, nothing to draw into");
        engine
            .attach_window(Window::from_backend(Arc::new(FakeWindow)))
            .expect("attach");
        engine.push_input(focus(true));
        assert!(engine.input_events().is_empty(), "queued for the next frame");
        engine.start().expect("start");
        engine.tick();
        assert_eq!(engine.input_events(), [focus(true)]);
        engine.pause();
        engine.tick();
        assert!(engine.input_events().is_empty());
        assert_eq!(hook.take(), ["attach 7", "render 2 ran=true", "render 3 ran=false"]);
    }
}
