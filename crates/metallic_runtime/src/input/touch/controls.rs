//! Finger routing (spec 7.5): each finger is captured by the first thing it lands on, for its
//! whole life, and turned into raw events on `Device::Touch`.
use super::layout::{DeviceProfile, TouchLayout, dist, layout};
use super::stick::Stick;
use super::{MENU_SLOT, TouchSet};
use crate::input::control::{Control, Device, RawEvent, TOUCH_BUTTON_SLOTS};
use crate::input::runtime::InputRuntime;
use metallic_script::TouchButtonInfo;

/// Fingers followed at once; more are ignored.
pub const MAX_FINGERS: usize = 10;
/// A finger that slides further than this many button radii from the centre cancels the press.
const CANCEL_RADII: f32 = 2.0;

/// What a finger does in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    /// It touched the screen.
    Down,
    /// It moved.
    Move,
    /// It was lifted.
    Up,
    /// The system took it (a gesture, the window lost the touch): presses end without
    /// `Triggered`.
    Cancel,
}

/// What took a finger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// The 8 dp around a button, or a second finger on a held stick's half: nothing happens.
    Ignored,
    /// The thumbstick.
    Stick,
    /// The look pad.
    Look,
    /// A button on control slot `slot`; `live` turns false when the finger slid off.
    Button {
        /// Control slot.
        slot: usize,
        /// Still pressing.
        live: bool,
    },
    /// The menu button, which acts when the finger is lifted on it.
    Menu {
        /// The finger is still close enough to act on release.
        armed: bool,
    },
    /// A pointer touch: `MousePosition` in pixels, `MouseButton1` while down.
    Pointer,
}

#[derive(Debug, Clone, Copy)]
struct Finger {
    id: u64,
    capture: Capture,
    last: [f32; 2],
    /// The button it landed on, where it was then (centre, radius): sliding more than
    /// [`CANCEL_RADII`] from there ends the press, even when a new layout moved the button.
    anchor: Option<([f32; 2], f32)>,
}

/// The touch overlay's input half: layout plus finger state. The Player (and Studio's preview)
/// owns one, calls [`sync`](Self::sync) each frame and [`touch`](Self::touch) for each finger
/// event; [`paint`](super::paint) draws it. Nothing here allocates per event; a new layout is
/// computed only when the screen or the touch actions change.
#[derive(Debug, Clone)]
pub struct TouchControls {
    profile: DeviceProfile,
    set: TouchSet,
    layout: TouchLayout,
    generation: Option<u64>,
    fingers: [Option<Finger>; MAX_FINGERS],
    stick: Option<Stick>,
    held: [u8; TOUCH_BUTTON_SLOTS],
    /// Pointer fingers down (`MouseButton1` is held while any is).
    pointers: u8,
    touched: bool,
    px_per_dp: f32,
}

impl TouchControls {
    /// Controls for a screen; empty until the first [`sync`](Self::sync).
    pub fn new(profile: DeviceProfile) -> TouchControls {
        TouchControls {
            profile,
            set: TouchSet::default(),
            layout: layout(&profile, &TouchSet::default()),
            generation: None,
            fingers: [None; MAX_FINGERS],
            stick: None,
            held: [0; TOUCH_BUTTON_SLOTS],
            pointers: 0,
            touched: false,
            px_per_dp: 1.0,
        }
    }

    /// The screen.
    pub fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    /// Changes the screen (a resize or rotation) and places the controls again. Held fingers keep
    /// what they hold.
    pub fn set_profile(&mut self, profile: DeviceProfile) {
        if profile != self.profile {
            self.profile = profile;
            self.layout = layout(&self.profile, &self.set);
        }
    }

    /// Pixels per dp, for the pointer positions sent as `MousePosition`.
    pub fn set_px_per_dp(&mut self, px_per_dp: f32) {
        if px_per_dp.is_finite() && px_per_dp > 0.0 {
            self.px_per_dp = px_per_dp;
        }
    }

    /// Follows the runtime's touch actions: when they changed (a scene edit, `BindAction`,
    /// `SetTitle`), the controls are placed again and held fingers keep what they hold, matched
    /// by action path. A finger whose control went away, or whose action changed its role, holds
    /// nothing from then on (the runtime ends its press as a cancel). Returns true when they changed.
    pub fn sync(&mut self, rt: &mut InputRuntime, time: f64) -> bool {
        if self.generation == Some(rt.touch_generation()) {
            return false;
        }
        self.generation = Some(rt.touch_generation());
        let old = std::mem::replace(&mut self.set, rt.touch_set().clone());
        self.layout = layout(&self.profile, &self.set);
        self.remap(rt, &old, time);
        true
    }

    fn remap(&mut self, rt: &mut InputRuntime, old: &TouchSet, time: f64) {
        self.held = [0; TOUCH_BUTTON_SLOTS];
        for i in 0..MAX_FINGERS {
            let Some(mut f) = self.fingers[i] else {
                continue;
            };
            let same = |slot: usize| old.button_in_slot(slot).and_then(|d| self.set.follow(d));
            f.capture = match f.capture {
                Capture::Button { slot, live } => match same(slot) {
                    Some(s) if s != MENU_SLOT => {
                        if live {
                            self.held[s] = self.held[s].saturating_add(1);
                        }
                        Capture::Button { slot: s, live }
                    }
                    _ => Capture::Ignored,
                },
                Capture::Menu { armed } => match same(MENU_SLOT) {
                    Some(MENU_SLOT) => Capture::Menu { armed },
                    _ => Capture::Ignored,
                },
                Capture::Stick if self.set.stick.is_none() => {
                    if self.stick.take().is_some() {
                        rt.push(RawEvent::vector(
                            Device::Touch,
                            Control::TouchStick,
                            [0.0; 2],
                            time,
                        ));
                    }
                    Capture::Ignored
                }
                Capture::Look if self.set.look.is_none() => Capture::Ignored,
                c => c,
            };
            self.fingers[i] = Some(f);
        }
        // The runtime moved held buttons with their actions; saying so again changes nothing
        // and covers several loads between two syncs.
        for slot in 0..TOUCH_BUTTON_SLOTS {
            if self.held[slot] > 0 {
                push_button(rt, slot, 1.0, time);
            }
        }
    }

    /// Which actions have touch controls.
    pub fn set(&self) -> &TouchSet {
        &self.set
    }

    /// Where the controls are.
    pub fn layout(&self) -> &TouchLayout {
        &self.layout
    }

    /// The stick while a finger holds it.
    pub fn held_stick(&self) -> Option<&Stick> {
        self.stick.as_ref()
    }

    /// True while a finger presses the button on control slot `slot` (the menu button: while an
    /// armed finger is on it).
    pub fn is_pressed(&self, slot: usize) -> bool {
        if slot == MENU_SLOT {
            return self
                .fingers
                .iter()
                .flatten()
                .any(|f| f.capture == Capture::Menu { armed: true });
        }
        self.held.get(slot).is_some_and(|&n| n > 0)
    }

    /// True after the first touch of the session (the stick's hint ring hides then).
    pub fn touched_once(&self) -> bool {
        self.touched
    }

    /// What finger `id` is captured by.
    pub fn capture_of(&self, id: u64) -> Option<Capture> {
        self.fingers
            .iter()
            .flatten()
            .find(|f| f.id == id)
            .map(|f| f.capture)
    }

    /// The placed buttons of scene actions, for the scripts (`ContextActionService:GetButton`):
    /// clears `out` and fills it. `visible` is whether the overlay shows.
    pub fn button_infos(&self, visible: bool, out: &mut Vec<TouchButtonInfo>) {
        out.clear();
        for b in self.layout.buttons.iter().chain(self.layout.menu.iter()) {
            let Some(def) = self.set.button_in_slot(b.slot) else {
                continue;
            };
            let Some(action) = def.instance else {
                continue;
            };
            let r = b.rect();
            out.push(TouchButtonInfo {
                action,
                position: [r.x, r.y],
                size: [r.w, r.h],
                title: def.label.clone(),
                image: def.image.clone(),
                visible,
            });
        }
    }

    /// Fingers on the screen.
    pub fn finger_count(&self) -> usize {
        self.fingers.iter().flatten().count()
    }

    /// One finger event at `at` dp, at input-clock `time`. A non-finite position is ignored,
    /// except that `Up` and `Cancel` still lift the finger (where it last was). A `Down` for a
    /// finger that is still down ends the old touch as a cancel first (its `Up` was lost).
    pub fn touch(
        &mut self,
        rt: &mut InputRuntime,
        id: u64,
        phase: TouchPhase,
        at: [f32; 2],
        time: f64,
    ) {
        // A bad time becomes 0: the runtime never runs its clock backwards.
        let time = if time.is_finite() { time } else { 0.0 };
        let finite = at[0].is_finite() && at[1].is_finite();
        let slot = self
            .fingers
            .iter()
            .position(|f| f.is_some_and(|f| f.id == id));
        match (phase, slot) {
            (TouchPhase::Down, Some(i)) => {
                self.lift(rt, i, true, time);
                if finite {
                    self.down(rt, id, at, time);
                }
            }
            (TouchPhase::Down, None) if finite => self.down(rt, id, at, time),
            (TouchPhase::Move, Some(i)) if finite => self.moved(rt, i, at, time),
            (TouchPhase::Up | TouchPhase::Cancel, Some(i)) => {
                if finite {
                    self.moved(rt, i, at, time);
                }
                self.lift(rt, i, phase == TouchPhase::Cancel, time);
            }
            _ => {}
        }
    }

    /// Lifts every finger without `Triggered` (focus loss).
    pub fn release_all(&mut self, rt: &mut InputRuntime, time: f64) {
        for i in 0..MAX_FINGERS {
            if self.fingers[i].is_some() {
                self.lift(rt, i, true, time);
            }
        }
    }

    fn down(&mut self, rt: &mut InputRuntime, id: u64, at: [f32; 2], time: f64) {
        let Some(free) = self.fingers.iter().position(Option::is_none) else {
            return;
        };
        self.touched = true;
        let l = &self.layout;
        let anchor = l.button_at(at).map(|b| (b.center, b.radius));
        let capture = if let Some(b) = l.button_at(at) {
            if b.slot == MENU_SLOT {
                Capture::Menu { armed: true }
            } else {
                Capture::Button {
                    slot: b.slot,
                    live: true,
                }
            }
        } else if l.near_button(at) {
            Capture::Ignored
        } else if self.set.stick.is_some() && l.stick_zone.contains(at) {
            if self.stick.is_some() {
                Capture::Ignored
            } else {
                Capture::Stick
            }
        } else if self.set.look.is_some() && l.look_zone.contains(at) {
            Capture::Look
        } else {
            Capture::Pointer
        };
        match capture {
            Capture::Button { slot, .. } => {
                self.held[slot] = self.held[slot].saturating_add(1);
                if self.held[slot] == 1 {
                    push_button(rt, slot, 1.0, time);
                }
            }
            Capture::Stick => {
                if let Some(rest) = l.stick {
                    let stick = Stick::begin(self.profile.stick_mode, rest.center, rest.radius, at);
                    rt.push(RawEvent::vector(
                        Device::Touch,
                        Control::TouchStick,
                        stick.value(),
                        time,
                    ));
                    self.stick = Some(stick);
                }
            }
            Capture::Pointer => {
                self.pointer_at(rt, at, time);
                self.pointers = self.pointers.saturating_add(1);
                if self.pointers == 1 {
                    rt.push(RawEvent::button(
                        Device::Touch,
                        Control::MouseButton1,
                        1.0,
                        time,
                    ));
                }
            }
            _ => {}
        }
        self.fingers[free] = Some(Finger {
            id,
            capture,
            last: at,
            anchor,
        });
    }

    fn moved(&mut self, rt: &mut InputRuntime, i: usize, at: [f32; 2], time: f64) {
        let Some(mut f) = self.fingers[i] else {
            return;
        };
        match f.capture {
            Capture::Stick => {
                if let Some(stick) = self.stick.as_mut()
                    && at != f.last
                {
                    stick.drag(at);
                    rt.push(RawEvent::vector(
                        Device::Touch,
                        Control::TouchStick,
                        stick.value(),
                        time,
                    ));
                }
            }
            Capture::Look => {
                let d = [at[0] - f.last[0], at[1] - f.last[1]];
                if d != [0.0; 2] {
                    rt.push(RawEvent::vector(Device::Touch, Control::TouchLook, d, time));
                }
            }
            Capture::Button { slot, live: true } => {
                let off = f.anchor.is_none_or(|(c, r)| dist(c, at) > CANCEL_RADII * r);
                if off {
                    f.capture = Capture::Button { slot, live: false };
                    self.unhold(rt, slot, true, time);
                }
            }
            Capture::Menu { .. } => {
                let armed = f
                    .anchor
                    .is_some_and(|(c, r)| dist(c, at) <= CANCEL_RADII * r);
                f.capture = Capture::Menu { armed };
            }
            Capture::Pointer if at != f.last => self.pointer_at(rt, at, time),
            _ => {}
        }
        f.last = at;
        self.fingers[i] = Some(f);
    }

    fn lift(&mut self, rt: &mut InputRuntime, i: usize, cancel: bool, time: f64) {
        let Some(f) = self.fingers[i].take() else {
            return;
        };
        match f.capture {
            Capture::Stick => {
                if self.stick.take().is_some() {
                    rt.push(RawEvent::vector(
                        Device::Touch,
                        Control::TouchStick,
                        [0.0; 2],
                        time,
                    ));
                }
            }
            Capture::Button { slot, live: true } => self.unhold(rt, slot, cancel, time),
            Capture::Menu { armed: true } if !cancel => {
                // Menu buttons act on release (spec 7.5).
                push_button(rt, MENU_SLOT, 1.0, time);
                push_button(rt, MENU_SLOT, 0.0, time);
            }
            Capture::Pointer => {
                // Every pointer finger holds MouseButton1; it goes up with the last one.
                self.pointers = self.pointers.saturating_sub(1);
                if self.pointers == 0 {
                    rt.push(if cancel {
                        RawEvent::cancel(Device::Touch, Control::MouseButton1, time)
                    } else {
                        RawEvent::button(Device::Touch, Control::MouseButton1, 0.0, time)
                    });
                }
            }
            _ => {}
        }
    }

    fn unhold(&mut self, rt: &mut InputRuntime, slot: usize, cancel: bool, time: f64) {
        self.held[slot] = self.held[slot].saturating_sub(1);
        if self.held[slot] == 0
            && let Some(control) = Control::touch_button(slot)
        {
            rt.push(if cancel {
                RawEvent::cancel(Device::Touch, control, time)
            } else {
                RawEvent::button(Device::Touch, control, 0.0, time)
            });
        }
    }

    fn pointer_at(&self, rt: &mut InputRuntime, at: [f32; 2], time: f64) {
        let px = [at[0] * self.px_per_dp, at[1] * self.px_per_dp];
        rt.push(RawEvent::vector(
            Device::Touch,
            Control::MousePosition,
            px,
            time,
        ));
    }
}

fn push_button(rt: &mut InputRuntime, slot: usize, value: f32, time: f64) {
    if let Some(control) = Control::touch_button(slot) {
        rt.push(RawEvent::button(Device::Touch, control, value, time));
    }
}
