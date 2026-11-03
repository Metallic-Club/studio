//! The platform layer: the only engine crate that knows which operating system it runs on.
//!
//! Owns: windows and the event loop, keyboard, mouse, gamepad and touch input, user config paths, and the seams
//! that private console crates implement. `#[cfg(target_os = ...)]` is allowed here and in app packaging only.
//!
//! Must not depend on: any other Metallic crate. See `docs/adr/0004-runtime-rendering-platform.md`.
//!
//! # Using it
//!
//! [`Platform::new`] picks the backend for this build and [`Platform::run`] drives an [`AppHandler`]: it creates
//! [`Window`]s through the [`EventLoopContext`] it is given and receives [`WindowEvent`]s, [`DeviceEvent`]s and
//! [`GamepadEvent`]s as plain engine types. A [`Window`] implements `HasWindowHandle` and `HasDisplayHandle`, which
//! is what `metallic_render` takes; the renderer never depends on this crate (ADR 0001 rule 4). [`UserPaths`] gives
//! an app's config, data and cache folders.
//!
//! # Backends and the console seam
//!
//! | Platform | Backend |
//! |---|---|
//! | Windows, Linux, macOS | winit, built in |
//! | Android, iOS | winit, built in; the event loop returns [`PlatformError::Unsupported`] until M9 |
//! | PlayStation, Xbox | a private crate (`Metallic-Club/platform-<console>`) |
//!
//! A console crate implements three traits over its SDK, with no change to the engine above:
//!
//! - [`PlatformBackend`]: runs the event loop and calls the [`AppHandler`], and supplies [`UserPaths`];
//! - [`WindowBackend`]: the screen the game draws to, wrapped by [`Window::from_backend`];
//! - [`GamepadSource`]: the console's controllers.
//!
//! It then starts the engine with [`Platform::with_backend`]. No console SDK code lives in this repository
//! (ADR 0001 rule 9).

mod app;
mod error;
mod event;
mod gamepad;
mod input;
mod paths;
mod window;
mod winit_backend;

pub use app::{AppHandler, EventLoopContext, LoopMode, Platform, PlatformBackend};
pub use error::PlatformError;
pub use event::{DeviceEvent, WindowEvent, WindowId};
pub use gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, GamepadSource, NoGamepads, default_gamepads};
pub use input::{
    ButtonState, Key, KeyCode, KeyEvent, Modifiers, MouseButton, NamedKey, Position, ScrollDelta, Size, Touch,
    TouchPhase,
};
pub use paths::{AppId, UserPaths};
/// The window-handle traits [`Window`] implements, re-exported so the engine and console crates name the same
/// version.
pub use raw_window_handle;
pub use window::{CursorGrab, Window, WindowBackend, WindowConfig, WindowMode};
