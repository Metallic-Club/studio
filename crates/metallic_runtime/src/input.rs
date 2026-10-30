//! Player input, independent of the window library.
//!
//! The app maps its keys, mouse and gamepad to [`Action`]s and raw deltas on an [`InputState`]
//! and takes one [`FrameInput`] per frame from it. Default bindings (done by the app):
//! WASD or arrows move, Space jumps, Shift runs, right mouse drag orbits the camera, the wheel
//! zooms; gamepad left stick moves, A jumps, right stick orbits.

/// Radians the camera turns per pixel of right-drag mouse movement.
pub const MOUSE_ORBIT_SPEED: f32 = 0.006;
/// Radians per second the camera turns with the right stick fully tilted.
pub const STICK_ORBIT_SPEED: f32 = 3.0;
/// Studs the camera moves in or out per wheel notch.
pub const ZOOM_PER_NOTCH: f32 = 2.0;
/// Stick tilt below this counts as centred.
pub const STICK_DEADZONE: f32 = 0.18;

/// A held button the game reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Walk away from the camera.
    Forward,
    /// Walk toward the camera.
    Back,
    /// Walk to the camera's left.
    Left,
    /// Walk to the camera's right.
    Right,
    /// Jump (held: jump again on landing).
    Jump,
    /// Run at 1.5 times `Humanoid.WalkSpeed`.
    Run,
}

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

/// Collects input events between frames and turns them into [`FrameInput`]s.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    held: [bool; 6],
    orbit: [f32; 2],
    zoom: f32,
    move_stick: [f32; 2],
    look_stick: [f32; 2],
    pad_jump: bool,
}

fn slot(action: Action) -> usize {
    match action {
        Action::Forward => 0,
        Action::Back => 1,
        Action::Left => 2,
        Action::Right => 3,
        Action::Jump => 4,
        Action::Run => 5,
    }
}

/// Zeroes a stick inside the dead zone and rescales the rest to start from 0.
fn dead_zone(stick: [f32; 2]) -> [f32; 2] {
    let len = (stick[0] * stick[0] + stick[1] * stick[1]).sqrt();
    if len < STICK_DEADZONE || !len.is_finite() {
        return [0.0, 0.0];
    }
    let scaled = ((len - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).min(1.0);
    [stick[0] / len * scaled, stick[1] / len * scaled]
}

impl InputState {
    /// A state with nothing held.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a button press (`down`) or release.
    pub fn set(&mut self, action: Action, down: bool) {
        self.held[slot(action)] = down;
    }

    /// True while the action is held on the keyboard.
    pub fn is_held(&self, action: Action) -> bool {
        self.held[slot(action)]
    }

    /// Releases everything, for example when the window loses focus.
    pub fn release_all(&mut self) {
        *self = InputState::default();
    }

    /// Adds right-drag mouse movement in pixels.
    pub fn mouse_orbit(&mut self, dx: f32, dy: f32) {
        self.orbit[0] += dx * MOUSE_ORBIT_SPEED;
        self.orbit[1] += dy * MOUSE_ORBIT_SPEED;
    }

    /// Adds wheel movement in notches (positive = wheel up = zoom in).
    pub fn wheel(&mut self, notches: f32) {
        self.zoom -= notches * ZOOM_PER_NOTCH;
    }

    /// Sets the gamepad state: left stick (x right, y up = forward), right stick, A button.
    pub fn gamepad(&mut self, left: [f32; 2], right: [f32; 2], jump: bool) {
        self.move_stick = dead_zone(left);
        self.look_stick = dead_zone(right);
        self.pad_jump = jump;
    }

    /// The input for a frame lasting `dt` seconds; clears the per-frame mouse and wheel deltas.
    pub fn take_frame(&mut self, dt: f32) -> FrameInput {
        let key = |a: Action| if self.is_held(a) { 1.0 } else { 0.0 };
        let mut movement = [
            key(Action::Right) - key(Action::Left) + self.move_stick[0],
            key(Action::Forward) - key(Action::Back) + self.move_stick[1],
        ];
        let len = (movement[0] * movement[0] + movement[1] * movement[1]).sqrt();
        if len > 1.0 {
            movement = [movement[0] / len, movement[1] / len];
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let frame = FrameInput {
            movement,
            jump: self.is_held(Action::Jump) || self.pad_jump,
            run: self.is_held(Action::Run),
            orbit: [
                self.orbit[0] + self.look_stick[0] * STICK_ORBIT_SPEED * dt,
                self.orbit[1] - self.look_stick[1] * STICK_ORBIT_SPEED * dt,
            ],
            zoom: self.zoom,
        };
        self.orbit = [0.0, 0.0];
        self.zoom = 0.0;
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_keys_are_normalised() {
        let mut s = InputState::new();
        s.set(Action::Forward, true);
        s.set(Action::Right, true);
        let f = s.take_frame(1.0 / 60.0);
        let len = (f.movement[0].powi(2) + f.movement[1].powi(2)).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
        assert!(f.movement[0] > 0.0 && f.movement[1] > 0.0);
    }

    #[test]
    fn mouse_and_wheel_are_cleared_after_a_frame() {
        let mut s = InputState::new();
        s.mouse_orbit(100.0, 0.0);
        s.wheel(1.0);
        let f = s.take_frame(1.0 / 60.0);
        assert!(f.orbit[0] > 0.0);
        assert!(f.zoom < 0.0);
        let g = s.take_frame(1.0 / 60.0);
        assert_eq!(g.orbit, [0.0, 0.0]);
        assert_eq!(g.zoom, 0.0);
    }

    #[test]
    fn small_stick_tilt_is_ignored_and_half_tilt_walks_slower() {
        let mut s = InputState::new();
        s.gamepad([0.1, 0.0], [0.0, 0.0], false);
        assert_eq!(s.take_frame(0.016).movement, [0.0, 0.0]);
        s.gamepad([0.0, 0.6], [0.0, 0.0], true);
        let f = s.take_frame(0.016);
        assert!(
            f.movement[1] > 0.3 && f.movement[1] < 0.8,
            "{:?}",
            f.movement
        );
        assert!(f.jump);
    }
}
