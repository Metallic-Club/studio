//! Gamepads: the [`GamepadSource`] seam and the sources this crate ships.
//!
//! Windows and macOS read gamepads through gilrs. Linux has [`NoGamepads`] for now: gilrs links libudev there,
//! which every Linux build (CI, dedicated servers) would then need. Android, iOS and consoles get their own sources
//! with their entry points.

use crate::input::ButtonState;

/// Identifies a gamepad while it is connected. A reconnected gamepad may get a new id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GamepadId(u32);

impl GamepadId {
    /// Wraps a source's raw gamepad id. Sources call this; the engine only compares ids.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// The source's raw gamepad id.
    pub const fn to_raw(self) -> u32 {
        self.0
    }
}

/// A gamepad button, named by position on a standard (Xbox-style) layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GamepadButton {
    /// The bottom face button (Xbox A, PlayStation Cross).
    South,
    /// The right face button (Xbox B, PlayStation Circle).
    East,
    /// The top face button (Xbox Y, PlayStation Triangle).
    North,
    /// The left face button (Xbox X, PlayStation Square).
    West,
    /// The left shoulder button (LB, L1).
    LeftBumper,
    /// The right shoulder button (RB, R1).
    RightBumper,
    /// The left trigger (LT, L2), which also reports how far it is pulled.
    LeftTrigger,
    /// The right trigger (RT, R2), which also reports how far it is pulled.
    RightTrigger,
    /// The left menu button (View, Share, Select).
    Select,
    /// The right menu button (Menu, Options, Start).
    Start,
    /// The vendor button (Xbox, PS).
    Mode,
    /// Pressing the left stick in.
    LeftStick,
    /// Pressing the right stick in.
    RightStick,
    /// D-pad up.
    DPadUp,
    /// D-pad down.
    DPadDown,
    /// D-pad left.
    DPadLeft,
    /// D-pad right.
    DPadRight,
}

/// A gamepad axis. Sticks range from -1 to 1 (positive is right and up).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GamepadAxis {
    /// Left stick, horizontal.
    LeftStickX,
    /// Left stick, vertical.
    LeftStickY,
    /// Right stick, horizontal.
    RightStickX,
    /// Right stick, vertical.
    RightStickY,
}

/// Something that happened on a gamepad.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GamepadEvent {
    /// A gamepad was connected (also sent for gamepads already connected when the source starts).
    Connected {
        /// The new gamepad.
        id: GamepadId,
        /// The name the OS gives it.
        name: String,
    },
    /// A gamepad was disconnected; its id sends nothing more.
    Disconnected {
        /// The gamepad that left.
        id: GamepadId,
    },
    /// A button went down or up.
    Button {
        /// Which gamepad.
        id: GamepadId,
        /// Which button.
        button: GamepadButton,
        /// Pressed or released.
        state: ButtonState,
    },
    /// An analog button (a trigger) moved, from 0 (released) to 1 (fully pulled).
    ButtonValue {
        /// Which gamepad.
        id: GamepadId,
        /// Which button.
        button: GamepadButton,
        /// How far it is pulled.
        value: f32,
    },
    /// A stick moved.
    Axis {
        /// Which gamepad.
        id: GamepadId,
        /// Which axis.
        axis: GamepadAxis,
        /// The new position, from -1 to 1.
        value: f32,
    },
}

/// Where gamepad events come from: the gamepad seam that platform backends and console crates implement.
///
/// The event loop polls it once per iteration, just before [`crate::AppHandler::about_to_wait`], and hands each
/// event to [`crate::AppHandler::gamepad_event`]. In [`crate::LoopMode::Wait`] gamepads do not wake the loop, so
/// games run in [`crate::LoopMode::Poll`].
pub trait GamepadSource {
    /// Appends every event since the last poll to `events`, oldest first.
    fn poll(&mut self, events: &mut Vec<GamepadEvent>);
}

/// A [`GamepadSource`] that never reports a gamepad: for platforms without gamepad support yet, and for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoGamepads;

impl GamepadSource for NoGamepads {
    fn poll(&mut self, _events: &mut Vec<GamepadEvent>) {}
}

/// The gamepad source for the platform this build targets: gilrs on Windows and macOS, [`NoGamepads`] elsewhere.
pub fn default_gamepads() -> Box<dyn GamepadSource> {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        gilrs_source::GilrsGamepads::new_or_none()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Box::new(NoGamepads)
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod gilrs_source {
    use gilrs::{Axis, Button, EventType, Gilrs};

    use super::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, GamepadSource, NoGamepads};
    use crate::input::ButtonState;

    /// Gamepads read through gilrs.
    pub(super) struct GilrsGamepads {
        gilrs: Gilrs,
        /// Gamepads connected before the first poll, announced by it.
        initial: Vec<GamepadEvent>,
    }

    impl GilrsGamepads {
        /// Starts gilrs, or falls back to [`NoGamepads`] (with a warning) when the OS refuses.
        pub(super) fn new_or_none() -> Box<dyn GamepadSource> {
            match Gilrs::new() {
                Ok(gilrs) => {
                    let initial = gilrs
                        .gamepads()
                        .map(|(id, pad)| GamepadEvent::Connected {
                            id: gamepad_id(id),
                            name: pad.name().to_owned(),
                        })
                        .collect();
                    Box::new(Self { gilrs, initial })
                }
                Err(err) => {
                    log::warn!("gamepads unavailable: {err}");
                    Box::new(NoGamepads)
                }
            }
        }
    }

    impl GamepadSource for GilrsGamepads {
        fn poll(&mut self, events: &mut Vec<GamepadEvent>) {
            events.append(&mut self.initial);
            while let Some(event) = self.gilrs.next_event() {
                let id = gamepad_id(event.id);
                let mapped = match event.event {
                    EventType::Connected => {
                        let name = self.gilrs.gamepad(event.id).name().to_owned();
                        Some(GamepadEvent::Connected { id, name })
                    }
                    EventType::Disconnected => Some(GamepadEvent::Disconnected { id }),
                    EventType::ButtonPressed(button, _) => map_button(button).map(|button| GamepadEvent::Button {
                        id,
                        button,
                        state: ButtonState::Pressed,
                    }),
                    EventType::ButtonReleased(button, _) => map_button(button).map(|button| GamepadEvent::Button {
                        id,
                        button,
                        state: ButtonState::Released,
                    }),
                    EventType::ButtonChanged(button, value, _) => map_button(button)
                        .filter(|b| matches!(b, GamepadButton::LeftTrigger | GamepadButton::RightTrigger))
                        .map(|button| GamepadEvent::ButtonValue { id, button, value }),
                    EventType::AxisChanged(axis, value, _) => {
                        map_axis(axis).map(|axis| GamepadEvent::Axis { id, axis, value })
                    }
                    _ => None,
                };
                events.extend(mapped);
            }
        }
    }

    fn gamepad_id(id: gilrs::GamepadId) -> GamepadId {
        GamepadId::from_raw(u32::try_from(usize::from(id)).unwrap_or(u32::MAX))
    }

    /// gilrs names the shoulder buttons `LeftTrigger` and the analog triggers `LeftTrigger2`.
    pub(super) fn map_button(button: Button) -> Option<GamepadButton> {
        Some(match button {
            Button::South => GamepadButton::South,
            Button::East => GamepadButton::East,
            Button::North => GamepadButton::North,
            Button::West => GamepadButton::West,
            Button::LeftTrigger => GamepadButton::LeftBumper,
            Button::RightTrigger => GamepadButton::RightBumper,
            Button::LeftTrigger2 => GamepadButton::LeftTrigger,
            Button::RightTrigger2 => GamepadButton::RightTrigger,
            Button::Select => GamepadButton::Select,
            Button::Start => GamepadButton::Start,
            Button::Mode => GamepadButton::Mode,
            Button::LeftThumb => GamepadButton::LeftStick,
            Button::RightThumb => GamepadButton::RightStick,
            Button::DPadUp => GamepadButton::DPadUp,
            Button::DPadDown => GamepadButton::DPadDown,
            Button::DPadLeft => GamepadButton::DPadLeft,
            Button::DPadRight => GamepadButton::DPadRight,
            Button::C | Button::Z | Button::Unknown => return None,
        })
    }

    pub(super) fn map_axis(axis: Axis) -> Option<GamepadAxis> {
        Some(match axis {
            Axis::LeftStickX => GamepadAxis::LeftStickX,
            Axis::LeftStickY => GamepadAxis::LeftStickY,
            Axis::RightStickX => GamepadAxis::RightStickX,
            Axis::RightStickY => GamepadAxis::RightStickY,
            // Triggers arrive as ButtonChanged and the d-pad as buttons on the layouts gilrs maps.
            Axis::LeftZ | Axis::RightZ | Axis::DPadX | Axis::DPadY | Axis::Unknown => return None,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_standard_gilrs_button_maps_to_a_distinct_engine_button() {
            let buttons = [
                Button::South,
                Button::East,
                Button::North,
                Button::West,
                Button::LeftTrigger,
                Button::LeftTrigger2,
                Button::RightTrigger,
                Button::RightTrigger2,
                Button::Select,
                Button::Start,
                Button::Mode,
                Button::LeftThumb,
                Button::RightThumb,
                Button::DPadUp,
                Button::DPadDown,
                Button::DPadLeft,
                Button::DPadRight,
            ];
            let mapped: Vec<_> = buttons.iter().filter_map(|b| map_button(*b)).collect();
            assert_eq!(mapped.len(), buttons.len());
            for (i, a) in mapped.iter().enumerate() {
                assert!(!mapped[i + 1..].contains(a), "{a:?} mapped twice");
            }
            assert_eq!(map_button(Button::Unknown), None);
        }

        #[test]
        fn sticks_map_and_other_axes_do_not() {
            assert_eq!(map_axis(Axis::LeftStickX), Some(GamepadAxis::LeftStickX));
            assert_eq!(map_axis(Axis::RightStickY), Some(GamepadAxis::RightStickY));
            assert_eq!(map_axis(Axis::LeftZ), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gamepads_reports_nothing() {
        let mut events = Vec::new();
        NoGamepads.poll(&mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn gamepad_ids_round_trip() {
        assert_eq!(GamepadId::from_raw(7).to_raw(), 7);
    }
}
