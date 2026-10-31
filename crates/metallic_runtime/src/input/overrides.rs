//! The player's input overrides: rebound controls, hold or toggle, stick and mouse settings and
//! the accessibility assists, kept as a small JSON file of differences per game and local profile
//! (docs/design/input-actions.md sections 8.4 and 11).
//!
//! ```json
//! {
//!   "version": 1,
//!   "overrides": {
//!     "CharacterControls/Jump": { "bindings": [ { "device": "Keyboard", "key": "E" } ] },
//!     "CharacterControls/Run": { "toggle": true }
//!   },
//!   "sticks": { "Thumbstick2": { "sensitivity": 1.25, "invertY": true, "deadzone": 0.1 } },
//!   "assist": { "toggleAll": false, "replaceHolds": true, "holdTimeScale": 1.5 },
//!   "glyphs": { "gamepadStyle": "Letters" },
//!   "touch": { "stick": "Fixed", "opacity": 0.75 },
//!   "updated": "2026-10-30T12:00:00Z"
//! }
//! ```
//!
//! - Only differences are written, keyed by the stable path `Context/Action`, so a game update
//!   that adds actions or changes the creator's bindings still applies.
//! - An action's `bindings` replace the creator's bindings of the devices they name; other devices
//!   keep the creator's. Entries are slots in order (Primary, Alternate); `"key": "None"` keeps an
//!   empty slot in its place. Composites name `up`, `down`, `left` and `right`.
//! - Unknown keys, at any level this module reads, are kept and written back (a newer Player may
//!   use them). A wrong value for one field is
//!   ignored without losing the rest of the file.
//! - [`OverrideStore`] reads a missing or damaged file as "no overrides" (a damaged file is moved
//!   aside, not deleted) and writes atomically: a temporary file, then a rename. An empty set of
//!   overrides removes the file, so deleting the file and resetting everything are the same.
use super::config::{
    ActionDef, ActionType, BindingDef, InputConfig, MENU_ACTION, MOUSE_ORBIT_SPEED, PAD_ZOOM_SPEED,
    Role, STICK_ORBIT_SPEED, Trigger, ZOOM_PER_NOTCH,
};
use super::control::{Control, ControlKind, DeviceKind};
use super::touch::StickMode;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file format version this Player writes.
pub const OVERRIDES_VERSION: u64 = 1;
/// The local profile's file name until sign-in exists (spec 8.4).
pub const LOCAL_PROFILE: &str = "local";
/// The largest override file that is read (a real one is a few kilobytes).
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The most slots one action keeps per device when a file is read.
pub const MAX_SLOTS_PER_DEVICE: usize = super::rebind::MAX_SLOTS;
/// Stick and mouse sensitivity range (spec 8.1: 25% to 200%).
pub const SENSITIVITY_RANGE: (f32, f32) = (0.25, 2.0);
/// Stick dead zone range (spec 8.1).
pub const DEADZONE_RANGE: (f32, f32) = (0.0, 0.4);
/// `HoldTime` scale range (spec 11.1).
pub const HOLD_SCALE_RANGE: (f32, f32) = (0.5, 3.0);

/// One binding slot of an action on one device, as the player sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// One control (`None`: the slot is empty).
    Single(Option<Control>),
    /// Four keys that make a direction: Up, Down, Left, Right.
    Composite([Option<Control>; 4]),
}

impl Slot {
    /// True when the slot holds no control.
    pub fn is_empty(&self) -> bool {
        match self {
            Slot::Single(c) => c.is_none(),
            Slot::Composite(parts) => parts.iter().all(Option::is_none),
        }
    }

    /// The control of one part: `None` for a single slot's only part.
    pub fn part(&self, part: Option<usize>) -> Option<Control> {
        match (self, part) {
            (Slot::Single(c), None) => *c,
            (Slot::Composite(parts), Some(i)) => parts.get(i).copied().flatten(),
            _ => None,
        }
    }

    /// Sets one part; a part index that does not fit the slot is ignored.
    pub fn set_part(&mut self, part: Option<usize>, control: Option<Control>) {
        match (self, part) {
            (Slot::Single(c), None) => *c = control,
            (Slot::Composite(parts), Some(i)) if i < 4 => parts[i] = control,
            _ => {}
        }
    }

    /// Every control in the slot.
    pub fn controls(&self) -> impl Iterator<Item = Control> + '_ {
        let (single, parts): (Option<Control>, &[Option<Control>]) = match self {
            Slot::Single(c) => (*c, &[]),
            Slot::Composite(p) => (None, p),
        };
        single.into_iter().chain(parts.iter().flatten().copied())
    }
}

/// The device a control belongs to.
pub fn control_device(c: Control) -> DeviceKind {
    if c.is_gamepad() {
        DeviceKind::Gamepad
    } else if c.is_touch() {
        DeviceKind::Touch
    } else {
        DeviceKind::KeyboardMouse
    }
}

/// The device a binding belongs to: that of its key or first composite part.
pub fn binding_device(b: &BindingDef) -> Option<DeviceKind> {
    b.key
        .or_else(|| b.composite.iter().flatten().next().copied())
        .map(control_device)
}

/// The slot a creator binding shows as.
pub fn binding_slot(b: &BindingDef) -> Slot {
    if b.composite.iter().any(Option::is_some) {
        Slot::Composite([
            b.composite[0],
            b.composite[1],
            b.composite[2],
            b.composite[3],
        ])
    } else {
        Slot::Single(b.key)
    }
}

/// The creator's slots of an action on one device, in binding order.
pub fn creator_slots(action: &ActionDef, device: DeviceKind) -> Vec<Slot> {
    action
        .bindings
        .iter()
        .filter(|b| binding_device(b) == Some(device))
        .map(binding_slot)
        .collect()
}

/// Device names in the file.
fn device_name(d: DeviceKind) -> &'static str {
    match d {
        DeviceKind::KeyboardMouse => "Keyboard",
        DeviceKind::Gamepad => "Gamepad",
        DeviceKind::Touch => "Touch",
    }
}

fn device_from_name(s: &str) -> Option<DeviceKind> {
    Some(match s {
        "Keyboard" => DeviceKind::KeyboardMouse,
        "Gamepad" => DeviceKind::Gamepad,
        "Touch" => DeviceKind::Touch,
        _ => return None,
    })
}

/// What the player changed on one action.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionOverride {
    /// Devices whose bindings the player replaced, with the slots in order.
    pub devices: Vec<(DeviceKind, Vec<Slot>)>,
    /// Hold (false) or toggle (true); `None` follows "Toggle every hold action".
    pub toggle: Option<bool>,
    /// Binding entries this Player could not read (an unknown device): written back unchanged.
    unknown_bindings: Vec<Value>,
    /// Unknown keys of the action entry.
    extra: Map<String, Value>,
}

impl ActionOverride {
    /// The slots the player set for `device`, if any.
    pub fn slots(&self, device: DeviceKind) -> Option<&[Slot]> {
        self.devices
            .iter()
            .find(|(d, _)| *d == device)
            .map(|(_, s)| s.as_slice())
    }

    /// Replaces the slots of `device` (`None`: back to the creator's).
    pub fn set_slots(&mut self, device: DeviceKind, slots: Option<Vec<Slot>>) {
        self.devices.retain(|(d, _)| *d != device);
        if let Some(slots) = slots {
            self.devices.push((device, slots));
            self.devices.sort_by_key(|(d, _)| d.index());
        }
    }

    /// True when the entry changes nothing and carries nothing unknown.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
            && self.toggle.is_none()
            && self.unknown_bindings.is_empty()
            && self.extra.is_empty()
    }
}

/// Which stick (or the mouse) a [`StickSettings`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stick {
    /// `Thumbstick1`.
    Left,
    /// `Thumbstick2`.
    Right,
    /// `MouseDelta` (mouse look).
    Mouse,
}

impl Stick {
    /// All three, in file order.
    pub const ALL: [Stick; 3] = [Stick::Left, Stick::Right, Stick::Mouse];

    /// The control the settings apply to.
    pub fn control(self) -> Control {
        match self {
            Stick::Left => Control::Thumbstick1,
            Stick::Right => Control::Thumbstick2,
            Stick::Mouse => Control::MouseDelta,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Sensitivity, inversion and dead zone of one stick or the mouse (spec 8.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickSettings {
    /// Multiplies the binding's `Scale`: 0.25 to 2.
    pub sensitivity: f32,
    /// Flips the x axis.
    pub invert_x: bool,
    /// Flips the y axis.
    pub invert_y: bool,
    /// Replaces the binding's dead zone (sticks only): 0 to 0.4. `None` keeps the creator's.
    pub deadzone: Option<f32>,
}

impl Default for StickSettings {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
            deadzone: None,
        }
    }
}

/// The hold and toggle assists (spec 11.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AssistSettings {
    /// "Toggle every hold action": every Bool action with `CanToggle` toggles, unless the
    /// player set it to Hold. Jump and Menu are never included.
    pub toggle_all: bool,
    /// "Replace holds with presses": a `Hold` trigger fires on the press.
    pub replace_holds: bool,
    /// Multiplies `HoldTime`: 0.5 to 3.
    pub hold_time_scale: f32,
}

impl Default for AssistSettings {
    fn default() -> Self {
        Self {
            toggle_all: false,
            replace_holds: false,
            hold_time_scale: 1.0,
        }
    }
}

/// The touch overlay's settings (spec 7.4): floating or fixed stick, and its opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchSettings {
    /// Floating (the base appears under the thumb, the default) or fixed.
    pub stick_mode: StickMode,
    /// Opacity of every touch control: 0.25 to 1.
    pub opacity: f32,
}

/// The range of [`TouchSettings::opacity`].
pub const TOUCH_OPACITY_RANGE: (f32, f32) = (0.25, 1.0);

impl Default for TouchSettings {
    fn default() -> Self {
        Self {
            stick_mode: StickMode::Floating,
            opacity: 1.0,
        }
    }
}

/// Which face-button art the gamepad glyphs use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GamepadGlyphPref {
    /// Chosen from the pad's vendor (see [`super::glyph::auto_gamepad_style`]).
    #[default]
    Auto,
    /// Letters A, B, X, Y.
    Letters,
    /// Positions only: a diamond with the pressed button filled (any pad).
    Positions,
}

impl GamepadGlyphPref {
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Letters => "Letters",
            Self::Positions => "Positions",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "Auto" => Self::Auto,
            "Letters" => Self::Letters,
            "Positions" => Self::Positions,
            _ => return None,
        })
    }
}

/// Everything the player changed about input for one game.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerOverrides {
    /// Per action path (`Context/Action`).
    pub actions: BTreeMap<String, ActionOverride>,
    sticks: [StickSettings; 3],
    /// Hold and toggle assists.
    pub assist: AssistSettings,
    /// Face-button art.
    pub gamepad_glyphs: GamepadGlyphPref,
    /// The touch overlay: stick mode and opacity (opacity clamped on read).
    pub touch: TouchSettings,
    /// When the file was last written (UTC, RFC 3339), as read.
    pub updated: Option<String>,
    /// Unknown keys: top level, and inside `sticks`, `assist` and `glyphs`.
    extra: Map<String, Value>,
    sticks_extra: Map<String, Value>,
    stick_extra: [Map<String, Value>; 3],
    assist_extra: Map<String, Value>,
    glyphs_extra: Map<String, Value>,
    touch_extra: Map<String, Value>,
}

impl PlayerOverrides {
    /// The settings of one stick or the mouse.
    pub fn stick(&self, stick: Stick) -> StickSettings {
        self.sticks[stick.index()]
    }

    /// Sets the settings of one stick or the mouse; values are clamped to their ranges.
    pub fn set_stick(&mut self, stick: Stick, mut s: StickSettings) {
        s.sensitivity = clamp_or(s.sensitivity, SENSITIVITY_RANGE, 1.0);
        s.deadzone = s
            .deadzone
            .filter(|d| d.is_finite())
            .map(|d| d.clamp(DEADZONE_RANGE.0, DEADZONE_RANGE.1));
        if stick == Stick::Mouse {
            s.deadzone = None;
        }
        self.sticks[stick.index()] = s;
    }

    /// Sets the touch settings; the opacity is clamped to [`TOUCH_OPACITY_RANGE`].
    pub fn set_touch(&mut self, mut t: TouchSettings) {
        t.opacity = clamp_or(t.opacity, TOUCH_OPACITY_RANGE, 1.0);
        self.touch = t;
    }

    /// The override of one action, if the player changed it.
    pub fn action(&self, path: &str) -> Option<&ActionOverride> {
        self.actions.get(path)
    }

    /// The override of one action, created when missing. Call [`prune`](Self::prune) after
    /// changing it so an entry that changes nothing is not written.
    pub fn action_mut(&mut self, path: &str) -> &mut ActionOverride {
        self.actions.entry(path.to_string()).or_default()
    }

    /// Drops action entries that change nothing.
    pub fn prune(&mut self) {
        self.actions.retain(|_, a| !a.is_empty());
    }

    /// True when nothing differs from the creator's controls and nothing unknown is kept: such a
    /// set is not written to disk (an existing file is removed).
    pub fn is_empty(&self) -> bool {
        self.actions.values().all(ActionOverride::is_empty)
            && self.sticks == [StickSettings::default(); 3]
            && self.assist == AssistSettings::default()
            && self.gamepad_glyphs == GamepadGlyphPref::Auto
            && self.touch == TouchSettings::default()
            && self.extra.is_empty()
            && self.sticks_extra.is_empty()
            && self.stick_extra.iter().all(Map::is_empty)
            && self.assist_extra.is_empty()
            && self.glyphs_extra.is_empty()
            && self.touch_extra.is_empty()
    }

    /// Resets what the player can change; unknown keys are kept.
    pub fn reset_known(&mut self) {
        for a in self.actions.values_mut() {
            a.devices.clear();
            a.toggle = None;
        }
        self.prune();
        self.sticks = [StickSettings::default(); 3];
        self.assist = AssistSettings::default();
        self.gamepad_glyphs = GamepadGlyphPref::Auto;
        self.touch = TouchSettings::default();
    }

    // -----------------------------------------------------------------------------------------
    // JSON

    /// Reads the file's text. A field of the wrong type is skipped; text that is not a JSON
    /// object is an error.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut o = Self::read_json(text)?;
        o.restore_menu();
        Ok(o)
    }

    /// The Menu rule on files edited by hand (spec 8.3): a device whose Menu slots are all
    /// empty gets the creator's Menu bindings back, so the player can always open the page.
    fn restore_menu(&mut self) {
        if let Some(a) = self.actions.get_mut(MENU_ACTION) {
            a.devices
                .retain(|(_, slots)| slots.iter().any(|s| !s.is_empty()));
            if a.is_empty() {
                self.actions.remove(MENU_ACTION);
            }
        }
    }

    fn read_json(text: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Value::Object(mut root) = root else {
            return Err("the file is not a JSON object".into());
        };
        let mut o = PlayerOverrides::default();
        root.remove("version");
        if let Some(Value::String(s)) = root.remove("updated") {
            o.updated = Some(s);
        }
        match root.remove("overrides") {
            Some(Value::Object(map)) => {
                for (path, entry) in map {
                    if let Value::Object(entry) = entry {
                        o.actions.insert(path, read_action(entry));
                    }
                }
            }
            Some(other) => {
                root.insert("overrides".into(), other);
            }
            None => {}
        }
        if let Some(v) = root.remove("sticks") {
            match v {
                Value::Object(mut map) => {
                    for stick in Stick::ALL {
                        if let Some(Value::Object(entry)) = map.remove(stick.control().name()) {
                            let (settings, extra) = read_stick(entry);
                            o.set_stick(stick, settings);
                            o.stick_extra[stick.index()] = extra;
                        }
                    }
                    o.sticks_extra = map;
                }
                other => {
                    root.insert("sticks".into(), other);
                }
            }
        }
        if let Some(v) = root.remove("assist") {
            match v {
                Value::Object(mut map) => {
                    if let Some(b) = take_bool(&mut map, "toggleAll") {
                        o.assist.toggle_all = b;
                    }
                    if let Some(b) = take_bool(&mut map, "replaceHolds") {
                        o.assist.replace_holds = b;
                    }
                    if let Some(x) = take_num(&mut map, "holdTimeScale") {
                        o.assist.hold_time_scale = clamp_or(x, HOLD_SCALE_RANGE, 1.0);
                    }
                    o.assist_extra = map;
                }
                other => {
                    root.insert("assist".into(), other);
                }
            }
        }
        if let Some(v) = root.remove("glyphs") {
            match v {
                Value::Object(mut map) => {
                    let style = map.get("gamepadStyle").and_then(Value::as_str);
                    if let Some(pref) = style.and_then(GamepadGlyphPref::from_name) {
                        o.gamepad_glyphs = pref;
                        map.remove("gamepadStyle");
                    }
                    o.glyphs_extra = map;
                }
                other => {
                    root.insert("glyphs".into(), other);
                }
            }
        }
        if let Some(v) = root.remove("touch") {
            match v {
                Value::Object(mut map) => {
                    let mode = match map.get("stick").and_then(Value::as_str) {
                        Some("Fixed") => Some(StickMode::Fixed),
                        Some("Floating") => Some(StickMode::Floating),
                        _ => None,
                    };
                    if let Some(mode) = mode {
                        o.touch.stick_mode = mode;
                        map.remove("stick");
                    }
                    if let Some(x) = take_num(&mut map, "opacity") {
                        o.touch.opacity = clamp_or(x, TOUCH_OPACITY_RANGE, 1.0);
                    }
                    o.touch_extra = map;
                }
                other => {
                    root.insert("touch".into(), other);
                }
            }
        }
        o.extra = root;
        Ok(o)
    }

    /// The file's text: differences only, unknown keys included, `updated` set to `updated`.
    pub fn to_json(&self, updated: &str) -> String {
        let mut root = Map::new();
        root.insert("version".into(), Value::from(OVERRIDES_VERSION));
        let mut actions = Map::new();
        for (path, a) in &self.actions {
            if !a.is_empty() {
                actions.insert(path.clone(), Value::Object(write_action(a)));
            }
        }
        if !actions.is_empty() {
            root.insert("overrides".into(), Value::Object(actions));
        }
        let mut sticks = self.sticks_extra.clone();
        for stick in Stick::ALL {
            let entry = write_stick(self.stick(stick), &self.stick_extra[stick.index()]);
            if !entry.is_empty() {
                sticks.insert(stick.control().name().into(), Value::Object(entry));
            }
        }
        if !sticks.is_empty() {
            root.insert("sticks".into(), Value::Object(sticks));
        }
        let mut assist = self.assist_extra.clone();
        let d = AssistSettings::default();
        if self.assist.toggle_all != d.toggle_all {
            assist.insert("toggleAll".into(), Value::Bool(self.assist.toggle_all));
        }
        if self.assist.replace_holds != d.replace_holds {
            assist.insert(
                "replaceHolds".into(),
                Value::Bool(self.assist.replace_holds),
            );
        }
        if self.assist.hold_time_scale != d.hold_time_scale {
            assist.insert("holdTimeScale".into(), num(self.assist.hold_time_scale));
        }
        if !assist.is_empty() {
            root.insert("assist".into(), Value::Object(assist));
        }
        let mut glyphs = self.glyphs_extra.clone();
        if self.gamepad_glyphs != GamepadGlyphPref::Auto {
            glyphs.insert(
                "gamepadStyle".into(),
                Value::String(self.gamepad_glyphs.name().into()),
            );
        }
        if !glyphs.is_empty() {
            root.insert("glyphs".into(), Value::Object(glyphs));
        }
        let mut touch = self.touch_extra.clone();
        if self.touch.stick_mode != StickMode::Floating {
            touch.insert("stick".into(), Value::String("Fixed".into()));
        }
        if self.touch.opacity != TouchSettings::default().opacity {
            touch.insert("opacity".into(), num(self.touch.opacity));
        }
        if !touch.is_empty() {
            root.insert("touch".into(), Value::Object(touch));
        }
        for (k, v) in &self.extra {
            root.entry(k.clone()).or_insert_with(|| v.clone());
        }
        root.insert("updated".into(), Value::String(updated.into()));
        let mut text = serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default();
        text.push('\n');
        text
    }

    // -----------------------------------------------------------------------------------------
    // Applying

    /// The slots of an action on a device as the player sees them: the player's, else the
    /// creator's.
    pub fn slots(&self, path: &str, action: &ActionDef, device: DeviceKind) -> Vec<Slot> {
        match self.action(path).and_then(|a| a.slots(device)) {
            Some(slots) => slots.to_vec(),
            None => creator_slots(action, device),
        }
    }

    /// True when `action` (at `path`) toggles after these overrides.
    pub fn toggles(&self, path: &str, action: &ActionDef) -> bool {
        if action.ty != ActionType::Bool || !action.can_toggle {
            return false;
        }
        let all = self.assist.toggle_all && !matches!(action.role, Role::Jump | Role::Menu);
        self.action(path).and_then(|a| a.toggle).unwrap_or(all)
    }

    /// `config` with these overrides applied (spec 8.4): bindings of the devices the player
    /// changed, then stick and mouse settings, then hold/toggle and the hold assists. The
    /// creator's config is not changed. Overrides of actions that no longer exist are ignored.
    pub fn apply(&self, config: &InputConfig) -> InputConfig {
        let mut out = config.clone();
        for ctx in &mut out.contexts {
            for action in &mut ctx.actions {
                let path = format!("{}/{}", ctx.name, action.name);
                self.apply_action(&path, action);
            }
        }
        out
    }

    fn apply_action(&self, path: &str, action: &mut ActionDef) {
        if let Some(over) = self.action(path) {
            for (device, slots) in &over.devices {
                replace_device_bindings(action, *device, slots);
            }
        }
        for b in &mut action.bindings {
            let Some(stick) = Stick::ALL.into_iter().find(|s| b.key == Some(s.control())) else {
                continue;
            };
            let s = self.stick(stick);
            b.scale *= s.sensitivity;
            if s.invert_x {
                b.vector2_scale[0] = -b.vector2_scale[0];
            }
            if s.invert_y {
                b.vector2_scale[1] = -b.vector2_scale[1];
            }
            if let Some(dz) = s.deadzone {
                b.deadzone = Some(dz);
            }
        }
        action.toggle = self.toggles(path, action);
        if action.ty == ActionType::Bool {
            if self.assist.replace_holds && action.trigger == Trigger::Hold {
                action.trigger = Trigger::Press;
            }
            action.hold_time *= self.assist.hold_time_scale;
        }
    }
}

/// Replaces the bindings of `device` in `action` with bindings made from `slots`. Each new
/// binding copies its shape values (scale, dead zone, modifiers, ...) from the creator binding in
/// the same slot when it has the same shape, else from another creator binding of that shape,
/// else from the defaults for the action's role.
fn replace_device_bindings(action: &mut ActionDef, device: DeviceKind, slots: &[Slot]) {
    let creator: Vec<BindingDef> = action
        .bindings
        .iter()
        .filter(|b| binding_device(b) == Some(device))
        .cloned()
        .collect();
    let at = action
        .bindings
        .iter()
        .position(|b| binding_device(b) == Some(device))
        .unwrap_or(action.bindings.len());
    let mut made = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        if slot.is_empty() {
            continue;
        }
        let shape = slot_shape(slot);
        let template = creator
            .get(i)
            .filter(|b| slot_shape(&binding_slot(b)) == shape)
            .or_else(|| {
                action
                    .bindings
                    .iter()
                    .find(|b| slot_shape(&binding_slot(b)) == shape)
            })
            .cloned()
            .unwrap_or_else(|| role_default(action.role, shape));
        let mut b = template;
        b.instance = None;
        match slot {
            Slot::Single(c) => {
                b.key = *c;
                b.composite = [None; 6];
            }
            Slot::Composite(parts) => {
                b.key = None;
                b.composite = [parts[0], parts[1], parts[2], parts[3], None, None];
            }
        }
        made.push(b);
    }
    action
        .bindings
        .retain(|b| binding_device(b) != Some(device));
    let at = at.min(action.bindings.len());
    action.bindings.splice(at..at, made);
}

/// The shape of a slot: composite, or the kind of its control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Composite,
    Control(ControlKind),
}

fn slot_shape(slot: &Slot) -> Shape {
    match slot {
        Slot::Composite(_) => Shape::Composite,
        Slot::Single(c) => Shape::Control(c.map_or(ControlKind::Button, Control::kind)),
    }
}

/// A binding with the values a new binding of `shape` needs for an action of `role`.
fn role_default(role: Role, shape: Shape) -> BindingDef {
    let b = BindingDef::default();
    match (role, shape) {
        (Role::Look, Shape::Control(ControlKind::Delta)) => b
            .with_modifier(Control::MouseButton2)
            .with_scale(MOUSE_ORBIT_SPEED)
            .with_vector2_scale([1.0, -1.0]),
        (Role::Look, Shape::Control(ControlKind::Stick)) => b
            .with_scale(STICK_ORBIT_SPEED)
            .per_second()
            .with_clamp(false),
        (Role::Look, _) => b.with_scale(STICK_ORBIT_SPEED).per_second(),
        (Role::Zoom, Shape::Control(ControlKind::Wheel)) => b.with_scale(-ZOOM_PER_NOTCH),
        (Role::Zoom, _) => b.with_scale(PAD_ZOOM_SPEED).per_second(),
        _ => b,
    }
}

fn clamp_or(x: f32, (lo, hi): (f32, f32), fallback: f32) -> f32 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        fallback
    }
}

fn num(x: f32) -> Value {
    // Round to 3 decimals so 1.25 stays 1.25 and not 1.2500000476837158.
    let v = (f64::from(x) * 1000.0).round() / 1000.0;
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

fn take_bool(map: &mut Map<String, Value>, key: &str) -> Option<bool> {
    match map.get(key) {
        Some(Value::Bool(b)) => {
            let b = *b;
            map.remove(key);
            Some(b)
        }
        _ => None,
    }
}

fn take_num(map: &mut Map<String, Value>, key: &str) -> Option<f32> {
    let x = map.get(key).and_then(Value::as_f64)? as f32;
    map.remove(key);
    Some(x)
}

fn control_value(c: Option<Control>) -> Value {
    Value::String(c.map_or("None", Control::name).into())
}

/// Reads a control name: `Some(None)` for "None", `None` for something unreadable.
fn read_control(v: Option<&Value>) -> Option<Option<Control>> {
    let s = v?.as_str()?;
    if s == "None" {
        return Some(None);
    }
    Control::from_name(s).map(Some)
}

fn read_action(mut entry: Map<String, Value>) -> ActionOverride {
    let mut a = ActionOverride {
        toggle: take_bool(&mut entry, "toggle"),
        ..ActionOverride::default()
    };
    if let Some(Value::Array(list)) = entry.get("bindings") {
        let mut devices: Vec<(DeviceKind, Vec<Slot>)> = Vec::new();
        let mut unknown = Vec::new();
        for item in list {
            match read_binding(item) {
                Some((device, slot)) => match devices.iter_mut().find(|(d, _)| *d == device) {
                    // At most MAX_SLOTS_PER_DEVICE; extra slots in a hand-edited file are dropped.
                    Some((_, slots)) if slots.len() >= MAX_SLOTS_PER_DEVICE => {}
                    Some((_, slots)) => slots.push(slot),
                    None => devices.push((device, vec![slot])),
                },
                None => unknown.push(item.clone()),
            }
        }
        devices.sort_by_key(|(d, _)| d.index());
        a.devices = devices;
        a.unknown_bindings = unknown;
        entry.remove("bindings");
    }
    a.extra = entry;
    a
}

fn read_binding(item: &Value) -> Option<(DeviceKind, Slot)> {
    let obj = item.as_object()?;
    let device = device_from_name(obj.get("device")?.as_str()?)?;
    if obj.contains_key("key") {
        let c = read_control(obj.get("key"))?;
        if c.is_some_and(|c| control_device(c) != device) {
            return None;
        }
        return Some((device, Slot::Single(c)));
    }
    let mut parts = [None; 4];
    for (i, name) in ["up", "down", "left", "right"].iter().enumerate() {
        parts[i] = match obj.get(*name) {
            None => None,
            v => read_control(v)?,
        };
    }
    if parts.iter().flatten().any(|&c| control_device(c) != device) {
        return None;
    }
    Some((device, Slot::Composite(parts)))
}

fn write_action(a: &ActionOverride) -> Map<String, Value> {
    let mut entry = a.extra.clone();
    let mut list = Vec::new();
    for (device, slots) in &a.devices {
        for slot in slots {
            let mut item = Map::new();
            item.insert("device".into(), Value::String(device_name(*device).into()));
            match slot {
                Slot::Single(c) => {
                    item.insert("key".into(), control_value(*c));
                }
                Slot::Composite(parts) => {
                    for (i, name) in ["up", "down", "left", "right"].iter().enumerate() {
                        item.insert((*name).into(), control_value(parts[i]));
                    }
                }
            }
            list.push(Value::Object(item));
        }
        if slots.is_empty() {
            // The player removed every binding of the device: one empty slot says so.
            let mut item = Map::new();
            item.insert("device".into(), Value::String(device_name(*device).into()));
            item.insert("key".into(), control_value(None));
            list.push(Value::Object(item));
        }
    }
    list.extend(a.unknown_bindings.iter().cloned());
    if !list.is_empty() {
        entry.insert("bindings".into(), Value::Array(list));
    }
    if let Some(t) = a.toggle {
        entry.insert("toggle".into(), Value::Bool(t));
    }
    entry
}

fn read_stick(mut entry: Map<String, Value>) -> (StickSettings, Map<String, Value>) {
    let mut s = StickSettings::default();
    if let Some(x) = take_num(&mut entry, "sensitivity") {
        s.sensitivity = x;
    }
    if let Some(b) = take_bool(&mut entry, "invertX") {
        s.invert_x = b;
    }
    if let Some(b) = take_bool(&mut entry, "invertY") {
        s.invert_y = b;
    }
    if let Some(x) = take_num(&mut entry, "deadzone") {
        s.deadzone = Some(x);
    }
    (s, entry)
}

fn write_stick(s: StickSettings, extra: &Map<String, Value>) -> Map<String, Value> {
    let mut entry = extra.clone();
    let d = StickSettings::default();
    if s.sensitivity != d.sensitivity {
        entry.insert("sensitivity".into(), num(s.sensitivity));
    }
    if s.invert_x {
        entry.insert("invertX".into(), Value::Bool(true));
    }
    if s.invert_y {
        entry.insert("invertY".into(), Value::Bool(true));
    }
    if let Some(dz) = s.deadzone {
        entry.insert("deadzone".into(), num(dz));
    }
    entry
}

// ---------------------------------------------------------------------------------------------
// Storage

/// How [`OverrideStore::load`] found the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    /// The file was read.
    Loaded,
    /// There was no file: the creator's controls apply.
    Missing,
    /// The file could not be read or parsed; it was moved to `moved_to` (when that worked) and
    /// the creator's controls apply.
    Corrupt {
        /// Why it was rejected.
        reason: String,
        /// Where the damaged file now is.
        moved_to: Option<PathBuf>,
    },
}

/// The override file of one game and local profile: `<data dir>/input/<game id>/<profile>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideStore {
    path: PathBuf,
}

impl OverrideStore {
    /// The store of `game_id` (see [`game_id`]) and `profile` under the Player's data folder.
    pub fn new(data_dir: &Path, game_id: &str, profile: &str) -> Self {
        Self {
            path: data_dir
                .join("input")
                .join(game_id)
                .join(format!("{profile}.json")),
        }
    }

    /// A store at exactly this file.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the overrides. A missing file gives none; a damaged one gives none and is renamed
    /// to `<profile>.json.corrupt` so the next save does not destroy what the player may want to
    /// recover.
    pub fn load(&self) -> (PlayerOverrides, LoadStatus) {
        let bytes = match read_capped(&self.path) {
            Ok(Some(b)) => b,
            Ok(None) => {
                // Too large to be ours: set aside like a damaged file, defaults apply.
                let aside = self.path.with_extension("json.corrupt");
                let moved_to = std::fs::rename(&self.path, &aside).ok().map(|()| aside);
                return (
                    PlayerOverrides::default(),
                    LoadStatus::Corrupt {
                        reason: format!("larger than {MAX_FILE_BYTES} bytes"),
                        moved_to,
                    },
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (PlayerOverrides::default(), LoadStatus::Missing);
            }
            Err(e) => {
                return (
                    PlayerOverrides::default(),
                    LoadStatus::Corrupt {
                        reason: e.to_string(),
                        moved_to: None,
                    },
                );
            }
        };
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(PlayerOverrides::from_json);
        match parsed {
            Ok(o) => (o, LoadStatus::Loaded),
            Err(reason) => {
                let aside = self.path.with_extension("json.corrupt");
                let moved_to = std::fs::rename(&self.path, &aside).ok().map(|()| aside);
                (
                    PlayerOverrides::default(),
                    LoadStatus::Corrupt { reason, moved_to },
                )
            }
        }
    }

    /// Writes the overrides atomically (temporary file, then rename). Empty overrides remove the
    /// file instead.
    pub fn save(&self, o: &PlayerOverrides) -> std::io::Result<()> {
        if o.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, o.to_json(&utc_now()))?;
        std::fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

/// Reads at most [`MAX_FILE_BYTES`]; `None` when the file is larger.
fn read_capped(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= MAX_FILE_BYTES).then_some(bytes))
}

/// A folder name for a game: its name in lower case, letters and digits kept, anything else as
/// `-` (at most 48 characters, `game` when empty), then `-` and 8 hex digits of a hash of the
/// exact name. The hash keeps games whose names differ only in case or punctuation apart, and
/// no id can be a Windows reserved name (`CON`, `NUL`, `COM1` ...).
pub fn game_id(name: &str) -> String {
    let mut id = slug(name);
    id.push('-');
    id.push_str(&format!("{:08x}", fnv1a(name)));
    id
}

/// FNV-1a over the UTF-8 bytes: stable across builds and platforms (unlike `DefaultHasher`).
fn fnv1a(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

fn slug(name: &str) -> String {
    let mut id = String::with_capacity(name.len().min(64));
    for ch in name.chars() {
        if id.len() >= 48 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        id.push_str("game");
    }
    id
}

/// The current UTC time as RFC 3339 (`2026-10-30T12:00:00Z`).
pub fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

/// Seconds since 1970 as RFC 3339 in UTC.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}
