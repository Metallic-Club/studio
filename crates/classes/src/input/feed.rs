//! The engine's side of `UserInputService`: [`InputFeed`] turns input events into the service's state and events.
//!
//! The engine owns one feed per game and calls it in the first step of every frame: [`InputFeed::begin_frame`],
//! then one call per event the frame took ([`InputFeed::key`], [`InputFeed::mouse_motion`], ...), then
//! [`InputFeed::end_frame`]. Then it reads [`InputFeed::cursor`] and applies it to the window. The types here are
//! plain values, so this crate does not depend on the platform crate; the engine translates the platform's events.
//!
//! # What an event costs
//!
//! - **Mouse movement** only adds to the frame's totals. At the end of the frame the hidden `MouseLocation` and
//!   `MouseDelta` are written (a write of an unchanged value does nothing), and `InputChanged` fires once with the
//!   frame's movement, only while something is connected to it (or to the movement object itself). With nothing
//!   connected, mouse movement creates no instance and allocates nothing.
//! - **Keys and mouse buttons** always update their input object, which is what `IsKeyDown` reads. Each key's
//!   object is created the first time that key is pressed, then reused.
//! - **Signals** fire only while the game runs ([`InputFeed::begin_frame`]'s `firing`) and only when something
//!   listens to that signal ([`DataModel::is_signal_watched`]); building their arguments is the one allocation.
//!
//! # One `InputChanged` per frame
//!
//! Signals are delivered after the input step, when listeners read the input objects as they are at that point.
//! Mouse movement, the wheel and finger movement are therefore summed over the frame and fire `InputChanged` once,
//! at its end, so each listener sees the movement it is told about. A key pressed and released within one frame
//! fires both `InputBegan` and `InputEnded`, and both listeners see the object in its `End` state.

use metallic_core::{ClassId, DataModel, DmError, EnumItem, Id, PropId, ScriptValue, SignalId, Value, Vector2, Vector3};

use crate::GameError;
use crate::input::input_object::{DELTA, KEY_CODE, NEXT_INPUT, POSITION, USER_INPUT_STATE, USER_INPUT_TYPE};
use crate::input::key_code::UNKNOWN;
use crate::services::user_input_service::{
    FIRST_INPUT, GAMEPAD_ENABLED, KEYBOARD_ENABLED, MOUSE_BEHAVIOR, MOUSE_DELTA, MOUSE_ENABLED, MOUSE_ICON_ENABLED,
    MOUSE_LOCATION, TOUCH_ENABLED,
};

/// `UserInputService.MouseBehavior`, as the engine applies it to the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseBehavior {
    /// The cursor moves freely.
    #[default]
    Default,
    /// The cursor is held at the middle of the window.
    LockCenter,
    /// The cursor is held where it is.
    LockCurrentPosition,
}

/// A mouse button scripts see: `UserInputType.MouseButton1` to `MouseButton3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The primary (usually left) button: `MouseButton1`.
    Left,
    /// The secondary (usually right) button: `MouseButton2`.
    Right,
    /// The middle button or wheel click: `MouseButton3`.
    Middle,
}

impl MouseButton {
    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
        }
    }
}

/// The stage of one touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TouchPhase {
    /// A finger touched the screen: `InputBegan`.
    Began,
    /// The finger moved: one `InputChanged` per frame.
    Moved,
    /// The finger left the screen: `InputEnded`, in the `End` state.
    Ended,
    /// The system took the touch over: `InputEnded`, in the `Cancel` state.
    Cancelled,
}

/// What the scripts asked of the cursor: `MouseBehavior` and `MouseIconEnabled`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorState {
    /// How the cursor is held.
    pub behavior: MouseBehavior,
    /// Whether the cursor is shown.
    pub icon_visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            behavior: MouseBehavior::Default,
            icon_visible: true,
        }
    }
}

/// The service and the ids of its members, looked up once.
#[derive(Debug)]
struct Service {
    id: Id,
    mouse_location: PropId,
    mouse_delta: PropId,
    first_input: PropId,
    mouse_behavior: PropId,
    mouse_icon_enabled: PropId,
    devices: [PropId; 4],
    began: SignalId,
    changed: SignalId,
    ended: SignalId,
    focused: SignalId,
    focus_released: SignalId,
}

/// `InputObject` and the ids of its fields.
#[derive(Debug)]
struct Object {
    class: ClassId,
    key_code: PropId,
    input_type: PropId,
    state: PropId,
    position: PropId,
    delta: PropId,
    next: PropId,
}

/// The enum items the feed writes, shared with the registry so writing them allocates nothing.
#[derive(Debug)]
struct Items {
    begin: EnumItem,
    change: EnumItem,
    end: EnumItem,
    cancel: EnumItem,
    keyboard: EnumItem,
    buttons: [EnumItem; 3],
    wheel: EnumItem,
    movement: EnumItem,
    touch: EnumItem,
}

/// The devices whose `...Enabled` flag the feed sets, in [`Service::devices`] order.
#[derive(Clone, Copy, Debug)]
enum Device {
    Mouse = 0,
    Keyboard = 1,
    Touch = 2,
    Gamepad = 3,
}

/// A key's or a button's input object, and whether it is held.
#[derive(Clone, Copy, Debug)]
struct Held {
    id: Id,
    down: bool,
}

/// A touch's input object, free when `touch` is `None`.
#[derive(Clone, Copy, Debug)]
struct TouchSlot {
    id: Id,
    touch: Option<u64>,
    position: (f64, f64),
    /// Movement since the frame began.
    delta: (f64, f64),
    moved: bool,
}

/// Turns input into `UserInputService`'s state and events. See the module docs.
#[derive(Debug)]
pub struct InputFeed {
    service: Service,
    object: Object,
    items: Items,
    /// One object per key pressed so far, by `Enum.KeyCode` item name.
    keys: Vec<(&'static str, Held)>,
    buttons: [Option<Held>; 3],
    movement: Option<Id>,
    wheel_object: Option<Id>,
    touches: Vec<TouchSlot>,
    /// The `...Enabled` flags written so far.
    devices: [bool; 4],
    firing: bool,
    /// The cursor, in physical pixels from the window's top-left corner.
    location: Option<(f64, f64)>,
    /// Raw movement this frame.
    motion: (f64, f64),
    raw_motion: bool,
    /// How far the cursor moved this frame.
    cursor_shift: (f64, f64),
    cursor_moved: bool,
    /// Wheel notches this frame.
    wheel: f64,
    /// The data model's structure revision at the last check for destroyed input objects.
    revision: u64,
}

/// Looks up a member, or names what is missing.
fn member<T>(found: Option<T>, owner: &'static str, member: &'static str) -> Result<T, GameError> {
    found.ok_or(GameError::MissingMember { owner, member })
}

fn vector3(x: f64, y: f64, z: f64) -> Value {
    // Pixels and notches fit an f32 with room to spare; Vector3 is f32 like every vector in the data model.
    Value::Vector3(Vector3::new(x as f32, y as f32, z as f32))
}

impl InputFeed {
    /// A feed for the game's `UserInputService`, which is created if the game lacks it.
    pub fn new(dm: &mut DataModel) -> Result<Self, GameError> {
        const SERVICE: &str = "UserInputService";
        const OBJECT: &str = "InputObject";
        let registry = std::sync::Arc::clone(dm.registry());
        let class = registry.class_id(SERVICE).ok_or(GameError::MissingClass(SERVICE))?;
        let id = match dm.service(class) {
            Some(id) => id,
            None => dm.create_service(class)?,
        };
        let prop = |name: &'static str| member(registry.prop_id(class, name), SERVICE, name);
        let signal = |name: &'static str| member(registry.signal_id(class, name), SERVICE, name);
        let service = Service {
            id,
            mouse_location: prop(MOUSE_LOCATION)?,
            mouse_delta: prop(MOUSE_DELTA)?,
            first_input: prop(FIRST_INPUT)?,
            mouse_behavior: prop(MOUSE_BEHAVIOR)?,
            mouse_icon_enabled: prop(MOUSE_ICON_ENABLED)?,
            devices: [
                prop(MOUSE_ENABLED)?,
                prop(KEYBOARD_ENABLED)?,
                prop(TOUCH_ENABLED)?,
                prop(GAMEPAD_ENABLED)?,
            ],
            began: signal("InputBegan")?,
            changed: signal("InputChanged")?,
            ended: signal("InputEnded")?,
            focused: signal("WindowFocused")?,
            focus_released: signal("WindowFocusReleased")?,
        };
        let object_class = registry.class_id(OBJECT).ok_or(GameError::MissingClass(OBJECT))?;
        let field = |name: &'static str| member(registry.prop_id(object_class, name), OBJECT, name);
        let object = Object {
            class: object_class,
            key_code: field(KEY_CODE)?,
            input_type: field(USER_INPUT_TYPE)?,
            state: field(USER_INPUT_STATE)?,
            position: field(POSITION)?,
            delta: field(DELTA)?,
            next: field(NEXT_INPUT)?,
        };
        let state = |name: &'static str| member(registry.enum_item("UserInputState", name), "UserInputState", name);
        let kind = |name: &'static str| member(registry.enum_item("UserInputType", name), "UserInputType", name);
        let items = Items {
            begin: state("Begin")?,
            change: state("Change")?,
            end: state("End")?,
            cancel: state("Cancel")?,
            keyboard: kind("Keyboard")?,
            buttons: [kind("MouseButton1")?, kind("MouseButton2")?, kind("MouseButton3")?],
            wheel: kind("MouseWheel")?,
            movement: kind("MouseMovement")?,
            touch: kind("Touch")?,
        };
        let mut devices = [false; 4];
        for (flag, &prop) in devices.iter_mut().zip(&service.devices) {
            *flag = dm.bool(id, prop)?;
        }
        Ok(Self {
            service,
            object,
            items,
            keys: Vec::new(),
            buttons: [None; 3],
            movement: None,
            wheel_object: None,
            touches: Vec::new(),
            devices,
            firing: false,
            location: None,
            motion: (0.0, 0.0),
            raw_motion: false,
            cursor_shift: (0.0, 0.0),
            cursor_moved: false,
            wheel: 0.0,
            revision: dm.structure_revision(),
        })
    }

    /// The game's `UserInputService`.
    pub fn service(&self) -> Id {
        self.service.id
    }

    /// Starts the frame's input step. `firing` says whether the game runs: a stopped or paused game keeps its
    /// state current (held keys, the cursor) but fires nothing, so no stale input reaches scripts on resume.
    ///
    /// When anything was destroyed since the last frame, the input objects scripts destroyed are dropped and the
    /// chain the methods walk is repaired; the next input of that kind gets a new object.
    pub fn begin_frame(&mut self, dm: &mut DataModel, firing: bool) -> Result<(), DmError> {
        let revision = dm.structure_revision();
        if revision != self.revision {
            self.revision = revision;
            self.drop_destroyed(dm)?;
        }
        self.firing = firing;
        self.motion = (0.0, 0.0);
        self.raw_motion = false;
        self.cursor_shift = (0.0, 0.0);
        self.cursor_moved = false;
        self.wheel = 0.0;
        for slot in &mut self.touches {
            slot.delta = (0.0, 0.0);
            slot.moved = false;
        }
        Ok(())
    }

    /// Forgets the input objects that no longer exist and relinks the chain if there were any.
    fn drop_destroyed(&mut self, dm: &mut DataModel) -> Result<(), DmError> {
        let before = self.object_count();
        self.keys.retain(|(_, held)| dm.contains(held.id));
        for slot in &mut self.buttons {
            if slot.is_some_and(|held| !dm.contains(held.id)) {
                *slot = None;
            }
        }
        self.movement = self.movement.filter(|&id| dm.contains(id));
        self.wheel_object = self.wheel_object.filter(|&id| dm.contains(id));
        self.touches.retain(|slot| dm.contains(slot.id));
        if self.object_count() == before {
            return Ok(());
        }
        self.relink(dm)
    }

    /// A key went down (`pressed`) or up. `key` is its `Enum.KeyCode` item name; [`UNKNOWN`] keys are not tracked.
    /// A press of a key already down (an OS key repeat) and a release of a key not down do nothing.
    pub fn key(&mut self, dm: &mut DataModel, key: &'static str, pressed: bool) -> Result<(), DmError> {
        self.enable(dm, Device::Keyboard)?;
        if key == UNKNOWN {
            return Ok(());
        }
        let index = match self.keys.iter().position(|(name, _)| *name == key) {
            Some(index) => index,
            None if pressed => {
                let Some(item) = dm.registry().enum_item("KeyCode", key) else {
                    return Ok(());
                };
                let id = self.make(dm, &self.items.keyboard, Some(&item))?;
                self.keys.push((key, Held { id, down: false }));
                self.relink(dm)?;
                self.keys.len() - 1
            }
            None => return Ok(()),
        };
        let Some(&(_, held)) = self.keys.get(index) else {
            return Ok(());
        };
        if held.down == pressed {
            return Ok(());
        }
        let id = if dm.contains(held.id) {
            held.id
        } else {
            let item = dm.registry().enum_item("KeyCode", key);
            let id = self.make(dm, &self.items.keyboard, item.as_ref())?;
            self.relink_after(dm, |feed| {
                if let Some(entry) = feed.keys.get_mut(index) {
                    entry.1.id = id;
                }
            })?;
            id
        };
        if let Some(entry) = self.keys.get_mut(index) {
            entry.1 = Held { id, down: pressed };
        }
        self.press(dm, id, pressed)
    }

    /// A mouse button went down (`pressed`) or up.
    pub fn mouse_button(&mut self, dm: &mut DataModel, button: MouseButton, pressed: bool) -> Result<(), DmError> {
        self.enable(dm, Device::Mouse)?;
        let index = button.index();
        let current = self.buttons.get(index).copied().flatten();
        let id = match current {
            Some(held) if held.down == pressed => return Ok(()),
            Some(held) if dm.contains(held.id) => held.id,
            None if !pressed => return Ok(()),
            _ => {
                let Some(kind) = self.items.buttons.get(index) else {
                    return Ok(());
                };
                let id = self.make(dm, kind, None)?;
                self.relink_after(dm, |feed| {
                    if let Some(slot) = feed.buttons.get_mut(index) {
                        *slot = Some(Held { id, down: false });
                    }
                })?;
                id
            }
        };
        if let Some(slot) = self.buttons.get_mut(index) {
            *slot = Some(Held { id, down: pressed });
        }
        self.press(dm, id, pressed)
    }

    /// The cursor moved over the window, to `(x, y)` physical pixels from its top-left corner.
    pub fn cursor_moved(&mut self, dm: &mut DataModel, x: f64, y: f64) -> Result<(), DmError> {
        self.enable(dm, Device::Mouse)?;
        if let Some((old_x, old_y)) = self.location {
            self.cursor_shift.0 += x - old_x;
            self.cursor_shift.1 += y - old_y;
        }
        self.location = Some((x, y));
        self.cursor_moved = true;
        Ok(())
    }

    /// The mouse moved by `(dx, dy)` raw units, wherever the cursor is (it keeps arriving while it is locked).
    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        self.motion.0 += dx;
        self.motion.1 += dy;
        self.raw_motion = true;
    }

    /// The wheel turned by `notches` (positive away from the user).
    pub fn wheel(&mut self, dm: &mut DataModel, notches: f64) -> Result<(), DmError> {
        self.enable(dm, Device::Mouse)?;
        self.wheel += notches;
        Ok(())
    }

    /// A finger touched, moved on or left the screen, at `(x, y)` physical pixels. `touch` identifies the finger
    /// from `Began` to `Ended` or `Cancelled`.
    pub fn touch(&mut self, dm: &mut DataModel, touch: u64, phase: TouchPhase, x: f64, y: f64) -> Result<(), DmError> {
        self.enable(dm, Device::Touch)?;
        if phase == TouchPhase::Began {
            return self.touch_began(dm, touch, x, y);
        }
        let Some(index) = self.touches.iter().position(|slot| slot.touch == Some(touch)) else {
            return Ok(());
        };
        let Some(slot) = self.touches.get_mut(index) else {
            return Ok(());
        };
        slot.delta.0 += x - slot.position.0;
        slot.delta.1 += y - slot.position.1;
        slot.position = (x, y);
        if phase == TouchPhase::Moved {
            slot.moved = true;
            return Ok(());
        }
        let ended = *slot;
        slot.touch = None;
        slot.moved = false;
        if !dm.contains(ended.id) {
            return Ok(());
        }
        let state = if phase == TouchPhase::Cancelled {
            &self.items.cancel
        } else {
            &self.items.end
        };
        dm.set(ended.id, self.object.state, Value::Enum(state.clone()))?;
        dm.set(ended.id, self.object.position, vector3(x, y, 0.0))?;
        dm.set(ended.id, self.object.delta, vector3(ended.delta.0, ended.delta.1, 0.0))?;
        self.fire(dm, self.service.ended, ended.id)
    }

    fn touch_began(&mut self, dm: &mut DataModel, touch: u64, x: f64, y: f64) -> Result<(), DmError> {
        let free = self
            .touches
            .iter()
            .position(|slot| slot.touch.is_none() && dm.contains(slot.id));
        let index = match free {
            Some(index) => index,
            None => {
                let id = self.make(dm, &self.items.touch, None)?;
                self.relink_after(dm, |feed| {
                    feed.touches.push(TouchSlot {
                        id,
                        touch: None,
                        position: (x, y),
                        delta: (0.0, 0.0),
                        moved: false,
                    });
                })?;
                self.touches.len() - 1
            }
        };
        let Some(slot) = self.touches.get_mut(index) else {
            return Ok(());
        };
        slot.touch = Some(touch);
        slot.position = (x, y);
        slot.delta = (0.0, 0.0);
        slot.moved = false;
        let id = slot.id;
        dm.set(id, self.object.state, Value::Enum(self.items.begin.clone()))?;
        dm.set(id, self.object.position, vector3(x, y, 0.0))?;
        dm.set(id, self.object.delta, vector3(0.0, 0.0, 0.0))?;
        self.fire(dm, self.service.began, id)
    }

    /// The window gained (`true`) or lost keyboard focus. Losing it releases every held key and button (firing
    /// `InputEnded`), since the system does not report releases that happen while another window has focus.
    pub fn focus(&mut self, dm: &mut DataModel, focused: bool) -> Result<(), DmError> {
        if focused {
            return self.fire_bare(dm, self.service.focused);
        }
        for index in 0..self.keys.len() {
            if let Some(entry) = self.keys.get_mut(index)
                && entry.1.down
            {
                entry.1.down = false;
                let id = entry.1.id;
                if dm.contains(id) {
                    self.press(dm, id, false)?;
                }
            }
        }
        for index in 0..self.buttons.len() {
            if let Some(Some(held)) = self.buttons.get_mut(index)
                && held.down
            {
                held.down = false;
                let id = held.id;
                if dm.contains(id) {
                    self.press(dm, id, false)?;
                }
            }
        }
        self.fire_bare(dm, self.service.focus_released)
    }

    /// Sets `GamepadEnabled`: whether any gamepad is connected.
    pub fn set_gamepad_connected(&mut self, dm: &mut DataModel, connected: bool) -> Result<(), DmError> {
        self.set_device(dm, Device::Gamepad, connected)
    }

    /// Ends the frame's input step: writes the cursor and the frame's movement for `GetMouseLocation` and
    /// `GetMouseDelta`, and fires the frame's `InputChanged` events (mouse movement, wheel, moved fingers).
    pub fn end_frame(&mut self, dm: &mut DataModel) -> Result<(), DmError> {
        let service = self.service.id;
        let (x, y) = self.location.unwrap_or_default();
        let delta = if self.raw_motion {
            self.motion
        } else {
            self.cursor_shift
        };
        if self.cursor_moved {
            dm.set(
                service,
                self.service.mouse_location,
                Value::Vector2(Vector2::new(x as f32, y as f32)),
            )?;
        }
        // Written every frame, so a frame without movement reads zero; an unchanged value costs a compare.
        dm.set(
            service,
            self.service.mouse_delta,
            Value::Vector2(Vector2::new(delta.0 as f32, delta.1 as f32)),
        )?;
        if !self.firing {
            return Ok(());
        }
        let changed_watched = dm.is_signal_watched(service, self.service.changed);
        if self.raw_motion || self.cursor_moved {
            let watched = changed_watched || self.movement.is_some_and(|id| dm.is_watched(id));
            if watched {
                let id = self.changing_object(dm, Changing::Movement)?;
                dm.set(id, self.object.position, vector3(x, y, 0.0))?;
                dm.set(id, self.object.delta, vector3(delta.0, delta.1, 0.0))?;
                self.fire(dm, self.service.changed, id)?;
            }
        }
        if self.wheel != 0.0 {
            let watched = changed_watched || self.wheel_object.is_some_and(|id| dm.is_watched(id));
            if watched {
                let id = self.changing_object(dm, Changing::Wheel)?;
                dm.set(id, self.object.position, vector3(x, y, self.wheel))?;
                self.fire(dm, self.service.changed, id)?;
            }
        }
        for index in 0..self.touches.len() {
            let Some(&slot) = self.touches.get(index) else {
                continue;
            };
            if !slot.moved || slot.touch.is_none() || !dm.contains(slot.id) {
                continue;
            }
            dm.set(slot.id, self.object.state, Value::Enum(self.items.change.clone()))?;
            dm.set(slot.id, self.object.position, vector3(slot.position.0, slot.position.1, 0.0))?;
            dm.set(slot.id, self.object.delta, vector3(slot.delta.0, slot.delta.1, 0.0))?;
            self.fire(dm, self.service.changed, slot.id)?;
        }
        Ok(())
    }

    /// What the scripts asked of the cursor.
    pub fn cursor(&self, dm: &DataModel) -> CursorState {
        let behavior = match dm.enum_item(self.service.id, self.service.mouse_behavior).map(EnumItem::name) {
            Ok("LockCenter") => MouseBehavior::LockCenter,
            Ok("LockCurrentPosition") => MouseBehavior::LockCurrentPosition,
            _ => MouseBehavior::Default,
        };
        let icon_visible = dm.bool(self.service.id, self.service.mouse_icon_enabled).unwrap_or(true);
        CursorState { behavior, icon_visible }
    }

    /// How many input objects the feed keeps.
    pub fn object_count(&self) -> usize {
        self.objects().count()
    }

    /// A key's or a button's object goes to `Begin` (`pressed`) or `End`, at the cursor, and its event fires.
    fn press(&self, dm: &mut DataModel, id: Id, pressed: bool) -> Result<(), DmError> {
        let (state, signal) = if pressed {
            (&self.items.begin, self.service.began)
        } else {
            (&self.items.end, self.service.ended)
        };
        let (x, y) = self.location.unwrap_or_default();
        dm.set(id, self.object.state, Value::Enum(state.clone()))?;
        dm.set(id, self.object.position, vector3(x, y, 0.0))?;
        self.fire(dm, signal, id)
    }

    /// The movement or wheel object, created (or replaced, if a script destroyed it) on first use, in the
    /// `Change` state.
    fn changing_object(&mut self, dm: &mut DataModel, which: Changing) -> Result<Id, DmError> {
        let current = match which {
            Changing::Movement => self.movement,
            Changing::Wheel => self.wheel_object,
        };
        if let Some(id) = current.filter(|&id| dm.contains(id)) {
            return Ok(id);
        }
        let kind = match which {
            Changing::Movement => &self.items.movement,
            Changing::Wheel => &self.items.wheel,
        };
        let id = self.make(dm, kind, None)?;
        dm.set(id, self.object.state, Value::Enum(self.items.change.clone()))?;
        self.relink_after(dm, |feed| match which {
            Changing::Movement => feed.movement = Some(id),
            Changing::Wheel => feed.wheel_object = Some(id),
        })?;
        Ok(id)
    }

    /// A new input object of `kind`, for the key `key` if given.
    fn make(&self, dm: &mut DataModel, kind: &EnumItem, key: Option<&EnumItem>) -> Result<Id, DmError> {
        let id = dm.create_internal(self.object.class)?;
        dm.set(id, self.object.input_type, Value::Enum(kind.clone()))?;
        if let Some(key) = key {
            dm.set(id, self.object.key_code, Value::Enum(key.clone()))?;
        }
        Ok(id)
    }

    /// Changes the pool with `change`, then rewrites the chain the service's methods walk.
    fn relink_after(&mut self, dm: &mut DataModel, change: impl FnOnce(&mut Self)) -> Result<(), DmError> {
        change(self);
        self.relink(dm)
    }

    /// Rewrites the chain from `FirstInput` through every live object the feed keeps. Runs only when an object is
    /// created, so its list is no hot-path allocation.
    fn relink(&self, dm: &mut DataModel) -> Result<(), DmError> {
        let ids: Vec<Id> = self.objects().filter(|&id| dm.contains(id)).collect();
        let mut next = None;
        for &id in ids.iter().rev() {
            dm.set(id, self.object.next, Value::Ref(next))?;
            next = Some(id);
        }
        dm.set(self.service.id, self.service.first_input, Value::Ref(next))
    }

    fn objects(&self) -> impl Iterator<Item = Id> + '_ {
        self.keys
            .iter()
            .map(|(_, held)| held.id)
            .chain(self.buttons.iter().flatten().map(|held| held.id))
            .chain(self.movement)
            .chain(self.wheel_object)
            .chain(self.touches.iter().map(|slot| slot.id))
    }

    /// Fires one of the input events with `input`, while the game runs and something listens to it.
    fn fire(&self, dm: &mut DataModel, signal: SignalId, input: Id) -> Result<(), DmError> {
        if self.firing && dm.is_signal_watched(self.service.id, signal) {
            dm.fire(
                self.service.id,
                signal,
                vec![ScriptValue::Instance(input), ScriptValue::Bool(false)],
            )?;
        }
        Ok(())
    }

    /// Fires a signal without arguments, while the game runs and something listens to it.
    fn fire_bare(&self, dm: &mut DataModel, signal: SignalId) -> Result<(), DmError> {
        if self.firing && dm.is_signal_watched(self.service.id, signal) {
            dm.fire(self.service.id, signal, Vec::new())?;
        }
        Ok(())
    }

    /// Sets a device's `...Enabled` flag the first time its input arrives.
    fn enable(&mut self, dm: &mut DataModel, device: Device) -> Result<(), DmError> {
        if self.devices.get(device as usize).copied().unwrap_or(true) {
            return Ok(());
        }
        self.set_device(dm, device, true)
    }

    fn set_device(&mut self, dm: &mut DataModel, device: Device, on: bool) -> Result<(), DmError> {
        let index = device as usize;
        let (Some(flag), Some(&prop)) = (self.devices.get_mut(index), self.service.devices.get(index)) else {
            return Ok(());
        };
        if *flag != on {
            *flag = on;
            dm.set(self.service.id, prop, Value::Bool(on))?;
        }
        Ok(())
    }
}

/// The objects that only ever change: mouse movement and the wheel.
#[derive(Clone, Copy, Debug)]
enum Changing {
    Movement,
    Wheel,
}

#[cfg(test)]
mod tests {
    use metallic_core::{DataModel, Delivery, EnumItem, Id, ListenerKey, ScriptValue, Value, Vector2, Vector3};

    use super::{InputFeed, MouseBehavior, MouseButton, TouchPhase};
    use crate::testing::{call, game};

    fn feed(dm: &mut DataModel) -> InputFeed {
        InputFeed::new(dm).unwrap()
    }

    fn key_code(dm: &DataModel, name: &str) -> ScriptValue {
        ScriptValue::Enum(dm.registry().enum_item("KeyCode", name).unwrap())
    }

    fn input_type(dm: &DataModel, name: &str) -> ScriptValue {
        ScriptValue::Enum(dm.registry().enum_item("UserInputType", name).unwrap())
    }

    fn is_key_down(dm: &mut DataModel, service: Id, name: &str) -> bool {
        let key = key_code(dm, name);
        call(dm, service, "IsKeyDown", vec![key]) == Ok(ScriptValue::Bool(true))
    }

    fn connect(dm: &mut DataModel, service: Id, signal: &str, listener: u64) {
        let signal = dm.signal_id(service, signal).unwrap();
        dm.connect(service, signal, ListenerKey(listener)).unwrap();
    }

    fn drain(dm: &mut DataModel) -> Vec<Delivery> {
        let mut out = Vec::new();
        dm.drain_deliveries(&mut out);
        out
    }

    fn field(dm: &DataModel, id: Id, name: &str) -> Value {
        dm.get_by_name(id, name).unwrap()
    }

    fn item(enum_type: &str, name: &str) -> Value {
        Value::Enum(EnumItem::new(enum_type, name))
    }

    fn v3(x: f32, y: f32, z: f32) -> Value {
        Value::Vector3(Vector3::new(x, y, z))
    }

    fn instance(value: &ScriptValue) -> Id {
        match value {
            ScriptValue::Instance(id) => *id,
            other => panic!("expected an instance, got {other:?}"),
        }
    }

    fn keys_pressed(dm: &mut DataModel, service: Id) -> Vec<Id> {
        match call(dm, service, "GetKeysPressed", vec![]).unwrap() {
            ScriptValue::List(list) => list.iter().map(instance).collect(),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    /// One frame with `events` in it.
    fn frame(dm: &mut DataModel, feed: &mut InputFeed, firing: bool, events: impl FnOnce(&mut DataModel, &mut InputFeed)) {
        feed.begin_frame(dm, firing).unwrap();
        events(dm, feed);
        feed.end_frame(dm).unwrap();
    }

    #[test]
    fn keys_are_held_between_their_begin_and_end() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        assert!(!is_key_down(&mut dm, uis, "W"));
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "W", true).unwrap());
        assert!(is_key_down(&mut dm, uis, "W"));
        assert!(!is_key_down(&mut dm, uis, "A"));
        let pressed = keys_pressed(&mut dm, uis);
        assert_eq!(pressed.len(), 1);
        let w = pressed[0];
        assert_eq!(field(&dm, w, "KeyCode"), item("KeyCode", "W"));
        assert_eq!(field(&dm, w, "UserInputType"), item("UserInputType", "Keyboard"));
        assert_eq!(field(&dm, w, "UserInputState"), item("UserInputState", "Begin"));
        assert_eq!(dm.parent(w), Ok(None), "input objects have no parent");
        assert_eq!(field(&dm, uis, "KeyboardEnabled"), Value::Bool(true));

        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "W", false).unwrap());
        assert!(!is_key_down(&mut dm, uis, "W"));
        assert_eq!(field(&dm, w, "UserInputState"), item("UserInputState", "End"));
        assert!(keys_pressed(&mut dm, uis).is_empty());

        // The next press reuses the key's object.
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "W", true).unwrap());
        assert_eq!(feed.object_count(), 1);
        assert_eq!(field(&dm, w, "UserInputState"), item("UserInputState", "Begin"));
    }

    #[test]
    fn events_fire_only_while_running_and_listened_to() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "A", true).unwrap());
        assert!(drain(&mut dm).is_empty());

        connect(&mut dm, uis, "InputBegan", 1);
        connect(&mut dm, uis, "InputEnded", 2);
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "A", true).unwrap(); // already down: an OS repeat
            feed.key(dm, "A", false).unwrap();
            feed.key(dm, "B", true).unwrap();
            feed.key(dm, "Unknown", true).unwrap();
        });
        let got: Vec<(u64, ScriptValue)> = drain(&mut dm)
            .into_iter()
            .map(|d| (d.listener.0, d.args[1].clone()))
            .collect();
        assert_eq!(got, [(2, ScriptValue::Bool(false)), (1, ScriptValue::Bool(false))]);

        // A stopped or paused game keeps the state but fires nothing.
        frame(&mut dm, &mut feed, false, |dm, feed| feed.key(dm, "B", false).unwrap());
        assert!(drain(&mut dm).is_empty());
        assert!(!is_key_down(&mut dm, uis, "B"));
    }

    #[test]
    fn mouse_delta_is_the_frames_movement() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.cursor_moved(dm, 10.0, 20.0).unwrap();
            feed.mouse_motion(3.0, -1.0);
            feed.mouse_motion(2.0, 4.0);
        });
        assert_eq!(
            call(&mut dm, uis, "GetMouseDelta", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(5.0, 3.0)))
        );
        assert_eq!(
            call(&mut dm, uis, "GetMouseLocation", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(10.0, 20.0)))
        );
        assert_eq!(feed.object_count(), 0, "nothing listens: no movement object");
        frame(&mut dm, &mut feed, true, |_, _| {});
        assert_eq!(
            call(&mut dm, uis, "GetMouseDelta", vec![]),
            Ok(ScriptValue::Vector2(Vector2::ZERO))
        );
        assert_eq!(
            call(&mut dm, uis, "GetMouseLocation", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(10.0, 20.0)))
        );
    }

    #[test]
    fn mouse_movement_fires_once_per_frame_with_the_frames_delta() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputChanged", 1);
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.cursor_moved(dm, 1.0, 2.0).unwrap();
            feed.mouse_motion(1.0, 1.0);
            feed.mouse_motion(1.0, 1.0);
            feed.wheel(dm, 1.0).unwrap();
        });
        let got = drain(&mut dm);
        assert_eq!(got.len(), 2, "movement, then the wheel");
        let movement = instance(&got[0].args[0]);
        assert_eq!(field(&dm, movement, "UserInputType"), item("UserInputType", "MouseMovement"));
        assert_eq!(field(&dm, movement, "UserInputState"), item("UserInputState", "Change"));
        assert_eq!(field(&dm, movement, "Delta"), v3(2.0, 2.0, 0.0));
        assert_eq!(field(&dm, movement, "Position"), v3(1.0, 2.0, 0.0));
        let wheel = instance(&got[1].args[0]);
        assert_eq!(field(&dm, wheel, "Position"), v3(1.0, 2.0, 1.0));
        frame(&mut dm, &mut feed, true, |_, _| {});
        assert!(drain(&mut dm).is_empty(), "no movement, no event");
    }

    #[test]
    fn mouse_buttons_and_focus_loss() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        let left = input_type(&dm, "MouseButton1");
        let right = input_type(&dm, "MouseButton2");
        let keyboard = input_type(&dm, "Keyboard");
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.mouse_button(dm, MouseButton::Left, true).unwrap();
            feed.key(dm, "Space", true).unwrap();
        });
        let pressed = |dm: &mut DataModel, button: &ScriptValue| {
            call(dm, uis, "IsMouseButtonPressed", vec![button.clone()]) == Ok(ScriptValue::Bool(true))
        };
        assert!(pressed(&mut dm, &left));
        assert!(!pressed(&mut dm, &right));
        assert!(!pressed(&mut dm, &keyboard), "not a mouse button");
        let w = key_code(&dm, "W");
        assert!(call(&mut dm, uis, "IsMouseButtonPressed", vec![w]).is_err());

        connect(&mut dm, uis, "InputEnded", 1);
        connect(&mut dm, uis, "WindowFocusReleased", 2);
        frame(&mut dm, &mut feed, true, |dm, feed| feed.focus(dm, false).unwrap());
        assert!(!pressed(&mut dm, &left));
        assert!(!is_key_down(&mut dm, uis, "Space"));
        let listeners: Vec<u64> = drain(&mut dm).iter().map(|d| d.listener.0).collect();
        assert_eq!(listeners, [1, 1, 2], "two releases, then the focus event");
    }

    #[test]
    fn touches_begin_change_once_per_frame_and_end() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        for (n, signal) in (0..).zip(["InputBegan", "InputChanged", "InputEnded"]) {
            connect(&mut dm, uis, signal, n);
        }
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 9, TouchPhase::Began, 5.0, 5.0).unwrap();
        });
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 9, TouchPhase::Moved, 6.0, 5.0).unwrap();
            feed.touch(dm, 9, TouchPhase::Moved, 8.0, 7.0).unwrap();
        });
        let got = drain(&mut dm);
        let listeners: Vec<u64> = got.iter().map(|d| d.listener.0).collect();
        assert_eq!(listeners, [0, 1]);
        let finger = instance(&got[0].args[0]);
        assert_eq!(field(&dm, finger, "Delta"), v3(3.0, 2.0, 0.0));
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 9, TouchPhase::Ended, 8.0, 7.0).unwrap();
        });
        let listeners: Vec<u64> = drain(&mut dm).iter().map(|d| d.listener.0).collect();
        assert_eq!(listeners, [2]);
        assert_eq!(field(&dm, finger, "UserInputState"), item("UserInputState", "End"));
        assert_eq!(field(&dm, uis, "TouchEnabled"), Value::Bool(true));
        // The next finger reuses the free object.
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 10, TouchPhase::Began, 1.0, 1.0).unwrap();
        });
        assert_eq!(feed.object_count(), 1);
    }

    #[test]
    fn a_destroyed_input_object_is_replaced() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "Q", true).unwrap();
            feed.key(dm, "E", true).unwrap();
        });
        let q = keys_pressed(&mut dm, uis)[0];
        call(&mut dm, q, "Destroy", vec![]).unwrap();
        assert!(!is_key_down(&mut dm, uis, "Q"), "a destroyed object fails no call");
        // The next frame repairs the chain: the other keys are found again.
        frame(&mut dm, &mut feed, true, |_, _| {});
        assert!(is_key_down(&mut dm, uis, "E"));
        assert_eq!(feed.object_count(), 1);
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "Q", false).unwrap());
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "Q", true).unwrap());
        assert!(is_key_down(&mut dm, uis, "Q"));
        assert!(is_key_down(&mut dm, uis, "E"));
    }

    #[test]
    fn scripts_cannot_create_input_objects_or_write_their_fields() {
        let dm = game();
        let registry = dm.registry();
        let class = registry.class_id("InputObject").unwrap();
        assert!(!registry.class(class).unwrap().creatable());
        for name in ["KeyCode", "UserInputType", "UserInputState", "Position", "Delta"] {
            let prop = registry.prop_id(class, name).unwrap();
            let meta = registry.prop(class, prop).unwrap();
            assert!(meta.meta.flags.contains(metallic_core::PropFlags::READ_ONLY), "{name}");
        }
    }

    #[test]
    fn cursor_reads_what_scripts_set() {
        let mut dm = game();
        let feed = feed(&mut dm);
        let uis = feed.service();
        assert_eq!(feed.cursor(&dm).behavior, MouseBehavior::Default);
        assert!(feed.cursor(&dm).icon_visible);
        dm.set_by_name(uis, "MouseBehavior", item("MouseBehavior", "LockCenter")).unwrap();
        dm.set_by_name(uis, "MouseIconEnabled", Value::Bool(false)).unwrap();
        assert_eq!(feed.cursor(&dm).behavior, MouseBehavior::LockCenter);
        assert!(!feed.cursor(&dm).icon_visible);
    }

    #[test]
    fn gamepads_set_gamepad_enabled() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        feed.set_gamepad_connected(&mut dm, true).unwrap();
        assert_eq!(field(&dm, uis, "GamepadEnabled"), Value::Bool(true));
        feed.set_gamepad_connected(&mut dm, false).unwrap();
        assert_eq!(field(&dm, uis, "GamepadEnabled"), Value::Bool(false));
    }
}
