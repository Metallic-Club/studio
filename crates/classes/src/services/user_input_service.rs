//! `UserInputService`: keyboard, mouse and touch input for scripts, and how the mouse cursor behaves.
//!
//! # Who feeds it
//!
//! The engine (`metallic_engine`), in the first step of every frame, through [`InputFeed`](crate::input::feed):
//! it turns the platform's events into the state the methods read (held keys and buttons, the cursor, the frame's
//! mouse movement), fires `InputBegan`, `InputChanged` and `InputEnded` with [`InputObject`](crate::input::
//! input_object)s, and applies `MouseBehavior` and `MouseIconEnabled` to the window. A headless engine has no input:
//! the service exists and nothing changes it.
//!
//! # Where the state lives
//!
//! Methods only see the data model, so the engine keeps what they read there, in hidden properties:
//!
//! - `HeldKeys1` to `HeldKeys3` and `HeldButtons`: the held keys and mouse buttons as bits, by `Enum.KeyCode` item
//!   index and button number. `IsKeyDown` and `IsMouseButtonPressed` test one bit.
//! - `MouseLocation` and `MouseDelta`: what `GetMouseLocation` and `GetMouseDelta` return.
//! - `FirstInput`: the first of the input objects of held keys, each pointing to the next (`InputObject.NextInput`).
//!   A press has an object only when something listened for it, so `GetKeysPressed` makes the missing ones and
//!   adds them to the chain; the engine takes them over from there and ends them with the press.
//!
//! # `gameProcessedEvent`
//!
//! The second argument of the input events is always `false` for now: there is no game UI yet to take input first.
//! When there is, the engine will pass `true` for input a UI element (a text box, a button) used.

use metallic_core::{
    ClassDesc, DataModel, EnumDesc, EnumItem, Id, MethodCall, MethodDesc, Param, PropFlags, PropMeta, ScriptError,
    ScriptValue, SignalDesc, Ty, Value, Vector2,
};

use crate::input::input_object::{KEY_CODE, NEXT_INPUT, USER_INPUT_STATE, USER_INPUT_TYPE};
use crate::input::key_code;

/// `MouseBehavior`'s items, in order.
pub static ENUMS: &[EnumDesc] = &[
    EnumDesc::new("MouseBehavior", &["Default", "LockCenter", "LockCurrentPosition"]).describe(
        "How the mouse cursor behaves: free (Default), or held so the mouse turns the camera (LockCenter at the \
         middle of the window, LockCurrentPosition where it is). Movement keeps arriving while it is held.",
    ),
];

/// The group the engine-written properties are listed under in the API dump.
const GROUP: &str = "Input";

/// Set by the engine, read by scripts, never saved.
const DEVICE: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

/// Engine bookkeeping the methods read: hidden from Studio's Properties panel too.
const STATE: PropFlags = DEVICE.union(PropFlags::HIDDEN);

/// Scripts set it; a scene does not keep it.
const SETTING: PropFlags = PropFlags::NO_SAVE.union(PropFlags::RUNTIME);

/// `MouseBehavior`'s name.
pub const MOUSE_BEHAVIOR: &str = "MouseBehavior";
/// `MouseIconEnabled`'s name.
pub const MOUSE_ICON_ENABLED: &str = "MouseIconEnabled";
/// `MouseEnabled`'s name.
pub const MOUSE_ENABLED: &str = "MouseEnabled";
/// `KeyboardEnabled`'s name.
pub const KEYBOARD_ENABLED: &str = "KeyboardEnabled";
/// `TouchEnabled`'s name.
pub const TOUCH_ENABLED: &str = "TouchEnabled";
/// `GamepadEnabled`'s name.
pub const GAMEPAD_ENABLED: &str = "GamepadEnabled";
/// The hidden cursor position `GetMouseLocation` returns.
pub const MOUSE_LOCATION: &str = "MouseLocation";
/// The hidden frame movement `GetMouseDelta` returns.
pub const MOUSE_DELTA: &str = "MouseDelta";
/// The hidden first input object of the chain of held keys' objects.
pub const FIRST_INPUT: &str = "FirstInput";
/// The hidden held-key bits, 64 keys per property, by `Enum.KeyCode` item index.
pub const HELD_KEYS: [&str; key_code::WORDS] = ["HeldKeys1", "HeldKeys2", "HeldKeys3"];
/// The hidden held mouse-button bits: bit 0 is `MouseButton1`.
pub const HELD_BUTTONS: &str = "HeldButtons";

/// The mouse buttons scripts name, in bit order.
pub const MOUSE_BUTTONS: [&str; 3] = ["MouseButton1", "MouseButton2", "MouseButton3"];

static PROPS: &[PropMeta] = &[
    PropMeta::enumeration(MOUSE_BEHAVIOR, "MouseBehavior", "Default")
        .flags(SETTING)
        .group(GROUP)
        .order(0)
        .desc(
            "How the cursor behaves. Default leaves it free; LockCenter and LockCurrentPosition hold it in the \
             window so mouse movement can turn a camera (read GetMouseDelta). Where the system cannot lock the \
             cursor (Windows), it is kept inside the window instead.",
        ),
    PropMeta::boolean(MOUSE_ICON_ENABLED, true)
        .flags(SETTING)
        .group(GROUP)
        .order(1)
        .desc("Whether the cursor is shown over the game's window."),
    PropMeta::boolean(MOUSE_ENABLED, false)
        .flags(DEVICE)
        .group(GROUP)
        .order(2)
        .desc("Whether a mouse is in use. Set by the engine once mouse input arrives."),
    PropMeta::boolean(KEYBOARD_ENABLED, false)
        .flags(DEVICE)
        .group(GROUP)
        .order(3)
        .desc("Whether a keyboard is in use. Set by the engine once keyboard input arrives."),
    PropMeta::boolean(TOUCH_ENABLED, false)
        .flags(DEVICE)
        .group(GROUP)
        .order(4)
        .desc("Whether a touch screen is in use. Set by the engine once a touch arrives."),
    PropMeta::boolean(GAMEPAD_ENABLED, false)
        .flags(DEVICE)
        .group(GROUP)
        .order(5)
        .desc("Whether a gamepad is connected. Set by the engine as gamepads come and go."),
    PropMeta::vector2(MOUSE_LOCATION, 0.0, 0.0)
        .flags(STATE)
        .group(GROUP)
        .order(6)
        .desc("Engine bookkeeping: where the cursor is. Read it with GetMouseLocation()."),
    PropMeta::vector2(MOUSE_DELTA, 0.0, 0.0)
        .flags(STATE)
        .group(GROUP)
        .order(7)
        .desc("Engine bookkeeping: how far the mouse moved this frame. Read it with GetMouseDelta()."),
    PropMeta::int(HELD_KEYS[0], 0)
        .flags(STATE)
        .group(GROUP)
        .order(8)
        .desc("Engine bookkeeping: held keys 0 to 63, as bits by KeyCode item index. Read them with IsKeyDown()."),
    PropMeta::int(HELD_KEYS[1], 0)
        .flags(STATE)
        .group(GROUP)
        .order(9)
        .desc("Engine bookkeeping: held keys 64 to 127, as bits by KeyCode item index. Read them with IsKeyDown()."),
    PropMeta::int(HELD_KEYS[2], 0)
        .flags(STATE)
        .group(GROUP)
        .order(10)
        .desc("Engine bookkeeping: held keys 128 to 191, as bits by KeyCode item index. Read them with IsKeyDown()."),
    PropMeta::int(HELD_BUTTONS, 0)
        .flags(STATE)
        .group(GROUP)
        .order(11)
        .desc("Engine bookkeeping: held mouse buttons as bits (1 is MouseButton1). Read them with IsMouseButtonPressed()."),
    PropMeta::reference(FIRST_INPUT, "InputObject")
        .flags(STATE)
        .group(GROUP)
        .order(12)
        .desc(
            "Engine bookkeeping: the first input object of the held keys, each pointing to the next. Read them with \
             GetKeysPressed().",
        ),
];

/// Whether bit `index` is set in the bits kept in the integer property `name` of `service`.
fn bit(dm: &DataModel, service: Id, name: &str, index: usize) -> Result<bool, ScriptError> {
    let bits = match dm.get_by_name(service, name)? {
        Value::Int(bits) => bits as u64,
        _ => 0,
    };
    Ok(index < 64 && bits & (1 << index) != 0)
}

/// Argument `index` as an item of the enum `enum_type`.
fn enum_arg(call: &MethodCall<'_>, index: usize, enum_type: &str) -> Result<EnumItem, ScriptError> {
    let item = call.args.enum_item(index)?;
    if item.enum_type() != enum_type {
        return Err(ScriptError::runtime(format!(
            "expected an Enum.{enum_type} item, got {item}"
        )));
    }
    Ok(item)
}

fn is_key_down(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let key = enum_arg(call, 0, "KeyCode")?;
    let Some(index) = key_code::index(key.name()) else {
        return Ok(ScriptValue::Bool(false));
    };
    let Some(word) = HELD_KEYS.get(index / 64) else {
        return Ok(ScriptValue::Bool(false));
    };
    Ok(ScriptValue::Bool(bit(call.dm, call.this, word, index % 64)?))
}

fn is_mouse_button_pressed(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let button = enum_arg(call, 0, "UserInputType")?;
    let Some(index) = MOUSE_BUTTONS.iter().position(|name| *name == button.name()) else {
        return Ok(ScriptValue::Bool(false));
    };
    Ok(ScriptValue::Bool(bit(call.dm, call.this, HELD_BUTTONS, index)?))
}

/// The input object in the chain from `FirstInput` for the key `key`, if there is one. The walk stops at a
/// destroyed object (the engine repairs the chain at the next frame) and after `limit` objects.
fn chained_key(dm: &DataModel, service: Id, key: &str, limit: usize) -> Result<Option<Id>, ScriptError> {
    let mut next = dm.reference(service, dm.prop_id(service, FIRST_INPUT)?)?;
    for _ in 0..limit {
        let Some(id) = next else {
            break;
        };
        if !dm.contains(id) {
            break;
        }
        if matches!(dm.get_by_name(id, KEY_CODE), Ok(Value::Enum(item)) if item.name() == key) {
            return Ok(Some(id));
        }
        next = dm.reference(id, dm.prop_id(id, NEXT_INPUT)?).unwrap_or(None);
    }
    Ok(None)
}

fn get_keys_pressed(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let service = call.this;
    let mut pressed = Vec::new();
    for (word, name) in HELD_KEYS.iter().enumerate() {
        for bit_index in 0..64 {
            if !bit(call.dm, service, name, bit_index)? {
                continue;
            }
            let Some(key) = key_code::name(word * 64 + bit_index) else {
                continue;
            };
            let id = match chained_key(call.dm, service, key, key_code::COUNT)? {
                Some(id) => id,
                None => new_key_object(call.dm, service, key)?,
            };
            pressed.push(ScriptValue::Instance(id));
        }
    }
    Ok(ScriptValue::List(pressed))
}

/// The input object of a held key nobody listened for when it went down, made for `GetKeysPressed` and put first in
/// the chain, where the engine finds it to end it with the press.
fn new_key_object(dm: &mut DataModel, service: Id, key: &str) -> Result<Id, ScriptError> {
    let class = dm
        .registry()
        .class_id("InputObject")
        .ok_or_else(|| ScriptError::runtime("there is no InputObject class"))?;
    let id = dm.create_internal(class)?;
    dm.set_by_name(id, USER_INPUT_TYPE, Value::Enum(EnumItem::new("UserInputType", "Keyboard")))?;
    dm.set_by_name(id, KEY_CODE, Value::Enum(EnumItem::new("KeyCode", key)))?;
    dm.set_by_name(id, USER_INPUT_STATE, Value::Enum(EnumItem::new("UserInputState", "Begin")))?;
    let first = dm.get_by_name(service, FIRST_INPUT)?;
    dm.set_by_name(id, NEXT_INPUT, first)?;
    dm.set_by_name(service, FIRST_INPUT, Value::Ref(Some(id)))?;
    Ok(id)
}

/// Reads the hidden `Vector2` `name` of the service the method was called on.
fn vector2(call: &MethodCall<'_>, name: &str) -> Result<ScriptValue, ScriptError> {
    match call.dm.get_by_name(call.this, name)? {
        Value::Vector2(value) => Ok(ScriptValue::Vector2(value)),
        _ => Ok(ScriptValue::Vector2(Vector2::ZERO)),
    }
}

fn get_mouse_location(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    vector2(call, MOUSE_LOCATION)
}

fn get_mouse_delta(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    vector2(call, MOUSE_DELTA)
}

/// What the three input events pass.
static INPUT_EVENT: &[Param] = &[
    Param::new("input", Ty::Class("InputObject")),
    Param::new("gameProcessedEvent", Ty::Bool),
];

/// A list of input objects.
const INPUT_OBJECTS: Ty = Ty::Array(&Ty::Class("InputObject"));

/// Keyboard, mouse and touch input for scripts.
pub static CLASS: ClassDesc = ClassDesc::new("UserInputService", Some("Instance"))
    .describe(
        "Keyboard, mouse and touch input: events for every key, button, movement and touch, what is held right now, \
         and how the mouse cursor behaves. Input arrives at the start of each frame. Get it with \
         game:GetService(\"UserInputService\").",
    )
    .service(6)
    .hidden_in_explorer()
    .props(PROPS)
    .methods(&[
        MethodDesc::new("IsKeyDown", is_key_down)
            .params(&[Param::new("keyCode", Ty::Enum("KeyCode"))])
            .returns(Ty::Bool)
            .describe(
                "Returns true while the key is held down. A key pressed and released within one frame is down for \
                 that frame.",
            ),
        MethodDesc::new("IsMouseButtonPressed", is_mouse_button_pressed)
            .params(&[Param::new("mouseButton", Ty::Enum("UserInputType"))])
            .returns(Ty::Bool)
            .describe(
                "Returns true while the mouse button (UserInputType.MouseButton1, 2 or 3) is held down; false for \
                 any other input type.",
            ),
        MethodDesc::new("GetMouseLocation", get_mouse_location)
            .params(&[])
            .returns(Ty::Vector2)
            .describe("Returns where the cursor is, in physical pixels from the window's top-left corner."),
        MethodDesc::new("GetMouseDelta", get_mouse_delta)
            .params(&[])
            .returns(Ty::Vector2)
            .describe(
                "Returns how far the mouse moved during this frame, in raw mouse units (X right, Y down). It keeps \
                 counting while the cursor is locked (MouseBehavior), which is what a camera reads; it is zero in a \
                 frame without movement.",
            ),
        MethodDesc::new("GetKeysPressed", get_keys_pressed)
            .params(&[])
            .returns(INPUT_OBJECTS)
            .describe(
                "Returns the input objects of every key held down right now: the same objects InputBegan passed for \
                 those presses, and InputEnded will pass.",
            ),
    ])
    .signals(&[
        SignalDesc::new("InputBegan").params(INPUT_EVENT).describe(
            "Fires when an input begins: a key or mouse button goes down, a finger touches the screen. Each press \
             gets a new InputObject, which InputChanged and InputEnded pass again for that press. \
             gameProcessedEvent is true when the game's UI used the input (always false until there is game UI).",
        ),
        SignalDesc::new("InputChanged").params(INPUT_EVENT).describe(
            "Fires when an input changes: the mouse moves (once per frame, with the frame's movement as Delta), \
             the wheel turns, a finger moves (once per frame per finger).",
        ),
        SignalDesc::new("InputEnded").params(INPUT_EVENT).describe(
            "Fires when an input ends: a key or mouse button is released, a finger leaves the screen, or the \
             window loses focus while it is held. A press shorter than a frame ends in the next frame, so \
             InputBegan always sees it begun.",
        ),
        SignalDesc::new("WindowFocused")
            .params(&[])
            .describe("Fires when the game's window gains keyboard focus."),
        SignalDesc::new("WindowFocusReleased")
            .params(&[])
            .describe("Fires when the game's window loses keyboard focus."),
    ]);
