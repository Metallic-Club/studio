//! "Preview as" (spec 10.2), the touch simulation behind the phone frame, the virtual gamepad,
//! the Studio-or-game key rule for Play (spec 10.4) and the panel's keymap data (#129).
//!
//! Preview never changes the scene: it is editor state only, like the Play camera options.

use metallic_core::InstanceId;
use metallic_runtime::input::{Control, Device, DeviceKind, RawEvent};

use super::touch::{ControlRole, DevicePreset, DeviceProfile, Orientation, Placed};

/// What Play pretends to run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewAs {
    /// Keyboard and mouse (the default).
    #[default]
    KeyboardMouse,
    /// A phone: touch overlay, phone viewport.
    Phone,
    /// A tablet: touch overlay, tablet viewport.
    Tablet,
    /// A gamepad: pad glyphs, virtual gamepad when no pad is connected.
    Gamepad,
}

impl PreviewAs {
    /// Every mode, in menu order.
    pub const ALL: [PreviewAs; 4] = [
        PreviewAs::KeyboardMouse,
        PreviewAs::Phone,
        PreviewAs::Tablet,
        PreviewAs::Gamepad,
    ];

    /// Menu text.
    pub fn label(self) -> &'static str {
        match self {
            PreviewAs::KeyboardMouse => "Keyboard and mouse",
            PreviewAs::Phone => "Phone",
            PreviewAs::Tablet => "Tablet",
            PreviewAs::Gamepad => "Gamepad",
        }
    }

    /// The `last_device` Play is forced to.
    pub fn forced_device(self) -> DeviceKind {
        match self {
            PreviewAs::KeyboardMouse => DeviceKind::KeyboardMouse,
            PreviewAs::Phone | PreviewAs::Tablet => DeviceKind::Touch,
            PreviewAs::Gamepad => DeviceKind::Gamepad,
        }
    }

    /// True for the modes with a touch overlay.
    pub fn is_touch(self) -> bool {
        matches!(self, PreviewAs::Phone | PreviewAs::Tablet)
    }
}

/// How gamepad face buttons are drawn (spec 9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphStyle {
    /// A, B, X, Y.
    #[default]
    Letters,
    /// Cross, circle, square, triangle outlines.
    Shapes,
}

impl GlyphStyle {
    /// Picker text.
    pub fn label(self) -> &'static str {
        match self {
            GlyphStyle::Letters => "Letters",
            GlyphStyle::Shapes => "Shapes",
        }
    }
}

/// Editor state of the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewState {
    /// The mode.
    pub mode: PreviewAs,
    /// Phone size preset (Phone mode); Tablet mode always uses the tablet preset.
    pub phone: DevicePreset,
    /// Orientation.
    pub orientation: Orientation,
    /// Draw the insets.
    pub show_safe_area: bool,
    /// Draw the 16 dp margin, 8 dp spacing and stick zone guides.
    pub show_guides: bool,
    /// Gamepad face-button style.
    pub glyphs: GlyphStyle,
    /// "Edit touch layout" is on.
    pub editing_layout: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            mode: PreviewAs::Phone,
            phone: DevicePreset::Phone,
            orientation: Orientation::Landscape,
            show_safe_area: true,
            show_guides: true,
            glyphs: GlyphStyle::Letters,
            editing_layout: false,
        }
    }
}

impl PreviewState {
    /// The screen the preview shows (touch modes only).
    pub fn profile(&self) -> Option<DeviceProfile> {
        match self.mode {
            PreviewAs::Phone => Some(self.phone.profile(self.orientation)),
            PreviewAs::Tablet => Some(DevicePreset::Tablet.profile(self.orientation)),
            _ => None,
        }
    }

    /// The viewport size in dp (touch modes) for the Play viewport to letterbox to.
    pub fn viewport_dp(&self) -> Option<[f32; 2]> {
        self.profile().map(|p| [p.width, p.height])
    }
}

/// Spec 10.4: true when Studio keeps this key while Play has focus. F1 to F12 and every chord
/// with Ctrl or Alt stay with Studio; Esc, plain keys, Shift+key and the mouse go to the game.
pub fn studio_keeps(control: Control, ctrl: bool, alt: bool) -> bool {
    use Control as C;
    if ctrl || alt {
        return true;
    }
    matches!(
        control,
        C::F1
            | C::F2
            | C::F3
            | C::F4
            | C::F5
            | C::F6
            | C::F7
            | C::F8
            | C::F9
            | C::F10
            | C::F11
            | C::F12
    )
}

/// Who gets a key while Play has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    /// A Studio shortcut.
    Studio,
    /// The game.
    Game,
}

/// Routes a key during Play (spec 10.4).
pub fn route_key(control: Control, ctrl: bool, alt: bool) -> KeyRoute {
    if studio_keeps(control, ctrl, alt) {
        KeyRoute::Studio
    } else {
        KeyRoute::Game
    }
}

/// One entry of the panel's keymap data: a command id, where it applies and its default keys.
/// The keymap system (#129) reads this table; nothing here takes a new single-key shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    /// Stable command id.
    pub command: &'static str,
    /// What it does.
    pub label: &'static str,
    /// Where it applies: `global` (command palette, View menu) or `input-actions` (panel focus).
    pub scope: &'static str,
    /// Default keys, or empty for "palette and menu only".
    pub keys: &'static str,
}

/// The panel's commands and default keys.
pub const KEYMAP: &[Shortcut] = &[
    Shortcut {
        command: "view.input_actions",
        label: "Input actions",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "play.preview_as.keyboard",
        label: "Preview as: Keyboard and mouse",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "play.preview_as.phone",
        label: "Preview as: Phone",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "play.preview_as.tablet",
        label: "Preview as: Tablet",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "play.preview_as.gamepad",
        label: "Preview as: Gamepad",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "input_actions.edit_touch_layout",
        label: "Edit touch layout",
        scope: "global",
        keys: "",
    },
    Shortcut {
        command: "input_actions.listen",
        label: "Listen for a binding",
        scope: "input-actions",
        keys: "Enter",
    },
    Shortcut {
        command: "input_actions.delete",
        label: "Delete selection",
        scope: "input-actions",
        keys: "Delete",
    },
    Shortcut {
        command: "input_actions.rename",
        label: "Rename",
        scope: "input-actions",
        keys: "F2",
    },
    Shortcut {
        command: "input_actions.tree_up",
        label: "Select previous",
        scope: "input-actions",
        keys: "Up",
    },
    Shortcut {
        command: "input_actions.tree_down",
        label: "Select next",
        scope: "input-actions",
        keys: "Down",
    },
    Shortcut {
        command: "touch_layout.nudge",
        label: "Nudge control 4 dp",
        scope: "touch-layout",
        keys: "Arrows",
    },
    Shortcut {
        command: "touch_layout.nudge_big",
        label: "Nudge control 16 dp",
        scope: "touch-layout",
        keys: "Shift+Arrows",
    },
    Shortcut {
        command: "touch_layout.smaller",
        label: "Make control smaller",
        scope: "touch-layout",
        keys: "[",
    },
    Shortcut {
        command: "touch_layout.larger",
        label: "Make control larger",
        scope: "touch-layout",
        keys: "]",
    },
    Shortcut {
        command: "touch_layout.next",
        label: "Next control",
        scope: "touch-layout",
        keys: "Tab",
    },
    Shortcut {
        command: "edit.undo",
        label: "Undo",
        scope: "global",
        keys: "Ctrl+Z",
    },
    Shortcut {
        command: "edit.redo",
        label: "Redo",
        scope: "global",
        keys: "Ctrl+Y",
    },
];

// ---------------------------------------------------------------------------------------------
// Touch simulation: the mouse as a finger, right click pins a second finger.

/// What a simulated finger is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FingerTarget {
    /// Driving the stick; the base sits where the finger landed (floating stick).
    Stick {
        /// Where the base is (dp).
        base: [f32; 2],
    },
    /// Holding the button of this action (index into the layout).
    Button(usize),
    /// Dragging the look pad.
    Look,
}

/// A simulated finger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finger {
    /// What it captured on touch down (fingers keep it for their life, spec 7.5).
    pub target: FingerTarget,
    /// Where it is now (dp).
    pub pos: [f32; 2],
}

/// Up to two fingers: the mouse finger and a pinned one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TouchSim {
    /// The finger that follows the mouse while the left button is down.
    pub mouse: Option<Finger>,
    /// The finger a right click left on the screen.
    pub pinned: Option<Finger>,
    /// Look delta accumulated this frame (dp).
    pub look_delta: [f32; 2],
}

/// What the touch overlay feeds the game this frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TouchState {
    /// Stick output, x right and y up, length at most 1.
    pub stick: [f32; 2],
    /// Actions whose buttons are held.
    pub held: Vec<Option<InstanceId>>,
    /// Look pad movement this frame (dp).
    pub look: [f32; 2],
}

impl TouchSim {
    fn land(profile: &DeviceProfile, placed: &[Placed], pos: [f32; 2]) -> Option<FingerTarget> {
        // Buttons (with their 8 dp inactive ring) first, then the stick zone, then the look pad.
        for (i, p) in placed.iter().enumerate() {
            if p.role == ControlRole::Stick {
                continue;
            }
            let d = ((pos[0] - p.center[0]).powi(2) + (pos[1] - p.center[1]).powi(2)).sqrt();
            if d <= p.radius() {
                return (p.role != ControlRole::Menu).then_some(FingerTarget::Button(i));
            }
            if d <= p.radius() + super::touch::MIN_GAP {
                return None;
            }
        }
        let zone = profile.stick_zone();
        if pos[0] >= zone.x && pos[0] <= zone.right() && pos[1] >= zone.y && pos[1] <= zone.bottom()
        {
            return Some(FingerTarget::Stick { base: pos });
        }
        let safe = profile.safe_area();
        (pos[0] > safe.x + safe.w / 2.0).then_some(FingerTarget::Look)
    }

    /// The mouse went down at `pos` (dp).
    pub fn mouse_down(&mut self, profile: &DeviceProfile, placed: &[Placed], pos: [f32; 2]) {
        self.mouse = Self::land(profile, placed, pos).map(|target| Finger { target, pos });
    }

    /// The mouse moved to `pos` while down.
    pub fn mouse_move(&mut self, pos: [f32; 2]) {
        if let Some(f) = &mut self.mouse {
            if f.target == FingerTarget::Look {
                self.look_delta[0] += pos[0] - f.pos[0];
                self.look_delta[1] += pos[1] - f.pos[1];
            }
            f.pos = pos;
        }
    }

    /// The mouse went up.
    pub fn mouse_up(&mut self) {
        self.mouse = None;
    }

    /// Right click: pins a finger at `pos`, or lifts the pinned one when the click is near it.
    pub fn toggle_pin(&mut self, profile: &DeviceProfile, placed: &[Placed], pos: [f32; 2]) {
        if let Some(p) = self.pinned {
            let d = ((pos[0] - p.pos[0]).powi(2) + (pos[1] - p.pos[1]).powi(2)).sqrt();
            self.pinned = None;
            if d < 32.0 {
                return;
            }
        }
        self.pinned = Self::land(profile, placed, pos).map(|target| Finger { target, pos });
    }

    /// Lifts every finger.
    pub fn clear(&mut self) {
        *self = TouchSim::default();
    }

    /// The overlay's output for this frame; resets the look delta.
    pub fn take_state(&mut self, profile: &DeviceProfile, placed: &[Placed]) -> TouchState {
        let mut out = TouchState {
            look: std::mem::take(&mut self.look_delta),
            ..TouchState::default()
        };
        let r = profile.stick_radius();
        for f in [self.mouse, self.pinned].into_iter().flatten() {
            match f.target {
                FingerTarget::Stick { base } => {
                    let dx = (f.pos[0] - base[0]) / r;
                    let dy = (base[1] - f.pos[1]) / r;
                    let len = (dx * dx + dy * dy).sqrt();
                    // Dead zone 0.15 R, rescaled so the output starts at 0 (spec 7.1).
                    let dz = 0.15;
                    out.stick = if len < dz {
                        [0.0, 0.0]
                    } else {
                        let k = ((len - dz) / (1.0 - dz)).min(1.0) / len;
                        [dx * k, dy * k]
                    };
                }
                FingerTarget::Button(i) => {
                    if let Some(p) = placed.get(i) {
                        out.held.push(p.id);
                    }
                }
                FingerTarget::Look => {}
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Virtual gamepad

/// Buttons on the virtual gamepad, in drawing order.
pub const PAD_BUTTONS: [Control; 14] = [
    Control::ButtonA,
    Control::ButtonB,
    Control::ButtonX,
    Control::ButtonY,
    Control::ButtonL1,
    Control::ButtonR1,
    Control::ButtonL2,
    Control::ButtonR2,
    Control::ButtonSelect,
    Control::ButtonStart,
    Control::DPadUp,
    Control::DPadDown,
    Control::DPadLeft,
    Control::DPadRight,
];

/// State of the virtual gamepad window; turns changes into raw events for the input runtime.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualPad {
    /// Held buttons.
    pub held: Vec<Control>,
    /// Left and right stick, x right and y up.
    pub sticks: [[f32; 2]; 2],
    sent_held: Vec<Control>,
    sent_sticks: [[f32; 2]; 2],
}

impl VirtualPad {
    /// Presses or releases a button.
    pub fn set_button(&mut self, c: Control, down: bool) {
        let has = self.held.contains(&c);
        if down && !has {
            self.held.push(c);
        } else if !down && has {
            self.held.retain(|&h| h != c);
        }
    }

    /// The events since the last call, stamped `time`.
    pub fn events(&mut self, time: f64) -> Vec<RawEvent> {
        let dev = Device::Gamepad(0);
        let mut out = Vec::new();
        for &c in &self.held {
            if !self.sent_held.contains(&c) {
                out.push(RawEvent::button(dev, c, 1.0, time));
            }
        }
        for &c in &self.sent_held {
            if !self.held.contains(&c) {
                out.push(RawEvent::button(dev, c, 0.0, time));
            }
        }
        for (i, stick) in [Control::Thumbstick1, Control::Thumbstick2]
            .into_iter()
            .enumerate()
        {
            if self.sticks[i] != self.sent_sticks[i] {
                out.push(RawEvent::vector(dev, stick, self.sticks[i], time));
            }
        }
        self.sent_held = self.held.clone();
        self.sent_sticks = self.sticks;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_actions::touch::{TouchAction, layout};

    fn phone() -> (DeviceProfile, Vec<Placed>) {
        let p = DevicePreset::Phone.profile(Orientation::Landscape);
        let actions = vec![
            TouchAction {
                id: None,
                path: "C/Move".into(),
                label: "Move".into(),
                role: ControlRole::Stick,
                custom: None,
            },
            TouchAction {
                id: None,
                path: "C/Jump".into(),
                label: "Jump".into(),
                role: ControlRole::Primary,
                custom: None,
            },
            TouchAction {
                id: None,
                path: "C/Run".into(),
                label: "Run".into(),
                role: ControlRole::Secondary,
                custom: None,
            },
        ];
        let placed = layout(&p, &actions);
        (p, placed)
    }

    #[test]
    fn studio_keeps_function_keys_and_chords() {
        assert_eq!(route_key(Control::F5, false, false), KeyRoute::Studio);
        assert_eq!(route_key(Control::K, true, false), KeyRoute::Studio);
        assert_eq!(route_key(Control::J, false, true), KeyRoute::Studio);
        assert_eq!(route_key(Control::W, false, false), KeyRoute::Game);
        assert_eq!(route_key(Control::Escape, false, false), KeyRoute::Game);
        assert_eq!(
            route_key(Control::MouseButton1, false, false),
            KeyRoute::Game
        );
    }

    #[test]
    fn keymap_takes_no_new_single_key_shortcut_globally() {
        for s in KEYMAP.iter().filter(|s| s.scope == "global") {
            assert!(
                s.keys.is_empty() || s.keys.starts_with("Ctrl+"),
                "{} binds {}",
                s.command,
                s.keys
            );
        }
        let mut ids: Vec<&str> = KEYMAP.iter().map(|s| s.command).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KEYMAP.len());
    }

    #[test]
    fn preview_modes_force_the_device() {
        assert_eq!(PreviewAs::Phone.forced_device(), DeviceKind::Touch);
        assert_eq!(PreviewAs::Gamepad.forced_device(), DeviceKind::Gamepad);
        let mut s = PreviewState::default();
        assert_eq!(s.viewport_dp(), Some([844.0, 390.0]));
        s.mode = PreviewAs::Tablet;
        s.orientation = Orientation::Portrait;
        assert_eq!(s.viewport_dp(), Some([820.0, 1180.0]));
        s.mode = PreviewAs::Gamepad;
        assert_eq!(s.viewport_dp(), None);
    }

    #[test]
    fn mouse_finger_drives_stick_and_pinned_finger_holds_a_button() {
        let (p, placed) = phone();
        let mut sim = TouchSim::default();
        let jump = &placed[1];
        // Pin a finger on Jump, then drag the stick with the mouse.
        sim.toggle_pin(&p, &placed, jump.center);
        let start = [150.0, 300.0];
        sim.mouse_down(&p, &placed, start);
        sim.mouse_move([start[0] + 60.0, start[1]]);
        let s = sim.take_state(&p, &placed);
        assert!((s.stick[0] - 1.0).abs() < 1e-5 && s.stick[1].abs() < 1e-5);
        assert_eq!(s.held.len(), 1);
        // Small moves stay in the dead zone.
        sim.mouse_move([start[0] + 5.0, start[1]]);
        assert_eq!(sim.take_state(&p, &placed).stick, [0.0, 0.0]);
        // Right click near the pinned finger lifts it.
        sim.toggle_pin(&p, &placed, jump.center);
        sim.mouse_up();
        let s = sim.take_state(&p, &placed);
        assert!(s.held.is_empty());
        assert_eq!(s.stick, [0.0, 0.0]);
    }

    #[test]
    fn touches_in_the_inactive_ring_do_nothing() {
        let (p, placed) = phone();
        let mut sim = TouchSim::default();
        let jump = &placed[1];
        sim.mouse_down(
            &p,
            &placed,
            [jump.center[0] - jump.radius() - 4.0, jump.center[1]],
        );
        assert!(sim.mouse.is_none());
    }

    #[test]
    fn virtual_pad_sends_edges_once() {
        let mut pad = VirtualPad::default();
        pad.set_button(Control::ButtonA, true);
        pad.sticks[0] = [0.0, 1.0];
        let e = pad.events(1.0);
        assert_eq!(e.len(), 2);
        assert!(pad.events(1.1).is_empty());
        pad.set_button(Control::ButtonA, false);
        let e = pad.events(1.2);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].value, 0.0);
    }
}
