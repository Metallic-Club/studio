//! Keyboard, mouse and touch input as plain engine types.
//!
//! Nothing here names a backend type: the winit backend (and later the console crates) translate their own events
//! into these, so the engine and scripts see the same input on every platform.

/// Declares a fieldless enum together with `ALL`, the list of every variant, so a mapping can be tested
/// exhaustively against it.
macro_rules! listed_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
        }
    };
}

/// Declares the physical key enum. Each key is named after its W3C UI Events `code` value (as winit's are), and
/// the doc comment is generated from that name.
macro_rules! key_codes {
    ($($key:ident),+ $(,)?) => {
        listed_enum! {
            /// A physical key, named by its position on a US keyboard (W3C UI Events `code` values): `KeyW` is the
            /// key where W sits on QWERTY, whatever the layout prints on it. Use this for game controls; use
            /// [`Key`] or [`KeyEvent::text`] for text.
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            #[non_exhaustive]
            pub enum KeyCode {
                /// A key this enum does not name yet, or one the OS could not identify.
                Unidentified,
                $(
                    #[doc = concat!("The `", stringify!($key), "` key.")]
                    $key
                ),+
            }
        }
    };
}

/// Calls `$callback!` with every [`KeyCode`] name (except `Unidentified`). The one list both declares the enum and
/// generates each backend's mapping, so they cannot drift apart.
macro_rules! with_key_codes {
    ($callback:ident) => {
        $callback! {
            // Writing system keys.
            Backquote, Backslash, BracketLeft, BracketRight, Comma, Equal, IntlBackslash, IntlRo, IntlYen, Minus,
            Period, Quote, Semicolon, Slash,
            Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
            KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ,
            KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
            // Functional keys.
            AltLeft, AltRight, Backspace, CapsLock, ContextMenu, ControlLeft, ControlRight, Enter, SuperLeft,
            SuperRight, ShiftLeft, ShiftRight, Space, Tab, Convert, KanaMode, NonConvert,
            // Control pad and arrows.
            Delete, End, Help, Home, Insert, PageDown, PageUp, ArrowDown, ArrowLeft, ArrowRight, ArrowUp,
            // Numpad.
            NumLock, Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
            NumpadAdd, NumpadComma, NumpadDecimal, NumpadDivide, NumpadEnter, NumpadEqual, NumpadMultiply,
            NumpadSubtract,
            // Function section.
            Escape, PrintScreen, ScrollLock, Pause,
            F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22,
            F23, F24,
            // Media keys.
            BrowserBack, BrowserForward, MediaPlayPause, MediaStop, MediaTrackNext, MediaTrackPrevious,
            AudioVolumeDown, AudioVolumeMute, AudioVolumeUp,
        }
    };
}
pub(crate) use with_key_codes;

with_key_codes!(key_codes);

listed_enum! {
    /// A named key the layout produced: what the key means, as opposed to where it is ([`KeyCode`]). Only keys that
    /// do not produce text are named; text arrives as [`Key::Character`].
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum NamedKey {
        /// Enter or Return.
        Enter,
        /// Tab.
        Tab,
        /// The space bar.
        Space,
        /// Backspace.
        Backspace,
        /// Delete (forward delete).
        Delete,
        /// Escape.
        Escape,
        /// Insert.
        Insert,
        /// Home.
        Home,
        /// End.
        End,
        /// Page Up.
        PageUp,
        /// Page Down.
        PageDown,
        /// Up arrow.
        ArrowUp,
        /// Down arrow.
        ArrowDown,
        /// Left arrow.
        ArrowLeft,
        /// Right arrow.
        ArrowRight,
        /// Either Shift key.
        Shift,
        /// Either Control key.
        Control,
        /// Either Alt (Option) key.
        Alt,
        /// Either Super key (Windows, Command).
        Super,
        /// Caps Lock.
        CapsLock,
        /// F1.
        F1,
        /// F2.
        F2,
        /// F3.
        F3,
        /// F4.
        F4,
        /// F5.
        F5,
        /// F6.
        F6,
        /// F7.
        F7,
        /// F8.
        F8,
        /// F9.
        F9,
        /// F10.
        F10,
        /// F11.
        F11,
        /// F12.
        F12,
    }
}

/// What a key press means under the current keyboard layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    /// A key that produces text, with the text it produces (`"a"`, `"A"`, `"é"`).
    Character(String),
    /// A key that does not produce text.
    Named(NamedKey),
    /// A dead key (an accent that combines with the next key), with the accent when the OS reports it.
    Dead(Option<char>),
    /// A key the OS could not identify, or a named key [`NamedKey`] does not list yet.
    Unidentified,
}

/// Whether a key or button went down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonState {
    /// The key or button went down.
    Pressed,
    /// The key or button went up.
    Released,
}

impl ButtonState {
    /// Whether this is [`ButtonState::Pressed`].
    pub fn is_pressed(self) -> bool {
        self == ButtonState::Pressed
    }
}

/// One key going down, repeating or going up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Where the key is on the keyboard; use this for game controls.
    pub key_code: KeyCode,
    /// What the key means under the current layout.
    pub key: Key,
    /// The text the press produced, if any (none on release). Use this for text fields.
    pub text: Option<String>,
    /// Pressed or released.
    pub state: ButtonState,
    /// Whether this press is an OS key repeat of a key held down.
    pub repeat: bool,
}

/// The modifier keys held down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Either Shift key.
    pub shift: bool,
    /// Either Control key.
    pub control: bool,
    /// Either Alt (Option) key.
    pub alt: bool,
    /// Either Super key (Windows, Command).
    pub super_key: bool,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The primary (usually left) button.
    Left,
    /// The secondary (usually right) button.
    Right,
    /// The middle button or wheel click.
    Middle,
    /// The "back" side button.
    Back,
    /// The "forward" side button.
    Forward,
    /// Any other button, by the number the OS gives it.
    Other(u16),
}

/// How far a mouse wheel or touchpad scrolled. Positive `y` scrolls up (away from the user), positive `x` right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// Wheel notches (lines); what most mice report.
    Lines {
        /// Horizontal lines.
        x: f32,
        /// Vertical lines.
        y: f32,
    },
    /// Exact pixels; what touchpads and some high-resolution wheels report.
    Pixels {
        /// Horizontal pixels.
        x: f64,
        /// Vertical pixels.
        y: f64,
    },
}

/// The stage of one touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TouchPhase {
    /// A finger touched the screen.
    Started,
    /// The finger moved.
    Moved,
    /// The finger left the screen.
    Ended,
    /// The OS cancelled the touch (for example a system gesture took it over).
    Cancelled,
}

/// One touch on a touch screen (the mobile seam; desktop touch screens report it too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    /// Identifies the finger for the life of the touch, from [`TouchPhase::Started`] to `Ended` or `Cancelled`.
    pub id: u64,
    /// The stage of the touch.
    pub phase: TouchPhase,
    /// Where the finger is, in physical pixels from the window's top-left corner.
    pub position: Position,
    /// How hard the finger presses, from 0 to 1, on screens that measure it.
    pub force: Option<f64>,
}

/// A point in physical pixels, from the window's top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    /// Pixels to the right.
    pub x: f64,
    /// Pixels down.
    pub y: f64,
}

/// A size in pixels. Whether they are physical or logical (scaled by [`crate::Window::scale_factor`]) is stated
/// where the size is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Size {
    /// A size of `width` by `height` pixels.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}
