//! The frame's input step for `UserInputService`: the platform's events become the service's state and events
//! (through `metallic_classes`' [`InputFeed`]), and what scripts asked of the cursor reaches the window.

use metallic_classes::GameError;
use metallic_classes::input::feed::{CursorState, InputFeed, MouseBehavior, MouseButton, TouchPhase};
use metallic_classes::input::key_code::UNKNOWN;
use metallic_core::{DataModel, DmError, Id};
use metallic_platform::{
    CursorGrab, DeviceEvent, GamepadEvent, KeyCode, ScrollDelta, TouchPhase as PlatformTouchPhase, Window, WindowEvent,
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

/// The engine's `UserInputService` driver: the feed, plus what it last applied to the window.
#[derive(Debug)]
pub(crate) struct UserInput {
    feed: InputFeed,
    /// The cursor state the window has, or `None` when it has not been told yet (no window, or a new one).
    applied: Option<CursorState>,
    /// Gamepads connected now.
    gamepads: usize,
}

impl UserInput {
    pub(crate) fn new(dm: &mut DataModel) -> Result<Self, GameError> {
        Ok(Self {
            feed: InputFeed::new(dm)?,
            applied: None,
            gamepads: 0,
        })
    }

    /// The game's `UserInputService`.
    pub(crate) fn service(&self) -> Id {
        self.feed.service()
    }

    /// The input step: the frame's events, in arrival order. `firing` says whether the game runs this frame.
    pub(crate) fn process(&mut self, dm: &mut DataModel, events: &[InputEvent], firing: bool) {
        if let Err(error) = self.feed.begin_frame(dm, firing) {
            log::warn!("UserInputService could not start the frame's input: {error}");
        }
        for event in events {
            if let Err(error) = self.event(dm, event) {
                log::warn!("UserInputService could not take {event:?}: {error}");
            }
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
            WindowEvent::Focused(focused) => self.feed.focus(dm, *focused),
            _ => Ok(()),
        }
    }

    /// Applies `MouseBehavior` and `MouseIconEnabled` to `window` when they differ from what it was last told.
    pub(crate) fn apply_cursor(&mut self, dm: &DataModel, window: Option<&Window>) {
        let Some(window) = window else {
            return;
        };
        let wanted = self.feed.cursor(dm);
        let applied = self.applied;
        if applied == Some(wanted) {
            return;
        }
        if applied.map(|a| a.behavior) != Some(wanted.behavior) {
            // The platform falls back to what it can do (Windows confines instead of locking) and says so.
            let grab = match wanted.behavior {
                MouseBehavior::Default => CursorGrab::None,
                MouseBehavior::LockCenter | MouseBehavior::LockCurrentPosition => CursorGrab::Lock,
            };
            match window.set_cursor_grab(grab) {
                Ok(got) if got != grab => log::debug!("MouseBehavior {:?}: the cursor is {got:?}", wanted.behavior),
                Ok(_) => {}
                Err(error) => log::warn!("MouseBehavior {:?}: {error}", wanted.behavior),
            }
        }
        if applied.map(|a| a.icon_visible) != Some(wanted.icon_visible) {
            window.set_cursor_visible(wanted.icon_visible);
        }
        self.applied = Some(wanted);
    }

    /// A window came or went: the next one is told the cursor state afresh.
    pub(crate) fn forget_window(&mut self) {
        self.applied = None;
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
