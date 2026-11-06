//! What a binding is (ADR 0009, Keybinds): a key, a mouse button, a drag or the wheel, with modifiers, in one step or
//! two.
//!
//! Bindings are stored as text, the same on every platform:
//!
//! | Text | Means |
//! |---|---|
//! | `Primary+S` | Cmd+S on macOS, Ctrl+S elsewhere; the **logical** S (the key labelled S in the user's layout) |
//! | `[W]` | the **physical** key where W is on a US keyboard (Z on AZERTY), for movement |
//! | `Primary+K, S` | a two-step sequence: Primary+K, then S |
//! | `MouseRight`, `DragRight`, `Alt+DragLeft` | a click, or a button held and dragged (`Left`, `Right`, `Middle`, `Back`, `Forward`) |
//! | `Wheel`, `Shift+Wheel` | the mouse wheel |
//!
//! Modifiers are `Primary`, `Ctrl`, `Alt` (`Option`), `Shift` and `Super` (`Cmd`, `Win`). Keys use egui's names
//! (`Comma`, `F2`, `Delete`, ...); their symbols (`,`) are read too.

use std::collections::HashMap;
use std::fmt;

use egui::{Key, Modifiers, PointerButton};

/// Which platform's modifiers a keymap resolves "Primary" to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS: Primary is Cmd.
    Mac,
    /// Windows and Linux: Primary is Ctrl.
    Other,
}

impl Platform {
    /// The platform Studio runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Other
        }
    }
}

/// Held modifier keys. As written in a binding, `primary` is the platform's command key; [`Mods::canonical`] resolves
/// it, so `Primary+S` and `Ctrl+S` are the same binding on Windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    /// Cmd on macOS, Ctrl elsewhere.
    pub primary: bool,
    /// Control, also on macOS.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt (Option on macOS).
    pub alt: bool,
    /// The system key: Cmd on macOS, the Windows or Super key elsewhere (which egui does not report there, so a
    /// `Super` binding only fires on macOS).
    pub super_key: bool,
}

impl Mods {
    /// No modifiers.
    pub const NONE: Mods = Mods {
        primary: false,
        ctrl: false,
        shift: false,
        alt: false,
        super_key: false,
    };

    /// The same keys with Primary resolved for `platform`: on macOS Super folds into Primary (both are Cmd), elsewhere
    /// Ctrl does (both are Ctrl).
    pub fn canonical(self, platform: Platform) -> Mods {
        match platform {
            Platform::Mac => Mods {
                primary: self.primary || self.super_key,
                super_key: false,
                ..self
            },
            Platform::Other => Mods {
                primary: self.primary || self.ctrl,
                ctrl: false,
                ..self
            },
        }
    }

    /// The canonical modifiers of an egui event on `platform`.
    pub fn from_egui(modifiers: Modifiers, platform: Platform) -> Mods {
        match platform {
            Platform::Mac => Mods {
                primary: modifiers.mac_cmd || modifiers.command,
                ctrl: modifiers.ctrl,
                shift: modifiers.shift,
                alt: modifiers.alt,
                super_key: false,
            },
            Platform::Other => Mods {
                primary: modifiers.ctrl || modifiers.command,
                ctrl: false,
                shift: modifiers.shift,
                alt: modifiers.alt,
                super_key: false,
            },
        }
    }
}

/// Whether a key binding follows the key's label or its place on the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// The key labelled so in the user's layout: command shortcuts, so Primary+S is the S the user sees.
    Logical,
    /// The key at that place on a US keyboard: movement, so WASD stays under the left hand on AZERTY.
    Physical,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The primary (usually left) button.
    Left,
    /// The secondary (usually right) button.
    Right,
    /// The middle button (the wheel pressed).
    Middle,
    /// The first extra button: back.
    Back,
    /// The second extra button: forward.
    Forward,
}

impl MouseButton {
    /// Every button.
    pub const ALL: [MouseButton; 5] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ];

    /// The button of an egui event.
    pub fn from_egui(button: PointerButton) -> Self {
        match button {
            PointerButton::Primary => MouseButton::Left,
            PointerButton::Secondary => MouseButton::Right,
            PointerButton::Middle => MouseButton::Middle,
            PointerButton::Extra1 => MouseButton::Back,
            PointerButton::Extra2 => MouseButton::Forward,
        }
    }

    /// Its name in binding text.
    pub fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "Left",
            MouseButton::Right => "Right",
            MouseButton::Middle => "Middle",
            MouseButton::Back => "Back",
            MouseButton::Forward => "Forward",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|button| button.name().eq_ignore_ascii_case(name))
    }
}

/// What is pressed, besides the modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// A key, by its label or its place.
    Key {
        /// The key: for a physical binding, its name on a US keyboard.
        key: Key,
        /// Label or place.
        kind: KeyKind,
    },
    /// A mouse button pressed.
    Mouse(MouseButton),
    /// A mouse button held while the pointer moves; the command is held until the button is released.
    Drag(MouseButton),
    /// The mouse wheel turned; the command gets how far.
    Wheel,
}

impl Trigger {
    /// A logical key.
    pub const fn logical(key: Key) -> Self {
        Trigger::Key {
            key,
            kind: KeyKind::Logical,
        }
    }

    /// A physical key.
    pub const fn physical(key: Key) -> Self {
        Trigger::Key {
            key,
            kind: KeyKind::Physical,
        }
    }

    /// What this trigger occupies, for conflicts: a physical key is the logical key it gives in `layout` (the user's
    /// layout as seen so far; US when unknown), and a drag uses its button.
    pub fn place(self, layout: &HashMap<Key, Key>) -> Place {
        match self {
            Trigger::Key {
                key,
                kind: KeyKind::Logical,
            } => Place::Key(key),
            Trigger::Key {
                key,
                kind: KeyKind::Physical,
            } => Place::Key(layout.get(&key).copied().unwrap_or(key)),
            Trigger::Mouse(button) | Trigger::Drag(button) => Place::Button(button),
            Trigger::Wheel => Place::Wheel,
        }
    }
}

/// The input a trigger occupies (see [`Trigger::place`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// A key, as labelled.
    Key(Key),
    /// A mouse button, clicked or dragged.
    Button(MouseButton),
    /// The wheel.
    Wheel,
}

/// One step of a binding: modifiers and a trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The modifiers held, as written.
    pub mods: Mods,
    /// What is pressed.
    pub trigger: Trigger,
}

impl Chord {
    /// `trigger` with `mods`.
    pub const fn new(mods: Mods, trigger: Trigger) -> Self {
        Self { mods, trigger }
    }

    /// The same chord with its modifiers resolved for `platform`: two chords that are the same keys there are equal.
    pub fn canonical(self, platform: Platform) -> Self {
        Self {
            mods: self.mods.canonical(platform),
            trigger: self.trigger,
        }
    }

    /// The modifiers and place it occupies on `platform` in `layout`.
    pub fn place(self, platform: Platform, layout: &HashMap<Key, Key>) -> (Mods, Place) {
        (self.mods.canonical(platform), self.trigger.place(layout))
    }

    /// The chord as people read it on `platform`, physical keys shown as `layout` labels them.
    pub fn label(self, platform: Platform, layout: &HashMap<Key, Key>) -> String {
        let m = self.mods.canonical(platform);
        let mut parts: Vec<&str> = Vec::with_capacity(5);
        match platform {
            // Apple's order: Control, Option, Shift, Command.
            Platform::Mac => {
                for (on, name) in [
                    (m.ctrl, "Ctrl"),
                    (m.alt, "Option"),
                    (m.shift, "Shift"),
                    (m.primary, "Cmd"),
                ] {
                    if on {
                        parts.push(name);
                    }
                }
            }
            Platform::Other => {
                for (on, name) in [
                    (m.primary, "Ctrl"),
                    (m.super_key, "Super"),
                    (m.alt, "Alt"),
                    (m.shift, "Shift"),
                ] {
                    if on {
                        parts.push(name);
                    }
                }
            }
        }
        let trigger = match self.trigger {
            Trigger::Key {
                key,
                kind: KeyKind::Logical,
            } => key_label(key).to_owned(),
            Trigger::Key {
                key,
                kind: KeyKind::Physical,
            } => key_label(layout.get(&key).copied().unwrap_or(key)).to_owned(),
            Trigger::Mouse(button) => format!("{} click", button.name()),
            Trigger::Drag(button) => format!("{} drag", button.name()),
            Trigger::Wheel => "Wheel".to_owned(),
        };
        parts.push(&trigger);
        parts.join("+")
    }
}

/// How a key is labelled: its symbol where it has one.
fn key_label(key: Key) -> &'static str {
    key.symbol_or_name()
}

impl fmt::Display for Chord {
    /// The chord as stored: `Primary+Shift+S`, `[W]`, `Alt+DragLeft`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.mods;
        for (on, name) in [
            (m.primary, "Primary"),
            (m.ctrl, "Ctrl"),
            (m.super_key, "Super"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.trigger {
            Trigger::Key {
                key,
                kind: KeyKind::Logical,
            } => f.write_str(key.name()),
            Trigger::Key {
                key,
                kind: KeyKind::Physical,
            } => write!(f, "[{}]", key.name()),
            Trigger::Mouse(button) => write!(f, "Mouse{}", button.name()),
            Trigger::Drag(button) => write!(f, "Drag{}", button.name()),
            Trigger::Wheel => f.write_str("Wheel"),
        }
    }
}

/// Why a binding's text could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` is not a binding: {reason}")]
pub struct BindingError {
    /// The text.
    pub text: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

/// A binding: one chord, or a two-step sequence of key chords (`Primary+K, S`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    /// The first (or only) step.
    pub first: Chord,
    /// The second step of a sequence: always a key.
    pub second: Option<Chord>,
}

impl Binding {
    /// A one-step binding.
    pub const fn single(chord: Chord) -> Self {
        Self {
            first: chord,
            second: None,
        }
    }

    /// A two-step sequence.
    pub const fn sequence(first: Chord, second: Chord) -> Self {
        Self {
            first,
            second: Some(second),
        }
    }

    /// Reads a binding's text (see the module's documentation).
    pub fn parse(text: &str) -> Result<Self, BindingError> {
        let error = |reason| BindingError {
            text: text.to_owned(),
            reason,
        };
        let mut steps = text.split(", ");
        let first = parse_chord(steps.next().unwrap_or_default()).map_err(error)?;
        let second = steps.next().map(parse_chord).transpose().map_err(error)?;
        if steps.next().is_some() {
            return Err(error("a sequence has at most two steps"));
        }
        if let Some(second) = second
            && !matches!(second.trigger, Trigger::Key { .. })
        {
            return Err(error("the second step of a sequence must be a key"));
        }
        if second.is_some() && !matches!(first.trigger, Trigger::Key { .. }) {
            return Err(error("only keys can start a sequence"));
        }
        Ok(Self { first, second })
    }

    /// The binding as people read it on `platform`.
    pub fn label(&self, platform: Platform, layout: &HashMap<Key, Key>) -> String {
        match self.second {
            Some(second) => format!(
                "{}, {}",
                self.first.label(platform, layout),
                second.label(platform, layout)
            ),
            None => self.first.label(platform, layout),
        }
    }

    /// Whether this binding and `other` fire on the same input on `platform`: the same chord, or one is the first step
    /// of the other's sequence (pressing it would be ambiguous).
    pub fn overlaps(&self, other: &Binding, platform: Platform, layout: &HashMap<Key, Key>) -> bool {
        if self.first.place(platform, layout) != other.first.place(platform, layout) {
            return false;
        }
        match (self.second, other.second) {
            (Some(a), Some(b)) => a.place(platform, layout) == b.place(platform, layout),
            _ => true,
        }
    }
}

impl fmt::Display for Binding {
    /// The binding as stored.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.first)?;
        if let Some(second) = self.second {
            write!(f, ", {second}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Binding {
    type Err = BindingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Binding::parse(text)
    }
}

fn parse_chord(text: &str) -> Result<Chord, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("it is empty");
    }
    // A trailing "+" after a "+" is the plus key itself: `Primary++`.
    let (body, plus) = match text.strip_suffix("++") {
        Some(body) => (body, true),
        None if text == "+" => ("", true),
        None => (text, false),
    };
    let mut tokens: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('+').map(str::trim).collect()
    };
    let last = if plus {
        "+"
    } else {
        tokens.pop().ok_or("it has no key")?
    };
    let mut mods = Mods::NONE;
    for token in tokens {
        let flag = match token.to_ascii_lowercase().as_str() {
            "primary" => &mut mods.primary,
            "ctrl" | "control" => &mut mods.ctrl,
            "shift" => &mut mods.shift,
            "alt" | "option" => &mut mods.alt,
            "super" | "cmd" | "command" | "win" | "meta" => &mut mods.super_key,
            _ => return Err("it has an unknown modifier"),
        };
        *flag = true;
    }
    Ok(Chord::new(mods, parse_trigger(last)?))
}

fn parse_trigger(token: &str) -> Result<Trigger, &'static str> {
    if let Some(inner) = token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return Key::from_name(inner.trim())
            .map(Trigger::physical)
            .ok_or("it has an unknown key");
    }
    if token.eq_ignore_ascii_case("wheel") {
        return Ok(Trigger::Wheel);
    }
    let lower = token.to_ascii_lowercase();
    if let Some(button) = lower.strip_prefix("mouse").and_then(MouseButton::parse) {
        return Ok(Trigger::Mouse(button));
    }
    if let Some(button) = lower.strip_prefix("drag").and_then(MouseButton::parse) {
        return Ok(Trigger::Drag(button));
    }
    Key::from_name(token)
        .or_else(|| {
            // Letters in either case.
            let mut chars = token.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() => Key::from_name(&c.to_ascii_uppercase().to_string()),
                _ => None,
            }
        })
        .map(Trigger::logical)
        .ok_or("it has an unknown key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Binding {
        Binding::parse(text).unwrap()
    }

    #[test]
    fn bindings_read_and_write_their_text() {
        for text in [
            "Primary+S",
            "Primary+Shift+S",
            "[W]",
            "Shift+[Q]",
            "Primary+K, S",
            "MouseRight",
            "DragRight",
            "Alt+DragLeft",
            "Wheel",
            "Shift+Wheel",
            "Primary+Comma",
            "Ctrl+Super+Alt+F2",
            "Primary+Plus",
        ] {
            assert_eq!(parse(text).to_string(), text);
        }
        // Symbols, other spellings and letter case are read too, and written back by name.
        assert_eq!(parse("primary+,").to_string(), "Primary+Comma");
        assert_eq!(parse("cmd+option+s").to_string(), "Super+Alt+S");
        assert_eq!(parse("Primary++").to_string(), "Primary+Plus");
        assert_eq!(
            parse("Primary+K, S"),
            Binding::sequence(
                Chord::new(
                    Mods {
                        primary: true,
                        ..Mods::NONE
                    },
                    Trigger::logical(Key::K)
                ),
                Chord::new(Mods::NONE, Trigger::logical(Key::S)),
            )
        );
    }

    #[test]
    fn every_key_name_reads_back() {
        for &key in Key::ALL {
            let binding = Binding::single(Chord::new(Mods::NONE, Trigger::logical(key)));
            assert_eq!(parse(&binding.to_string()), binding, "{key:?}");
            let physical = Binding::single(Chord::new(Mods::NONE, Trigger::physical(key)));
            assert_eq!(parse(&physical.to_string()), physical, "{key:?}");
        }
    }

    #[test]
    fn bad_text_is_refused() {
        for text in [
            "",
            "Primary+",
            "Hyper+S",
            "Primary+Banana",
            "[Nope]",
            "A, B, C",
            "Wheel, S",
            "Primary+K, DragLeft",
        ] {
            assert!(Binding::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn primary_is_cmd_on_macos_and_ctrl_elsewhere() {
        let no_layout = HashMap::new();
        let save = parse("Primary+Shift+S");
        assert_eq!(save.label(Platform::Other, &no_layout), "Ctrl+Shift+S");
        assert_eq!(save.label(Platform::Mac, &no_layout), "Shift+Cmd+S");
        // Ctrl+S is Primary+S on Windows, not on macOS.
        let ctrl = parse("Ctrl+S");
        let primary = parse("Primary+S");
        assert!(ctrl.overlaps(&primary, Platform::Other, &no_layout));
        assert!(!ctrl.overlaps(&primary, Platform::Mac, &no_layout));
        assert!(parse("Super+S").overlaps(&primary, Platform::Mac, &no_layout));
        assert_eq!(parse("Primary+Comma").label(Platform::Other, &no_layout), "Ctrl+,");
        assert_eq!(
            parse("Alt+DragLeft").label(Platform::Mac, &no_layout),
            "Option+Left drag"
        );
    }

    #[test]
    fn a_physical_key_is_labelled_as_the_layout_has_it() {
        let mut azerty = HashMap::new();
        azerty.insert(Key::W, Key::Z);
        let forward = parse("[W]");
        assert_eq!(forward.label(Platform::Other, &HashMap::new()), "W");
        assert_eq!(forward.label(Platform::Other, &azerty), "Z");
        // On AZERTY the physical W is the logical Z.
        assert!(forward.overlaps(&parse("Z"), Platform::Other, &azerty));
        assert!(!forward.overlaps(&parse("W"), Platform::Other, &azerty));
    }

    #[test]
    fn a_sequence_overlaps_its_first_step() {
        let no_layout = HashMap::new();
        let sequence = parse("Primary+K, S");
        assert!(sequence.overlaps(&parse("Primary+K"), Platform::Other, &no_layout));
        assert!(sequence.overlaps(&parse("Primary+K, S"), Platform::Other, &no_layout));
        assert!(!sequence.overlaps(&parse("Primary+K, D"), Platform::Other, &no_layout));
        assert!(parse("DragRight").overlaps(&parse("MouseRight"), Platform::Other, &no_layout));
        assert!(!parse("Wheel").overlaps(&parse("Shift+Wheel"), Platform::Other, &no_layout));
    }
}
