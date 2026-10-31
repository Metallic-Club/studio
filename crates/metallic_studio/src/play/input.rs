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
//! - **The game's GUI first** (#79), exactly as in the Player: the pointer position and the
//!   primary button go to [`PlayView::gui_pointer`] before the game's input. A primary press the
//!   GUI takes (a button, an `Active` frame) is not sent as `MouseButton1`, nor is its release,
//!   so a click on a button never also reaches the camera, a jump or `ContextActionService`.
//!   Touch screens reach it through egui's emulated pointer (its first finger is the primary
//!   button), so a tap on a button activates it; Studio gives the game no other touch input.
//! - Losing the keyboard (a text field took it) or the window's focus releases every control.
//!
//! The event buffers are kept between frames, so a frame allocates nothing once they have grown.
use eframe::egui::{self, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Rect};
use metallic_runtime::gui_input::{PointerId, PointerPhase};
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
    /// The game's GUI took the primary press that is still held.
    gui_held: bool,
    /// The pointer was over the viewport last frame (for the GUI's `MouseLeave`).
    gui_hovered: bool,
    /// Where the pointer last was for the GUI, in viewport points.
    gui_at: Option<[f32; 2]>,
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
        if self.gui_held {
            let at = self.gui_at.unwrap_or([f32::NAN; 2]);
            view.gui_pointer(PointerId::Mouse, PointerPhase::Cancel, at, [0.0; 2]);
        }
        self.modifiers = Modifiers::NONE;
        self.held = [false; 3];
        self.gui_held = false;
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
        let screen = [viewport.width(), viewport.height()];
        let to_gui = |p: egui::Pos2| [p.x - viewport.min.x, p.y - viewport.min.y];
        // Hover and moves first, so a press lands on what the pointer is over now.
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        if hovered || self.gui_held {
            if let Some(p) = pointer.map(to_gui)
                && self.gui_at != Some(p)
            {
                self.gui_at = Some(p);
                view.gui_pointer(PointerId::Mouse, PointerPhase::Move, p, screen);
            }
        } else if self.gui_hovered {
            view.gui_mouse_left(self.gui_at.take().unwrap_or([f32::NAN; 2]));
        }
        self.gui_hovered = hovered;
        let (modifiers, held) = (&mut self.modifiers, &mut self.held);
        let (gui_held, gui_at) = (&mut self.gui_held, &mut self.gui_at);
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
                    button: b,
                    pressed,
                    pos,
                    ..
                } => {
                    let Some((slot, control)) = mouse_control(*b) else {
                        return true;
                    };
                    // The primary button goes to the game's GUI first.
                    if slot == 0 && *pressed && hovered {
                        let at = to_gui(*pos);
                        *gui_at = Some(at);
                        if view.gui_pointer(PointerId::Mouse, PointerPhase::Down, at, screen) {
                            *gui_held = true;
                            return true;
                        }
                    } else if slot == 0 && !*pressed && *gui_held {
                        *gui_held = false;
                        let at = to_gui(*pos);
                        view.gui_pointer(PointerId::Mouse, PointerPhase::Up, at, screen);
                        return true;
                    }
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
        /// GUI calls: phase, position, screen.
        gui: Vec<(PointerPhase, [f32; 2], [f32; 2])>,
        left: usize,
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
        /// A GUI button covers x < 100.
        fn gui_pointer(
            &mut self,
            pointer: PointerId,
            phase: PointerPhase,
            at: [f32; 2],
            screen: [f32; 2],
        ) -> bool {
            assert_eq!(pointer, PointerId::Mouse);
            self.gui.push((phase, at, screen));
            let held = self
                .gui
                .iter()
                .rev()
                .skip(1)
                .find_map(|(p, a, _)| matches!(p, PointerPhase::Down).then_some(a[0] < 100.0));
            match phase {
                PointerPhase::Down => at[0] < 100.0,
                _ => held.unwrap_or(false),
            }
        }
        fn gui_mouse_left(&mut self, _: [f32; 2]) {
            self.left += 1;
        }
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

    fn click(at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn mouse1(view: &Recorder) -> Vec<f32> {
        view.events
            .borrow()
            .iter()
            .filter(|e| e.control == Control::MouseButton1)
            .map(|e| e.value)
            .collect()
    }

    /// #79: a click the game's GUI takes is not the game's input; one elsewhere is.
    #[test]
    fn the_game_gui_gets_the_primary_button_first() {
        let mut input = PlayInput::new();
        let mut view = Recorder::default();
        // The viewport starts at (0, 0) and is 800 x 600 points (see `run`).
        let on = egui::pos2(40.0, 30.0);
        run(
            vec![Event::PointerMoved(on), click(on, true)],
            true,
            &mut input,
            &mut view,
        );
        run(vec![click(on, false)], true, &mut input, &mut view);
        assert!(mouse1(&view).is_empty(), "the GUI took the click");
        let phases: Vec<PointerPhase> = view.gui.iter().map(|g| g.0).collect();
        assert_eq!(
            phases,
            [PointerPhase::Move, PointerPhase::Down, PointerPhase::Up]
        );
        assert_eq!(view.gui[1].1, [40.0, 30.0]);
        assert_eq!(view.gui[1].2, [800.0, 600.0]);
        let off = egui::pos2(400.0, 30.0);
        run(
            vec![Event::PointerMoved(off), click(off, true)],
            true,
            &mut input,
            &mut view,
        );
        run(vec![click(off, false)], true, &mut input, &mut view);
        assert_eq!(mouse1(&view), [1.0, 0.0], "off the GUI: the game's");
        // Leaving the viewport is the GUI's MouseLeave.
        run(Vec::new(), false, &mut input, &mut view);
        assert_eq!(view.left, 1);
    }

    #[test]
    fn a_held_gui_press_is_cancelled_when_input_is_released() {
        let mut input = PlayInput::new();
        let mut view = Recorder::default();
        let on = egui::pos2(40.0, 30.0);
        run(vec![click(on, true)], true, &mut input, &mut view);
        input.release(&mut view);
        assert_eq!(view.gui.last().map(|g| g.0), Some(PointerPhase::Cancel));
        run(vec![click(on, false)], true, &mut input, &mut view);
        assert!(mouse1(&view).is_empty(), "the stray release goes nowhere");
    }
}
