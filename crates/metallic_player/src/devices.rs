//! The Player's raw input sources: winit keys and mouse buttons, and gilrs gamepads, turned into
//! the engine's [`RawEvent`]s (docs/design/input-actions.md sections 4.1 and 4.2).
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use metallic_runtime::input::{Control, Device, MAX_GAMEPADS, RawEvent, RawSource};
use std::time::Instant;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// The control of a physical key (winit `PhysicalKey`, so WASD stays a block on AZERTY).
pub fn key_control(key: KeyCode) -> Option<Control> {
    use Control as C;
    Some(match key {
        KeyCode::KeyA => C::A,
        KeyCode::KeyB => C::B,
        KeyCode::KeyC => C::C,
        KeyCode::KeyD => C::D,
        KeyCode::KeyE => C::E,
        KeyCode::KeyF => C::F,
        KeyCode::KeyG => C::G,
        KeyCode::KeyH => C::H,
        KeyCode::KeyI => C::I,
        KeyCode::KeyJ => C::J,
        KeyCode::KeyK => C::K,
        KeyCode::KeyL => C::L,
        KeyCode::KeyM => C::M,
        KeyCode::KeyN => C::N,
        KeyCode::KeyO => C::O,
        KeyCode::KeyP => C::P,
        KeyCode::KeyQ => C::Q,
        KeyCode::KeyR => C::R,
        KeyCode::KeyS => C::S,
        KeyCode::KeyT => C::T,
        KeyCode::KeyU => C::U,
        KeyCode::KeyV => C::V,
        KeyCode::KeyW => C::W,
        KeyCode::KeyX => C::X,
        KeyCode::KeyY => C::Y,
        KeyCode::KeyZ => C::Z,
        KeyCode::Digit0 => C::Zero,
        KeyCode::Digit1 => C::One,
        KeyCode::Digit2 => C::Two,
        KeyCode::Digit3 => C::Three,
        KeyCode::Digit4 => C::Four,
        KeyCode::Digit5 => C::Five,
        KeyCode::Digit6 => C::Six,
        KeyCode::Digit7 => C::Seven,
        KeyCode::Digit8 => C::Eight,
        KeyCode::Digit9 => C::Nine,
        KeyCode::F1 => C::F1,
        KeyCode::F2 => C::F2,
        KeyCode::F3 => C::F3,
        KeyCode::F4 => C::F4,
        KeyCode::F5 => C::F5,
        KeyCode::F6 => C::F6,
        KeyCode::F7 => C::F7,
        KeyCode::F8 => C::F8,
        KeyCode::F9 => C::F9,
        KeyCode::F10 => C::F10,
        KeyCode::F11 => C::F11,
        KeyCode::F12 => C::F12,
        KeyCode::Space => C::Space,
        KeyCode::Enter => C::Return,
        KeyCode::Tab => C::Tab,
        KeyCode::Backspace => C::Backspace,
        KeyCode::Escape => C::Escape,
        KeyCode::ShiftLeft => C::LeftShift,
        KeyCode::ShiftRight => C::RightShift,
        KeyCode::ControlLeft => C::LeftControl,
        KeyCode::ControlRight => C::RightControl,
        KeyCode::AltLeft => C::LeftAlt,
        KeyCode::AltRight => C::RightAlt,
        KeyCode::ArrowUp => C::Up,
        KeyCode::ArrowDown => C::Down,
        KeyCode::ArrowLeft => C::Left,
        KeyCode::ArrowRight => C::Right,
        KeyCode::Home => C::Home,
        KeyCode::End => C::End,
        KeyCode::PageUp => C::PageUp,
        KeyCode::PageDown => C::PageDown,
        KeyCode::Insert => C::Insert,
        KeyCode::Delete => C::Delete,
        KeyCode::Comma => C::Comma,
        KeyCode::Period => C::Period,
        KeyCode::Slash => C::Slash,
        KeyCode::Semicolon => C::Semicolon,
        KeyCode::Quote => C::Quote,
        KeyCode::BracketLeft => C::LeftBracket,
        KeyCode::BracketRight => C::RightBracket,
        KeyCode::Backslash => C::BackSlash,
        KeyCode::Minus => C::Minus,
        KeyCode::Equal => C::Equals,
        KeyCode::Backquote => C::Backquote,
        _ => return None,
    })
}

/// The control of a mouse button; `None` for the side buttons.
pub fn mouse_control(button: MouseButton) -> Option<Control> {
    match button {
        MouseButton::Left => Some(Control::MouseButton1),
        MouseButton::Right => Some(Control::MouseButton2),
        MouseButton::Middle => Some(Control::MouseButton3),
        _ => None,
    }
}

/// The mapping table of spec 4.2: gilrs buttons are named by position, so one binding works on
/// any pad. `Mode`, `C`, `Z` and unknown buttons are not mapped (Mode stays with the OS).
pub fn gamepad_button(button: Button) -> Option<Control> {
    Some(match button {
        Button::South => Control::ButtonA,
        Button::East => Control::ButtonB,
        Button::West => Control::ButtonX,
        Button::North => Control::ButtonY,
        Button::LeftTrigger => Control::ButtonL1,
        Button::RightTrigger => Control::ButtonR1,
        Button::LeftTrigger2 => Control::ButtonL2,
        Button::RightTrigger2 => Control::ButtonR2,
        Button::LeftThumb => Control::ButtonL3,
        Button::RightThumb => Control::ButtonR3,
        Button::Select => Control::ButtonSelect,
        Button::Start => Control::ButtonStart,
        Button::DPadUp => Control::DPadUp,
        Button::DPadDown => Control::DPadDown,
        Button::DPadLeft => Control::DPadLeft,
        Button::DPadRight => Control::DPadRight,
        Button::Mode | Button::C | Button::Z | Button::Unknown => return None,
    })
}

/// Every control a gamepad can drive: released together when the pad goes away.
fn pad_controls() -> impl Iterator<Item = Control> {
    Control::ALL.iter().copied().filter(|c| c.is_gamepad())
}

/// Gamepads through gilrs: events for presses and connection changes, polled stick axes each
/// frame. Up to [`MAX_GAMEPADS`] pads feed the same actions.
pub struct GamepadSource {
    gilrs: Option<Gilrs>,
    started: Instant,
    slots: [Option<GamepadId>; MAX_GAMEPADS],
    /// The last stick levels sent, per pad: left, right.
    sticks: [[[f32; 2]; 2]; MAX_GAMEPADS],
}

impl GamepadSource {
    /// Starts gilrs; without gamepad support (or when it fails to start) the source stays silent.
    pub fn new(started: Instant) -> Self {
        Self {
            gilrs: Gilrs::new().ok(),
            started,
            slots: [None; MAX_GAMEPADS],
            sticks: [[[0.0; 2]; 2]; MAX_GAMEPADS],
        }
    }

    fn slot_of(&mut self, id: GamepadId) -> Option<u8> {
        if let Some(i) = self.slots.iter().position(|s| *s == Some(id)) {
            return Some(i as u8);
        }
        let free = self.slots.iter().position(Option::is_none)?;
        self.slots[free] = Some(id);
        Some(free as u8)
    }
}

impl RawSource for GamepadSource {
    fn poll(&mut self, out: &mut Vec<RawEvent>) {
        let time = self.started.elapsed().as_secs_f64();
        let Some(mut gilrs) = self.gilrs.take() else {
            return;
        };
        while let Some(event) = gilrs.next_event() {
            let Some(slot) = self.slot_of(event.id) else {
                continue;
            };
            let device = Device::Gamepad(slot);
            match event.event {
                EventType::ButtonPressed(b, _) => {
                    if let Some(c) = gamepad_button(b) {
                        out.push(RawEvent::button(device, c, 1.0, time));
                    }
                }
                EventType::ButtonReleased(b, _) => {
                    if let Some(c) = gamepad_button(b) {
                        out.push(RawEvent::button(device, c, 0.0, time));
                    }
                }
                // Triggers are analog: 0 to 1.
                EventType::ButtonChanged(b, v, _) => {
                    if matches!(b, Button::LeftTrigger2 | Button::RightTrigger2)
                        && let Some(c) = gamepad_button(b)
                    {
                        out.push(RawEvent::button(device, c, v.clamp(0.0, 1.0), time));
                    }
                }
                EventType::Disconnected => {
                    for c in pad_controls() {
                        out.push(match c {
                            Control::Thumbstick1 | Control::Thumbstick2 => {
                                RawEvent::vector(device, c, [0.0; 2], time)
                            }
                            _ => RawEvent::button(device, c, 0.0, time),
                        });
                    }
                    self.sticks[slot as usize] = [[0.0; 2]; 2];
                    self.slots[slot as usize] = None;
                }
                _ => {}
            }
        }
        // The sticks are read from the pad's state each frame, as before.
        for slot in 0..MAX_GAMEPADS {
            let Some(pad) = self.slots[slot].and_then(|id| gilrs.connected_gamepad(id)) else {
                continue;
            };
            let now = [
                [pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)],
                [pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)],
            ];
            for (i, control) in [Control::Thumbstick1, Control::Thumbstick2]
                .into_iter()
                .enumerate()
            {
                if now[i] != self.sticks[slot][i] {
                    self.sticks[slot][i] = now[i];
                    out.push(RawEvent::vector(
                        Device::Gamepad(slot as u8),
                        control,
                        now[i],
                        time,
                    ));
                }
            }
        }
        self.gilrs = Some(gilrs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamepad_buttons_are_named_by_position() {
        let table = [
            (Button::South, Control::ButtonA),
            (Button::East, Control::ButtonB),
            (Button::West, Control::ButtonX),
            (Button::North, Control::ButtonY),
            (Button::LeftTrigger, Control::ButtonL1),
            (Button::RightTrigger, Control::ButtonR1),
            (Button::LeftTrigger2, Control::ButtonL2),
            (Button::RightTrigger2, Control::ButtonR2),
            (Button::LeftThumb, Control::ButtonL3),
            (Button::RightThumb, Control::ButtonR3),
            (Button::Select, Control::ButtonSelect),
            (Button::Start, Control::ButtonStart),
            (Button::DPadUp, Control::DPadUp),
            (Button::DPadDown, Control::DPadDown),
            (Button::DPadLeft, Control::DPadLeft),
            (Button::DPadRight, Control::DPadRight),
        ];
        for (button, control) in table {
            assert_eq!(gamepad_button(button), Some(control), "{button:?}");
        }
        for button in [Button::Mode, Button::C, Button::Z, Button::Unknown] {
            assert_eq!(gamepad_button(button), None, "{button:?}");
        }
    }

    #[test]
    fn keys_map_to_the_roblox_names() {
        for (key, name) in [
            (KeyCode::KeyW, "W"),
            (KeyCode::Digit1, "One"),
            (KeyCode::Space, "Space"),
            (KeyCode::ShiftLeft, "LeftShift"),
            (KeyCode::ArrowUp, "Up"),
            (KeyCode::Enter, "Return"),
            (KeyCode::Backslash, "BackSlash"),
        ] {
            assert_eq!(key_control(key).map(Control::name), Some(name), "{key:?}");
        }
        assert_eq!(key_control(KeyCode::F13), None);
    }

    #[test]
    fn mouse_buttons_map_to_the_first_three() {
        assert_eq!(
            mouse_control(MouseButton::Left),
            Some(Control::MouseButton1)
        );
        assert_eq!(
            mouse_control(MouseButton::Right),
            Some(Control::MouseButton2)
        );
        assert_eq!(
            mouse_control(MouseButton::Middle),
            Some(Control::MouseButton3)
        );
        assert_eq!(mouse_control(MouseButton::Back), None);
    }
}
