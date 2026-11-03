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
//! Methods only see the data model, so the engine keeps what they read there: the hidden `MouseLocation` and
//! `MouseDelta`, and the chain of input objects that starts at the hidden `FirstInput` (see `input_object`'s docs).
//! A held key is a key whose object is in the `Begin` state.
//!
//! # `gameProcessedEvent`
//!
//! The second argument of the input events is always `false` for now: there is no game UI yet to take input first.
//! When there is, the engine will pass `true` for input a UI element (a text box, a button) used.

use metallic_core::{
    ClassDesc, DataModel, EnumDesc, EnumItem, Id, MethodCall, MethodDesc, Param, PropFlags, PropId, PropMeta,
    ScriptError, ScriptValue, SignalDesc, Ty, Value, Vector2,
};

use crate::input::input_object::{KEY_CODE, NEXT_INPUT, USER_INPUT_STATE, USER_INPUT_TYPE};

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
/// The hidden first input object of the chain.
pub const FIRST_INPUT: &str = "FirstInput";

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
    PropMeta::reference(FIRST_INPUT, "InputObject")
        .flags(STATE)
        .group(GROUP)
        .order(8)
        .desc(
            "Engine bookkeeping: the first of the input objects the engine keeps, each pointing to the next. Read \
             held keys with IsKeyDown() and GetKeysPressed().",
        ),
];

/// The ids of the input object fields the methods read, for one data model's registry.
struct Fields {
    key_code: PropId,
    input_type: PropId,
    state: PropId,
    next: PropId,
}

impl Fields {
    fn of(dm: &DataModel) -> Result<Self, ScriptError> {
        let registry = dm.registry();
        let class = registry
            .class_id("InputObject")
            .ok_or_else(|| ScriptError::runtime("there is no InputObject class"))?;
        let prop = |name: &str| {
            registry
                .prop_id(class, name)
                .ok_or_else(|| ScriptError::runtime(format!("InputObject has no {name}")))
        };
        Ok(Self {
            key_code: prop(KEY_CODE)?,
            input_type: prop(USER_INPUT_TYPE)?,
            state: prop(USER_INPUT_STATE)?,
            next: prop(NEXT_INPUT)?,
        })
    }
}

/// Every input object the engine keeps for the service `service`, first to last.
///
/// A script can destroy an input object it was handed. The engine repairs the chain at the next frame's input step;
/// until then the walk stops at the destroyed object rather than failing the call.
fn inputs(dm: &DataModel, service: Id, fields: &Fields) -> Result<Vec<Id>, ScriptError> {
    let mut found = Vec::new();
    let mut next = dm.reference(service, dm.prop_id(service, FIRST_INPUT)?)?;
    while let Some(id) = next {
        // Only the engine writes the chain and it never makes a loop; this only keeps a bug from hanging a script.
        if found.contains(&id) || !dm.contains(id) {
            break;
        }
        found.push(id);
        next = dm.reference(id, fields.next).unwrap_or(None);
    }
    Ok(found)
}

/// Whether the input object `id` is held: of type `input_type`, in the `Begin` state and, when `key` is given,
/// for that key.
fn held(dm: &DataModel, id: Id, fields: &Fields, input_type: &str, key: Option<&str>) -> bool {
    let is = |prop: PropId, name: &str| dm.enum_item(id, prop).is_ok_and(|item| item.name() == name);
    is(fields.input_type, input_type) && is(fields.state, "Begin") && key.is_none_or(|key| is(fields.key_code, key))
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
    let fields = Fields::of(call.dm)?;
    for id in inputs(call.dm, call.this, &fields)? {
        if held(call.dm, id, &fields, "Keyboard", Some(key.name())) {
            return Ok(ScriptValue::Bool(true));
        }
    }
    Ok(ScriptValue::Bool(false))
}

fn is_mouse_button_pressed(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let button = enum_arg(call, 0, "UserInputType")?;
    if !matches!(button.name(), "MouseButton1" | "MouseButton2" | "MouseButton3") {
        return Ok(ScriptValue::Bool(false));
    }
    let fields = Fields::of(call.dm)?;
    for id in inputs(call.dm, call.this, &fields)? {
        if held(call.dm, id, &fields, button.name(), None) {
            return Ok(ScriptValue::Bool(true));
        }
    }
    Ok(ScriptValue::Bool(false))
}

fn get_keys_pressed(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let fields = Fields::of(call.dm)?;
    let mut pressed = Vec::new();
    for id in inputs(call.dm, call.this, &fields)? {
        if held(call.dm, id, &fields, "Keyboard", None) {
            pressed.push(ScriptValue::Instance(id));
        }
    }
    Ok(ScriptValue::List(pressed))
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
            .describe("Returns true while the key is held down."),
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
            .describe("Returns the input objects of every key held down right now."),
    ])
    .signals(&[
        SignalDesc::new("InputBegan").params(INPUT_EVENT).describe(
            "Fires when an input begins: a key or mouse button goes down, a finger touches the screen. \
             gameProcessedEvent is true when the game's UI used the input (always false until there is game UI).",
        ),
        SignalDesc::new("InputChanged").params(INPUT_EVENT).describe(
            "Fires when an input changes: the mouse moves (once per frame, with the frame's movement as Delta), \
             the wheel turns, a finger moves.",
        ),
        SignalDesc::new("InputEnded").params(INPUT_EVENT).describe(
            "Fires when an input ends: a key or mouse button is released, a finger leaves the screen, or the \
             window loses focus while it is held.",
        ),
        SignalDesc::new("WindowFocused")
            .params(&[])
            .describe("Fires when the game's window gains keyboard focus."),
        SignalDesc::new("WindowFocusReleased")
            .params(&[])
            .describe("Fires when the game's window loses keyboard focus."),
    ]);
