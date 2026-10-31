//! The input tree as the panel sees it: contexts, actions and bindings read from the scene,
//! plus the names, devices and labels the matrix shows.
//!
//! Parsing is the runtime's ([`InputConfig::from_world`]), so the panel and the Player never
//! disagree about what a binding means. The touch properties the runtime does not read yet
//! (#252) are read here.

use metallic_core::{InstanceId, UDim2, Value, World};
use metallic_runtime::input::{
    ActionDef, ActionType, BindingDef, CHARACTER_CONTROLS, ContextDef, Control, InputConfig, Role,
    Trigger,
};

use super::commands::Snapshot;

/// Name of the folder new contexts go into (spec 3.1 convention).
pub const INPUT_FOLDER: &str = "Input";

/// The three device columns of the bindings matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceColumn {
    /// Keyboard and mouse.
    KeyboardMouse,
    /// Gamepad.
    Gamepad,
    /// Touch.
    Touch,
}

impl DeviceColumn {
    /// All columns in matrix order.
    pub const ALL: [DeviceColumn; 3] = [
        DeviceColumn::KeyboardMouse,
        DeviceColumn::Gamepad,
        DeviceColumn::Touch,
    ];

    /// Column title.
    pub fn title(self) -> &'static str {
        match self {
            DeviceColumn::KeyboardMouse => "Keyboard and mouse",
            DeviceColumn::Gamepad => "Gamepad",
            DeviceColumn::Touch => "Touch",
        }
    }
}

/// Which device a binding belongs to: a gamepad control anywhere makes it a gamepad binding,
/// a touch control a touch binding; everything else is keyboard and mouse.
pub fn binding_device(b: &BindingDef) -> DeviceColumn {
    let mut device = DeviceColumn::KeyboardMouse;
    for c in b.controls() {
        if c.is_gamepad() {
            return DeviceColumn::Gamepad;
        }
        if matches!(
            c,
            Control::TouchStick | Control::TouchLook | Control::TouchButton
        ) {
            device = DeviceColumn::Touch;
        }
    }
    device
}

/// `Enum.InputActionType` name.
pub fn action_type_name(t: ActionType) -> &'static str {
    match t {
        ActionType::Bool => "Bool",
        ActionType::Direction1D => "Direction1D",
        ActionType::Direction2D => "Direction2D",
        ActionType::Direction3D => "Direction3D",
        ActionType::ViewportPosition => "ViewportPosition",
    }
}

/// `Enum.InputTrigger` name.
pub fn trigger_name(t: Trigger) -> &'static str {
    match t {
        Trigger::Press => "Press",
        Trigger::Release => "Release",
        Trigger::Hold => "Hold",
        Trigger::Tap => "Tap",
        Trigger::DoubleTap => "DoubleTap",
        Trigger::Repeat => "Repeat",
    }
}

/// `Enum.InputRole` name.
pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::None => "None",
        Role::Move => "Move",
        Role::Look => "Look",
        Role::Jump => "Jump",
        Role::Run => "Run",
        Role::Primary => "Primary",
        Role::Secondary => "Secondary",
        Role::Interact => "Interact",
        Role::Menu => "Menu",
        Role::Zoom => "Zoom",
    }
}

/// Every role, in `Enum.InputRole` order.
pub const ROLES: [Role; 10] = [
    Role::None,
    Role::Move,
    Role::Look,
    Role::Jump,
    Role::Run,
    Role::Primary,
    Role::Secondary,
    Role::Interact,
    Role::Menu,
    Role::Zoom,
];

/// Short text for one control, as drawn on a keycap or pad glyph.
pub fn control_label(c: Control) -> &'static str {
    use Control as C;
    match c {
        C::Zero => "0",
        C::One => "1",
        C::Two => "2",
        C::Three => "3",
        C::Four => "4",
        C::Five => "5",
        C::Six => "6",
        C::Seven => "7",
        C::Eight => "8",
        C::Nine => "9",
        C::Return => "Enter",
        C::Backspace => "Bksp",
        C::Escape => "Esc",
        C::LeftShift => "L Shift",
        C::RightShift => "R Shift",
        C::LeftControl => "L Ctrl",
        C::RightControl => "R Ctrl",
        C::LeftAlt => "L Alt",
        C::RightAlt => "R Alt",
        C::Up => "Up",
        C::Down => "Down",
        C::Left => "Left",
        C::Right => "Right",
        C::PageUp => "PgUp",
        C::PageDown => "PgDn",
        C::Insert => "Ins",
        C::Delete => "Del",
        C::Comma => ",",
        C::Period => ".",
        C::Slash => "/",
        C::Semicolon => ";",
        C::Quote => "'",
        C::LeftBracket => "[",
        C::RightBracket => "]",
        C::BackSlash => "\\",
        C::Minus => "-",
        C::Equals => "=",
        C::Backquote => "`",
        C::MouseButton1 => "Left click",
        C::MouseButton2 => "Right click",
        C::MouseButton3 => "Middle click",
        C::MouseWheel => "Wheel",
        C::MouseDelta => "Mouse move",
        C::MousePosition => "Pointer",
        C::ButtonA => "A",
        C::ButtonB => "B",
        C::ButtonX => "X",
        C::ButtonY => "Y",
        C::ButtonL1 => "L1",
        C::ButtonR1 => "R1",
        C::ButtonL2 => "L2",
        C::ButtonR2 => "R2",
        C::ButtonL3 => "L3",
        C::ButtonR3 => "R3",
        C::ButtonSelect => "Select",
        C::ButtonStart => "Start",
        C::DPadUp => "D-pad up",
        C::DPadDown => "D-pad down",
        C::DPadLeft => "D-pad left",
        C::DPadRight => "D-pad right",
        C::Thumbstick1 => "Left stick",
        C::Thumbstick2 => "Right stick",
        C::TouchStick => "Touch stick",
        C::TouchLook => "Look pad",
        C::TouchButton => "Touch button",
        other => other.name(),
    }
}

/// Text for a whole binding: `Shift + W`, `W A S D`, `D-pad`, `Left stick`.
pub fn binding_label(b: &BindingDef) -> String {
    let mut parts: Vec<String> = b
        .modifiers
        .iter()
        .flatten()
        .map(|&m| control_label(m).to_string())
        .collect();
    if let Some(k) = b.key {
        parts.push(control_label(k).to_string());
    } else {
        let keys: Vec<Control> = b.composite.iter().flatten().copied().collect();
        if keys
            == [
                Control::DPadUp,
                Control::DPadDown,
                Control::DPadLeft,
                Control::DPadRight,
            ]
        {
            parts.push("D-pad".to_string());
        } else if !keys.is_empty() {
            // Composite in reading order: up, left, down, right (W A S D).
            let order = [0usize, 2, 1, 3, 4, 5];
            let names: Vec<&str> = order
                .iter()
                .filter_map(|&i| b.composite[i])
                .map(control_label)
                .collect();
            parts.push(names.join(" "));
        }
    }
    if parts.is_empty() {
        "Unbound".to_string()
    } else {
        parts.join(" + ")
    }
}

/// How an action's touch control is placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchProps {
    /// `TouchSlot`: "Auto", "Stick" or "Look".
    pub slot: TouchSlot,
    /// `TouchLayout` is `Custom`.
    pub custom: bool,
    /// `TouchPosition`.
    pub position: UDim2,
    /// `TouchSize` in dp (0 = automatic).
    pub size: f32,
}

/// `Enum.TouchSlot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchSlot {
    /// First Direction2D action gets the stick, the second the look pad.
    Auto,
    /// The left thumbstick.
    Stick,
    /// The look pad.
    Look,
}

impl Default for TouchProps {
    fn default() -> Self {
        Self {
            slot: TouchSlot::Auto,
            custom: false,
            position: UDim2::new(0.0, 0.0, 0.0, 0.0),
            size: 0.0,
        }
    }
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// A `TouchPosition` with every non-finite component replaced by 0 (scene files can carry them).
pub fn sanitize_udim2(u: UDim2) -> UDim2 {
    UDim2::new(
        finite_or_zero(u.x_scale),
        finite_or_zero(u.x_offset),
        finite_or_zero(u.y_scale),
        finite_or_zero(u.y_offset),
    )
}

/// Reads the touch properties of an `InputAction` (defaults for built-in actions).
pub fn touch_props(world: &World, id: Option<InstanceId>) -> TouchProps {
    let Some(id) = id else {
        return TouchProps::default();
    };
    let slot = match world.get_ref(id, "TouchSlot") {
        Ok(Value::String(s)) if s == "Stick" => TouchSlot::Stick,
        Ok(Value::String(s)) if s == "Look" => TouchSlot::Look,
        _ => TouchSlot::Auto,
    };
    let custom = matches!(world.get_ref(id, "TouchLayout"), Ok(Value::String(s)) if s == "Custom");
    let position = match world.get_ref(id, "TouchPosition") {
        Ok(Value::UDim2(u)) => sanitize_udim2(*u),
        _ => UDim2::new(0.0, 0.0, 0.0, 0.0),
    };
    let size = match world.get_ref(id, "TouchSize") {
        Ok(Value::Number(n)) if n.is_finite() => *n as f32,
        _ => 0.0,
    };
    TouchProps {
        slot,
        custom,
        position,
        size,
    }
}

/// Reads a String property, empty when missing.
pub fn prop_string(world: &World, id: InstanceId, prop: &str) -> String {
    match world.get_ref(id, prop) {
        Ok(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// The input tree of a world.
#[derive(Debug, Clone)]
pub struct InputModel {
    /// What the runtime would run, in tree order. The built-in `CharacterControls` is included
    /// (with no instance ids) when the scene does not define its own.
    pub config: InputConfig,
}

impl InputModel {
    /// Reads the world.
    pub fn read(world: &World) -> Self {
        Self {
            config: InputConfig::from_world(world),
        }
    }

    /// True when the scene has no `CharacterControls` and the built-in one is shown.
    pub fn uses_builtin_character_controls(&self) -> bool {
        self.config
            .contexts
            .iter()
            .any(|c| c.name == CHARACTER_CONTROLS && c.instance.is_none())
    }

    /// The action read from instance `id`.
    pub fn action(&self, id: InstanceId) -> Option<(&ContextDef, &ActionDef)> {
        self.config.contexts.iter().find_map(|c| {
            c.actions
                .iter()
                .find(|a| a.instance == Some(id))
                .map(|a| (c, a))
        })
    }

    /// The context read from instance `id`.
    pub fn context(&self, id: InstanceId) -> Option<&ContextDef> {
        self.config.contexts.iter().find(|c| c.instance == Some(id))
    }

    /// The binding read from instance `id`, with its action.
    pub fn binding(&self, id: InstanceId) -> Option<(&ActionDef, &BindingDef)> {
        self.config.contexts.iter().find_map(|c| {
            c.actions.iter().find_map(|a| {
                a.bindings
                    .iter()
                    .find(|b| b.instance == Some(id))
                    .map(|b| (a, b))
            })
        })
    }

    /// Every action with its context, in tree order.
    pub fn actions(&self) -> impl Iterator<Item = (&ContextDef, &ActionDef)> {
        self.config
            .contexts
            .iter()
            .flat_map(|c| c.actions.iter().map(move |a| (c, a)))
    }
}

/// `Context/Action` path for every scene action (the key players' overrides use).
pub fn action_paths(world: &World) -> Vec<(InstanceId, String)> {
    let model = InputModel::read(world);
    model
        .actions()
        .filter_map(|(c, a)| a.instance.map(|id| (id, format!("{}/{}", c.name, a.name))))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Runtime definitions to instance snapshots

fn control_value(c: Option<Control>) -> Value {
    Value::String(c.map_or("None", Control::name).to_string())
}

/// An `InputBinding` snapshot for a runtime binding definition.
pub fn binding_snapshot(b: &BindingDef) -> Snapshot {
    let d = BindingDef::default();
    let mut s = Snapshot::new("InputBinding");
    if b.key.is_some() {
        s = s.with("KeyCode", control_value(b.key));
    }
    for (i, prop) in ["Up", "Down", "Left", "Right", "Forward", "Backward"]
        .iter()
        .enumerate()
    {
        if b.composite[i].is_some() {
            s = s.with(prop, control_value(b.composite[i]));
        }
    }
    for (i, prop) in ["PrimaryModifier", "SecondaryModifier"].iter().enumerate() {
        if b.modifiers[i].is_some() {
            s = s.with(prop, control_value(b.modifiers[i]));
        }
    }
    let num = |v: f32| Value::Number(f64::from(v));
    if b.scale != d.scale {
        s = s.with("Scale", num(b.scale));
    }
    if b.vector2_scale != d.vector2_scale {
        s = s.with(
            "Vector2Scale",
            Value::Vector3(metallic_core::Vec3::new(
                f64::from(b.vector2_scale[0]),
                f64::from(b.vector2_scale[1]),
                0.0,
            )),
        );
    }
    if b.response_curve != d.response_curve {
        s = s.with("ResponseCurve", num(b.response_curve));
    }
    if let Some(dz) = b.deadzone {
        s = s.with("Deadzone", num(dz));
    }
    if b.scale_by_dt {
        s = s.with("ScaleByDeltaTime", Value::Bool(true));
    }
    if !b.clamp_magnitude {
        s = s.with("ClampMagnitudeToOne", Value::Bool(false));
    }
    s
}

/// An `InputAction` snapshot (with its bindings) for a runtime action definition.
pub fn action_snapshot(a: &ActionDef) -> Snapshot {
    let mut s = Snapshot::new("InputAction")
        .with("Name", Value::String(a.name.clone()))
        .with("Type", Value::String(action_type_name(a.ty).into()));
    if a.trigger != Trigger::Press {
        s = s.with("Trigger", Value::String(trigger_name(a.trigger).into()));
    }
    if a.role != Role::None {
        s = s.with("Role", Value::String(role_name(a.role).into()));
    }
    if !a.display_name.is_empty() {
        s = s.with("DisplayName", Value::String(a.display_name.clone()));
    }
    if a.show_on_touch {
        s = s.with("ShowOnTouch", Value::Bool(true));
    }
    if a.touch_order != 0.0 {
        s = s.with("TouchOrder", Value::Number(f64::from(a.touch_order)));
    }
    if !a.can_toggle {
        s = s.with("CanToggle", Value::Bool(false));
    }
    for b in &a.bindings {
        s = s.child(binding_snapshot(b));
    }
    s
}

/// An `InputContext` snapshot (with actions and bindings) for a runtime context definition.
pub fn context_snapshot(c: &ContextDef) -> Snapshot {
    let mut s = Snapshot::new("InputContext").with("Name", Value::String(c.name.clone()));
    if c.priority != 0.0 {
        s = s.with("Priority", Value::Number(c.priority));
    }
    if c.sink {
        s = s.with("Sink", Value::Bool(true));
    }
    if !c.enabled {
        s = s.with("Enabled", Value::Bool(false));
    }
    for a in &c.actions {
        s = s.child(action_snapshot(a));
    }
    s
}

/// A name not used by any child of `parent`: `base`, then `base2`, `base3`, ...
pub fn unique_child_name(world: &World, parent: InstanceId, base: &str) -> String {
    let taken = |n: &str| world.find_first_child(parent, n).is_some();
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_runtime::input::default_character_controls;

    #[test]
    fn non_finite_touch_position_reads_as_zero() {
        let nan = f64::NAN;
        let inf = f64::INFINITY;
        assert_eq!(
            sanitize_udim2(UDim2::new(nan, 12.0, 0.5, inf)),
            UDim2::new(0.0, 12.0, 0.5, 0.0)
        );
        assert_eq!(
            sanitize_udim2(UDim2::new(f64::NEG_INFINITY, nan, nan, nan)),
            UDim2::new(0.0, 0.0, 0.0, 0.0)
        );
        // The world refuses non-finite values; a finite one passes through untouched.
        let mut w = World::new_with_services();
        let rs = w.root("ReplicatedStorage").unwrap();
        let ctx = w.create("InputContext", Some(rs)).unwrap();
        let a = w.create("InputAction", Some(ctx)).unwrap();
        w.set(
            a,
            "TouchPosition",
            Value::UDim2(UDim2::new(0.25, 4.0, 0.5, -8.0)),
        )
        .unwrap();
        assert_eq!(
            touch_props(&w, Some(a)).position,
            UDim2::new(0.25, 4.0, 0.5, -8.0)
        );
    }

    #[test]
    fn devices_of_bindings() {
        assert_eq!(
            binding_device(&BindingDef::key(Control::Space)),
            DeviceColumn::KeyboardMouse
        );
        assert_eq!(
            binding_device(&BindingDef::key(Control::ButtonA)),
            DeviceColumn::Gamepad
        );
        assert_eq!(
            binding_device(&BindingDef::key(Control::TouchStick)),
            DeviceColumn::Touch
        );
        assert_eq!(
            binding_device(
                &BindingDef::key(Control::MouseDelta).with_modifier(Control::MouseButton2)
            ),
            DeviceColumn::KeyboardMouse
        );
    }

    #[test]
    fn labels() {
        use Control as C;
        assert_eq!(
            binding_label(&BindingDef::composite(C::W, C::S, C::A, C::D)),
            "W A S D"
        );
        assert_eq!(
            binding_label(&BindingDef::composite(
                C::DPadUp,
                C::DPadDown,
                C::DPadLeft,
                C::DPadRight
            )),
            "D-pad"
        );
        assert_eq!(
            binding_label(&BindingDef::key(C::W).with_modifier(C::LeftShift)),
            "L Shift + W"
        );
        assert_eq!(binding_label(&BindingDef::default()), "Unbound");
    }

    #[test]
    fn default_context_round_trips_through_instances() {
        let mut w = World::new_with_services();
        let rs = w.root("ReplicatedStorage").unwrap();
        let mut b = super::super::commands::StepBuilder::new(&mut w);
        b.create(rs, context_snapshot(&default_character_controls()))
            .unwrap();
        let _ = b.finish("x");
        let model = InputModel::read(&w);
        assert!(!model.uses_builtin_character_controls());
        let scene = &model.config.contexts[0];
        let builtin = default_character_controls();
        assert_eq!(scene.actions.len(), builtin.actions.len());
        for (a, b) in scene.actions.iter().zip(&builtin.actions) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.ty, b.ty);
            assert_eq!(a.role, b.role);
            assert_eq!(a.show_on_touch, b.show_on_touch);
            let strip = |x: &BindingDef| BindingDef {
                instance: None,
                ..x.clone()
            };
            let left: Vec<BindingDef> = a.bindings.iter().map(strip).collect();
            assert_eq!(left, b.bindings, "bindings of {}", a.name);
        }
    }
}
