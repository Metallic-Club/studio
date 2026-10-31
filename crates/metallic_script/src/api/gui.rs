//! GUI element events (#79): `GuiButton.Activated`, `MouseButton1Click`, `MouseButton1Down/Up`
//! and `GuiObject.MouseEnter/Leave`, `InputBegan/Changed/Ended`.
//!
//! The runtime hit-tests the pointer against the laid-out GUI and reports what happened with
//! [`ScriptHost::gui_event`](crate::ScriptHost::gui_event); the next step fires the events in the
//! client VM, in the order they were reported. Pointer presses and releases also reach
//! `UserInputService.InputBegan`/`InputEnded` ([`ScriptHost::pointer_input`](crate::ScriptHost::pointer_input)),
//! with `gameProcessed` true when the GUI took them, as in Roblox.

use std::rc::Rc;

use metallic_core::{InstanceId, Vec3};
use mlua::{Lua, Table, Value};

use super::ApiState;
use super::datatypes::LuaVector3;
use super::signals::EventKind;

/// Most GUI events kept for one step; later ones are dropped until the next step. A click's
/// `MouseButton1Up`, `MouseButton1Click` and `Activated` are kept or dropped together, and the
/// last [`GUI_CLICK_RESERVE`] places are kept for presses and clicks (not hover or moves).
pub const MAX_GUI_EVENTS_PER_STEP: usize = 256;

/// Places of [`MAX_GUI_EVENTS_PER_STEP`] that `MouseEnter`, `MouseLeave` and `InputChanged`
/// may not take, so a flood of them never drops a click.
pub const GUI_CLICK_RESERVE: usize = 32;

/// Most `UserInputService.InputBegan`/`InputEnded` pointer events kept for one step.
pub const MAX_POINTER_INPUTS_PER_STEP: usize = 64;

/// A press or release of the mouse's left button or a finger, for
/// `UserInputService.InputBegan` and `InputEnded`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerInput {
    /// Mouse or finger.
    pub pointer: GuiPointer,
    /// Pressed (`InputBegan`) or released (`InputEnded`).
    pub began: bool,
    /// The pointer in GUI points from the top-left of the screen.
    pub position: [f32; 2],
    /// The game's GUI took it (`gameProcessed`).
    pub game_processed: bool,
}

/// What happened to a GUI element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuiEventKind {
    /// The pointer moved onto the element (`MouseEnter(x, y)`).
    MouseEnter,
    /// The pointer left it (`MouseLeave(x, y)`).
    MouseLeave,
    /// Pressed on a button (`MouseButton1Down(x, y)`); also a finger.
    MouseButton1Down,
    /// Released on a button (`MouseButton1Up(x, y)`).
    MouseButton1Up,
    /// Pressed and released on the same button (`MouseButton1Click()`).
    MouseButton1Click,
    /// The button was used (`Activated(inputObject, clickCount)`), right after the click.
    Activated,
    /// A press began over the element (`InputBegan(inputObject)`).
    InputBegan,
    /// The pointer moved over it (`InputChanged(inputObject)`).
    InputChanged,
    /// A press that began over the element ended (`InputEnded(inputObject)`).
    InputEnded,
}

impl GuiEventKind {
    /// The event named `name` of an instance of `class`, if it has it.
    pub(crate) fn of(class: &str, name: &str) -> Option<GuiEventKind> {
        use metallic_core::classes::is_subclass;
        let kind = match name {
            "MouseEnter" => GuiEventKind::MouseEnter,
            "MouseLeave" => GuiEventKind::MouseLeave,
            "InputBegan" => GuiEventKind::InputBegan,
            "InputChanged" => GuiEventKind::InputChanged,
            "InputEnded" => GuiEventKind::InputEnded,
            "MouseButton1Down" => GuiEventKind::MouseButton1Down,
            "MouseButton1Up" => GuiEventKind::MouseButton1Up,
            "MouseButton1Click" => GuiEventKind::MouseButton1Click,
            "Activated" => GuiEventKind::Activated,
            _ => return None,
        };
        let button_only = matches!(
            kind,
            GuiEventKind::MouseButton1Down
                | GuiEventKind::MouseButton1Up
                | GuiEventKind::MouseButton1Click
                | GuiEventKind::Activated
        );
        let base = if button_only { "GuiButton" } else { "Frame" };
        is_subclass(class, base).then_some(kind)
    }
}

/// The pointer behind a GUI event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiPointer {
    /// The mouse (left button).
    Mouse,
    /// A finger.
    Touch,
}

/// One GUI event, reported by the runtime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuiEvent {
    /// The element (a Frame, TextLabel or button).
    pub target: InstanceId,
    /// What happened.
    pub kind: GuiEventKind,
    /// Mouse or finger.
    pub pointer: GuiPointer,
    /// The pointer in GUI points from the top-left of the screen.
    pub position: [f32; 2],
}

impl ApiState {
    /// Fires `event` on its target, if the target still exists.
    pub(crate) fn deliver_gui_event(self: &Rc<Self>, event: &GuiEvent) {
        let alive = self.world().is_ok_and(|w| w.is_alive(event.target));
        if !alive {
            return;
        }
        let key = (event.target, EventKind::Gui(event.kind));
        let [x, y] = event
            .position
            .map(|v| if v.is_finite() { v.round() } else { 0.0 });
        let (x, y) = (f64::from(x), f64::from(y));
        self.fire(key, &|lua| {
            Ok(match event.kind {
                GuiEventKind::MouseEnter
                | GuiEventKind::MouseLeave
                | GuiEventKind::MouseButton1Down
                | GuiEventKind::MouseButton1Up => vec![Value::Number(x), Value::Number(y)],
                GuiEventKind::MouseButton1Click => Vec::new(),
                GuiEventKind::Activated => vec![
                    Value::Table(self.gui_input_object(lua, event, x, y)?),
                    Value::Integer(1),
                ],
                GuiEventKind::InputBegan
                | GuiEventKind::InputChanged
                | GuiEventKind::InputEnded => {
                    vec![Value::Table(self.gui_input_object(lua, event, x, y)?)]
                }
            })
        });
    }

    /// Fires `UserInputService.InputBegan` or `InputEnded(inputObject, gameProcessed)`.
    pub(crate) fn deliver_pointer_input(self: &Rc<Self>, input: &PointerInput) {
        let Some(carrier) = self.world().ok().and_then(|w| w.root("RunService")) else {
            return;
        };
        let (kind, signal) = if input.began {
            (GuiEventKind::InputBegan, EventKind::UisInputBegan)
        } else {
            (GuiEventKind::InputEnded, EventKind::UisInputEnded)
        };
        let event = GuiEvent {
            target: carrier,
            kind,
            pointer: input.pointer,
            position: input.position,
        };
        let [x, y] = input
            .position
            .map(|v| if v.is_finite() { v.round() } else { 0.0 });
        let (x, y) = (f64::from(x), f64::from(y));
        self.fire((carrier, signal), &|lua| {
            Ok(vec![
                Value::Table(self.gui_input_object(lua, &event, x, y)?),
                Value::Boolean(input.game_processed),
            ])
        });
    }

    /// The `InputObject` of a GUI event (a read-only table, like the other input objects).
    fn gui_input_object(&self, lua: &Lua, e: &GuiEvent, x: f64, y: f64) -> mlua::Result<Table> {
        let input_type = match (e.pointer, e.kind) {
            (GuiPointer::Touch, _) => "Touch",
            (GuiPointer::Mouse, GuiEventKind::InputChanged) => "MouseMovement",
            (GuiPointer::Mouse, _) => "MouseButton1",
        };
        let state = match e.kind {
            GuiEventKind::InputBegan | GuiEventKind::MouseButton1Down => "Begin",
            GuiEventKind::InputChanged | GuiEventKind::MouseEnter => "Change",
            _ => "End",
        };
        let t = lua.create_table()?;
        t.set("KeyCode", self.enum_value("KeyCode", "Unknown"))?;
        t.set(
            "UserInputType",
            self.enum_value("UserInputType", input_type),
        )?;
        t.set("UserInputState", self.enum_value("UserInputState", state))?;
        t.set("Position", LuaVector3(Vec3::new(x, y, 0.0)))?;
        t.set("Delta", LuaVector3(Vec3::new(0.0, 0.0, 0.0)))?;
        t.set_readonly(true);
        Ok(t)
    }
}
