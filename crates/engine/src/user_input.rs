//! The frame's input step for `UserInputService`: the platform's events become the service's state and events
//! (through `metallic_classes`' [`InputFeed`]), and what scripts asked of the cursor reaches the window.
//!
//! # Holding the cursor
//!
//! `MouseBehavior.LockCenter` holds the cursor at the window's centre, and `LockCurrentPosition` where it was when
//! the behaviour was set. Both ask the window for [`CursorGrab::Lock`], which holds the cursor where it is, so the
//! engine first moves the cursor to the point to hold (Windows locks it there). It does that again whenever the
//! window gets focus back, since the system may have let go of the cursor meanwhile.
//!
//! Where the platform cannot lock the cursor (X11 under winit 0.30) it confines it instead, and the engine holds
//! the cursor itself: at each input step, while the window has focus, it moves the cursor back to the point.
//!
//! The engine's moves come back as cursor-moved events. While the cursor is held, those events are dropped:
//! `GetMouseLocation` reads the held point, and `GetMouseDelta` and `InputChanged` carry raw device motion only,
//! which never includes the engine's own moves.

use metallic_classes::GameError;
use metallic_classes::input::feed::{CursorState, InputFeed, MouseBehavior, MouseButton, TouchPhase};
use metallic_classes::input::key_code::UNKNOWN;
use metallic_core::{DataModel, DmError, Id};
use metallic_platform::{
    CursorGrab, DeviceEvent, GamepadEvent, KeyCode, PlatformError, Position, ScrollDelta,
    TouchPhase as PlatformTouchPhase, Window, WindowEvent,
};

use crate::input::InputEvent;

/// How many pixels of a touchpad's smooth scrolling count as one wheel notch (Roblox reports the wheel in notches).
const PIXELS_PER_NOTCH: f64 = 100.0;

/// Generates [`key_code_name`] from `metallic_classes`' key table, the same table `Enum.KeyCode` is built from.
macro_rules! platform_key_names {
    ($($name:ident = $number:literal => $platform:ident),+ $(,)?) => {
        /// The `Enum.KeyCode` item name of a platform key; `Unknown` for keys the table does not list.
        fn key_code_name(code: KeyCode) -> &'static str {
            match code {
                $(KeyCode::$platform => stringify!($name),)+
                _ => UNKNOWN,
            }
        }
    };
}

metallic_classes::with_key_codes!(platform_key_names);

/// Where `MouseBehavior` holds the cursor (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    /// At the window's centre (`LockCenter`).
    Center,
    /// Where the cursor was when `LockCurrentPosition` was set, in physical pixels; `None` until the cursor has
    /// been seen over the window.
    At(Option<(f64, f64)>),
}

/// The engine's `UserInputService` driver: the feed, plus what it last applied to the window.
#[derive(Debug)]
pub(crate) struct UserInput {
    feed: InputFeed,
    /// The cursor state the window has, or `None` when it has not been told yet (no window, or a new one).
    applied: Option<CursorState>,
    /// Where the cursor is held; `None` under `MouseBehavior.Default`.
    hold: Option<Hold>,
    /// Whether the engine moves the cursor back at each input step: the window could not lock it, and moving it
    /// works.
    hold_each_step: bool,
    /// Whether the window has keyboard focus. A window reports focus when it gets it, so until then it has none:
    /// the engine never moves the cursor of a window the user is not in.
    focused: bool,
    /// Focus came back: the cursor is moved and grabbed again at the next input step.
    regrab: bool,
    /// The cursor was moved to the held point in this input step already.
    placed: bool,
    /// Gamepads connected now.
    gamepads: usize,
}

impl UserInput {
    pub(crate) fn new(dm: &mut DataModel) -> Result<Self, GameError> {
        Ok(Self {
            feed: InputFeed::new(dm)?,
            applied: None,
            hold: None,
            hold_each_step: false,
            focused: false,
            regrab: false,
            placed: false,
            gamepads: 0,
        })
    }

    /// The game's `UserInputService`.
    pub(crate) fn service(&self) -> Id {
        self.feed.service()
    }

    /// The input step: the frame's events, in arrival order, then the cursor state scripts asked for reaches
    /// `window` (and the engine moves the cursor, when it holds it). `firing` says whether the game runs this frame.
    pub(crate) fn process(&mut self, dm: &mut DataModel, events: &[InputEvent], firing: bool, window: Option<&Window>) {
        if let Err(error) = self.feed.begin_frame(dm, firing) {
            log::warn!("UserInputService could not start the frame's input: {error}");
        }
        for event in events {
            if let Err(error) = self.event(dm, event) {
                log::warn!("UserInputService could not take {event:?}: {error}");
            }
        }
        if let Some(window) = window {
            self.placed = false;
            self.apply_cursor(dm, window);
            self.hold_cursor(window);
        }
        if let Err(error) = self.feed.end_frame(dm) {
            log::warn!("UserInputService could not end the frame's input: {error}");
        }
    }

    fn event(&mut self, dm: &mut DataModel, event: &InputEvent) -> Result<(), DmError> {
        match event {
            InputEvent::Window { event, .. } => self.window_event(dm, event),
            InputEvent::Device(DeviceEvent::MouseMotion { dx, dy }) => {
                self.feed.mouse_motion(*dx, *dy);
                Ok(())
            }
            // The wheel arrives as a window event too, under the cursor; taking both would count it twice.
            InputEvent::Device(_) => Ok(()),
            InputEvent::Gamepad(GamepadEvent::Connected { .. }) => {
                self.gamepads += 1;
                self.feed.set_gamepad_connected(dm, true)
            }
            InputEvent::Gamepad(GamepadEvent::Disconnected { .. }) => {
                self.gamepads = self.gamepads.saturating_sub(1);
                self.feed.set_gamepad_connected(dm, self.gamepads > 0)
            }
            // Gamepad buttons and sticks are not scripts' input yet (gamepad `KeyCode`s come with them).
            InputEvent::Gamepad(_) => Ok(()),
        }
    }

    fn window_event(&mut self, dm: &mut DataModel, event: &WindowEvent) -> Result<(), DmError> {
        match event {
            // Scripts see a held key once; the OS's repeats are for text.
            WindowEvent::Key(key) if key.repeat => Ok(()),
            WindowEvent::Key(key) => self.feed.key(dm, key_code_name(key.key_code), key.state.is_pressed()),
            WindowEvent::MouseButton { button, state } => {
                let button = match button {
                    metallic_platform::MouseButton::Left => MouseButton::Left,
                    metallic_platform::MouseButton::Right => MouseButton::Right,
                    metallic_platform::MouseButton::Middle => MouseButton::Middle,
                    // Roblox has no side buttons; they reach no script yet.
                    _ => return Ok(()),
                };
                self.feed.mouse_button(dm, button, state.is_pressed())
            }
            // While the engine holds the cursor, where the OS says it went is the user's push away from the held
            // point or the engine's own move back to it; scripts read the held point and raw motion instead.
            WindowEvent::CursorMoved(_) if self.holding() => Ok(()),
            WindowEvent::CursorMoved(position) => self.feed.cursor_moved(dm, position.x, position.y),
            WindowEvent::MouseWheel(delta) => {
                let notches = match *delta {
                    ScrollDelta::Lines { y, .. } => f64::from(y),
                    ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_NOTCH,
                };
                self.feed.wheel(dm, notches)
            }
            WindowEvent::Touch(touch) => {
                let phase = match touch.phase {
                    PlatformTouchPhase::Started => TouchPhase::Began,
                    PlatformTouchPhase::Moved => TouchPhase::Moved,
                    PlatformTouchPhase::Ended => TouchPhase::Ended,
                    PlatformTouchPhase::Cancelled => TouchPhase::Cancelled,
                };
                self.feed.touch(dm, touch.id, phase, touch.position.x, touch.position.y)
            }
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                self.regrab |= *focused;
                self.feed.focus(dm, *focused)
            }
            _ => Ok(()),
        }
    }

    /// Applies `MouseBehavior` and `MouseIconEnabled` to `window` when they differ from what it was last told, and
    /// grabs the cursor again when focus came back.
    fn apply_cursor(&mut self, dm: &DataModel, window: &Window) {
        let wanted = self.feed.cursor(dm);
        let applied = self.applied;
        let regrab = std::mem::take(&mut self.regrab) && self.hold.is_some();
        if applied == Some(wanted) && !regrab {
            return;
        }
        if applied.map(|a| a.behavior) != Some(wanted.behavior) {
            self.hold = match wanted.behavior {
                MouseBehavior::Default => None,
                MouseBehavior::LockCenter => Some(Hold::Center),
                MouseBehavior::LockCurrentPosition => Some(Hold::At(self.feed.cursor_location())),
            };
            self.grab(window, wanted.behavior);
        } else if regrab {
            self.grab(window, wanted.behavior);
        }
        if applied.map(|a| a.icon_visible) != Some(wanted.icon_visible) {
            window.set_cursor_visible(wanted.icon_visible);
        }
        self.applied = Some(wanted);
    }

    /// Asks `window` for the grab `behavior` needs. A lock holds the cursor where it is, so the cursor goes to the
    /// held point first.
    fn grab(&mut self, window: &Window, behavior: MouseBehavior) {
        let grab = if self.hold.is_some() {
            CursorGrab::Lock
        } else {
            CursorGrab::None
        };
        let placed = grab == CursorGrab::Lock && self.place_cursor(window).is_ok();
        let got = window.set_cursor_grab(grab);
        match &got {
            Ok(got) if *got != grab => log::debug!("MouseBehavior {behavior:?}: the cursor is {got:?}"),
            Ok(_) => {}
            Err(error) => log::warn!("MouseBehavior {behavior:?}: {error}"),
        }
        let locked = matches!(got, Ok(CursorGrab::Lock));
        if locked && !placed {
            // Some platforms (Wayland) move the cursor only while it is locked.
            let _ = self.place_cursor(window);
        }
        // Short of a real lock (confined, or not grabbed at all), the engine holds the cursor itself.
        self.hold_each_step = self.hold.is_some() && !locked;
    }

    /// Whether the cursor is held at a known point now: cursor-moved events are then the engine's own moves or the
    /// user's pushes away from that point, not where the cursor is.
    fn holding(&self) -> bool {
        self.focused && matches!(self.hold, Some(Hold::Center | Hold::At(Some(_))))
    }

    /// Moves the cursor back to the held point at each input step, where the window could not lock it. Stops when
    /// the platform cannot move the cursor.
    fn hold_cursor(&mut self, window: &Window) {
        if !self.hold_each_step || self.placed {
            return;
        }
        if let Err(error) = self.place_cursor(window) {
            log::warn!("cannot hold the cursor for MouseBehavior: {error}");
            self.hold_each_step = false;
        }
    }

    /// Moves the cursor to the held point, while the window has focus, and tells the feed it is there. Does nothing
    /// without focus or a point.
    fn place_cursor(&mut self, window: &Window) -> Result<(), PlatformError> {
        if !self.focused {
            return Ok(());
        }
        let (x, y) = match self.hold {
            None => return Ok(()),
            Some(Hold::Center) => {
                // Whole pixels, as the OS places the cursor, so `GetMouseLocation` reads where it really is.
                let size = window.inner_size();
                (f64::from(size.width / 2), f64::from(size.height / 2))
            }
            Some(Hold::At(Some(point))) => point,
            // `LockCurrentPosition` before the cursor was ever seen: it is held where it first shows up.
            Some(Hold::At(None)) => match self.feed.cursor_location() {
                Some(point) => {
                    self.hold = Some(Hold::At(Some(point)));
                    point
                }
                None => return Ok(()),
            },
        };
        window.set_cursor_position(Position { x, y })?;
        self.feed.place_cursor(x, y);
        self.placed = true;
        Ok(())
    }

    /// A window came or went: the next one is told the cursor state afresh.
    pub(crate) fn forget_window(&mut self) {
        self.applied = None;
        self.hold = None;
        self.hold_each_step = false;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use metallic_platform::KeyCode;

    use super::key_code_name;

    #[test]
    fn every_platform_key_has_a_key_code() {
        let registry = metallic_classes::shared_registry().expect("registry");
        let mut seen = HashSet::new();
        for &code in KeyCode::ALL {
            let name = key_code_name(code);
            if code == KeyCode::Unidentified {
                assert_eq!(name, "Unknown");
                continue;
            }
            assert_ne!(
                name, "Unknown",
                "{code:?} has no row in metallic_classes::with_key_codes"
            );
            assert!(registry.enum_item("KeyCode", name).is_some(), "{name}");
            assert!(seen.insert(name), "{name} is mapped twice");
        }
        let items = registry.enum_desc("KeyCode").expect("KeyCode").items.len();
        assert_eq!(
            seen.len() + 1,
            items,
            "every item but Unknown comes from one platform key"
        );
        assert_eq!(key_code_name(KeyCode::KeyW), "W");
        assert_eq!(key_code_name(KeyCode::ShiftLeft), "LeftShift");
    }
}
