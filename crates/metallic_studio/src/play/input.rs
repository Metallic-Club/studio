//! Studio's keyboard and mouse as the game's raw input while Play runs.
//!
//! The Player turns winit events into [`RawEvent`]s; in Studio the window belongs to egui, so
//! [`PlayInput::feed`] reads the frame's egui events instead and pushes the same events to the
//! running [`PlayView`]:
//!
//! - **Keyboard** goes to the game while no text field has focus. Studio's own shortcuts are
//!   dispatched first (the keymap consumes only the ones Studio keeps while playing), so Esc and
//!   Shift+F5 stop and never reach the game. Forwarded key events are taken out of egui's list
//!   so no focused widget reacts too. Shift, Ctrl and Alt come from the modifier state (egui
//!   reports no key events for them).
//! - **Tab, Space and Enter** are taken out even earlier, in the app's raw input hook
//!   ([`PlayInput::hold_game_keys`]): egui handles Tab focus moves before any widget runs, so a
//!   later filter could not stop Tab from focusing a Studio button, or Space from pressing it.
//! - **Mouse** goes to the game while the pointer is over the viewport, or while a button
//!   pressed there is still held (a right-drag camera turn may leave the panel). Releases always
//!   go through, so nothing stays held.
//! - Losing the keyboard (a text field took it) or the window's focus releases every control.
//!
//! The event buffers are kept between frames, so a frame allocates nothing once they have grown.
use eframe::egui::{self, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Rect};
use metallic_runtime::input::{Control, Device, RawEvent};

use super::PlayView;
use crate::input_actions::egui_key_to_control;

/// Pixels of a smooth (touchpad) scroll counted as one wheel notch.
const POINTS_PER_NOTCH: f32 = 60.0;

/// Keys egui would use for focus and activation; while the game has the keyboard they are the
/// game's alone.
const GAME_ONLY: [Key; 3] = [Key::Tab, Key::Space, Key::Enter];

/// Per-session input state: what is held, so focus changes release it.
#[derive(Debug, Default)]
pub struct PlayInput {
    modifiers: Modifiers,
    /// Mouse buttons pressed inside the viewport and not yet released.
    held: [bool; 3],
    /// The game had the keyboard last frame.
    had_keyboard: bool,
    /// Tab, Space and Enter events taken from the raw input before egui saw them.
    game_keys: Vec<Event>,
    /// This frame's events for the game (reused).
    out: Vec<RawEvent>,
}

impl PlayInput {
    /// Fresh state for a new session. The game owns the keyboard from the start.
    pub fn new() -> PlayInput {
        PlayInput {
            had_keyboard: true,
            ..PlayInput::default()
        }
    }

    /// Takes Tab, Space and Enter out of `raw` while the game has the keyboard; [`Self::feed`]
    /// sends them to the game. Call it before egui runs the frame (eframe's raw input hook).
    pub fn hold_game_keys(&mut self, raw: &mut egui::RawInput) {
        if !self.had_keyboard {
            return;
        }
        let held = &mut self.game_keys;
        raw.events.retain(|e| match e {
            Event::Key {
                key, physical_key, ..
            } if GAME_ONLY.contains(&physical_key.unwrap_or(*key)) => {
                held.push(e.clone());
                false
            }
            _ => true,
        });
    }

    /// Releases everything the game holds (a prompt took over the keyboard).
    pub fn release(&mut self, view: &mut dyn PlayView) {
        if self.had_keyboard || self.held.iter().any(|&h| h) {
            view.release_input();
        }
        self.modifiers = Modifiers::NONE;
        self.held = [false; 3];
        self.had_keyboard = false;
        self.game_keys.clear();
    }

    /// Sends this frame's egui input to `view`. `viewport` is the 3D view's screen rectangle and
    /// `hovered` whether the pointer is over it (both from the last drawn frame).
    pub fn feed(
        &mut self,
        ctx: &egui::Context,
        view: &mut dyn PlayView,
        viewport: Rect,
        hovered: bool,
    ) {
        // Only a text field takes the keyboard from the game; a focused button does not.
        let keyboard = !ctx.text_edit_focused() && ctx.input(|i| i.focused);
        if self.had_keyboard && !keyboard {
            self.release(view);
        }
        self.had_keyboard = keyboard;
        let time = view.input_time();
        let ppp = ctx.pixels_per_point();
        let out = &mut self.out;
        out.clear();
        if keyboard {
            for e in &self.game_keys {
                if let Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat: false,
                    ..
                } = e
                    && let Some(c) = egui_key_to_control(physical_key.unwrap_or(*key))
                {
                    out.push(button(c, *pressed, time));
                }
            }
        }
        self.game_keys.clear();
        let (modifiers, held) = (&mut self.modifiers, &mut self.held);
        ctx.input_mut(|i| {
            if keyboard {
                let now = i.modifiers;
                for (was, is, control) in [
                    (modifiers.shift, now.shift, Control::LeftShift),
                    (modifiers.ctrl, now.ctrl, Control::LeftControl),
                    (modifiers.alt, now.alt, Control::LeftAlt),
                ] {
                    if was != is {
                        out.push(button(control, is, time));
                    }
                }
                *modifiers = now;
            }
            let pointer_owned = hovered || held.iter().any(|&h| h);
            i.events.retain(|event| match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    ..
                } if keyboard => {
                    if !repeat && let Some(c) = egui_key_to_control(physical_key.unwrap_or(*key)) {
                        out.push(button(c, *pressed, time));
                    }
                    false
                }
                Event::PointerButton {
                    button: b, pressed, ..
                } => {
                    let Some((slot, control)) = mouse_control(*b) else {
                        return true;
                    };
                    if *pressed && hovered {
                        held[slot] = true;
                        out.push(button(control, true, time));
                    } else if !*pressed && held[slot] {
                        held[slot] = false;
                        out.push(button(control, false, time));
                    }
                    true
                }
                Event::MouseWheel { unit, delta, .. } if hovered => {
                    let notches = match unit {
                        MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
                        MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
                    };
                    if notches != 0.0 {
                        out.push(RawEvent::wheel(Device::KeyboardMouse, notches, time));
                    }
                    true
                }
                _ => true,
            });
            if pointer_owned {
                let motion = i.pointer.motion().unwrap_or_else(|| i.pointer.delta()) * ppp;
                if motion != egui::Vec2::ZERO {
                    out.push(RawEvent::vector(
                        Device::KeyboardMouse,
                        Control::MouseDelta,
                        [motion.x, motion.y],
                        time,
                    ));
                }
                if let Some(p) = i.pointer.latest_pos() {
                    let local = (p - viewport.min) * ppp;
                    out.push(RawEvent::vector(
                        Device::KeyboardMouse,
                        Control::MousePosition,
                        [local.x, local.y],
                        time,
                    ));
                }
            }
            if keyboard {
                // Keys were taken out of the event list; clear the derived state too.
                i.keys_down.clear();
            }
        });
        for &e in &self.out {
            view.push_input(e);
        }
    }
}

fn button(control: Control, down: bool, time: f64) -> RawEvent {
    RawEvent::button(
        Device::KeyboardMouse,
        control,
        f32::from(u8::from(down)),
        time,
    )
}

fn mouse_control(b: PointerButton) -> Option<(usize, Control)> {
    match b {
        PointerButton::Primary => Some((0, Control::MouseButton1)),
        PointerButton::Secondary => Some((1, Control::MouseButton2)),
        PointerButton::Middle => Some((2, Control::MouseButton3)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Recorder {
        events: RefCell<Vec<RawEvent>>,
        released: usize,
    }

    impl PlayView for Recorder {
        fn input_time(&self) -> f64 {
            1.5
        }
        fn push_input(&mut self, event: RawEvent) {
            self.events.borrow_mut().push(event);
        }
        fn release_input(&mut self) {
            self.released += 1;
        }
        fn render(&self, _: &mut metallic_render::Renderer, _: &wgpu::TextureView, _: (u32, u32)) {}
        fn paint_gui(&self, _: &egui::Painter) {}
        fn inspect_world(&self, _: &mut dyn FnMut(&metallic_core::World)) {}
    }

    fn key(key: egui::Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn run(events: Vec<Event>, hovered: bool, input: &mut PlayInput, view: &mut Recorder) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events,
            focused: true,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            input.feed(
                &ctx,
                view,
                Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
                hovered,
            );
        });
        out.textures_delta.clear();
    }

    #[test]
    fn keys_become_controls_and_wheel_needs_the_pointer() {
        let mut input = PlayInput::new();
        let mut view = Recorder::default();
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Wheel,
        };
        run(
            vec![
                key(egui::Key::W, true),
                key(egui::Key::Space, false),
                wheel.clone(),
            ],
            false,
            &mut input,
            &mut view,
        );
        let events = view.events.borrow().clone();
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].control, Control::W);
        assert_eq!(events[0].value, 1.0);
        assert_eq!(events[0].time, 1.5);
        assert_eq!(events[1].control, Control::Space);
        assert_eq!(events[1].value, 0.0);
        view.events.borrow_mut().clear();
        run(vec![wheel], true, &mut input, &mut view);
        assert!(
            view.events
                .borrow()
                .iter()
                .any(|e| e.control == Control::MouseWheel)
        );
    }
}
