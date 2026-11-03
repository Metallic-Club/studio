//! Windows: how to ask for one, and the handle the engine and renderer hold.

use std::fmt;
use std::sync::Arc;

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

use crate::error::PlatformError;
use crate::event::WindowId;
use crate::input::Size;

/// How a window covers the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowMode {
    /// A normal window with a title bar.
    #[default]
    Windowed,
    /// A borderless window covering the whole display it is on. Switching in and out is instant.
    BorderlessFullscreen,
    /// The display switches to the window's video mode (the display's own size at its highest refresh rate).
    /// Falls back to [`WindowMode::BorderlessFullscreen`] where the platform has no exclusive mode.
    ExclusiveFullscreen,
}

/// How the cursor is held by a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorGrab {
    /// The cursor moves freely.
    #[default]
    None,
    /// The cursor cannot leave the window.
    Confine,
    /// The cursor stays where it is; movement arrives only as [`crate::DeviceEvent::MouseMotion`]
    /// (first-person cameras).
    Lock,
}

/// What a new window looks like.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowConfig {
    /// The title bar text.
    pub title: String,
    /// The drawable area in logical pixels (scaled by the display's scale factor) when windowed.
    pub size: Size,
    /// Windowed or fullscreen.
    pub mode: WindowMode,
    /// Whether the user can resize the window.
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Metallic".to_owned(),
            size: Size::new(1280, 720),
            mode: WindowMode::Windowed,
            resizable: true,
        }
    }
}

impl WindowConfig {
    /// A resizable 1280 by 720 window with the given title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// The same config with the drawable area set to `width` by `height` logical pixels.
    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.size = Size::new(width, height);
        self
    }

    /// The same config with the window mode set.
    pub fn with_mode(mut self, mode: WindowMode) -> Self {
        self.mode = mode;
        self
    }
}

/// A window as a backend implements it: the console seam for windows.
///
/// The winit backend implements it on desktop; a private console crate implements it over the console SDK. The
/// engine never sees this trait directly, only [`Window`].
pub trait WindowBackend: HasWindowHandle + HasDisplayHandle + Send + Sync {
    /// This window's id, as it appears in [`crate::AppHandler::window_event`].
    fn id(&self) -> WindowId;
    /// The drawable area in physical pixels.
    fn inner_size(&self) -> Size;
    /// Physical pixels per logical pixel.
    fn scale_factor(&self) -> f64;
    /// Changes the title bar text (ignored where windows have none).
    fn set_title(&self, title: &str);
    /// Switches between windowed and fullscreen.
    fn set_mode(&self, mode: WindowMode) -> Result<(), PlatformError>;
    /// Holds the cursor as asked, or as close to it as the platform allows; returns what was applied.
    fn set_cursor_grab(&self, grab: CursorGrab) -> Result<CursorGrab, PlatformError>;
    /// Shows or hides the cursor over the window.
    fn set_cursor_visible(&self, visible: bool);
    /// Asks for a [`crate::WindowEvent::RedrawRequested`] for this window.
    fn request_redraw(&self);
}

/// An open window. Cheap to clone; every clone is the same window. Implements [`HasWindowHandle`] and
/// [`HasDisplayHandle`], which is all `metallic_render` needs to draw into it (ADR 0001 rule 4).
#[derive(Clone)]
pub struct Window {
    backend: Arc<dyn WindowBackend>,
}

impl Window {
    /// Wraps a backend's window. Backends call this from [`crate::EventLoopContext::create_window`].
    pub fn from_backend(backend: Arc<dyn WindowBackend>) -> Self {
        Self { backend }
    }

    /// This window's id, as it appears in [`crate::AppHandler::window_event`].
    pub fn id(&self) -> WindowId {
        self.backend.id()
    }

    /// The drawable area in physical pixels; what the renderer's surface should match.
    pub fn inner_size(&self) -> Size {
        self.backend.inner_size()
    }

    /// Physical pixels per logical pixel (1.0 on a standard display, 2.0 on most high-DPI ones).
    pub fn scale_factor(&self) -> f64 {
        self.backend.scale_factor()
    }

    /// Changes the title bar text.
    pub fn set_title(&self, title: &str) {
        self.backend.set_title(title);
    }

    /// Switches between windowed, borderless fullscreen and exclusive fullscreen.
    pub fn set_mode(&self, mode: WindowMode) -> Result<(), PlatformError> {
        self.backend.set_mode(mode)
    }

    /// Enters borderless fullscreen (`true`) or returns to a window (`false`); the usual fullscreen toggle.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<(), PlatformError> {
        self.set_mode(if fullscreen {
            WindowMode::BorderlessFullscreen
        } else {
            WindowMode::Windowed
        })
    }

    /// Holds the cursor in the window. Platforms that cannot lock the cursor confine it instead (X11) and the
    /// other way round (macOS); the returned value says which grab is in effect. Mouse motion keeps arriving as
    /// [`crate::DeviceEvent::MouseMotion`] either way. Does not hide the cursor; see [`Self::set_cursor_visible`].
    pub fn set_cursor_grab(&self, grab: CursorGrab) -> Result<CursorGrab, PlatformError> {
        self.backend.set_cursor_grab(grab)
    }

    /// Shows or hides the cursor over the window.
    pub fn set_cursor_visible(&self, visible: bool) {
        self.backend.set_cursor_visible(visible);
    }

    /// Asks for a [`crate::WindowEvent::RedrawRequested`] for this window.
    pub fn request_redraw(&self) {
        self.backend.request_redraw();
    }
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Window").field("id", &self.id()).finish_non_exhaustive()
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.backend.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}
