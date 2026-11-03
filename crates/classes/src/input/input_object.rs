//! `InputObject`: one input (a key, a mouse button, the mouse, the wheel, a touch), as `UserInputService`'s events
//! pass it. The engine creates and updates them; scripts cannot.
//!
//! # One object per press
//!
//! As in Roblox, each press of a key or mouse button, and each finger on the screen, is a new object: `InputBegan`
//! passes it in the `Begin` state, `InputChanged` (a moving finger) and `InputEnded` pass the same object, and after
//! `End` the engine destroys it. A script that kept it (`held[input] = true`) still reads it. Objects are made only
//! while something listens for them (`input::feed` has the rules). Mouse movement and the wheel are the exception:
//! one object each, reused every frame, as Roblox does for movement.
//!
//! The objects have no parent. `UserInputService` chains the objects of held keys through the hidden `NextInput`
//! reference (from its own hidden `FirstInput`), so `GetKeysPressed` finds them without a list type in the data
//! model.

use metallic_core::{ClassDesc, EnumDesc, PropFlags, PropMeta};

/// The enums an input object's fields use, besides `KeyCode`.
pub static ENUMS: &[EnumDesc] = &[
    EnumDesc::new(
        "UserInputType",
        &[
            "MouseButton1",
            "MouseButton2",
            "MouseButton3",
            "MouseWheel",
            "MouseMovement",
            "Touch",
            "Keyboard",
            "Focus",
            "None",
        ],
    )
    .numbers(&[0, 1, 2, 3, 4, 7, 8, 9, 22])
    .describe(
        "What kind of input an InputObject is: a mouse button (1 is the left, 2 the right, 3 the middle), the \
         wheel, mouse movement, a touch or a key.",
    ),
    EnumDesc::new("UserInputState", &["Begin", "Change", "End", "Cancel", "None"]).describe(
        "Where an input is: it began (a key or button went down, a finger touched), changed (moved), ended \
         (released) or was cancelled by the system.",
    ),
];

/// Set only by the engine, never saved: an input object lives for one session.
const FIELD: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

/// The group the fields are listed under in the API dump.
const GROUP: &str = "Input";

/// `KeyCode`'s name; the engine and `UserInputService` look the fields up by these.
pub const KEY_CODE: &str = "KeyCode";
/// `UserInputType`'s name.
pub const USER_INPUT_TYPE: &str = "UserInputType";
/// `UserInputState`'s name.
pub const USER_INPUT_STATE: &str = "UserInputState";
/// `Position`'s name.
pub const POSITION: &str = "Position";
/// `Delta`'s name.
pub const DELTA: &str = "Delta";
/// The hidden reference to the next object of `UserInputService`'s chain.
pub const NEXT_INPUT: &str = "NextInput";

static PROPS: &[PropMeta] = &[
    PropMeta::enumeration(KEY_CODE, "KeyCode", "Unknown")
        .flags(FIELD)
        .group(GROUP)
        .order(0)
        .desc("The key, for keyboard input; Unknown for every other kind."),
    PropMeta::enumeration(USER_INPUT_TYPE, "UserInputType", "None")
        .flags(FIELD)
        .group(GROUP)
        .order(1)
        .desc("What kind of input this is: a key, a mouse button, mouse movement, the wheel or a touch."),
    PropMeta::enumeration(USER_INPUT_STATE, "UserInputState", "None")
        .flags(FIELD)
        .group(GROUP)
        .order(2)
        .desc("Where the input is: Begin, Change, End or Cancel."),
    PropMeta::vector3(POSITION, 0.0, 0.0, 0.0)
        .flags(FIELD)
        .group(GROUP)
        .order(3)
        .desc(
            "Where the input is, in physical pixels from the window's top-left corner (X right, Y down): the cursor \
             for keys, mouse buttons and movement, the finger for a touch. For the wheel, Z is how many notches it \
             turned this frame (positive away from the user).",
        ),
    PropMeta::vector3(DELTA, 0.0, 0.0, 0.0)
        .flags(FIELD)
        .group(GROUP)
        .order(4)
        .desc(
            "How far the input moved since the last frame: raw mouse movement for MouseMovement (it keeps coming \
             while the cursor is locked), pixels for a touch. Zero for keys and buttons.",
        ),
    PropMeta::reference(NEXT_INPUT, "InputObject")
        .flags(FIELD.union(PropFlags::HIDDEN))
        .group(GROUP)
        .order(5)
        .desc("Engine bookkeeping: the next held key's input object (see UserInputService.FirstInput)."),
];

/// One input, as `UserInputService`'s events pass it.
pub static CLASS: ClassDesc = ClassDesc::new("InputObject", Some("Instance"))
    .describe(
        "One input: a press of a key or mouse button, a finger on the screen, the mouse's movement or the wheel. \
         UserInputService's InputBegan, InputChanged and InputEnded pass one. Each press is a new object that goes \
         from Begin to End and is then destroyed (it stays readable while you hold it), so a table keyed by input \
         objects tracks what is held. Mouse movement and the wheel reuse one object each, updated every frame.",
    )
    .props(PROPS);
