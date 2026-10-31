//! Pointer input for the in-game GUI (#79): hit-tests the mouse and fingers against the laid-out
//! GUI ([`crate::gui::layout_into`]) and turns them into the GUI events scripts connect to.
//!
//! - A press (left mouse button or finger) fires `InputBegan` on every element under it, front to
//!   back, down to the first that takes clicks ([`crate::gui::pick`]: a GuiButton or an `Active`
//!   element). When there is one, the GUI keeps the press: the caller must not pass it, its moves
//!   or its release to the game, so a click on a button never also turns the camera or jumps.
//! - Releasing a button press over the same button fires `MouseButton1Up`, `MouseButton1Click`
//!   and, when the button is `Active` (a button's default), `Activated`, in that order; a release
//!   elsewhere only ends the press. `InputEnded` goes to the elements that saw `InputBegan`.
//!   Roblox's docs give no order for these; this one follows their descriptions (the button is
//!   released, which completes a click, which activates it).
//! - At most [`MAX_PRESSES`] presses are held: a new one past that ends the oldest (no click),
//!   so a lost release can never make later presses go untracked and leak to the game.
//! - The mouse (not a finger) hovers: `MouseEnter` and `MouseLeave` as the set of elements under
//!   it changes, `InputChanged` (at most one per element per frame) while it moves over them.
//!
//! - Every press and release also goes to `UserInputService.InputBegan`/`InputEnded`
//!   ([`GuiInput::drain_pointer_inputs`]) with `gameProcessed` true when the GUI took the press,
//!   as in Roblox; a release only when its press was seen.
//!
//! Hit-testing respects `Visible`, `Enabled`, `ZIndex`, `DisplayOrder` and `ClipsDescendants`
//! through the layout. A step allocates nothing once its buffers have grown (a press copies the
//! short list of elements it began on).
use metallic_core::{InstanceId, Value, World};
use metallic_script::{GuiEvent, GuiEventKind, GuiPointer, PointerInput};

use crate::gui::{self, GuiItem, LayoutScratch, Pointer, Shade};

/// Which pointer an event is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerId {
    /// The mouse's left button and position.
    Mouse,
    /// A finger, by the platform's touch id.
    Touch(u64),
}

/// What a pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    /// Pressed (button down, finger down).
    Down,
    /// Moved.
    Move,
    /// Released.
    Up,
    /// Lost without a release (no click).
    Cancel,
}

/// Most presses held at once (a mouse and fifteen fingers). A new press past that ends the
/// oldest, without a click.
pub const MAX_PRESSES: usize = 16;

/// One held press that began on the GUI.
#[derive(Debug, Clone)]
struct Press {
    pointer: PointerId,
    /// The element that took it (a button or an `Active` element), if any.
    target: Option<InstanceId>,
    /// The target is a GuiButton.
    button: bool,
    /// The button is `Active`, so a click fires `Activated`.
    activates: bool,
    /// The elements that saw `InputBegan`, front to back.
    began: Vec<InstanceId>,
}

/// The GUI's pointer state: hover, held presses and the events not yet sent.
#[derive(Debug, Default)]
pub struct GuiInput {
    items: Vec<GuiItem>,
    scratch: LayoutScratch,
    /// Elements under the mouse, front to back.
    hovered: Vec<InstanceId>,
    under: Vec<InstanceId>,
    presses: Vec<Press>,
    events: Vec<GuiEvent>,
    /// Pointers pressed and not yet released, oldest first, for `UserInputService`.
    down: Vec<PointerId>,
    pointer_inputs: Vec<PointerInput>,
}

impl GuiInput {
    /// No hover, no presses.
    pub fn new() -> GuiInput {
        GuiInput::default()
    }

    /// Forgets every hover and press without events (the game stopped or restarted).
    pub fn reset(&mut self) {
        self.hovered.clear();
        self.presses.clear();
        self.events.clear();
        self.down.clear();
        self.pointer_inputs.clear();
    }

    /// Handles one event of `pointer` at `at` (GUI points from the top-left of a `screen` of
    /// width x height points, with the top `inset` points left free, as in
    /// [`gui::layout`]). Returns true when the GUI took it: the caller must not give it to the
    /// game's input. A move or release returns what the press's `Down` returned.
    pub fn pointer(
        &mut self,
        world: &World,
        screen: [f32; 2],
        inset: f32,
        pointer: PointerId,
        phase: PointerPhase,
        at: [f32; 2],
    ) -> bool {
        let taken = self.pointer_gui(world, screen, inset, pointer, phase, at);
        let kind = match pointer {
            PointerId::Mouse => GuiPointer::Mouse,
            PointerId::Touch(_) => GuiPointer::Touch,
        };
        let began = match phase {
            PointerPhase::Move => return taken,
            PointerPhase::Down => {
                if !self.down.contains(&pointer) {
                    if self.down.len() >= MAX_PRESSES {
                        self.down.remove(0);
                    }
                    self.down.push(pointer);
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let Some(i) = self.down.iter().position(|&p| p == pointer) else {
                    return taken;
                };
                self.down.remove(i);
                false
            }
        };
        self.pointer_inputs.push(PointerInput {
            pointer: kind,
            began,
            position: at,
            game_processed: taken,
        });
        taken
    }

    fn pointer_gui(
        &mut self,
        world: &World,
        screen: [f32; 2],
        inset: f32,
        pointer: PointerId,
        phase: PointerPhase,
        at: [f32; 2],
    ) -> bool {
        let held = self.presses.iter().position(|p| p.pointer == pointer);
        if !(at[0].is_finite() && at[1].is_finite()) {
            // A lost position: end a held press without a click.
            return match held {
                Some(i) if phase != PointerPhase::Move => self.end(i, None, at),
                Some(i) => self.presses[i].target.is_some(),
                None => false,
            };
        }
        gui::layout_into(
            world,
            screen,
            inset,
            false,
            &mut self.scratch,
            &mut self.items,
        );
        let kind = match pointer {
            PointerId::Mouse => GuiPointer::Mouse,
            PointerId::Touch(_) => GuiPointer::Touch,
        };
        if pointer == PointerId::Mouse {
            self.hover(at, phase == PointerPhase::Move);
        }
        match phase {
            PointerPhase::Down => {
                if let Some(i) = held {
                    // Its release was lost: end it first.
                    self.end(i, None, at);
                }
                gui::under(&self.items, at, &mut self.under);
                for &id in &self.under {
                    self.events
                        .push(event(id, GuiEventKind::InputBegan, kind, at));
                }
                let target = gui::pick(&self.items, at).map(|i| {
                    let item = &self.items[i];
                    (item.id, matches!(item.pointer, Pointer::Button { .. }))
                });
                let (target, button) = match target {
                    Some((id, button)) => (Some(id), button),
                    None => (None, false),
                };
                if let (Some(id), true) = (target, button) {
                    self.events
                        .push(event(id, GuiEventKind::MouseButton1Down, kind, at));
                }
                if target.is_some() || !self.under.is_empty() {
                    if self.presses.len() >= MAX_PRESSES {
                        // The oldest press is the likeliest to have lost its release.
                        let oldest = self.presses.remove(0);
                        self.end_press(oldest, None, at);
                    }
                    let activates = target
                        .is_some_and(|id| matches!(world.get(id, "Active"), Ok(Value::Bool(true))));
                    self.presses.push(Press {
                        pointer,
                        target,
                        button,
                        activates,
                        began: self.under.clone(),
                    });
                }
                target.is_some()
            }
            PointerPhase::Move => match held {
                Some(i) => {
                    let press = &self.presses[i];
                    if pointer != PointerId::Mouse
                        && let Some(id) = press.target
                    {
                        changed(&mut self.events, id, kind, at);
                    }
                    press.target.is_some()
                }
                None => false,
            },
            PointerPhase::Up => match held {
                Some(i) => {
                    let over = gui::pick(&self.items, at).map(|j| self.items[j].id);
                    self.end(i, Some(over), at)
                }
                None => false,
            },
            PointerPhase::Cancel => match held {
                Some(i) => self.end(i, None, at),
                None => false,
            },
        }
    }

    /// The mouse left the window: `MouseLeave` for everything it was over.
    pub fn mouse_left(&mut self, at: [f32; 2]) {
        for &id in &self.hovered {
            self.events
                .push(event(id, GuiEventKind::MouseLeave, GuiPointer::Mouse, at));
        }
        self.hovered.clear();
    }

    /// How button `id` looks now: pressed while a press on it is held, hovered while the mouse
    /// is over it.
    pub fn shade(&self, id: InstanceId) -> Shade {
        if self
            .presses
            .iter()
            .any(|p| p.button && p.target == Some(id))
        {
            Shade::Pressed
        } else if self.hovered.contains(&id) {
            Shade::Hovered
        } else {
            Shade::Normal
        }
    }

    /// True while `pointer` holds a press the GUI took.
    pub fn holds(&self, pointer: PointerId) -> bool {
        self.presses
            .iter()
            .any(|p| p.pointer == pointer && p.target.is_some())
    }

    /// Takes the events produced since the last call, oldest first.
    pub fn drain_events(&mut self) -> std::vec::Drain<'_, GuiEvent> {
        self.events.drain(..)
    }

    /// Takes the `UserInputService` presses and releases since the last call, oldest first.
    pub fn drain_pointer_inputs(&mut self) -> std::vec::Drain<'_, PointerInput> {
        self.pointer_inputs.drain(..)
    }

    /// Updates the hover set for the mouse at `at`.
    fn hover(&mut self, at: [f32; 2], moved: bool) {
        gui::under(&self.items, at, &mut self.under);
        for &id in &self.hovered {
            if !self.under.contains(&id) {
                self.events
                    .push(event(id, GuiEventKind::MouseLeave, GuiPointer::Mouse, at));
            }
        }
        // Back to front, so the front element's MouseEnter comes last, as the topmost.
        for &id in self.under.iter().rev() {
            if !self.hovered.contains(&id) {
                self.events
                    .push(event(id, GuiEventKind::MouseEnter, GuiPointer::Mouse, at));
            }
        }
        if moved {
            for &id in &self.under {
                changed(&mut self.events, id, GuiPointer::Mouse, at);
            }
        }
        std::mem::swap(&mut self.hovered, &mut self.under);
    }

    /// Ends press `i`. `released_over` is `Some(the element that takes clicks at the release
    /// point)` for a release, `None` for a cancel. Returns whether the GUI had taken the press.
    fn end(&mut self, i: usize, released_over: Option<Option<InstanceId>>, at: [f32; 2]) -> bool {
        // Kept in press order, so the oldest is first (see MAX_PRESSES).
        let press = self.presses.remove(i);
        self.end_press(press, released_over, at)
    }

    fn end_press(
        &mut self,
        press: Press,
        released_over: Option<Option<InstanceId>>,
        at: [f32; 2],
    ) -> bool {
        let kind = match press.pointer {
            PointerId::Mouse => GuiPointer::Mouse,
            PointerId::Touch(_) => GuiPointer::Touch,
        };
        for &id in &press.began {
            self.events
                .push(event(id, GuiEventKind::InputEnded, kind, at));
        }
        if let (Some(target), true, Some(over)) = (press.target, press.button, released_over)
            && over == Some(target)
        {
            let clicks = [
                GuiEventKind::MouseButton1Up,
                GuiEventKind::MouseButton1Click,
                GuiEventKind::Activated,
            ];
            let n = if press.activates { 3 } else { 2 };
            for k in &clicks[..n] {
                self.events.push(event(target, *k, kind, at));
            }
        }
        press.target.is_some()
    }
}

fn event(target: InstanceId, kind: GuiEventKind, pointer: GuiPointer, at: [f32; 2]) -> GuiEvent {
    GuiEvent {
        target,
        kind,
        pointer,
        position: at,
    }
}

/// Queues `InputChanged` for `id`, or moves the one already queued (one per element per frame).
fn changed(events: &mut Vec<GuiEvent>, id: InstanceId, pointer: GuiPointer, at: [f32; 2]) {
    let queued = events
        .iter_mut()
        .rev()
        .find(|e| e.target == id && e.kind == GuiEventKind::InputChanged && e.pointer == pointer);
    match queued {
        Some(e) => e.position = at,
        None => events.push(event(id, GuiEventKind::InputChanged, pointer, at)),
    }
}
