//! The winit backend: windows, the event loop and input on Windows, Linux and macOS.
//!
//! Everything winit stays in this module; the rest of the crate and the engine see only the engine types it
//! translates to. On Android and iOS the module compiles (so the cross-checks keep winit building there) but
//! creating the event loop returns [`PlatformError::Unsupported`] until their entry points exist (M9).

use std::sync::Arc;

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{self as wevent, ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{self as wkey, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, WindowAttributes};

use crate::app::{AppHandler, EventLoopContext, LoopMode, PlatformBackend};
use crate::error::PlatformError;
use crate::event::{DeviceEvent, WindowEvent, WindowId};
use crate::gamepad::{GamepadEvent, GamepadSource, default_gamepads};
use crate::input::{
    ButtonState, Key, KeyCode, KeyEvent, Modifiers, MouseButton, NamedKey, Position, ScrollDelta, Size, Touch,
    TouchPhase, with_key_codes,
};
use crate::paths::{AppId, UserPaths};
use crate::window::{CursorGrab, Window, WindowBackend, WindowConfig, WindowMode};

/// The desktop platform backend.
pub(crate) struct WinitBackend {
    event_loop: EventLoop<()>,
    gamepads: Box<dyn GamepadSource>,
}

impl WinitBackend {
    /// Creates the event loop (once per process, on the main thread) and the gamepad source.
    pub(crate) fn new() -> Result<Self, PlatformError> {
        let event_loop = create_event_loop()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        Ok(Self {
            event_loop,
            gamepads: default_gamepads(),
        })
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn create_event_loop() -> Result<EventLoop<()>, PlatformError> {
    EventLoop::new().map_err(|err| PlatformError::EventLoop(err.to_string()))
}

/// Android needs the `AndroidApp` from `android_main`, and iOS has to start from `UIApplicationMain`; both entry
/// points are M9 work.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn create_event_loop() -> Result<EventLoop<()>, PlatformError> {
    Err(PlatformError::unsupported(
        "the event loop (mobile entry points are planned for M9)",
    ))
}

impl PlatformBackend for WinitBackend {
    fn name(&self) -> &'static str {
        "winit"
    }

    fn user_paths(&self, app: &AppId) -> Result<UserPaths, PlatformError> {
        UserPaths::for_app(app)
    }

    fn run(self: Box<Self>, handler: &mut dyn AppHandler) -> Result<(), PlatformError> {
        let Self { event_loop, gamepads } = *self;
        let mut app = WinitApp {
            handler,
            gamepads,
            pending: Vec::new(),
        };
        event_loop
            .run_app(&mut app)
            .map_err(|err| PlatformError::EventLoop(err.to_string()))
    }
}

/// Adapts an [`AppHandler`] to winit's `ApplicationHandler`.
struct WinitApp<'a> {
    handler: &'a mut dyn AppHandler,
    gamepads: Box<dyn GamepadSource>,
    /// Reused between iterations so polling gamepads does not allocate every frame.
    pending: Vec<GamepadEvent>,
}

impl ApplicationHandler for WinitApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.handler.resumed(&mut Context { event_loop });
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.handler.suspended(&mut Context { event_loop });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: winit::window::WindowId, event: wevent::WindowEvent) {
        if let Some(event) = map_window_event(event) {
            self.handler
                .window_event(&mut Context { event_loop }, WindowId::from_raw(u64::from(id)), event);
        }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _device: wevent::DeviceId, event: wevent::DeviceEvent) {
        if let Some(event) = map_device_event(event) {
            self.handler.device_event(&mut Context { event_loop }, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut cx = Context { event_loop };
        self.gamepads.poll(&mut self.pending);
        for event in self.pending.drain(..) {
            self.handler.gamepad_event(&mut cx, event);
        }
        self.handler.about_to_wait(&mut cx);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.handler.exiting();
    }
}

/// The running loop, as [`EventLoopContext`] sees it.
struct Context<'a> {
    event_loop: &'a ActiveEventLoop,
}

impl EventLoopContext for Context<'_> {
    fn create_window(&mut self, config: &WindowConfig) -> Result<Window, PlatformError> {
        let attributes = WindowAttributes::default()
            .with_title(config.title.clone())
            .with_inner_size(LogicalSize::new(config.size.width, config.size.height))
            .with_resizable(config.resizable);
        let window = self
            .event_loop
            .create_window(attributes)
            .map_err(|err| PlatformError::CreateWindow(err.to_string()))?;
        let window = WinitWindow { window };
        if config.mode != WindowMode::Windowed {
            window.set_mode(config.mode)?;
        }
        Ok(Window::from_backend(Arc::new(window)))
    }

    fn set_loop_mode(&mut self, mode: LoopMode) {
        self.event_loop.set_control_flow(match mode {
            LoopMode::Poll => ControlFlow::Poll,
            LoopMode::Wait => ControlFlow::Wait,
        });
    }

    fn exit(&mut self) {
        self.event_loop.exit();
    }
}

/// A winit window behind [`WindowBackend`].
struct WinitWindow {
    window: winit::window::Window,
}

impl WindowBackend for WinitWindow {
    fn id(&self) -> WindowId {
        WindowId::from_raw(u64::from(self.window.id()))
    }

    fn inner_size(&self) -> Size {
        let size = self.window.inner_size();
        Size::new(size.width, size.height)
    }

    fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    fn set_mode(&self, mode: WindowMode) -> Result<(), PlatformError> {
        let fullscreen = match mode {
            WindowMode::Windowed => None,
            WindowMode::BorderlessFullscreen => Some(Fullscreen::Borderless(None)),
            WindowMode::ExclusiveFullscreen => Some(match self.exclusive_video_mode() {
                Some(video_mode) => Fullscreen::Exclusive(video_mode),
                None => {
                    log::warn!("no exclusive fullscreen video mode on this display; using borderless fullscreen");
                    Fullscreen::Borderless(None)
                }
            }),
        };
        self.window.set_fullscreen(fullscreen);
        Ok(())
    }

    fn set_cursor_grab(&self, grab: CursorGrab) -> Result<CursorGrab, PlatformError> {
        // winit 0.30 cannot lock the cursor on X11 or confine it on macOS, so each grab falls back to the other.
        let attempts: &[CursorGrab] = match grab {
            CursorGrab::None => &[CursorGrab::None],
            CursorGrab::Confine => &[CursorGrab::Confine, CursorGrab::Lock],
            CursorGrab::Lock => &[CursorGrab::Lock, CursorGrab::Confine],
        };
        let mut last_error = String::new();
        for &attempt in attempts {
            let mode = match attempt {
                CursorGrab::None => CursorGrabMode::None,
                CursorGrab::Confine => CursorGrabMode::Confined,
                CursorGrab::Lock => CursorGrabMode::Locked,
            };
            match self.window.set_cursor_grab(mode) {
                Ok(()) => return Ok(attempt),
                Err(err) => last_error = err.to_string(),
            }
        }
        Err(PlatformError::Window(format!(
            "cannot grab the cursor ({grab:?}): {last_error}"
        )))
    }

    fn set_cursor_visible(&self, visible: bool) {
        self.window.set_cursor_visible(visible);
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

impl WinitWindow {
    /// The video mode for exclusive fullscreen: the display's own resolution at its highest refresh rate, or the
    /// largest mode if none matches the display.
    fn exclusive_video_mode(&self) -> Option<winit::monitor::VideoModeHandle> {
        let monitor = self.window.current_monitor()?;
        let native = monitor.size();
        let key = |mode: &winit::monitor::VideoModeHandle| {
            let size = mode.size();
            (
                size == native,
                u64::from(size.width) * u64::from(size.height),
                mode.refresh_rate_millihertz(),
            )
        };
        monitor.video_modes().max_by_key(key)
    }
}

impl HasWindowHandle for WinitWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for WinitWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}

/// Translates a winit window event; `None` for events the engine does not use (yet).
fn map_window_event(event: wevent::WindowEvent) -> Option<WindowEvent> {
    use wevent::WindowEvent as W;
    Some(match event {
        W::Resized(size) => WindowEvent::Resized(Size::new(size.width, size.height)),
        W::ScaleFactorChanged { scale_factor, .. } => WindowEvent::ScaleFactorChanged { scale_factor },
        W::CloseRequested => WindowEvent::CloseRequested,
        W::Destroyed => WindowEvent::Destroyed,
        W::Focused(focused) => WindowEvent::Focused(focused),
        W::Occluded(occluded) => WindowEvent::Occluded(occluded),
        W::KeyboardInput { event, .. } => WindowEvent::Key(map_key_event(event)),
        W::ModifiersChanged(modifiers) => {
            let state = modifiers.state();
            WindowEvent::ModifiersChanged(Modifiers {
                shift: state.shift_key(),
                control: state.control_key(),
                alt: state.alt_key(),
                super_key: state.super_key(),
            })
        }
        W::CursorMoved { position, .. } => WindowEvent::CursorMoved(Position {
            x: position.x,
            y: position.y,
        }),
        W::CursorEntered { .. } => WindowEvent::CursorEntered,
        W::CursorLeft { .. } => WindowEvent::CursorLeft,
        W::MouseInput { state, button, .. } => WindowEvent::MouseButton {
            button: map_mouse_button(button),
            state: map_state(state),
        },
        W::MouseWheel { delta, .. } => WindowEvent::MouseWheel(map_scroll(delta)),
        W::Touch(touch) => WindowEvent::Touch(Touch {
            id: touch.id,
            phase: match touch.phase {
                wevent::TouchPhase::Started => TouchPhase::Started,
                wevent::TouchPhase::Moved => TouchPhase::Moved,
                wevent::TouchPhase::Ended => TouchPhase::Ended,
                wevent::TouchPhase::Cancelled => TouchPhase::Cancelled,
            },
            position: Position {
                x: touch.location.x,
                y: touch.location.y,
            },
            force: touch.force.map(|force| force.normalized()),
        }),
        W::RedrawRequested => WindowEvent::RedrawRequested,
        _ => return None,
    })
}

fn map_device_event(event: wevent::DeviceEvent) -> Option<DeviceEvent> {
    match event {
        wevent::DeviceEvent::MouseMotion { delta: (dx, dy) } => Some(DeviceEvent::MouseMotion { dx, dy }),
        wevent::DeviceEvent::MouseWheel { delta } => Some(DeviceEvent::MouseWheel(map_scroll(delta))),
        _ => None,
    }
}

fn map_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

fn map_scroll(delta: MouseScrollDelta) -> ScrollDelta {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines { x, y },
        MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels {
            x: position.x,
            y: position.y,
        },
    }
}

fn map_key_event(event: wevent::KeyEvent) -> KeyEvent {
    KeyEvent {
        key_code: map_physical_key(event.physical_key),
        key: map_logical_key(&event.logical_key),
        text: event.text.map(|text| text.to_string()),
        state: map_state(event.state),
        repeat: event.repeat,
    }
}

fn map_physical_key(key: PhysicalKey) -> KeyCode {
    match key {
        PhysicalKey::Code(code) => key_code_from_winit(code),
        PhysicalKey::Unidentified(_) => KeyCode::Unidentified,
    }
}

/// Generates the winit ↔ engine key code mapping from the same list that declares [`KeyCode`], so every engine key
/// maps to the winit key of the same name.
macro_rules! winit_key_mapping {
    ($($key:ident),+ $(,)?) => {
        fn key_code_from_winit(code: wkey::KeyCode) -> KeyCode {
            match code {
                $(wkey::KeyCode::$key => KeyCode::$key,)+
                _ => KeyCode::Unidentified,
            }
        }

        /// The reverse mapping, exhaustive over [`KeyCode`]; tests check the round trip.
        #[cfg(test)]
        fn key_code_to_winit(key: KeyCode) -> Option<wkey::KeyCode> {
            match key {
                KeyCode::Unidentified => None,
                $(KeyCode::$key => Some(wkey::KeyCode::$key),)+
            }
        }
    };
}

with_key_codes!(winit_key_mapping);

fn map_logical_key(key: &wkey::Key) -> Key {
    match key {
        wkey::Key::Character(text) => Key::Character(text.to_string()),
        wkey::Key::Named(named) => map_named_key(*named).map_or(Key::Unidentified, Key::Named),
        wkey::Key::Dead(accent) => Key::Dead(*accent),
        wkey::Key::Unidentified(_) => Key::Unidentified,
    }
}

fn map_named_key(key: wkey::NamedKey) -> Option<NamedKey> {
    use wkey::NamedKey as W;
    Some(match key {
        W::Enter => NamedKey::Enter,
        W::Tab => NamedKey::Tab,
        W::Space => NamedKey::Space,
        W::Backspace => NamedKey::Backspace,
        W::Delete => NamedKey::Delete,
        W::Escape => NamedKey::Escape,
        W::Insert => NamedKey::Insert,
        W::Home => NamedKey::Home,
        W::End => NamedKey::End,
        W::PageUp => NamedKey::PageUp,
        W::PageDown => NamedKey::PageDown,
        W::ArrowUp => NamedKey::ArrowUp,
        W::ArrowDown => NamedKey::ArrowDown,
        W::ArrowLeft => NamedKey::ArrowLeft,
        W::ArrowRight => NamedKey::ArrowRight,
        W::Shift => NamedKey::Shift,
        W::Control => NamedKey::Control,
        W::Alt => NamedKey::Alt,
        W::Super | W::Meta => NamedKey::Super,
        W::CapsLock => NamedKey::CapsLock,
        W::F1 => NamedKey::F1,
        W::F2 => NamedKey::F2,
        W::F3 => NamedKey::F3,
        W::F4 => NamedKey::F4,
        W::F5 => NamedKey::F5,
        W::F6 => NamedKey::F6,
        W::F7 => NamedKey::F7,
        W::F8 => NamedKey::F8,
        W::F9 => NamedKey::F9,
        W::F10 => NamedKey::F10,
        W::F11 => NamedKey::F11,
        W::F12 => NamedKey::F12,
        _ => return None,
    })
}

fn map_mouse_button(button: wevent::MouseButton) -> MouseButton {
    match button {
        wevent::MouseButton::Left => MouseButton::Left,
        wevent::MouseButton::Right => MouseButton::Right,
        wevent::MouseButton::Middle => MouseButton::Middle,
        wevent::MouseButton::Back => MouseButton::Back,
        wevent::MouseButton::Forward => MouseButton::Forward,
        wevent::MouseButton::Other(n) => MouseButton::Other(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_code_round_trips_through_winit() {
        assert!(KeyCode::ALL.len() > 100, "the key list looks truncated");
        for &key in KeyCode::ALL {
            match key_code_to_winit(key) {
                Some(code) => {
                    assert_eq!(key_code_from_winit(code), key);
                    assert_eq!(map_physical_key(PhysicalKey::Code(code)), key);
                }
                None => assert_eq!(key, KeyCode::Unidentified),
            }
        }
    }

    #[test]
    fn key_codes_are_listed_once() {
        for (i, key) in KeyCode::ALL.iter().enumerate() {
            assert!(!KeyCode::ALL[i + 1..].contains(key), "{key:?} listed twice");
        }
    }

    #[test]
    fn common_game_keys_map_by_position() {
        assert_eq!(key_code_from_winit(wkey::KeyCode::KeyW), KeyCode::KeyW);
        assert_eq!(key_code_from_winit(wkey::KeyCode::Space), KeyCode::Space);
        assert_eq!(key_code_from_winit(wkey::KeyCode::Escape), KeyCode::Escape);
        assert_eq!(key_code_from_winit(wkey::KeyCode::ShiftLeft), KeyCode::ShiftLeft);
    }

    #[test]
    fn keys_the_engine_does_not_name_are_unidentified() {
        assert_eq!(key_code_from_winit(wkey::KeyCode::F35), KeyCode::Unidentified);
        assert_eq!(key_code_from_winit(wkey::KeyCode::LaunchMail), KeyCode::Unidentified);
        let native = PhysicalKey::Unidentified(wkey::NativeKeyCode::Unidentified);
        assert_eq!(map_physical_key(native), KeyCode::Unidentified);
    }

    /// Exhaustive over [`NamedKey`]: adding a variant without a winit counterpart fails to compile here.
    fn named_key_to_winit(key: NamedKey) -> wkey::NamedKey {
        use wkey::NamedKey as W;
        match key {
            NamedKey::Enter => W::Enter,
            NamedKey::Tab => W::Tab,
            NamedKey::Space => W::Space,
            NamedKey::Backspace => W::Backspace,
            NamedKey::Delete => W::Delete,
            NamedKey::Escape => W::Escape,
            NamedKey::Insert => W::Insert,
            NamedKey::Home => W::Home,
            NamedKey::End => W::End,
            NamedKey::PageUp => W::PageUp,
            NamedKey::PageDown => W::PageDown,
            NamedKey::ArrowUp => W::ArrowUp,
            NamedKey::ArrowDown => W::ArrowDown,
            NamedKey::ArrowLeft => W::ArrowLeft,
            NamedKey::ArrowRight => W::ArrowRight,
            NamedKey::Shift => W::Shift,
            NamedKey::Control => W::Control,
            NamedKey::Alt => W::Alt,
            NamedKey::Super => W::Super,
            NamedKey::CapsLock => W::CapsLock,
            NamedKey::F1 => W::F1,
            NamedKey::F2 => W::F2,
            NamedKey::F3 => W::F3,
            NamedKey::F4 => W::F4,
            NamedKey::F5 => W::F5,
            NamedKey::F6 => W::F6,
            NamedKey::F7 => W::F7,
            NamedKey::F8 => W::F8,
            NamedKey::F9 => W::F9,
            NamedKey::F10 => W::F10,
            NamedKey::F11 => W::F11,
            NamedKey::F12 => W::F12,
        }
    }

    #[test]
    fn every_named_key_round_trips_through_winit() {
        for &key in NamedKey::ALL {
            assert_eq!(map_named_key(named_key_to_winit(key)), Some(key));
        }
        assert_eq!(map_named_key(wkey::NamedKey::Meta), Some(NamedKey::Super));
        assert_eq!(map_named_key(wkey::NamedKey::BrowserBack), None);
    }

    #[test]
    fn logical_keys_carry_text_dead_keys_and_names() {
        assert_eq!(
            map_logical_key(&wkey::Key::Character("é".into())),
            Key::Character("é".into())
        );
        assert_eq!(map_logical_key(&wkey::Key::Dead(Some('^'))), Key::Dead(Some('^')));
        assert_eq!(
            map_logical_key(&wkey::Key::Named(wkey::NamedKey::Escape)),
            Key::Named(NamedKey::Escape)
        );
        assert_eq!(
            map_logical_key(&wkey::Key::Named(wkey::NamedKey::BrowserBack)),
            Key::Unidentified
        );
    }

    /// Exhaustive over [`MouseButton`].
    fn mouse_button_to_winit(button: MouseButton) -> wevent::MouseButton {
        match button {
            MouseButton::Left => wevent::MouseButton::Left,
            MouseButton::Right => wevent::MouseButton::Right,
            MouseButton::Middle => wevent::MouseButton::Middle,
            MouseButton::Back => wevent::MouseButton::Back,
            MouseButton::Forward => wevent::MouseButton::Forward,
            MouseButton::Other(n) => wevent::MouseButton::Other(n),
        }
    }

    #[test]
    fn every_mouse_button_round_trips_through_winit() {
        let buttons = [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
            MouseButton::Other(0),
            MouseButton::Other(9),
        ];
        for button in buttons {
            assert_eq!(map_mouse_button(mouse_button_to_winit(button)), button);
        }
    }

    #[test]
    fn scroll_and_button_states_map() {
        assert_eq!(
            map_scroll(MouseScrollDelta::LineDelta(0.0, 1.0)),
            ScrollDelta::Lines { x: 0.0, y: 1.0 }
        );
        let pixels = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(2.0, -3.0));
        assert_eq!(map_scroll(pixels), ScrollDelta::Pixels { x: 2.0, y: -3.0 });
        assert!(map_state(ElementState::Pressed).is_pressed());
        assert!(!map_state(ElementState::Released).is_pressed());
    }

    #[test]
    fn window_events_without_a_device_map() {
        assert_eq!(
            map_window_event(wevent::WindowEvent::CloseRequested),
            Some(WindowEvent::CloseRequested)
        );
        assert_eq!(
            map_window_event(wevent::WindowEvent::Focused(false)),
            Some(WindowEvent::Focused(false))
        );
        assert_eq!(
            map_window_event(wevent::WindowEvent::Resized(winit::dpi::PhysicalSize::new(800, 600))),
            Some(WindowEvent::Resized(Size::new(800, 600)))
        );
        assert_eq!(
            map_device_event(wevent::DeviceEvent::MouseMotion { delta: (1.5, -2.0) }),
            Some(DeviceEvent::MouseMotion { dx: 1.5, dy: -2.0 })
        );
    }
}
