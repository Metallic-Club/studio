//! Raw input: devices, controls, events and the per-frame queue.
//!
//! Window libraries stay out of this crate: the Player turns winit and gilrs events into
//! [`RawEvent`]s, tests use a [`ScriptedSource`](super::ScriptedSource).
use std::collections::VecDeque;

/// Raw events kept per frame before the queue starts merging mouse moves.
pub const QUEUE_CAPACITY: usize = 1024;
/// Gamepads that can feed actions at the same time.
pub const MAX_GAMEPADS: usize = 4;
/// Number of device slots: keyboard and mouse, the gamepads, touch.
pub(crate) const DEVICE_SLOTS: usize = MAX_GAMEPADS + 2;

/// Where an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    /// Keyboard and mouse.
    KeyboardMouse,
    /// A gamepad; the slot is 0 to 3 (larger values count as 3).
    Gamepad(u8),
    /// The touch screen.
    Touch,
}

impl Device {
    pub(crate) fn slot(self) -> usize {
        match self {
            Device::KeyboardMouse => 0,
            Device::Gamepad(n) => 1 + (n as usize).min(MAX_GAMEPADS - 1),
            Device::Touch => 1 + MAX_GAMEPADS,
        }
    }

    /// The kind of device, for the last-used-device rule.
    pub fn kind(self) -> DeviceKind {
        match self {
            Device::KeyboardMouse => DeviceKind::KeyboardMouse,
            Device::Gamepad(_) => DeviceKind::Gamepad,
            Device::Touch => DeviceKind::Touch,
        }
    }
}

/// The three kinds of device that can be "last used" (`UserInputService.PreferredInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// Keyboard and mouse.
    KeyboardMouse,
    /// Any gamepad.
    Gamepad,
    /// The touch screen.
    Touch,
}

impl DeviceKind {
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// How a control reports its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// A key or button: `value` 0 to 1 (triggers are analog).
    Button,
    /// A two-axis level (sticks, touch pads): `pos` is the vector, each axis -1 to 1.
    Stick,
    /// A two-axis amount of movement since the last frame (mouse delta): events add up.
    Delta,
    /// A one-axis amount since the last frame (wheel notches): events add up.
    Wheel,
    /// A position in pixels (mouse, touch point): the latest event wins.
    Position,
}

macro_rules! controls {
    ($($name:ident),* $(,)?) => {
        /// One physical control, named like Roblox's `Enum.KeyCode` plus the Metallic mouse and
        /// touch controls. Keys are physical positions, so `W` is the same key on any layout.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Control {
            $(
                #[doc = concat!("The `", stringify!($name), "` control.")]
                $name
            ),*
        }

        impl Control {
            /// Every control, in declaration order.
            pub const ALL: &'static [Control] = &[$(Control::$name),*];

            /// The name used in `InputBinding.KeyCode` and in input scripts.
            pub fn name(self) -> &'static str {
                match self { $(Control::$name => stringify!($name)),* }
            }
        }
    };
}

controls! {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Zero, One, Two, Three, Four, Five, Six, Seven, Eight, Nine,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Space, Return, Tab, Backspace, Escape,
    LeftShift, RightShift, LeftControl, RightControl, LeftAlt, RightAlt,
    Up, Down, Left, Right, Home, End, PageUp, PageDown, Insert, Delete,
    Comma, Period, Slash, Semicolon, Quote, LeftBracket, RightBracket, BackSlash, Minus, Equals,
    Backquote,
    MouseButton1, MouseButton2, MouseButton3, MouseWheel, MouseDelta, MousePosition,
    ButtonA, ButtonB, ButtonX, ButtonY, ButtonL1, ButtonR1, ButtonL2, ButtonR2, ButtonL3, ButtonR3,
    ButtonSelect, ButtonStart, DPadUp, DPadDown, DPadLeft, DPadRight, Thumbstick1, Thumbstick2,
    TouchStick, TouchLook, TouchButton,
    TouchButton1, TouchButton2, TouchButton3, TouchButton4, TouchButton5, TouchButton6,
    TouchButton7,
}

/// Number of per-action touch button controls (`TouchButton1` to `TouchButton7`): the automatic
/// touch controls give each on-screen button its own control (spec 4.3, "TouchButton(action)"):
/// six buttons around the primary and the menu button.
pub const TOUCH_BUTTON_SLOTS: usize = 7;

/// Number of controls.
pub const CONTROL_COUNT: usize = Control::ALL.len();

impl Control {
    /// The control with this name (case sensitive); `None` for "None" and unknown names.
    pub fn from_name(name: &str) -> Option<Control> {
        Control::ALL.iter().copied().find(|c| c.name() == name)
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    /// The control of on-screen touch button `slot` (0 is `TouchButton1`); `None` past
    /// [`TOUCH_BUTTON_SLOTS`].
    pub fn touch_button(slot: usize) -> Option<Control> {
        (slot < TOUCH_BUTTON_SLOTS).then(|| Control::ALL[Control::TouchButton1.index() + slot])
    }

    /// The slot of a per-action touch button control (`TouchButton1` is 0).
    pub fn touch_button_slot(self) -> Option<usize> {
        let first = Control::TouchButton1.index();
        (self.index() >= first && self.index() < first + TOUCH_BUTTON_SLOTS)
            .then(|| self.index() - first)
    }

    /// How this control reports its value. `TouchLook` is the look pad: dp moved since the last
    /// frame, like `MouseDelta`.
    pub fn kind(self) -> ControlKind {
        match self {
            Control::Thumbstick1 | Control::Thumbstick2 | Control::TouchStick => ControlKind::Stick,
            Control::MouseDelta | Control::TouchLook => ControlKind::Delta,
            Control::MouseWheel => ControlKind::Wheel,
            Control::MousePosition => ControlKind::Position,
            _ => ControlKind::Button,
        }
    }

    /// The dead zone a binding uses when it does not set one (spec 3.4).
    pub fn default_deadzone(self) -> f32 {
        match self {
            Control::Thumbstick1 | Control::Thumbstick2 => 0.18,
            Control::ButtonL2 | Control::ButtonR2 => 0.05,
            _ => 0.0,
        }
    }

    /// True for the controls that point at the screen (the GUI may take them).
    pub fn is_pointer(self) -> bool {
        matches!(
            self,
            Control::MouseButton1
                | Control::MouseButton2
                | Control::MouseButton3
                | Control::MouseWheel
                | Control::MouseDelta
                | Control::MousePosition
                | Control::TouchStick
                | Control::TouchLook
                | Control::TouchButton
        ) || self.touch_button_slot().is_some()
    }

    /// True for the touch controls: the stick, the look pad and every on-screen button.
    pub fn is_touch(self) -> bool {
        matches!(
            self,
            Control::TouchStick | Control::TouchLook | Control::TouchButton
        ) || self.touch_button_slot().is_some()
    }

    /// True for the controls of a gamepad.
    pub fn is_gamepad(self) -> bool {
        self.index() >= Control::ButtonA.index() && self.index() <= Control::Thumbstick2.index()
    }
}

/// One thing that happened on one control.
///
/// Buttons and triggers use `value` (0 released, 1 pressed, analog in between). Two-axis
/// controls put the vector in `pos` and ignore `value`; the wheel puts its notches in `value`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawEvent {
    /// Where it came from.
    pub device: Device,
    /// Which control.
    pub control: Control,
    /// The value (see above).
    pub value: f32,
    /// The vector or position, for two-axis controls.
    pub pos: Option<[f32; 2]>,
    /// Monotonic seconds. Taps and holds are measured with this, not with frame counts.
    pub time: f64,
    /// A release that cancels the press: the action gets its `Released` edge but no
    /// `Triggered` (a finger that slid off a touch button, spec 7.5). Buttons only.
    pub cancel: bool,
}

impl RawEvent {
    /// A button or trigger change.
    pub fn button(device: Device, control: Control, value: f32, time: f64) -> Self {
        Self {
            device,
            control,
            value,
            pos: None,
            time,
            cancel: false,
        }
    }

    /// Releases a button without firing `Triggered` (spec 7.5: sliding off a touch button).
    pub fn cancel(device: Device, control: Control, time: f64) -> Self {
        Self {
            cancel: true,
            ..Self::button(device, control, 0.0, time)
        }
    }

    /// A two-axis change (stick level, mouse delta, pointer position).
    pub fn vector(device: Device, control: Control, pos: [f32; 2], time: f64) -> Self {
        Self {
            device,
            control,
            value: 0.0,
            pos: Some(pos),
            time,
            cancel: false,
        }
    }

    /// Wheel notches.
    pub fn wheel(device: Device, notches: f32, time: f64) -> Self {
        Self::button(device, Control::MouseWheel, notches, time)
    }

    fn is_edge(&self) -> bool {
        self.control.kind() == ControlKind::Button
    }
}

/// Something that produces [`RawEvent`]s once per frame: the Player's winit and gilrs sources,
/// or a [`ScriptedSource`](super::ScriptedSource) in tests.
pub trait RawSource {
    /// Appends the events that happened since the last call.
    fn poll(&mut self, out: &mut Vec<RawEvent>);
}

/// A bounded queue of raw events, filled between frames and drained by `evaluate`.
///
/// Button edges are never dropped. When the queue is full, mouse moves, wheel notches and stick
/// levels merge into the newest event of the same control, and a button edge takes the place of
/// the oldest event that is not an edge. Only when the queue holds nothing but edges does it
/// grow past [`QUEUE_CAPACITY`].
#[derive(Debug)]
pub struct InputQueue {
    events: VecDeque<RawEvent>,
}

impl Default for InputQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl InputQueue {
    /// An empty queue with room for [`QUEUE_CAPACITY`] events.
    pub fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(QUEUE_CAPACITY),
        }
    }

    /// Events waiting.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Adds an event, merging or dropping a mouse move when the queue is full.
    pub fn push(&mut self, e: RawEvent) {
        if self.events.len() < QUEUE_CAPACITY {
            self.events.push_back(e);
            return;
        }
        if e.is_edge() {
            // Make room by dropping the oldest event that is not a button edge.
            if let Some(i) = self.events.iter().position(|q| !q.is_edge()) {
                self.events.remove(i);
            }
            self.events.push_back(e);
            return;
        }
        let same = self
            .events
            .iter_mut()
            .rev()
            .find(|q| q.device == e.device && q.control == e.control);
        if let Some(q) = same {
            match e.control.kind() {
                ControlKind::Delta => {
                    let (a, b) = (q.pos.unwrap_or([0.0; 2]), e.pos.unwrap_or([0.0; 2]));
                    q.pos = Some([a[0] + b[0], a[1] + b[1]]);
                }
                ControlKind::Wheel => q.value += e.value,
                _ => {
                    q.pos = e.pos;
                    q.value = e.value;
                }
            }
            q.time = e.time;
        }
        // Nothing of this control is queued and the queue is full: the move is dropped.
    }

    pub(crate) fn pop(&mut self) -> Option<RawEvent> {
        self.events.pop_front()
    }
}
