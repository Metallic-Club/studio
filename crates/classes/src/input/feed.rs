//! The engine's side of `UserInputService`: [`InputFeed`] turns input events into the service's state and events.
//!
//! The engine owns one feed per game and calls it in the first step of every frame: [`InputFeed::begin_frame`],
//! then one call per event the frame took ([`InputFeed::key`], [`InputFeed::mouse_motion`], ...), then
//! [`InputFeed::end_frame`]. Before ending the frame it reads [`InputFeed::cursor`] and applies it to the window, and
//! calls [`InputFeed::place_cursor`] when it moves the cursor itself. The types here are plain values, so this crate
//! does not depend on the platform crate; the engine translates the platform's events.
//!
//! # Input objects
//!
//! - **A press is one object.** A key or mouse button going down, or a finger touching, gets a new `InputObject`
//!   in the `Begin` state; that object goes through `Change` (a moving finger) and `End`, then the engine destroys
//!   it. Scripts that kept it still read it (a destroyed instance stays readable while a script holds it), so a
//!   `held[input] = true` table works as in Roblox.
//! - **Only when someone listens.** A press gets an object only while something is connected to the event it would
//!   fire (`InputBegan` or `InputEnded`; for a finger, `InputChanged` too). The held state itself is bits in the
//!   service (`IsKeyDown`), and `GetKeysPressed` makes objects for held keys that have none.
//! - **Mouse movement and the wheel** reuse one object each, created the first time something listens: they fire
//!   every frame and carry the frame's totals.
//!
//! # A press shorter than a frame
//!
//! Signals are delivered after the input step, and listeners read the object as it is then. A key pressed and
//! released within one frame would show `InputBegan` an object already in `End`, so the release waits for the
//! next frame: each press, button and finger begins at most once and ends at most once per frame, and events that
//! would break that wait, in order, for the next one. `InputBegan` sees `Begin`, `InputEnded` sees `End`, and
//! `IsKeyDown` is true for one frame, so scripts that poll see the tap too.
//!
//! # What an event costs
//!
//! - **Mouse movement** only adds to the frame's totals. At the end of the frame the hidden `MouseLocation` and
//!   `MouseDelta` are written (a write of an unchanged value does nothing), and `InputChanged` fires once with the
//!   frame's movement, only while something listens to it (or to the movement object itself). With nothing
//!   connected, mouse movement creates no instance and allocates nothing.
//! - **Keys and buttons** flip a bit in the service. With nothing listening, a press creates no instance either.
//! - **Signals** fire only while the game runs ([`InputFeed::begin_frame`]'s `firing`) and only when something
//!   listens to that signal ([`DataModel::is_signal_watched`]); building their arguments is the one allocation.

use metallic_core::{ClassId, DataModel, DmError, EnumItem, Id, PropId, ScriptValue, SignalId, Value, Vector2, Vector3};

use crate::GameError;
use crate::input::input_object::{DELTA, KEY_CODE, NEXT_INPUT, POSITION, USER_INPUT_STATE, USER_INPUT_TYPE};
use crate::input::key_code::{self, UNKNOWN};
use crate::services::user_input_service::{
    FIRST_INPUT, GAMEPAD_ENABLED, HELD_BUTTONS, HELD_KEYS, KEYBOARD_ENABLED, MOUSE_BEHAVIOR, MOUSE_DELTA,
    MOUSE_ENABLED, MOUSE_ICON_ENABLED, MOUSE_LOCATION, TOUCH_ENABLED,
};

/// Events waiting for the next frame past this many are applied at once instead: only input far faster than any
/// hand (many presses of one key per frame) gets there, and the queue must not grow without bound.
const MAX_WAITING: usize = 256;

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
    held_keys: [PropId; key_code::WORDS],
    held_buttons: PropId,
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

/// What can begin and end: a key (by `Enum.KeyCode` item index), a mouse button, a finger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Key(usize),
    Button(usize),
    Touch(u64),
}

/// A begin, change or end event, applied now or in the next frame.
#[derive(Clone, Copy, Debug)]
enum Press {
    Key { index: usize, down: bool },
    Button { index: usize, down: bool },
    Touch { touch: u64, phase: TouchPhase, x: f64, y: f64 },
}

impl Press {
    fn source(self) -> Source {
        match self {
            Self::Key { index, .. } => Source::Key(index),
            Self::Button { index, .. } => Source::Button(index),
            Self::Touch { touch, .. } => Source::Touch(touch),
        }
    }

    fn ends(self) -> bool {
        match self {
            Self::Key { down, .. } | Self::Button { down, .. } => !down,
            Self::Touch { phase, .. } => matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled),
        }
    }
}

/// A finger on the screen.
#[derive(Clone, Copy, Debug)]
struct TouchSlot {
    touch: u64,
    /// Its input object, when something listened.
    object: Option<Id>,
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
    /// Held keys as bits by `Enum.KeyCode` item index, as written to `HeldKeys1` to `HeldKeys3`.
    held_keys: [u64; key_code::WORDS],
    /// Held mouse buttons as bits, as written to `HeldButtons`.
    held_buttons: u64,
    /// The input objects of held keys, by item index, in the order of the `FirstInput` chain.
    keys: Vec<(usize, Id)>,
    /// The input objects of held mouse buttons.
    buttons: [Option<Id>; 3],
    touches: Vec<TouchSlot>,
    movement: Option<Id>,
    wheel_object: Option<Id>,
    /// What began this frame: its end waits for the next one.
    began: Vec<Source>,
    /// Events waiting for the next frame, in arrival order.
    waiting: Vec<Press>,
    /// The previous frame's waiting events, while this frame applies them (kept to reuse its buffer).
    carried: Vec<Press>,
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
    /// The engine moved the cursor itself this frame ([`InputFeed::place_cursor`]).
    placed: bool,
    /// Wheel notches this frame.
    wheel: f64,
    /// The data model's structure revision at the last check of the objects the feed keeps.
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

/// The bits as the integer property stores them.
fn bits_value(bits: u64) -> Value {
    Value::Int(bits as i64)
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
            held_keys: [prop(HELD_KEYS[0])?, prop(HELD_KEYS[1])?, prop(HELD_KEYS[2])?],
            held_buttons: prop(HELD_BUTTONS)?,
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
        // A game handed over by another engine starts with nothing held: a press it saw never ends here.
        for &prop in service.held_keys.iter().chain([&service.held_buttons]) {
            dm.set(id, prop, Value::Int(0))?;
        }
        dm.set(id, service.first_input, Value::Ref(None))?;
        Ok(Self {
            service,
            object,
            items,
            held_keys: [0; key_code::WORDS],
            held_buttons: 0,
            keys: Vec::new(),
            buttons: [None; 3],
            touches: Vec::new(),
            movement: None,
            wheel_object: None,
            began: Vec::new(),
            waiting: Vec::new(),
            carried: Vec::new(),
            devices,
            firing: false,
            location: None,
            motion: (0.0, 0.0),
            raw_motion: false,
            cursor_shift: (0.0, 0.0),
            cursor_moved: false,
            placed: false,
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
    /// The events the previous frame left waiting are applied first. When the data model's structure changed
    /// since the last frame, the feed also takes over the key objects `GetKeysPressed` made and forgets the objects
    /// scripts destroyed.
    pub fn begin_frame(&mut self, dm: &mut DataModel, firing: bool) -> Result<(), DmError> {
        self.firing = firing;
        self.motion = (0.0, 0.0);
        self.raw_motion = false;
        self.cursor_shift = (0.0, 0.0);
        self.cursor_moved = false;
        self.placed = false;
        self.wheel = 0.0;
        for slot in &mut self.touches {
            slot.delta = (0.0, 0.0);
            slot.moved = false;
        }
        self.began.clear();
        let revision = dm.structure_revision();
        if revision != self.revision {
            self.revision = revision;
            self.sync_objects(dm)?;
        }
        std::mem::swap(&mut self.waiting, &mut self.carried);
        let mut carried = std::mem::take(&mut self.carried);
        let mut result = Ok(());
        for press in carried.drain(..) {
            if let Err(error) = self.submit(dm, press) {
                result = Err(error);
            }
        }
        self.carried = carried;
        result
    }

    /// A key went down (`pressed`) or up. `key` is its `Enum.KeyCode` item name; [`UNKNOWN`] keys are not tracked.
    /// A press of a key already down (an OS key repeat) and a release of a key not down do nothing.
    pub fn key(&mut self, dm: &mut DataModel, key: &str, pressed: bool) -> Result<(), DmError> {
        self.enable(dm, Device::Keyboard)?;
        if key == UNKNOWN {
            return Ok(());
        }
        let Some(index) = key_code::index(key) else {
            return Ok(());
        };
        self.submit(dm, Press::Key { index, down: pressed })
    }

    /// A mouse button went down (`pressed`) or up.
    pub fn mouse_button(&mut self, dm: &mut DataModel, button: MouseButton, pressed: bool) -> Result<(), DmError> {
        self.enable(dm, Device::Mouse)?;
        self.submit(
            dm,
            Press::Button {
                index: button.index(),
                down: pressed,
            },
        )
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

    /// The engine moved the cursor to `(x, y)` physical pixels itself, to hold it where `MouseBehavior` asks.
    /// `GetMouseLocation` reads the new place, but the move is not mouse movement: it adds nothing to the frame's
    /// delta and fires no `InputChanged`.
    pub fn place_cursor(&mut self, x: f64, y: f64) {
        self.location = Some((x, y));
        self.placed = true;
    }

    /// The cursor's last known place, in physical pixels from the window's top-left corner; `None` until the
    /// cursor has been seen over the window.
    pub fn cursor_location(&self) -> Option<(f64, f64)> {
        self.location
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
        self.submit(dm, Press::Touch { touch, phase, x, y })
    }

    /// The window gained (`true`) or lost keyboard focus. Losing it releases every held key and button (firing
    /// `InputEnded`), since the system does not report releases that happen while another window has focus.
    pub fn focus(&mut self, dm: &mut DataModel, focused: bool) -> Result<(), DmError> {
        if focused {
            return self.fire_bare(dm, self.service.focused);
        }
        for word in 0..key_code::WORDS {
            for bit in 0..64 {
                if self.held_keys.get(word).is_some_and(|bits| bits & (1 << bit) != 0) {
                    self.submit(
                        dm,
                        Press::Key {
                            index: word * 64 + bit,
                            down: false,
                        },
                    )?;
                }
            }
        }
        for index in 0..self.buttons.len() {
            if self.held_buttons & (1 << index) != 0 {
                self.submit(dm, Press::Button { index, down: false })?;
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
        if self.cursor_moved || self.placed {
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
        if (self.raw_motion || self.cursor_moved)
            && (changed_watched || self.movement.is_some_and(|id| dm.is_watched(id)))
        {
            let id = self.changing_object(dm, Changing::Movement)?;
            dm.set(id, self.object.position, vector3(x, y, 0.0))?;
            dm.set(id, self.object.delta, vector3(delta.0, delta.1, 0.0))?;
            self.fire(dm, self.service.changed, id)?;
        }
        if self.wheel != 0.0 && (changed_watched || self.wheel_object.is_some_and(|id| dm.is_watched(id))) {
            let id = self.changing_object(dm, Changing::Wheel)?;
            dm.set(id, self.object.position, vector3(x, y, self.wheel))?;
            self.fire(dm, self.service.changed, id)?;
        }
        for index in 0..self.touches.len() {
            let Some(&slot) = self.touches.get(index) else {
                continue;
            };
            if !slot.moved {
                continue;
            }
            let object = match slot.object.filter(|&id| dm.contains(id)) {
                Some(id) => Some(id),
                None if changed_watched => Some(self.make(dm, &self.items.touch, None)?),
                None => None,
            };
            let Some(id) = object else {
                continue;
            };
            if let Some(slot) = self.touches.get_mut(index) {
                slot.object = Some(id);
            }
            dm.set(id, self.object.state, Value::Enum(self.items.change.clone()))?;
            dm.set(id, self.object.position, vector3(slot.position.0, slot.position.1, 0.0))?;
            dm.set(id, self.object.delta, vector3(slot.delta.0, slot.delta.1, 0.0))?;
            self.fire(dm, self.service.changed, id)?;
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

    /// How many input objects the feed holds now: those of held keys, buttons and fingers, plus the movement and
    /// wheel objects once made.
    pub fn object_count(&self) -> usize {
        self.keys.len()
            + self.buttons.iter().flatten().count()
            + self.touches.iter().filter(|slot| slot.object.is_some()).count()
            + usize::from(self.movement.is_some())
            + usize::from(self.wheel_object.is_some())
    }

    /// How many events wait for the next frame.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Applies `press` now, or makes it wait for the next frame: an end of something that began this frame waits,
    /// and so does anything behind an event of the same source that already waits, to keep their order.
    fn submit(&mut self, dm: &mut DataModel, press: Press) -> Result<(), DmError> {
        let source = press.source();
        let waits = self.waiting.iter().any(|w| w.source() == source)
            || (press.ends() && self.began.contains(&source));
        if waits && self.waiting.len() < MAX_WAITING {
            self.waiting.push(press);
            return Ok(());
        }
        match press {
            Press::Key { index, down } => self.apply_key(dm, index, down),
            Press::Button { index, down } => self.apply_button(dm, index, down),
            Press::Touch { touch, phase, x, y } => self.apply_touch(dm, touch, phase, x, y),
        }
    }

    fn apply_key(&mut self, dm: &mut DataModel, index: usize, down: bool) -> Result<(), DmError> {
        let (word, bit) = (index / 64, 1u64 << (index % 64));
        let (Some(bits), Some(&prop)) = (self.held_keys.get_mut(word), self.service.held_keys.get(word)) else {
            return Ok(());
        };
        if (*bits & bit != 0) == down {
            return Ok(());
        }
        *bits ^= bit;
        let value = bits_value(*bits);
        dm.set(self.service.id, prop, value)?;
        if down {
            self.began.push(Source::Key(index));
            if self.wants_object(dm, false) {
                let key = key_code::name(index).and_then(|name| dm.registry().enum_item("KeyCode", name));
                let id = self.make(dm, &self.items.keyboard, key.as_ref())?;
                self.keys.push((index, id));
                self.relink(dm)?;
                self.begin(dm, id)?;
            }
            return Ok(());
        }
        let held = self.keys.iter().position(|&(key, _)| key == index);
        let object = held.map(|at| self.keys.remove(at).1);
        if object.is_some() {
            self.relink(dm)?;
        }
        let object = match object.filter(|&id| dm.contains(id)) {
            Some(id) => Some(id),
            None if self.wants_end_object(dm) => {
                let key = key_code::name(index).and_then(|name| dm.registry().enum_item("KeyCode", name));
                Some(self.make(dm, &self.items.keyboard, key.as_ref())?)
            }
            None => None,
        };
        match object {
            Some(id) => self.end(dm, id, &self.items.end, None),
            None => Ok(()),
        }
    }

    fn apply_button(&mut self, dm: &mut DataModel, index: usize, down: bool) -> Result<(), DmError> {
        let bit = 1u64 << index;
        if (self.held_buttons & bit != 0) == down {
            return Ok(());
        }
        self.held_buttons ^= bit;
        dm.set(self.service.id, self.service.held_buttons, bits_value(self.held_buttons))?;
        let Some(kind) = self.items.buttons.get(index) else {
            return Ok(());
        };
        if down {
            self.began.push(Source::Button(index));
            if self.wants_object(dm, false) {
                let id = self.make(dm, kind, None)?;
                if let Some(slot) = self.buttons.get_mut(index) {
                    *slot = Some(id);
                }
                self.begin(dm, id)?;
            }
            return Ok(());
        }
        let object = self.buttons.get_mut(index).and_then(Option::take);
        let object = match object.filter(|&id| dm.contains(id)) {
            Some(id) => Some(id),
            None if self.wants_end_object(dm) => Some(self.make(dm, kind, None)?),
            None => None,
        };
        match object {
            Some(id) => self.end(dm, id, &self.items.end, None),
            None => Ok(()),
        }
    }

    fn apply_touch(&mut self, dm: &mut DataModel, touch: u64, phase: TouchPhase, x: f64, y: f64) -> Result<(), DmError> {
        let at = self.touches.iter().position(|slot| slot.touch == touch);
        match (phase, at) {
            (TouchPhase::Began, None) => {
                self.began.push(Source::Touch(touch));
                let object = if self.wants_object(dm, true) {
                    Some(self.make(dm, &self.items.touch, None)?)
                } else {
                    None
                };
                self.touches.push(TouchSlot {
                    touch,
                    object,
                    position: (x, y),
                    delta: (0.0, 0.0),
                    moved: false,
                });
                match object {
                    Some(id) => {
                        dm.set(id, self.object.position, vector3(x, y, 0.0))?;
                        self.begin(dm, id)
                    }
                    None => Ok(()),
                }
            }
            (TouchPhase::Moved, Some(at)) => {
                if let Some(slot) = self.touches.get_mut(at) {
                    slot.delta.0 += x - slot.position.0;
                    slot.delta.1 += y - slot.position.1;
                    slot.position = (x, y);
                    slot.moved = true;
                }
                Ok(())
            }
            (TouchPhase::Ended | TouchPhase::Cancelled, Some(at)) => {
                let slot = self.touches.remove(at);
                let delta = (slot.delta.0 + x - slot.position.0, slot.delta.1 + y - slot.position.1);
                let object = match slot.object.filter(|&id| dm.contains(id)) {
                    Some(id) => Some(id),
                    None if self.wants_end_object(dm) => Some(self.make(dm, &self.items.touch, None)?),
                    None => None,
                };
                let Some(id) = object else {
                    return Ok(());
                };
                let state = if phase == TouchPhase::Cancelled {
                    &self.items.cancel
                } else {
                    &self.items.end
                };
                dm.set(id, self.object.position, vector3(x, y, 0.0))?;
                self.end(dm, id, state, Some(delta))
            }
            // A finger the feed does not know (it began before the engine started), or a second start of one.
            _ => Ok(()),
        }
    }

    /// Whether a press beginning now needs an object: while the game runs and something listens for its
    /// beginning or its end (or, for a finger, its movement).
    fn wants_object(&self, dm: &DataModel, finger: bool) -> bool {
        let service = self.service.id;
        self.firing
            && (dm.is_signal_watched(service, self.service.began)
                || dm.is_signal_watched(service, self.service.ended)
                || (finger && dm.is_signal_watched(service, self.service.changed)))
    }

    /// Whether a press ending without an object needs one made now: while the game runs and something listens
    /// for its end (a listener connected after it began).
    fn wants_end_object(&self, dm: &DataModel) -> bool {
        self.firing && dm.is_signal_watched(self.service.id, self.service.ended)
    }

    /// A press's object begins: `Begin`, at the cursor (a finger's position is set by the caller), and
    /// `InputBegan` fires.
    fn begin(&self, dm: &mut DataModel, id: Id) -> Result<(), DmError> {
        dm.set(id, self.object.state, Value::Enum(self.items.begin.clone()))?;
        if dm.enum_item(id, self.object.input_type).is_ok_and(|kind| *kind != self.items.touch) {
            let (x, y) = self.location.unwrap_or_default();
            dm.set(id, self.object.position, vector3(x, y, 0.0))?;
        }
        self.fire(dm, self.service.began, id)
    }

    /// A press's object ends in `state` (`End` or `Cancel`), `InputEnded` fires, and the object is destroyed. A
    /// script holding it still reads it; one that never did lets it go.
    fn end(&self, dm: &mut DataModel, id: Id, state: &EnumItem, delta: Option<(f64, f64)>) -> Result<(), DmError> {
        dm.set(id, self.object.state, Value::Enum(state.clone()))?;
        match delta {
            Some((dx, dy)) => dm.set(id, self.object.delta, vector3(dx, dy, 0.0))?,
            None => {
                let (x, y) = self.location.unwrap_or_default();
                dm.set(id, self.object.position, vector3(x, y, 0.0))?;
            }
        }
        self.fire(dm, self.service.ended, id)?;
        dm.destroy(id)
    }

    /// Takes over the key objects `GetKeysPressed` made (they are first in the chain), destroys any that are not
    /// held or duplicate one the feed has, forgets the objects scripts destroyed, and rewrites the chain if any of
    /// that changed it.
    fn sync_objects(&mut self, dm: &mut DataModel) -> Result<(), DmError> {
        let mut changed = false;
        let mut next = dm.reference(self.service.id, self.service.first_input).unwrap_or(None);
        // The chain holds at most one object per key; the bound only stops a walk the engine did not write.
        for _ in 0..key_code::COUNT * 2 {
            let Some(id) = next else {
                break;
            };
            if !dm.contains(id) {
                changed = true;
                break;
            }
            next = dm.reference(id, self.object.next).unwrap_or(None);
            if self.keys.iter().any(|&(_, known)| known == id) {
                continue;
            }
            changed = true;
            let index = dm
                .enum_item(id, self.object.key_code)
                .ok()
                .and_then(|item| key_code::index(item.name()));
            let held = index.is_some_and(|index| {
                self.held_keys
                    .get(index / 64)
                    .is_some_and(|bits| bits & (1 << (index % 64)) != 0)
            });
            match index {
                Some(index) if held && !self.keys.iter().any(|&(key, _)| key == index) => self.keys.push((index, id)),
                _ => dm.destroy(id)?,
            }
        }
        let before = self.keys.len();
        self.keys.retain(|&(_, id)| dm.contains(id));
        changed |= self.keys.len() != before;
        for slot in &mut self.buttons {
            *slot = slot.filter(|&id| dm.contains(id));
        }
        for slot in &mut self.touches {
            slot.object = slot.object.filter(|&id| dm.contains(id));
        }
        self.movement = self.movement.filter(|&id| dm.contains(id));
        self.wheel_object = self.wheel_object.filter(|&id| dm.contains(id));
        if changed {
            self.relink(dm)?;
        }
        Ok(())
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
        match which {
            Changing::Movement => self.movement = Some(id),
            Changing::Wheel => self.wheel_object = Some(id),
        }
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

    /// Rewrites the chain from `FirstInput` through the objects of held keys, which `GetKeysPressed` walks. Runs
    /// when a key with an object goes down or up, never for mouse movement.
    fn relink(&self, dm: &mut DataModel) -> Result<(), DmError> {
        let mut next = None;
        for &(_, id) in self.keys.iter().rev() {
            dm.set(id, self.object.next, Value::Ref(next))?;
            next = Some(id);
        }
        dm.set(self.service.id, self.service.first_input, Value::Ref(next))
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

    /// The listener and the input object of each delivery.
    fn delivered(dm: &mut DataModel) -> Vec<(u64, Id)> {
        drain(dm).iter().map(|d| (d.listener.0, instance(&d.args[0]))).collect()
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
    fn frame(
        dm: &mut DataModel,
        feed: &mut InputFeed,
        firing: bool,
        events: impl FnOnce(&mut DataModel, &mut InputFeed),
    ) {
        feed.begin_frame(dm, firing).unwrap();
        events(dm, feed);
        feed.end_frame(dm).unwrap();
    }

    #[test]
    fn unwatched_keys_are_held_bits_and_make_no_object() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        let instances = dm.len();
        assert!(!is_key_down(&mut dm, uis, "W"));
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "W", true).unwrap());
        assert!(is_key_down(&mut dm, uis, "W"));
        assert!(!is_key_down(&mut dm, uis, "A"));
        assert_eq!(dm.len(), instances, "nothing listens: no object");
        assert_eq!(field(&dm, uis, "KeyboardEnabled"), Value::Bool(true));

        // GetKeysPressed makes the object; the feed takes it over and ends it with the press.
        let pressed = keys_pressed(&mut dm, uis);
        assert_eq!(pressed.len(), 1);
        let w = pressed[0];
        assert_eq!(field(&dm, w, "KeyCode"), item("KeyCode", "W"));
        assert_eq!(field(&dm, w, "UserInputType"), item("UserInputType", "Keyboard"));
        assert_eq!(field(&dm, w, "UserInputState"), item("UserInputState", "Begin"));
        assert_eq!(dm.parent(w), Ok(None), "input objects have no parent");
        assert_eq!(keys_pressed(&mut dm, uis), [w], "the same object while the key is held");
        frame(&mut dm, &mut feed, true, |_, _| {});
        assert_eq!(feed.object_count(), 1, "taken over");

        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "W", false).unwrap());
        assert!(!is_key_down(&mut dm, uis, "W"));
        assert!(!dm.contains(w), "ended with the press");
        assert!(keys_pressed(&mut dm, uis).is_empty());
        assert_eq!(dm.len(), instances);
    }

    #[test]
    fn each_press_is_a_new_object_through_begin_and_end() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputBegan", 1);
        connect(&mut dm, uis, "InputEnded", 2);
        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "A", true).unwrap());
        let began = delivered(&mut dm);
        assert_eq!(began.len(), 1);
        let first = began[0].1;
        assert_eq!(field(&dm, first, "UserInputState"), item("UserInputState", "Begin"));
        assert_eq!(keys_pressed(&mut dm, uis), [first], "GetKeysPressed passes the press's object");

        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "A", true).unwrap(); // already down: an OS repeat
            feed.key(dm, "A", false).unwrap();
        });
        assert_eq!(delivered(&mut dm), [(2, first)], "InputEnded passes the same object");
        assert_eq!(dm.grave(first).map(|(class, _)| class), Some("InputObject"));
        assert_eq!(field(&dm, first, "UserInputState"), item("UserInputState", "End"));

        frame(&mut dm, &mut feed, true, |dm, feed| feed.key(dm, "A", true).unwrap());
        let second = delivered(&mut dm)[0].1;
        assert_ne!(second, first, "a new press, a new object");
    }

    #[test]
    fn a_tap_within_one_frame_ends_in_the_next() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputBegan", 1);
        connect(&mut dm, uis, "InputEnded", 2);
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "Space", true).unwrap();
            feed.key(dm, "Space", false).unwrap();
            feed.mouse_button(dm, MouseButton::Left, true).unwrap();
            feed.mouse_button(dm, MouseButton::Left, false).unwrap();
        });
        let began = delivered(&mut dm);
        assert_eq!(began.iter().map(|d| d.0).collect::<Vec<_>>(), [1, 1]);
        for &(_, id) in &began {
            assert_eq!(field(&dm, id, "UserInputState"), item("UserInputState", "Begin"));
        }
        assert!(is_key_down(&mut dm, uis, "Space"), "down for the frame it began in");
        assert_eq!(feed.waiting(), 2);

        frame(&mut dm, &mut feed, true, |_, _| {});
        let ended = delivered(&mut dm);
        assert_eq!(ended, [(2, began[0].1), (2, began[1].1)], "the same objects end");
        for &(_, id) in &ended {
            assert_eq!(field(&dm, id, "UserInputState"), item("UserInputState", "End"));
        }
        assert!(!is_key_down(&mut dm, uis, "Space"));
        assert_eq!(feed.waiting(), 0);

        // A press and release every frame keeps one release waiting, not a growing queue.
        for _ in 0..10 {
            frame(&mut dm, &mut feed, true, |dm, feed| {
                feed.key(dm, "Q", true).unwrap();
                feed.key(dm, "Q", false).unwrap();
            });
            assert_eq!(feed.waiting(), 1);
        }
    }

    #[test]
    fn events_fire_only_while_running() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputBegan", 1);
        let instances = dm.len();
        frame(&mut dm, &mut feed, false, |dm, feed| feed.key(dm, "B", true).unwrap());
        assert!(drain(&mut dm).is_empty());
        assert_eq!(dm.len(), instances, "no object for a stopped game");
        assert!(is_key_down(&mut dm, uis, "B"), "the state is kept");
        frame(&mut dm, &mut feed, false, |dm, feed| feed.key(dm, "B", false).unwrap());
        assert!(!is_key_down(&mut dm, uis, "B"));
    }

    #[test]
    fn ten_thousand_presses_leave_no_object_behind() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputBegan", 1);
        connect(&mut dm, uis, "InputEnded", 2);
        frame(&mut dm, &mut feed, true, |_, _| {});
        let instances = dm.len();
        for _ in 0..10_000 {
            frame(&mut dm, &mut feed, true, |dm, feed| {
                feed.key(dm, "E", true).unwrap();
                feed.key(dm, "E", false).unwrap();
            });
            drain(&mut dm);
            assert_eq!(dm.len(), instances + 1, "only the press in progress");
            assert!(dm.snapshot_count() <= 1, "an ended object stays readable for one drain");
        }
        // The last release, then one more drain for its grave to go.
        for _ in 0..2 {
            frame(&mut dm, &mut feed, true, |_, _| {});
            drain(&mut dm);
        }
        assert_eq!(dm.len(), instances);
        assert_eq!(dm.snapshot_count(), 0, "nothing holds the ended objects");
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
    fn a_placed_cursor_moves_the_location_but_is_not_movement() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputChanged", 1);
        assert_eq!(feed.cursor_location(), None);
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.cursor_moved(dm, 10.0, 20.0).unwrap();
        });
        drain(&mut dm);
        frame(&mut dm, &mut feed, true, |_, feed| feed.place_cursor(400.0, 300.0));
        assert!(delivered(&mut dm).is_empty(), "no InputChanged for the engine's own move");
        assert_eq!(feed.cursor_location(), Some((400.0, 300.0)));
        assert_eq!(
            call(&mut dm, uis, "GetMouseLocation", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(400.0, 300.0)))
        );
        assert_eq!(
            call(&mut dm, uis, "GetMouseDelta", vec![]),
            Ok(ScriptValue::Vector2(Vector2::ZERO))
        );
        // Raw motion in the same frame as a placement is the frame's delta, and the location stays put.
        frame(&mut dm, &mut feed, true, |_, feed| {
            feed.mouse_motion(4.0, -2.0);
            feed.place_cursor(400.0, 300.0);
        });
        assert_eq!(delivered(&mut dm).len(), 1);
        assert_eq!(
            call(&mut dm, uis, "GetMouseDelta", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(4.0, -2.0)))
        );
        assert_eq!(
            call(&mut dm, uis, "GetMouseLocation", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(400.0, 300.0)))
        );
        // A later cursor move counts from the placed point.
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.cursor_moved(dm, 410.0, 300.0).unwrap();
        });
        assert_eq!(
            call(&mut dm, uis, "GetMouseDelta", vec![]),
            Ok(ScriptValue::Vector2(Vector2::new(10.0, 0.0)))
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
        let got = delivered(&mut dm);
        assert_eq!(got.len(), 2, "movement, then the wheel");
        let movement = got[0].1;
        assert_eq!(field(&dm, movement, "UserInputType"), item("UserInputType", "MouseMovement"));
        assert_eq!(field(&dm, movement, "UserInputState"), item("UserInputState", "Change"));
        assert_eq!(field(&dm, movement, "Delta"), v3(2.0, 2.0, 0.0));
        assert_eq!(field(&dm, movement, "Position"), v3(1.0, 2.0, 0.0));
        let wheel = got[1].1;
        assert_eq!(field(&dm, wheel, "Position"), v3(1.0, 2.0, 1.0));
        frame(&mut dm, &mut feed, true, |_, feed| feed.mouse_motion(1.0, 0.0));
        assert_eq!(delivered(&mut dm), [(1, movement)], "the movement object is reused");
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
        assert_eq!(listeners, [1, 1, 2], "two releases (objects made for the new listener), then focus");
    }

    #[test]
    fn a_finger_is_one_object_from_touch_to_release() {
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
        let got = delivered(&mut dm);
        assert_eq!(got.iter().map(|d| d.0).collect::<Vec<_>>(), [0, 1]);
        let finger = got[0].1;
        assert_eq!(got[1].1, finger);
        assert_eq!(field(&dm, finger, "Delta"), v3(3.0, 2.0, 0.0));
        assert_eq!(field(&dm, finger, "Position"), v3(8.0, 7.0, 0.0));
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 9, TouchPhase::Ended, 8.0, 7.0).unwrap();
        });
        assert_eq!(delivered(&mut dm), [(2, finger)]);
        assert_eq!(field(&dm, finger, "UserInputState"), item("UserInputState", "End"));
        assert!(!dm.contains(finger));
        assert_eq!(field(&dm, uis, "TouchEnabled"), Value::Bool(true));

        // A touch and release within one frame: InputBegan sees Begin, the end waits.
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.touch(dm, 10, TouchPhase::Began, 1.0, 1.0).unwrap();
            feed.touch(dm, 10, TouchPhase::Cancelled, 1.0, 1.0).unwrap();
        });
        let began = delivered(&mut dm);
        assert_eq!(field(&dm, began[0].1, "UserInputState"), item("UserInputState", "Begin"));
        frame(&mut dm, &mut feed, true, |_, _| {});
        let ended = delivered(&mut dm);
        assert_eq!(ended, [(2, began[0].1)]);
        assert_eq!(field(&dm, ended[0].1, "UserInputState"), item("UserInputState", "Cancel"));
        assert_eq!(feed.object_count(), 0);
    }

    #[test]
    fn a_destroyed_input_object_is_forgotten() {
        let mut dm = game();
        let mut feed = feed(&mut dm);
        let uis = feed.service();
        connect(&mut dm, uis, "InputBegan", 1);
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "Q", true).unwrap();
            feed.key(dm, "E", true).unwrap();
        });
        let q = keys_pressed(&mut dm, uis)
            .into_iter()
            .find(|&id| field(&dm, id, "KeyCode") == item("KeyCode", "Q"))
            .unwrap();
        call(&mut dm, q, "Destroy", vec![]).unwrap();
        assert!(is_key_down(&mut dm, uis, "Q"), "held keys are bits, not objects");
        frame(&mut dm, &mut feed, true, |_, _| {});
        assert_eq!(feed.object_count(), 1);
        assert_eq!(keys_pressed(&mut dm, uis).len(), 2, "Q gets a new object");
        frame(&mut dm, &mut feed, true, |dm, feed| {
            feed.key(dm, "Q", false).unwrap();
            feed.key(dm, "E", false).unwrap();
        });
        assert!(keys_pressed(&mut dm, uis).is_empty());
        assert_eq!(feed.object_count(), 0);
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
