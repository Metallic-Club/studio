//! Player input as actions (docs/design/input-actions.md).
//!
//! Games listen to actions, never to raw keys. The flow of one frame:
//!
//! ```text
//! winit / gilrs / --input-script  ->  RawEvent  ->  InputQueue
//!        InputRuntime::evaluate(now, dt):  bindings -> actions (values, triggers, events)
//!        InputRuntime::frame_input()    :  CharacterControls actions -> FrameInput
//!        Game::frame(&FrameInput, dt)      (unchanged: MoveInput and the camera read FrameInput)
//! ```
//!
//! - [`control`]: devices, [`Control`]s (named like Roblox's `Enum.KeyCode`), [`RawEvent`], the
//!   [`RawSource`] trait and the bounded [`InputQueue`]. The window library stays out of this
//!   crate: `metallic_player` has the winit and gilrs sources.
//! - [`config`]: `InputContext`, `InputAction` and `InputBinding` as plain data, read from the
//!   scene ([`InputConfig::from_world`]), plus the built-in `CharacterControls` context that is
//!   used when the scene has none. Default keys: WASD or arrows move, Space jumps, Shift runs,
//!   right mouse drag turns the camera, the wheel zooms; gamepad left stick or d-pad moves, A
//!   jumps, L3 runs, right stick turns the camera, R1 and L1 zoom.
//! - [`InputRuntime`]: evaluates the bindings once per frame without allocating: modifiers, dead
//!   zone, response curve, scale; composites; Bool OR and largest-magnitude combine; the triggers
//!   Press, Release, Hold, Tap, DoubleTap and Repeat measured in event time; context priority
//!   and sink; the last-used device with its hysteresis rule.
//! - [`ScriptedSource`]: `--input-script` files and tests.

mod config;
mod control;
mod runtime;
mod script;
#[cfg(test)]
mod tests;

pub use config::{
    ActionDef, ActionType, BindingDef, CHARACTER_CONTROLS, ContextDef, InputConfig,
    MOUSE_ORBIT_SPEED, PAD_ZOOM_SPEED, Role, STICK_ORBIT_SPEED, Trigger, ZOOM_PER_NOTCH,
    default_character_controls,
};
pub use control::{
    CONTROL_COUNT, Control, ControlKind, Device, DeviceKind, InputQueue, MAX_GAMEPADS,
    QUEUE_CAPACITY, RawEvent, RawSource,
};
pub use runtime::{ActionEvent, ActionEventKind, ActionId, ActionValue, InputRuntime};
pub use script::ScriptedSource;

/// Stick tilt below this counts as centred (the default dead zone of `Thumbstick1` and
/// `Thumbstick2`, radial and rescaled so the output starts from 0).
pub const STICK_DEADZONE: f32 = 0.18;

/// Everything the game needs from the player for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameInput {
    /// Wish to move relative to the camera: x to the right, y forward, each -1..=1. Longer than 1
    /// counts as 1; a half-tilted stick walks at half speed.
    pub movement: [f32; 2],
    /// Jump is held.
    pub jump: bool,
    /// Run is held.
    pub run: bool,
    /// Camera orbit this frame in radians: yaw (positive turns the view right) and pitch
    /// (positive looks further down).
    pub orbit: [f32; 2],
    /// Camera zoom this frame in studs (positive moves the camera away).
    pub zoom: f32,
}
