//! `Enum.KeyCode`: the keyboard keys scripts name, generated from one table that the engine also maps the
//! platform's keys from.
//!
//! # One table
//!
//! [`with_key_codes!`](crate::with_key_codes) lists every key once, as `Name = number => PlatformKey`: the item name
//! scripts use (Roblox's, such as `W`, `LeftShift`, `Return`), its number (Roblox's, so `Enum.KeyCode.W.Value` is
//! 119), and the `metallic_platform::KeyCode` variant it comes from (a W3C UI Events `code` name, such as `KeyW`).
//! This crate builds the enum from the first two columns; `metallic_engine` builds its platform mapping from the
//! first and third. This crate does not depend on the platform crate: the third column is only tokens to it, which
//! the engine's expansion resolves. A key added to the platform without a row here is caught by the engine's test
//! that every platform key maps to an item other than `Unknown`.
//!
//! Keys Roblox does not name (the international keys, F16 to F24, media keys) have Metallic names numbered from
//! 2000, clear of Roblox's numbers. Gamepad buttons (Roblox's `ButtonA`, `DPadUp`, ...) come with gamepad input.

use metallic_core::EnumDesc;

/// Calls `$callback!` with every keyboard key of `Enum.KeyCode` except `Unknown`, as `Name = number =>
/// PlatformKey` rows: the item name, its number, and the `metallic_platform::KeyCode` variant it is mapped from.
/// The one list both declares the enum (here) and generates the engine's platform mapping, so the two cannot drift
/// apart (see the module docs).
#[macro_export]
macro_rules! with_key_codes {
    ($callback:ident) => {
        $callback! {
            // Writing system keys.
            Backquote = 96 => Backquote,
            BackSlash = 92 => Backslash,
            LeftBracket = 91 => BracketLeft,
            RightBracket = 93 => BracketRight,
            Comma = 44 => Comma,
            Equals = 61 => Equal,
            Minus = 45 => Minus,
            Period = 46 => Period,
            Quote = 39 => Quote,
            Semicolon = 59 => Semicolon,
            Slash = 47 => Slash,
            Zero = 48 => Digit0,
            One = 49 => Digit1,
            Two = 50 => Digit2,
            Three = 51 => Digit3,
            Four = 52 => Digit4,
            Five = 53 => Digit5,
            Six = 54 => Digit6,
            Seven = 55 => Digit7,
            Eight = 56 => Digit8,
            Nine = 57 => Digit9,
            A = 97 => KeyA,
            B = 98 => KeyB,
            C = 99 => KeyC,
            D = 100 => KeyD,
            E = 101 => KeyE,
            F = 102 => KeyF,
            G = 103 => KeyG,
            H = 104 => KeyH,
            I = 105 => KeyI,
            J = 106 => KeyJ,
            K = 107 => KeyK,
            L = 108 => KeyL,
            M = 109 => KeyM,
            N = 110 => KeyN,
            O = 111 => KeyO,
            P = 112 => KeyP,
            Q = 113 => KeyQ,
            R = 114 => KeyR,
            S = 115 => KeyS,
            T = 116 => KeyT,
            U = 117 => KeyU,
            V = 118 => KeyV,
            W = 119 => KeyW,
            X = 120 => KeyX,
            Y = 121 => KeyY,
            Z = 122 => KeyZ,
            // Functional keys.
            LeftAlt = 308 => AltLeft,
            RightAlt = 307 => AltRight,
            Backspace = 8 => Backspace,
            CapsLock = 301 => CapsLock,
            Menu = 319 => ContextMenu,
            LeftControl = 306 => ControlLeft,
            RightControl = 305 => ControlRight,
            Return = 13 => Enter,
            LeftSuper = 311 => SuperLeft,
            RightSuper = 312 => SuperRight,
            LeftShift = 304 => ShiftLeft,
            RightShift = 303 => ShiftRight,
            Space = 32 => Space,
            Tab = 9 => Tab,
            // Control pad and arrows.
            Delete = 127 => Delete,
            End = 279 => End,
            Help = 315 => Help,
            Home = 278 => Home,
            Insert = 277 => Insert,
            PageDown = 281 => PageDown,
            PageUp = 280 => PageUp,
            Down = 274 => ArrowDown,
            Left = 276 => ArrowLeft,
            Right = 275 => ArrowRight,
            Up = 273 => ArrowUp,
            // Numpad.
            NumLock = 300 => NumLock,
            KeypadZero = 256 => Numpad0,
            KeypadOne = 257 => Numpad1,
            KeypadTwo = 258 => Numpad2,
            KeypadThree = 259 => Numpad3,
            KeypadFour = 260 => Numpad4,
            KeypadFive = 261 => Numpad5,
            KeypadSix = 262 => Numpad6,
            KeypadSeven = 263 => Numpad7,
            KeypadEight = 264 => Numpad8,
            KeypadNine = 265 => Numpad9,
            KeypadPeriod = 266 => NumpadDecimal,
            KeypadDivide = 267 => NumpadDivide,
            KeypadMultiply = 268 => NumpadMultiply,
            KeypadMinus = 269 => NumpadSubtract,
            KeypadPlus = 270 => NumpadAdd,
            KeypadEnter = 271 => NumpadEnter,
            KeypadEquals = 272 => NumpadEqual,
            // Function section.
            Escape = 27 => Escape,
            Print = 316 => PrintScreen,
            ScrollLock = 302 => ScrollLock,
            Pause = 19 => Pause,
            F1 = 282 => F1,
            F2 = 283 => F2,
            F3 = 284 => F3,
            F4 = 285 => F4,
            F5 = 286 => F5,
            F6 = 287 => F6,
            F7 = 288 => F7,
            F8 = 289 => F8,
            F9 = 290 => F9,
            F10 = 291 => F10,
            F11 = 292 => F11,
            F12 = 293 => F12,
            F13 = 294 => F13,
            F14 = 295 => F14,
            F15 = 296 => F15,
            // Keys Roblox does not name: Metallic's own, numbered from 2000.
            IntlBackslash = 2000 => IntlBackslash,
            IntlRo = 2001 => IntlRo,
            IntlYen = 2002 => IntlYen,
            Convert = 2003 => Convert,
            KanaMode = 2004 => KanaMode,
            NonConvert = 2005 => NonConvert,
            KeypadComma = 2006 => NumpadComma,
            F16 = 2016 => F16,
            F17 = 2017 => F17,
            F18 = 2018 => F18,
            F19 = 2019 => F19,
            F20 = 2020 => F20,
            F21 = 2021 => F21,
            F22 = 2022 => F22,
            F23 = 2023 => F23,
            F24 = 2024 => F24,
            BrowserBack = 2030 => BrowserBack,
            BrowserForward = 2031 => BrowserForward,
            MediaPlayPause = 2032 => MediaPlayPause,
            MediaStop = 2033 => MediaStop,
            MediaNextTrack = 2034 => MediaTrackNext,
            MediaPreviousTrack = 2035 => MediaTrackPrevious,
            VolumeDown = 2036 => AudioVolumeDown,
            VolumeMute = 2037 => AudioVolumeMute,
            VolumeUp = 2038 => AudioVolumeUp,
        }
    };
}

/// Builds the item and number lists of `Enum.KeyCode` from the table, with `Unknown` (0) first.
macro_rules! key_code_lists {
    ($($name:ident = $number:literal => $platform:ident),+ $(,)?) => {
        /// Every item name of `Enum.KeyCode`, in order.
        const ITEMS: &[&str] = &["Unknown", $(stringify!($name)),+];
        /// The number of each item of [`ITEMS`].
        const NUMBERS: &[i32] = &[0, $($number),+];
    };
}

with_key_codes!(key_code_lists);

/// The item of a key the platform could not identify, and of every input that is not a key.
pub const UNKNOWN: &str = "Unknown";

/// How many items `Enum.KeyCode` has, `Unknown` included.
pub const COUNT: usize = ITEMS.len();

/// How many words of 64 bits a set of keys takes: `UserInputService` keeps the held keys as bits (`HeldKeys1` to
/// `HeldKeys3`), by item index.
pub const WORDS: usize = 3;

const _: () = assert!(COUNT <= WORDS * 64, "the held-key bits need another word");

/// The index of the item `name` in `Enum.KeyCode` (the registry keeps the items in this order).
pub fn index(name: &str) -> Option<usize> {
    ITEMS.iter().position(|item| *item == name)
}

/// The name of the item at `index`.
pub fn name(index: usize) -> Option<&'static str> {
    ITEMS.get(index).copied()
}

/// The keyboard keys.
pub static ENUMS: &[EnumDesc] = &[EnumDesc::new("KeyCode", ITEMS).numbers(NUMBERS).describe(
    "A keyboard key, named by where it sits on a US keyboard: KeyCode.W is the key where W is on QWERTY, whatever \
     the layout prints on it. InputObject.KeyCode and UserInputService:IsKeyDown use it. Unknown is a key the \
     computer could not identify, and the KeyCode of every input that is not a key.",
)];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{ITEMS, NUMBERS};

    #[test]
    fn names_and_numbers_are_unique() {
        assert_eq!(ITEMS.len(), NUMBERS.len());
        assert_eq!(ITEMS.iter().collect::<HashSet<_>>().len(), ITEMS.len());
        assert_eq!(NUMBERS.iter().collect::<HashSet<_>>().len(), NUMBERS.len());
    }

    #[test]
    fn numbers_match_roblox() {
        let registry = crate::shared_registry().unwrap();
        let desc = registry.enum_desc("KeyCode").unwrap();
        for (name, number) in [("Unknown", 0), ("W", 119), ("Space", 32), ("LeftShift", 304), ("F1", 282)] {
            let index = desc.items.iter().position(|item| *item == name).unwrap();
            assert_eq!(desc.number(index), Some(number), "{name}");
        }
    }
}
