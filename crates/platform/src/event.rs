//! Events the platform delivers to [`crate::AppHandler`].

use crate::input::{ButtonState, KeyEvent, Modifiers, MouseButton, Position, ScrollDelta, Size, Touch};

/// Identifies a window across events. Stable for the window's life; never reused while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// Wraps a backend's raw window id. Backends (winit, console crates) call this; the engine only compares ids.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The backend's raw window id.
    pub const fn to_raw(self) -> u64 {
        self.0
    }
}

/// Something that happened to one window.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WindowEvent {
    /// The drawable area changed size (physical pixels). The renderer resizes its surface.
    Resized(Size),
    /// The window moved to a display with a different scale, or the user changed the scale. A [`Self::Resized`]
    /// follows when the physical size changes too.
    ScaleFactorChanged {
        /// The new number of physical pixels per logical pixel.
        scale_factor: f64,
    },
    /// The user asked to close the window (close button, Alt+F4). The window stays open until the app exits or
    /// drops it.
    CloseRequested,
    /// The window was destroyed.
    Destroyed,
    /// The window gained (`true`) or lost (`false`) keyboard focus.
    Focused(bool),
    /// The window became fully hidden (`true`) or visible again (`false`); a renderer can stop drawing meanwhile.
    Occluded(bool),
    /// A key went down, repeated or went up while the window had focus.
    Key(KeyEvent),
    /// The modifier keys held down changed.
    ModifiersChanged(Modifiers),
    /// The cursor moved over the window, in physical pixels from its top-left corner.
    CursorMoved(Position),
    /// The cursor entered the window.
    CursorEntered,
    /// The cursor left the window.
    CursorLeft,
    /// A mouse button went down or up over the window.
    MouseButton {
        /// Which button.
        button: MouseButton,
        /// Pressed or released.
        state: ButtonState,
    },
    /// The mouse wheel or touchpad scrolled over the window.
    MouseWheel(ScrollDelta),
    /// A finger touched, moved on or left the screen.
    Touch(Touch),
    /// The window should be drawn now (after [`crate::Window::request_redraw`], or when the OS asks).
    RedrawRequested,
}

/// Raw input from a device, not tied to a window or to the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DeviceEvent {
    /// The mouse moved, in unaccelerated device units. Keeps arriving when the cursor is locked or at the screen
    /// edge, so it is what first-person cameras read.
    MouseMotion {
        /// Movement to the right.
        dx: f64,
        /// Movement down.
        dy: f64,
    },
    /// The wheel scrolled, read from the device rather than the window under the cursor.
    MouseWheel(ScrollDelta),
}
