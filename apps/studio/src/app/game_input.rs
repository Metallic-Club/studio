//! The Game tab's input hand-off (ADR 0010 §4): while the running game has the keyboard and mouse, the interface's
//! events become the platform's ([`InputEvent`]), as a window of the game's own would deliver them, and go to the
//! Player.
//!
//! - **Keys** by their physical place ([`KeyCode`], what game controls read), with the text a press typed.
//! - **Modifiers:** egui reports Shift, Ctrl, Alt and the Super key as state, not as keys, so their changes become
//!   key presses and releases of the left-hand keys, and a [`WindowEvent::ModifiersChanged`].
//! - **The pointer** in physical pixels from the tab's top-left corner; it enters and leaves with the tab.
//! - **Raw mouse motion** (`MouseMotion`), which keeps coming while the cursor is held (`MouseBehavior`).
//!
//! The game sees **Esc** too; Studio then takes the input back ([`GameInput::convert`] says so).

use egui::{Event, Modifiers as EguiModifiers, PointerButton, Pos2, Rect};
use metallic_engine::InputEvent;
use metallic_engine::metallic_platform::{
    ButtonState, DeviceEvent, Key, KeyCode, KeyEvent, Modifiers, MouseButton, NamedKey, Position, ScrollDelta,
    WindowEvent, WindowId,
};

/// The id the Game tab's events carry: the game has one window, the tab.
pub(crate) const GAME_WINDOW: WindowId = WindowId::from_raw(1);

/// What the interface's events became for the game.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Converted {
    /// The game's events, in order.
    pub events: Vec<InputEvent>,
    /// Esc was pressed: the game saw it, and Studio takes the input back.
    pub escape: bool,
}

/// The hand-off's memory between frames: the modifiers and whether the pointer is over the tab.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GameInput {
    modifiers: EguiModifiers,
    inside: bool,
}

impl GameInput {
    /// The game's events for this frame's `events`, over the tab at `rect` (points) drawn at `pixels_per_point`.
    pub(crate) fn convert(&mut self, events: &[Event], rect: Rect, pixels_per_point: f32) -> Converted {
        let mut out = Converted::default();
        for event in events {
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    self.modifiers_to(*modifiers, &mut out.events);
                    let code = key_code(physical_key.unwrap_or(*key));
                    if *pressed && code == KeyCode::Escape {
                        out.escape = true;
                    }
                    out.events.push(window(WindowEvent::Key(KeyEvent {
                        key_code: code,
                        key: logical(*key),
                        text: None,
                        state: if *pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        repeat: *repeat,
                    })));
                }
                Event::Text(text) => {
                    // The text a press typed belongs to that press, which came just before it.
                    if let Some(InputEvent::Window {
                        event: WindowEvent::Key(key),
                        ..
                    }) = out.events.last_mut()
                        && key.state == ButtonState::Pressed
                        && key.text.is_none()
                    {
                        key.text = Some(text.clone());
                    }
                }
                Event::PointerMoved(at) => self.pointer_to(*at, rect, pixels_per_point, &mut out.events),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    self.modifiers_to(*modifiers, &mut out.events);
                    self.pointer_to(*pos, rect, pixels_per_point, &mut out.events);
                    out.events.push(window(WindowEvent::MouseButton {
                        button: mouse_button(*button),
                        state: if *pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                    }));
                }
                Event::PointerGone if self.inside => {
                    self.inside = false;
                    out.events.push(window(WindowEvent::CursorLeft));
                }
                Event::MouseWheel { unit, delta, .. } => {
                    let scroll = match unit {
                        egui::MouseWheelUnit::Point => ScrollDelta::Pixels {
                            x: f64::from(delta.x * pixels_per_point),
                            y: f64::from(delta.y * pixels_per_point),
                        },
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                            ScrollDelta::Lines { x: delta.x, y: delta.y }
                        }
                    };
                    out.events.push(window(WindowEvent::MouseWheel(scroll)));
                }
                Event::MouseMoved(delta) => out.events.push(InputEvent::Device(DeviceEvent::MouseMotion {
                    dx: f64::from(delta.x),
                    dy: f64::from(delta.y),
                })),
                Event::WindowFocused(focused) => out.events.push(window(WindowEvent::Focused(*focused))),
                _ => {}
            }
        }
        out
    }

    /// The game takes the input: it gets the focus, and the modifiers held now.
    pub(crate) fn take(&mut self, modifiers: EguiModifiers) -> Vec<InputEvent> {
        let mut events = vec![window(WindowEvent::Focused(true))];
        self.modifiers = EguiModifiers::NONE;
        self.modifiers_to(modifiers, &mut events);
        events
    }

    /// The game lets go of the input: what it holds is released, and it loses the focus.
    pub(crate) fn release(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        self.modifiers_to(EguiModifiers::NONE, &mut events);
        if std::mem::take(&mut self.inside) {
            events.push(window(WindowEvent::CursorLeft));
        }
        events.push(window(WindowEvent::Focused(false)));
        events
    }

    /// Key events for the modifiers that changed to `now`, and the new set.
    fn modifiers_to(&mut self, now: EguiModifiers, out: &mut Vec<InputEvent>) {
        let before = self.modifiers;
        if before == now {
            return;
        }
        for (was, is, code, named) in [
            (before.shift, now.shift, KeyCode::ShiftLeft, NamedKey::Shift),
            (before.ctrl, now.ctrl, KeyCode::ControlLeft, NamedKey::Control),
            (before.alt, now.alt, KeyCode::AltLeft, NamedKey::Alt),
            (before.mac_cmd, now.mac_cmd, KeyCode::SuperLeft, NamedKey::Super),
        ] {
            if was != is {
                out.push(window(WindowEvent::Key(KeyEvent {
                    key_code: code,
                    key: Key::Named(named),
                    text: None,
                    state: if is {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                    repeat: false,
                })));
            }
        }
        out.push(window(WindowEvent::ModifiersChanged(Modifiers {
            shift: now.shift,
            control: now.ctrl,
            alt: now.alt,
            super_key: now.mac_cmd,
        })));
        self.modifiers = now;
    }

    /// The pointer at `at` (points): entering or leaving the tab, and moving in it.
    fn pointer_to(&mut self, at: Pos2, rect: Rect, pixels_per_point: f32, out: &mut Vec<InputEvent>) {
        let inside = rect.contains(at);
        if inside != self.inside {
            self.inside = inside;
            out.push(window(if inside {
                WindowEvent::CursorEntered
            } else {
                WindowEvent::CursorLeft
            }));
        }
        let from = (at - rect.min) * pixels_per_point;
        out.push(window(WindowEvent::CursorMoved(Position {
            x: f64::from(from.x),
            y: f64::from(from.y),
        })));
    }
}

/// `event` on the game's window.
fn window(event: WindowEvent) -> InputEvent {
    InputEvent::Window {
        window: GAME_WINDOW,
        event,
    }
}

fn mouse_button(button: PointerButton) -> MouseButton {
    match button {
        PointerButton::Primary => MouseButton::Left,
        PointerButton::Secondary => MouseButton::Right,
        PointerButton::Middle => MouseButton::Middle,
        PointerButton::Extra1 => MouseButton::Back,
        PointerButton::Extra2 => MouseButton::Forward,
    }
}

/// What a key means: a named key, or the text it stands for (egui reports a key's name, not the layout's text).
fn logical(key: egui::Key) -> Key {
    use egui::Key as K;
    let named = match key {
        K::Enter => NamedKey::Enter,
        K::Tab => NamedKey::Tab,
        K::Space => NamedKey::Space,
        K::Backspace => NamedKey::Backspace,
        K::Delete => NamedKey::Delete,
        K::Escape => NamedKey::Escape,
        K::Insert => NamedKey::Insert,
        K::Home => NamedKey::Home,
        K::End => NamedKey::End,
        K::PageUp => NamedKey::PageUp,
        K::PageDown => NamedKey::PageDown,
        K::ArrowUp => NamedKey::ArrowUp,
        K::ArrowDown => NamedKey::ArrowDown,
        K::ArrowLeft => NamedKey::ArrowLeft,
        K::ArrowRight => NamedKey::ArrowRight,
        other => {
            let symbol = other.symbol_or_name();
            return if symbol.chars().count() == 1 {
                Key::Character(symbol.to_lowercase())
            } else {
                Key::Unidentified
            };
        }
    };
    Key::Named(named)
}

/// Where a key is: egui's physical key as the platform names it.
fn key_code(key: egui::Key) -> KeyCode {
    use egui::Key as K;
    match key {
        K::ArrowDown => KeyCode::ArrowDown,
        K::ArrowLeft => KeyCode::ArrowLeft,
        K::ArrowRight => KeyCode::ArrowRight,
        K::ArrowUp => KeyCode::ArrowUp,
        K::Escape => KeyCode::Escape,
        K::Tab => KeyCode::Tab,
        K::Backspace => KeyCode::Backspace,
        K::Enter => KeyCode::Enter,
        K::Space => KeyCode::Space,
        K::Insert => KeyCode::Insert,
        K::Delete => KeyCode::Delete,
        K::Home => KeyCode::Home,
        K::End => KeyCode::End,
        K::PageUp => KeyCode::PageUp,
        K::PageDown => KeyCode::PageDown,
        K::Comma => KeyCode::Comma,
        K::Backslash | K::Pipe => KeyCode::Backslash,
        K::Slash | K::Questionmark => KeyCode::Slash,
        K::OpenBracket | K::OpenCurlyBracket => KeyCode::BracketLeft,
        K::CloseBracket | K::CloseCurlyBracket => KeyCode::BracketRight,
        K::Backtick => KeyCode::Backquote,
        K::Minus => KeyCode::Minus,
        K::Period => KeyCode::Period,
        K::Plus | K::Equals => KeyCode::Equal,
        K::Semicolon | K::Colon => KeyCode::Semicolon,
        K::Quote => KeyCode::Quote,
        K::Num0 => KeyCode::Digit0,
        K::Num1 | K::Exclamationmark => KeyCode::Digit1,
        K::Num2 => KeyCode::Digit2,
        K::Num3 => KeyCode::Digit3,
        K::Num4 => KeyCode::Digit4,
        K::Num5 => KeyCode::Digit5,
        K::Num6 => KeyCode::Digit6,
        K::Num7 => KeyCode::Digit7,
        K::Num8 => KeyCode::Digit8,
        K::Num9 => KeyCode::Digit9,
        K::A => KeyCode::KeyA,
        K::B => KeyCode::KeyB,
        K::C => KeyCode::KeyC,
        K::D => KeyCode::KeyD,
        K::E => KeyCode::KeyE,
        K::F => KeyCode::KeyF,
        K::G => KeyCode::KeyG,
        K::H => KeyCode::KeyH,
        K::I => KeyCode::KeyI,
        K::J => KeyCode::KeyJ,
        K::K => KeyCode::KeyK,
        K::L => KeyCode::KeyL,
        K::M => KeyCode::KeyM,
        K::N => KeyCode::KeyN,
        K::O => KeyCode::KeyO,
        K::P => KeyCode::KeyP,
        K::Q => KeyCode::KeyQ,
        K::R => KeyCode::KeyR,
        K::S => KeyCode::KeyS,
        K::T => KeyCode::KeyT,
        K::U => KeyCode::KeyU,
        K::V => KeyCode::KeyV,
        K::W => KeyCode::KeyW,
        K::X => KeyCode::KeyX,
        K::Y => KeyCode::KeyY,
        K::Z => KeyCode::KeyZ,
        K::F1 => KeyCode::F1,
        K::F2 => KeyCode::F2,
        K::F3 => KeyCode::F3,
        K::F4 => KeyCode::F4,
        K::F5 => KeyCode::F5,
        K::F6 => KeyCode::F6,
        K::F7 => KeyCode::F7,
        K::F8 => KeyCode::F8,
        K::F9 => KeyCode::F9,
        K::F10 => KeyCode::F10,
        K::F11 => KeyCode::F11,
        K::F12 => KeyCode::F12,
        K::BrowserBack => KeyCode::BrowserBack,
        K::BrowserForward => KeyCode::BrowserForward,
        _ => KeyCode::Unidentified,
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::*;

    const TAB: Rect = Rect {
        min: pos2(100.0, 50.0),
        max: pos2(500.0, 350.0),
    };

    fn key(key: egui::Key, pressed: bool, modifiers: EguiModifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn window_events(converted: &Converted) -> Vec<&WindowEvent> {
        converted
            .events
            .iter()
            .filter_map(|event| match event {
                InputEvent::Window { event, .. } => Some(event),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn keys_become_physical_keys_with_their_text() {
        let mut input = GameInput::default();
        let out = input.convert(
            &[key(egui::Key::W, true, EguiModifiers::NONE), Event::Text("w".into())],
            TAB,
            1.0,
        );
        let [WindowEvent::Key(press)] = window_events(&out)[..] else {
            panic!("{out:?}");
        };
        assert_eq!(press.key_code, KeyCode::KeyW);
        assert_eq!(press.key, Key::Character("w".into()));
        assert_eq!(press.text.as_deref(), Some("w"));
        assert_eq!(press.state, ButtonState::Pressed);
        assert!(!out.escape);

        let out = input.convert(&[key(egui::Key::Escape, true, EguiModifiers::NONE)], TAB, 1.0);
        assert!(out.escape, "Esc hands the input back");
        assert!(matches!(
            window_events(&out)[..],
            [WindowEvent::Key(KeyEvent {
                key_code: KeyCode::Escape,
                ..
            })]
        ));
    }

    #[test]
    fn modifier_changes_become_presses_and_releases() {
        let mut input = GameInput::default();
        let out = input.convert(&[key(egui::Key::A, true, EguiModifiers::SHIFT)], TAB, 1.0);
        let events = window_events(&out);
        assert!(matches!(
            events[0],
            WindowEvent::Key(KeyEvent {
                key_code: KeyCode::ShiftLeft,
                state: ButtonState::Pressed,
                ..
            })
        ));
        assert!(matches!(
            events[1],
            WindowEvent::ModifiersChanged(Modifiers { shift: true, .. })
        ));
        assert!(matches!(
            events[2],
            WindowEvent::Key(KeyEvent {
                key_code: KeyCode::KeyA,
                ..
            })
        ));
        // Held: nothing new. Let go: released.
        let out = input.convert(&[key(egui::Key::A, false, EguiModifiers::SHIFT)], TAB, 1.0);
        assert_eq!(window_events(&out).len(), 1);
        let out = Converted {
            events: input.release(),
            escape: false,
        };
        assert!(matches!(
            window_events(&out)[0],
            WindowEvent::Key(KeyEvent {
                key_code: KeyCode::ShiftLeft,
                state: ButtonState::Released,
                ..
            })
        ));
        assert_eq!(window_events(&out).last(), Some(&&WindowEvent::Focused(false)));
    }

    #[test]
    fn the_pointer_is_in_the_tabs_physical_pixels_and_enters_and_leaves() {
        let mut input = GameInput::default();
        let out = input.convert(&[Event::PointerMoved(pos2(110.0, 60.0))], TAB, 2.0);
        assert_eq!(
            window_events(&out),
            [
                &WindowEvent::CursorEntered,
                &WindowEvent::CursorMoved(Position { x: 20.0, y: 20.0 })
            ]
        );
        let out = input.convert(
            &[Event::PointerButton {
                pos: pos2(110.0, 60.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: EguiModifiers::NONE,
            }],
            TAB,
            2.0,
        );
        assert_eq!(
            window_events(&out)[1],
            &WindowEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed
            }
        );
        let out = input.convert(&[Event::PointerMoved(pos2(10.0, 10.0))], TAB, 2.0);
        assert_eq!(window_events(&out)[0], &WindowEvent::CursorLeft);

        let out = input.convert(&[Event::MouseMoved(vec2(3.0, -4.0))], TAB, 2.0);
        assert_eq!(
            out.events,
            [InputEvent::Device(DeviceEvent::MouseMotion { dx: 3.0, dy: -4.0 })]
        );
        let out = input.convert(
            &[Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                modifiers: EguiModifiers::NONE,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
            }],
            TAB,
            2.0,
        );
        assert_eq!(
            window_events(&out),
            [&WindowEvent::MouseWheel(ScrollDelta::Lines { x: 0.0, y: 1.0 })]
        );
    }
}
