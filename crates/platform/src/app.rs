//! The event loop driver ([`Platform`]), the callbacks it makes ([`AppHandler`]) and the backend seam behind it.

use std::fmt;

use crate::error::PlatformError;
use crate::event::{DeviceEvent, WindowEvent, WindowId};
use crate::gamepad::GamepadEvent;
use crate::paths::{AppId, UserPaths};
use crate::window::{Window, WindowConfig};

/// Whether the event loop runs continuously or sleeps until something happens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LoopMode {
    /// Run [`AppHandler::about_to_wait`] again as soon as it returns: games, which draw every frame.
    #[default]
    Poll,
    /// Sleep until an event arrives: tools that only redraw when something changes. Gamepads do not wake the loop.
    Wait,
}

/// What an [`AppHandler`] can ask of the running event loop. Passed to every callback.
pub trait EventLoopContext {
    /// Opens a window. Call it from [`AppHandler::resumed`] (or later); some platforms only allow windows while
    /// the app is resumed.
    fn create_window(&mut self, config: &WindowConfig) -> Result<Window, PlatformError>;
    /// Sets whether the loop polls or waits. The default is [`LoopMode::Poll`].
    fn set_loop_mode(&mut self, mode: LoopMode);
    /// Stops the loop after the current callback; [`AppHandler::exiting`] runs once more before [`Platform::run`]
    /// returns.
    fn exit(&mut self);
}

/// The engine's side of the event loop: the callbacks [`Platform::run`] makes.
///
/// The order on every platform: [`Self::resumed`] once the app may draw (create windows here), then for each loop
/// iteration the window, device and gamepad events, then [`Self::about_to_wait`] (where a game steps and draws).
/// Mobile apps go through [`Self::suspended`] and [`Self::resumed`] again when sent to the background and back;
/// desktop apps are resumed once. [`Self::exiting`] is last.
pub trait AppHandler {
    /// The app may draw: create windows and render surfaces. Called once at start on desktop, and every time the
    /// app returns to the foreground on mobile.
    fn resumed(&mut self, cx: &mut dyn EventLoopContext);

    /// The app went to the background (mobile): drop render surfaces, they are invalid until [`Self::resumed`].
    fn suspended(&mut self, cx: &mut dyn EventLoopContext) {
        let _ = cx;
    }

    /// Something happened to one window.
    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent);

    /// Raw device input (mouse motion for cameras), not tied to a window.
    fn device_event(&mut self, cx: &mut dyn EventLoopContext, event: DeviceEvent) {
        let _ = (cx, event);
    }

    /// A gamepad connected, disconnected or changed. Delivered just before [`Self::about_to_wait`].
    fn gamepad_event(&mut self, cx: &mut dyn EventLoopContext, event: GamepadEvent) {
        let _ = (cx, event);
    }

    /// Every event of this loop iteration has been delivered: step the game and request redraws here.
    fn about_to_wait(&mut self, cx: &mut dyn EventLoopContext) {
        let _ = cx;
    }

    /// The loop is stopping; the last callback. Save settings here.
    fn exiting(&mut self) {}
}

/// A platform implementation: the console seam for the event loop.
///
/// This crate ships the winit backend (Windows, Linux, macOS). A private console crate
/// (`Metallic-Club/platform-<console>`, ADR 0001 rule 9) implements this trait, [`crate::WindowBackend`] and
/// [`crate::GamepadSource`] over its SDK and hands the backend to [`Platform::with_backend`]; the engine above it
/// is unchanged. No SDK code lives in this repository.
pub trait PlatformBackend {
    /// A short name for logs and errors (`"winit"`, a console's name).
    fn name(&self) -> &'static str;

    /// The app's per-user folders on this platform.
    fn user_paths(&self, app: &AppId) -> Result<UserPaths, PlatformError>;

    /// Runs the event loop until [`EventLoopContext::exit`], calling `handler` for every event. Returns once the
    /// loop has stopped (on platforms whose loop never returns, it does not return).
    fn run(self: Box<Self>, handler: &mut dyn AppHandler) -> Result<(), PlatformError>;
}

/// The platform the engine runs on: owns the event loop and drives an [`AppHandler`].
///
/// ```no_run
/// use metallic_platform::{AppHandler, EventLoopContext, Platform, WindowConfig, WindowEvent, WindowId};
///
/// struct Game;
///
/// impl AppHandler for Game {
///     fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
///         if let Err(err) = cx.create_window(&WindowConfig::new("Game")) {
///             eprintln!("{err}");
///             cx.exit();
///         }
///     }
///     fn window_event(&mut self, cx: &mut dyn EventLoopContext, _window: WindowId, event: WindowEvent) {
///         if event == WindowEvent::CloseRequested {
///             cx.exit();
///         }
///     }
/// }
///
/// # fn main() -> Result<(), metallic_platform::PlatformError> {
/// Platform::new()?.run(&mut Game)
/// # }
/// ```
pub struct Platform {
    backend: Box<dyn PlatformBackend>,
}

impl Platform {
    /// The platform this build targets. On Windows, Linux and macOS that is winit; on Android and iOS this fails
    /// with [`PlatformError::Unsupported`] until their entry points exist (M9). Call it once, on the main thread.
    pub fn new() -> Result<Self, PlatformError> {
        Ok(Self::with_backend(Box::new(crate::winit_backend::WinitBackend::new()?)))
    }

    /// A platform over another backend (a console crate's).
    pub fn with_backend(backend: Box<dyn PlatformBackend>) -> Self {
        Self { backend }
    }

    /// The backend's name (`"winit"`, a console's name).
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }

    /// The per-user folders for `app` on this platform.
    pub fn user_paths(&self, app: &AppId) -> Result<UserPaths, PlatformError> {
        self.backend.user_paths(app)
    }

    /// Runs the event loop, calling `handler` until it calls [`EventLoopContext::exit`].
    pub fn run(self, handler: &mut dyn AppHandler) -> Result<(), PlatformError> {
        log::debug!("running the {} event loop", self.backend.name());
        self.backend.run(handler)
    }
}

impl fmt::Debug for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Platform")
            .field("backend", &self.backend.name())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend with no OS behind it: shows the seam is implementable without winit and runs without a display.
    struct ScriptedBackend {
        events: Vec<WindowEvent>,
    }

    struct ScriptedContext {
        exit: bool,
        mode: LoopMode,
    }

    impl EventLoopContext for ScriptedContext {
        fn create_window(&mut self, _config: &WindowConfig) -> Result<Window, PlatformError> {
            Err(PlatformError::unsupported("windows in tests"))
        }
        fn set_loop_mode(&mut self, mode: LoopMode) {
            self.mode = mode;
        }
        fn exit(&mut self) {
            self.exit = true;
        }
    }

    impl PlatformBackend for ScriptedBackend {
        fn name(&self) -> &'static str {
            "scripted"
        }
        fn user_paths(&self, _app: &AppId) -> Result<UserPaths, PlatformError> {
            Ok(UserPaths::new("c".into(), "d".into(), "k".into()))
        }
        fn run(self: Box<Self>, handler: &mut dyn AppHandler) -> Result<(), PlatformError> {
            let mut cx = ScriptedContext {
                exit: false,
                mode: LoopMode::Poll,
            };
            handler.resumed(&mut cx);
            for event in self.events {
                handler.window_event(&mut cx, WindowId::from_raw(1), event);
                if cx.exit {
                    break;
                }
                handler.about_to_wait(&mut cx);
            }
            handler.exiting();
            Ok(())
        }
    }

    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
    }

    impl AppHandler for Recorder {
        fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
            self.calls.push("resumed".into());
            assert!(cx.create_window(&WindowConfig::default()).is_err());
            cx.set_loop_mode(LoopMode::Wait);
        }
        fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
            self.calls.push(format!("{}:{event:?}", window.to_raw()));
            if event == WindowEvent::CloseRequested {
                cx.exit();
            }
        }
        fn about_to_wait(&mut self, _cx: &mut dyn EventLoopContext) {
            self.calls.push("about_to_wait".into());
        }
        fn exiting(&mut self) {
            self.calls.push("exiting".into());
        }
    }

    #[test]
    fn a_custom_backend_drives_the_handler_in_order() {
        let backend = ScriptedBackend {
            events: vec![
                WindowEvent::Focused(true),
                WindowEvent::CloseRequested,
                WindowEvent::RedrawRequested,
            ],
        };
        let platform = Platform::with_backend(Box::new(backend));
        assert_eq!(platform.name(), "scripted");
        assert!(
            platform
                .user_paths(&AppId::parse("dev.metallic.Player").unwrap())
                .is_ok()
        );
        let mut recorder = Recorder::default();
        platform.run(&mut recorder).unwrap();
        assert_eq!(
            recorder.calls,
            [
                "resumed",
                "1:Focused(true)",
                "about_to_wait",
                "1:CloseRequested",
                "exiting"
            ]
        );
    }
}
