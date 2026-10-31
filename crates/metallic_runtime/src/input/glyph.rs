//! Glyphs: how a binding looks on screen (docs/design/input-actions.md section 9).
//!
//! The art is Metallic's own and is described here as data ([`GlyphArt`]): keycaps, a mouse with
//! the pressed button filled, and a generic gamepad's parts (face buttons by position, bumpers,
//! triggers, sticks, d-pad, the two centre buttons). The Player paints it with vector shapes and
//! text in the tokens' graphite and silver; there are no logos, word marks, product names or
//! console colour schemes, and no art copied from any other engine, store or console maker.
//!
//! Face buttons come in two styles: [`GamepadStyle::Letters`] (A bottom, B right, X left, Y top,
//! in a plain font) and [`GamepadStyle::Positions`] (a diamond of four dots with the pressed one
//! filled, which is right on any pad). Spec 9.2's "Shapes" style is not shipped: whether it may
//! ship is an open owner question (spec section 17).
//!
//! [`control_glyph`] is a pure table and allocates nothing. [`slot_glyph`] builds the text and
//! image id once; the runtime caches the result per action and device
//! ([`InputRuntime::glyphs`](super::InputRuntime::glyphs)), so a lookup after the first call
//! allocates nothing.
use super::control::{Control, DeviceKind};
use super::overrides::Slot;

/// USB vendor ids whose pads label the face buttons with something other than A, B, X, Y in
/// the A-bottom positions; [`auto_gamepad_style`] gives them [`GamepadStyle::Positions`].
const POSITION_VENDORS: [u16; 2] = [0x054C, 0x057E];

/// Face-button art.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GamepadStyle {
    /// A, B, X, Y as letters in their positions (bottom, right, left, top).
    #[default]
    Letters,
    /// A diamond of four dots with the pressed position filled: no letters.
    Positions,
}

/// The style for a pad from the USB vendor id gilrs reports: Positions for the vendors whose
/// pads do not use the A-bottom letters, Letters otherwise (also when the id is unknown).
pub fn auto_gamepad_style(vendor: Option<u16>) -> GamepadStyle {
    match vendor {
        Some(v) if POSITION_VENDORS.contains(&v) => GamepadStyle::Positions,
        _ => GamepadStyle::Letters,
    }
}

/// A face-button position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacePos {
    /// Bottom (`ButtonA`).
    South,
    /// Right (`ButtonB`).
    East,
    /// Left (`ButtonX`).
    West,
    /// Top (`ButtonY`).
    North,
}

/// Left or right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Left.
    Left,
    /// Right.
    Right,
}

/// A direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    /// Up.
    Up,
    /// Down.
    Down,
    /// Left.
    Left,
    /// Right.
    Right,
}

/// A part of the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MousePart {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
    /// The wheel.
    Wheel,
    /// Movement.
    Move,
}

/// What to draw for one control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphArt {
    /// A keycap with a legend; `wide` for long legends (Space, Shift).
    Keycap {
        /// The legend.
        label: &'static str,
        /// A wider cap.
        wide: bool,
    },
    /// A keycap with an arrow.
    Arrow(Dir),
    /// A mouse with one part filled.
    Mouse(MousePart),
    /// A face button: the diamond with `pos` filled, and the letter in Letters style.
    Face {
        /// Position.
        pos: FacePos,
        /// The letter (Letters style), `None` in Positions style.
        letter: Option<&'static str>,
    },
    /// A shoulder bumper.
    Bumper(Side),
    /// An analog trigger.
    Trigger(Side),
    /// A stick; `press` for clicking it.
    Stick {
        /// Which stick.
        side: Side,
        /// Clicked rather than tilted.
        press: bool,
    },
    /// The d-pad with one arm filled (`None`: the whole pad).
    DPad(Option<Dir>),
    /// A small centre button: Select (left) or Start (right).
    Center(Side),
    /// A touch control (art belongs to the touch overlay, #252).
    Touch,
    /// No binding.
    Unbound,
}

/// The glyph of one control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlGlyph {
    /// The art.
    pub art: GlyphArt,
    /// Words for the control, for screen readers and text-only prompts ("Space", "A",
    /// "Bottom button", "Left stick").
    pub text: &'static str,
}

const fn cap(label: &'static str, text: &'static str) -> ControlGlyph {
    ControlGlyph {
        art: GlyphArt::Keycap { label, wide: false },
        text,
    }
}

const fn wide(label: &'static str, text: &'static str) -> ControlGlyph {
    ControlGlyph {
        art: GlyphArt::Keycap { label, wide: true },
        text,
    }
}

const fn art(art: GlyphArt, text: &'static str) -> ControlGlyph {
    ControlGlyph { art, text }
}

/// The glyph of `c` in `style`. A pure table: no allocation.
pub fn control_glyph(c: Control, style: GamepadStyle) -> ControlGlyph {
    use Control as C;
    use GlyphArt as G;
    let letters = style == GamepadStyle::Letters;
    let face = |pos, letter, words| {
        if letters {
            art(
                G::Face {
                    pos,
                    letter: Some(letter),
                },
                letter,
            )
        } else {
            art(G::Face { pos, letter: None }, words)
        }
    };
    match c {
        C::A => cap("A", "A"),
        C::B => cap("B", "B"),
        C::C => cap("C", "C"),
        C::D => cap("D", "D"),
        C::E => cap("E", "E"),
        C::F => cap("F", "F"),
        C::G => cap("G", "G"),
        C::H => cap("H", "H"),
        C::I => cap("I", "I"),
        C::J => cap("J", "J"),
        C::K => cap("K", "K"),
        C::L => cap("L", "L"),
        C::M => cap("M", "M"),
        C::N => cap("N", "N"),
        C::O => cap("O", "O"),
        C::P => cap("P", "P"),
        C::Q => cap("Q", "Q"),
        C::R => cap("R", "R"),
        C::S => cap("S", "S"),
        C::T => cap("T", "T"),
        C::U => cap("U", "U"),
        C::V => cap("V", "V"),
        C::W => cap("W", "W"),
        C::X => cap("X", "X"),
        C::Y => cap("Y", "Y"),
        C::Z => cap("Z", "Z"),
        C::Zero => cap("0", "0"),
        C::One => cap("1", "1"),
        C::Two => cap("2", "2"),
        C::Three => cap("3", "3"),
        C::Four => cap("4", "4"),
        C::Five => cap("5", "5"),
        C::Six => cap("6", "6"),
        C::Seven => cap("7", "7"),
        C::Eight => cap("8", "8"),
        C::Nine => cap("9", "9"),
        C::F1 => cap("F1", "F1"),
        C::F2 => cap("F2", "F2"),
        C::F3 => cap("F3", "F3"),
        C::F4 => cap("F4", "F4"),
        C::F5 => cap("F5", "F5"),
        C::F6 => cap("F6", "F6"),
        C::F7 => cap("F7", "F7"),
        C::F8 => cap("F8", "F8"),
        C::F9 => cap("F9", "F9"),
        C::F10 => cap("F10", "F10"),
        C::F11 => cap("F11", "F11"),
        C::F12 => cap("F12", "F12"),
        C::Space => wide("Space", "Space"),
        C::Return => wide("Enter", "Enter"),
        C::Tab => wide("Tab", "Tab"),
        C::Backspace => wide("Backspace", "Backspace"),
        C::Escape => cap("Esc", "Esc"),
        C::LeftShift => wide("L Shift", "Left Shift"),
        C::RightShift => wide("R Shift", "Right Shift"),
        C::LeftControl => wide("L Ctrl", "Left Ctrl"),
        C::RightControl => wide("R Ctrl", "Right Ctrl"),
        C::LeftAlt => wide("L Alt", "Left Alt"),
        C::RightAlt => wide("R Alt", "Right Alt"),
        C::Up => art(G::Arrow(Dir::Up), "Up Arrow"),
        C::Down => art(G::Arrow(Dir::Down), "Down Arrow"),
        C::Left => art(G::Arrow(Dir::Left), "Left Arrow"),
        C::Right => art(G::Arrow(Dir::Right), "Right Arrow"),
        C::Home => wide("Home", "Home"),
        C::End => cap("End", "End"),
        C::PageUp => wide("PgUp", "Page Up"),
        C::PageDown => wide("PgDn", "Page Down"),
        C::Insert => cap("Ins", "Insert"),
        C::Delete => cap("Del", "Delete"),
        C::Comma => cap(",", "Comma"),
        C::Period => cap(".", "Period"),
        C::Slash => cap("/", "Slash"),
        C::Semicolon => cap(";", "Semicolon"),
        C::Quote => cap("'", "Quote"),
        C::LeftBracket => cap("[", "Left Bracket"),
        C::RightBracket => cap("]", "Right Bracket"),
        C::BackSlash => cap("\\", "Backslash"),
        C::Minus => cap("-", "Minus"),
        C::Equals => cap("=", "Equals"),
        C::Backquote => cap("`", "Backquote"),
        C::MouseButton1 => art(G::Mouse(MousePart::Left), "Left Click"),
        C::MouseButton2 => art(G::Mouse(MousePart::Right), "Right Click"),
        C::MouseButton3 => art(G::Mouse(MousePart::Middle), "Middle Click"),
        C::MouseWheel => art(G::Mouse(MousePart::Wheel), "Mouse Wheel"),
        C::MouseDelta => art(G::Mouse(MousePart::Move), "Mouse"),
        C::MousePosition => art(G::Mouse(MousePart::Move), "Pointer"),
        C::ButtonA => face(FacePos::South, "A", "Bottom Button"),
        C::ButtonB => face(FacePos::East, "B", "Right Button"),
        C::ButtonX => face(FacePos::West, "X", "Left Button"),
        C::ButtonY => face(FacePos::North, "Y", "Top Button"),
        C::ButtonL1 => art(G::Bumper(Side::Left), "Left Bumper"),
        C::ButtonR1 => art(G::Bumper(Side::Right), "Right Bumper"),
        C::ButtonL2 => art(G::Trigger(Side::Left), "Left Trigger"),
        C::ButtonR2 => art(G::Trigger(Side::Right), "Right Trigger"),
        C::ButtonL3 => art(
            G::Stick {
                side: Side::Left,
                press: true,
            },
            "Left Stick Press",
        ),
        C::ButtonR3 => art(
            G::Stick {
                side: Side::Right,
                press: true,
            },
            "Right Stick Press",
        ),
        C::ButtonSelect => art(G::Center(Side::Left), "Select"),
        C::ButtonStart => art(G::Center(Side::Right), "Start"),
        C::DPadUp => art(G::DPad(Some(Dir::Up)), "D-Pad Up"),
        C::DPadDown => art(G::DPad(Some(Dir::Down)), "D-Pad Down"),
        C::DPadLeft => art(G::DPad(Some(Dir::Left)), "D-Pad Left"),
        C::DPadRight => art(G::DPad(Some(Dir::Right)), "D-Pad Right"),
        C::Thumbstick1 => art(
            G::Stick {
                side: Side::Left,
                press: false,
            },
            "Left Stick",
        ),
        C::Thumbstick2 => art(
            G::Stick {
                side: Side::Right,
                press: false,
            },
            "Right Stick",
        ),
        C::TouchStick => art(G::Touch, "Touch Stick"),
        C::TouchLook => art(G::Touch, "Touch Look"),
        C::TouchButton => art(G::Touch, "Tap"),
        C::TouchButton1 => art(G::Touch, "Touch Button 1"),
        C::TouchButton2 => art(G::Touch, "Touch Button 2"),
        C::TouchButton3 => art(G::Touch, "Touch Button 3"),
        C::TouchButton4 => art(G::Touch, "Touch Button 4"),
        C::TouchButton5 => art(G::Touch, "Touch Button 5"),
        C::TouchButton6 => art(G::Touch, "Touch Button 6"),
        C::TouchButton7 => art(G::Touch, "Touch Menu"),
    }
}

/// Text shown for a slot without a binding.
pub const UNBOUND_TEXT: &str = "Unbound";

/// The glyph of one binding slot.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// The device it is for.
    pub device: DeviceKind,
    /// One part for a single control, four (Up, Down, Left, Right) for a composite; an empty
    /// part of a composite is `None`.
    pub parts: [Option<ControlGlyph>; 4],
    /// Number of parts in use: 0 unbound, 1 single, 4 composite.
    pub count: u8,
    /// Words: "Space", "W A S D" (composites read Up, Left, Down, Right), "Unbound".
    pub text: String,
    /// Image id in the `metallic://glyph/` scheme of spec 9.1, for example
    /// `metallic://glyph/gamepad/letters/button_a`.
    pub image: String,
    /// The action is a toggle: the prompt shows a "Toggle" badge.
    pub toggle: bool,
}

impl Glyph {
    /// The glyph of an unbound slot.
    pub fn unbound(device: DeviceKind) -> Self {
        Self {
            device,
            parts: [None; 4],
            count: 0,
            text: UNBOUND_TEXT.to_string(),
            image: "metallic://glyph/unbound".to_string(),
            toggle: false,
        }
    }

    /// True when the slot has no control.
    pub fn is_unbound(&self) -> bool {
        self.count == 0
    }

    /// The parts in use.
    pub fn used(&self) -> impl Iterator<Item = Option<ControlGlyph>> + '_ {
        self.parts.iter().copied().take(self.count as usize)
    }
}

/// The folder of a control's image id.
fn image_folder(c: Control, style: GamepadStyle) -> &'static str {
    use Control as C;
    match c {
        C::ButtonA | C::ButtonB | C::ButtonX | C::ButtonY => match style {
            GamepadStyle::Letters => "gamepad/letters",
            GamepadStyle::Positions => "gamepad/positions",
        },
        c if c.is_gamepad() => "gamepad",
        C::MouseButton1
        | C::MouseButton2
        | C::MouseButton3
        | C::MouseWheel
        | C::MouseDelta
        | C::MousePosition => "mouse",
        c if c.is_touch() => "touch",
        _ => "keyboard",
    }
}

fn push_snake(out: &mut String, name: &str) {
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
}

/// The image id of one control: `metallic://glyph/<folder>/<snake_case name>`.
pub fn image_id(c: Control, style: GamepadStyle) -> String {
    let mut s = String::from("metallic://glyph/");
    s.push_str(image_folder(c, style));
    s.push('/');
    push_snake(&mut s, c.name());
    s
}

/// The glyph of a slot on `device`. Allocates the text and image id; cache the result.
pub fn slot_glyph(slot: &Slot, device: DeviceKind, style: GamepadStyle) -> Glyph {
    if slot.is_empty() {
        return Glyph::unbound(device);
    }
    let mut g = Glyph {
        device,
        parts: [None; 4],
        count: 0,
        text: String::new(),
        image: String::new(),
        toggle: false,
    };
    match slot {
        Slot::Single(Some(c)) => {
            g.parts[0] = Some(control_glyph(*c, style));
            g.count = 1;
            g.text.push_str(control_glyph(*c, style).text);
            g.image = image_id(*c, style);
        }
        // Empty slots return above; should one get here, it reads as unbound.
        Slot::Single(None) => return Glyph::unbound(device),
        Slot::Composite(parts) => {
            g.count = 4;
            g.image.push_str("metallic://glyph/composite/");
            for (i, part) in parts.iter().enumerate() {
                g.parts[i] = part.map(|c| control_glyph(c, style));
                if i > 0 {
                    g.image.push('+');
                }
                match part {
                    Some(c) => push_snake(&mut g.image, c.name()),
                    None => g.image.push_str("none"),
                }
            }
            // Read like a keyboard row: Up, Left, Down, Right ("W A S D").
            for (n, i) in [0, 2, 1, 3].into_iter().enumerate() {
                if n > 0 {
                    g.text.push(' ');
                }
                match parts[i] {
                    Some(c) => g.text.push_str(control_glyph(c, style).text),
                    None => g.text.push('-'),
                }
            }
        }
    }
    g
}
