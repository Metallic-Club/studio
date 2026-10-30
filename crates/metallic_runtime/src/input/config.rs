//! What the scene says about input: contexts, actions and bindings.
//!
//! [`InputConfig::from_world`] reads the `InputContext`, `InputAction` and `InputBinding`
//! instances (docs/design/input-actions.md section 3); [`InputConfig::character_controls`] is the
//! built-in `CharacterControls` context (section 6.1). The runtime compiles a config into dense
//! arrays ([`InputRuntime::load`](super::InputRuntime::load)).
use super::control::Control;
use metallic_core::{InstanceId, Value, World};

/// Name of the context that feeds the character and the camera.
pub const CHARACTER_CONTROLS: &str = "CharacterControls";

/// Radians the camera turns per pixel of right-drag mouse movement.
pub const MOUSE_ORBIT_SPEED: f32 = 0.006;
/// Radians per second the camera turns with the right stick fully tilted.
pub const STICK_ORBIT_SPEED: f32 = 3.0;
/// Studs the camera moves in or out per wheel notch.
pub const ZOOM_PER_NOTCH: f32 = 2.0;
/// Studs per second the camera moves with the gamepad shoulder buttons held (a start value).
pub const PAD_ZOOM_SPEED: f32 = 12.0;

/// The value an action holds (`Enum.InputActionType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    /// On or off.
    Bool,
    /// One axis.
    Direction1D,
    /// Two axes (x right, y up or forward).
    Direction2D,
    /// Three axes.
    Direction3D,
    /// A position in pixels.
    ViewportPosition,
}

impl ActionType {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "Bool" => Self::Bool,
            "Direction1D" => Self::Direction1D,
            "Direction2D" => Self::Direction2D,
            "Direction3D" => Self::Direction3D,
            "ViewportPosition" => Self::ViewportPosition,
            _ => return None,
        })
    }
}

/// When an action fires `Triggered` (spec 3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// On the press edge.
    Press,
    /// On the release edge.
    Release,
    /// Once, when the press has lasted `hold_time`.
    Hold,
    /// On release, if the press lasted at most `tap_time`.
    Tap,
    /// On the release of a second tap within `double_tap_window` of the first.
    DoubleTap,
    /// On the press edge, then after `repeat_delay`, then every `repeat_rate` while down.
    Repeat,
}

impl Trigger {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "Press" => Self::Press,
            "Release" => Self::Release,
            "Hold" => Self::Hold,
            "Tap" => Self::Tap,
            "DoubleTap" => Self::DoubleTap,
            "Repeat" => Self::Repeat,
            _ => return None,
        })
    }
}

/// What an action is for; presets and prompts look actions up by role (spec 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// No role.
    None,
    /// Walk.
    Move,
    /// Turn the camera.
    Look,
    /// Jump.
    Jump,
    /// Run.
    Run,
    /// Main action.
    Primary,
    /// Second action.
    Secondary,
    /// Interact with a prompt.
    Interact,
    /// Open the menu.
    Menu,
    /// Camera zoom.
    Zoom,
}

impl Role {
    fn from_name(s: &str) -> Self {
        match s {
            "Move" => Self::Move,
            "Look" => Self::Look,
            "Jump" => Self::Jump,
            "Run" => Self::Run,
            "Primary" => Self::Primary,
            "Secondary" => Self::Secondary,
            "Interact" => Self::Interact,
            "Menu" => Self::Menu,
            "Zoom" => Self::Zoom,
            _ => Self::None,
        }
    }
}

/// One `InputBinding`: how a control (or a composite of controls) feeds its action.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingDef {
    /// The single control (`KeyCode`).
    pub key: Option<Control>,
    /// Composite keys in the order Up, Down, Left, Right, Forward, Backward.
    pub composite: [Option<Control>; 6],
    /// `PrimaryModifier` and `SecondaryModifier`: controls that must be down.
    pub modifiers: [Option<Control>; 2],
    /// `Scale`.
    pub scale: f32,
    /// `Vector2Scale` (per axis).
    pub vector2_scale: [f32; 2],
    /// `Vector3Scale` (per axis).
    pub vector3_scale: [f32; 3],
    /// `ResponseCurve`: output = sign(v) * abs(v)^curve.
    pub response_curve: f32,
    /// `Deadzone`; `None` uses the control's default (0.18 on sticks).
    pub deadzone: Option<f32>,
    /// `ScaleByDeltaTime`.
    pub scale_by_dt: bool,
    /// `ClampMagnitudeToOne` (applies to sticks and composites).
    pub clamp_magnitude: bool,
    /// An analog control counts as pressed above this.
    pub pressed_threshold: f32,
    /// ... and as released below this.
    pub released_threshold: f32,
}

impl Default for BindingDef {
    fn default() -> Self {
        Self {
            key: None,
            composite: [None; 6],
            modifiers: [None; 2],
            scale: 1.0,
            vector2_scale: [1.0; 2],
            vector3_scale: [1.0; 3],
            response_curve: 1.0,
            deadzone: None,
            scale_by_dt: false,
            clamp_magnitude: true,
            pressed_threshold: 0.5,
            released_threshold: 0.4,
        }
    }
}

impl BindingDef {
    /// A binding on one control.
    pub fn key(control: Control) -> Self {
        Self {
            key: Some(control),
            ..Self::default()
        }
    }

    /// A composite of four keys: Up, Down, Left, Right.
    pub fn composite(up: Control, down: Control, left: Control, right: Control) -> Self {
        Self {
            composite: [Some(up), Some(down), Some(left), Some(right), None, None],
            ..Self::default()
        }
    }

    /// Requires `control` to be down.
    pub fn with_modifier(mut self, control: Control) -> Self {
        let slot = usize::from(self.modifiers[0].is_some());
        self.modifiers[slot.min(1)] = Some(control);
        self
    }

    /// Sets `Scale`.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Sets `Vector2Scale`.
    pub fn with_vector2_scale(mut self, scale: [f32; 2]) -> Self {
        self.vector2_scale = scale;
        self
    }

    /// Sets `ScaleByDeltaTime`.
    pub fn per_second(mut self) -> Self {
        self.scale_by_dt = true;
        self
    }

    /// Sets `Deadzone`.
    pub fn with_deadzone(mut self, deadzone: f32) -> Self {
        self.deadzone = Some(deadzone);
        self
    }

    /// Sets `ClampMagnitudeToOne`.
    pub fn with_clamp(mut self, clamp: bool) -> Self {
        self.clamp_magnitude = clamp;
        self
    }

    /// Sets `ResponseCurve`.
    pub fn with_curve(mut self, curve: f32) -> Self {
        self.response_curve = curve;
        self
    }

    /// True when this binding reads `control` (as key, composite part or modifier).
    pub fn reads(&self, control: Control) -> bool {
        self.controls().any(|c| c == control)
    }

    /// Every control this binding reads.
    pub fn controls(&self) -> impl Iterator<Item = Control> + '_ {
        self.key
            .into_iter()
            .chain(self.composite.iter().flatten().copied())
            .chain(self.modifiers.iter().flatten().copied())
    }
}

/// One `InputAction` with its bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionDef {
    /// Name; with the context name it forms the stable path `Context/Action`.
    pub name: String,
    /// Value type.
    pub ty: ActionType,
    /// Disabled actions read zero and fire nothing.
    pub enabled: bool,
    /// When `Triggered` fires (Bool actions).
    pub trigger: Trigger,
    /// Hold trigger: milliseconds.
    pub hold_time: f32,
    /// Tap and DoubleTap: longest press that still counts, milliseconds.
    pub tap_time: f32,
    /// DoubleTap: longest gap between two taps, milliseconds.
    pub double_tap_window: f32,
    /// Repeat: wait before the first repeat, milliseconds.
    pub repeat_delay: f32,
    /// Repeat: gap between repeats, milliseconds.
    pub repeat_rate: f32,
    /// What the action is for.
    pub role: Role,
    /// Text for glyph and rebind pages.
    pub display_name: String,
    /// Shows in the player's Controls page.
    pub rebindable: bool,
    /// The player cannot leave it without a binding.
    pub required: bool,
    /// A held Bool action may become a toggle.
    pub can_toggle: bool,
    /// Shows an on-screen control.
    pub show_on_touch: bool,
    /// Lower comes first among touch buttons.
    pub touch_order: f32,
    /// The bindings, in tree order.
    pub bindings: Vec<BindingDef>,
    /// The instance this came from (`None` for built-in actions).
    pub instance: Option<InstanceId>,
}

impl ActionDef {
    /// An action with the default trigger values and no bindings.
    pub fn new(name: &str, ty: ActionType) -> Self {
        Self {
            name: name.to_string(),
            ty,
            enabled: true,
            trigger: Trigger::Press,
            hold_time: 500.0,
            tap_time: 200.0,
            double_tap_window: 300.0,
            repeat_delay: 400.0,
            repeat_rate: 100.0,
            role: Role::None,
            display_name: String::new(),
            rebindable: true,
            required: false,
            can_toggle: true,
            show_on_touch: false,
            touch_order: 0.0,
            bindings: Vec::new(),
            instance: None,
        }
    }

    /// Adds a binding.
    pub fn with_binding(mut self, binding: BindingDef) -> Self {
        self.bindings.push(binding);
        self
    }

    /// Sets the trigger.
    pub fn with_trigger(mut self, trigger: Trigger) -> Self {
        self.trigger = trigger;
        self
    }

    /// Sets the role.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }
}

/// One `InputContext` with its actions.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextDef {
    /// Name.
    pub name: String,
    /// Disabled contexts are skipped.
    pub enabled: bool,
    /// Higher is asked first.
    pub priority: f64,
    /// A control this context's bindings use is not offered to lower contexts.
    pub sink: bool,
    /// Actions of one group are checked against each other for rebind conflicts.
    pub rebind_group: String,
    /// The actions.
    pub actions: Vec<ActionDef>,
    /// The instance this came from (`None` for built-in contexts).
    pub instance: Option<InstanceId>,
    /// Orders contexts of equal priority: the higher number is asked first (the context that was
    /// enabled most recently). Tree order at load time.
    pub order: u64,
}

impl ContextDef {
    /// An enabled context with priority 0 and no actions.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            enabled: true,
            priority: 0.0,
            sink: false,
            rebind_group: "Game".to_string(),
            actions: Vec::new(),
            instance: None,
            order: 0,
        }
    }
}

/// All input contexts of a game.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputConfig {
    /// The contexts.
    pub contexts: Vec<ContextDef>,
    /// Problems found while reading the scene (unknown control names, bad values).
    pub warnings: Vec<String>,
}

impl InputConfig {
    /// A config that holds only the built-in `CharacterControls`.
    pub fn character_controls() -> Self {
        Self {
            contexts: vec![default_character_controls()],
            warnings: Vec::new(),
        }
    }

    /// Reads every `InputContext` under `ReplicatedStorage`, `StarterPlayer` and `Players`
    /// (spec 3.1). When no context named `CharacterControls` exists, the built-in one is added;
    /// when one exists the scene wins entirely (spec 6.3).
    pub fn from_world(world: &World) -> Self {
        let mut config = InputConfig::default();
        for service in ["ReplicatedStorage", "StarterPlayer", "Players"] {
            let Some(root) = world.root(service) else {
                continue;
            };
            let mut stack = vec![root];
            // Depth-first in tree order.
            while let Some(id) = stack.pop() {
                if world.class_name(id) == Some("InputContext") {
                    let order = config.contexts.len() as u64;
                    let mut ctx = read_context(world, id, &mut config.warnings);
                    ctx.order = order;
                    config.contexts.push(ctx);
                    continue;
                }
                stack.extend(world.children(id).iter().rev());
            }
        }
        if !config.contexts.iter().any(|c| c.name == CHARACTER_CONTROLS) {
            let mut ctx = default_character_controls();
            ctx.order = config.contexts.len() as u64;
            config.contexts.push(ctx);
        }
        config
    }

    /// The instances this config was read from (to know which journal changes matter).
    pub fn instances(&self) -> Vec<InstanceId> {
        let mut ids = Vec::new();
        for c in &self.contexts {
            ids.extend(c.instance);
            for a in &c.actions {
                ids.extend(a.instance);
            }
        }
        ids
    }
}

/// The built-in `CharacterControls` context (spec 6.1).
pub fn default_character_controls() -> ContextDef {
    use Control as C;
    let mut ctx = ContextDef::new(CHARACTER_CONTROLS);
    let mut mv = ActionDef::new("Move", ActionType::Direction2D).with_role(Role::Move);
    mv.show_on_touch = true;
    mv = mv
        .with_binding(BindingDef::composite(C::W, C::S, C::A, C::D))
        .with_binding(BindingDef::composite(C::Up, C::Down, C::Left, C::Right))
        .with_binding(BindingDef::key(C::Thumbstick1))
        .with_binding(BindingDef::composite(
            C::DPadUp,
            C::DPadDown,
            C::DPadLeft,
            C::DPadRight,
        ));
    let mut jump = ActionDef::new("Jump", ActionType::Bool)
        .with_role(Role::Jump)
        .with_binding(BindingDef::key(C::Space))
        .with_binding(BindingDef::key(C::ButtonA));
    jump.show_on_touch = true;
    let mut run = ActionDef::new("Run", ActionType::Bool)
        .with_role(Role::Run)
        .with_binding(BindingDef::key(C::LeftShift))
        .with_binding(BindingDef::key(C::RightShift))
        .with_binding(BindingDef::key(C::ButtonL3));
    run.show_on_touch = true;
    run.touch_order = 1.0;
    let look = ActionDef::new("Look", ActionType::Direction2D)
        .with_role(Role::Look)
        .with_binding(
            BindingDef::key(C::MouseDelta)
                .with_modifier(C::MouseButton2)
                .with_scale(MOUSE_ORBIT_SPEED)
                .with_vector2_scale([1.0, -1.0]),
        )
        .with_binding(
            BindingDef::key(C::Thumbstick2)
                .with_scale(STICK_ORBIT_SPEED)
                .per_second()
                .with_clamp(false),
        );
    let zoom = ActionDef::new("Zoom", ActionType::Direction1D)
        .with_role(Role::Zoom)
        .with_binding(BindingDef::key(C::MouseWheel).with_scale(-ZOOM_PER_NOTCH))
        .with_binding(
            BindingDef::key(C::ButtonR1)
                .with_scale(PAD_ZOOM_SPEED)
                .per_second(),
        )
        .with_binding(
            BindingDef::key(C::ButtonL1)
                .with_scale(-PAD_ZOOM_SPEED)
                .per_second(),
        );
    ctx.actions = vec![mv, jump, run, look, zoom];
    ctx
}

// ---------------------------------------------------------------------------------------------
// Reading instances

fn prop_bool(w: &World, id: InstanceId, name: &str, default: bool) -> bool {
    match w.get(id, name) {
        Ok(Value::Bool(b)) => b,
        _ => default,
    }
}

fn prop_num(w: &World, id: InstanceId, name: &str, default: f64) -> f64 {
    match w.get(id, name) {
        Ok(Value::Number(n)) if n.is_finite() => n,
        _ => default,
    }
}

fn prop_str(w: &World, id: InstanceId, name: &str) -> String {
    match w.get(id, name) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn prop_vec(w: &World, id: InstanceId, name: &str, default: [f32; 3]) -> [f32; 3] {
    match w.get(id, name) {
        Ok(Value::Vector3(v)) => [v.x as f32, v.y as f32, v.z as f32],
        _ => default,
    }
}

fn read_context(w: &World, id: InstanceId, warnings: &mut Vec<String>) -> ContextDef {
    let mut ctx = ContextDef::new(&prop_str(w, id, "Name"));
    ctx.enabled = prop_bool(w, id, "Enabled", true);
    ctx.priority = prop_num(w, id, "Priority", 0.0);
    ctx.sink = prop_bool(w, id, "Sink", false);
    ctx.rebind_group = prop_str(w, id, "RebindGroup");
    ctx.instance = Some(id);
    for &child in w.children(id) {
        if w.class_name(child) == Some("InputAction") {
            let action = read_action(w, child, &ctx.name, warnings);
            ctx.actions.push(action);
        }
    }
    ctx
}

fn read_action(w: &World, id: InstanceId, context: &str, warnings: &mut Vec<String>) -> ActionDef {
    let name = prop_str(w, id, "Name");
    let ty_name = prop_str(w, id, "Type");
    let ty = ActionType::from_name(&ty_name).unwrap_or_else(|| {
        warnings.push(format!("{context}/{name}: unknown Type `{ty_name}`"));
        ActionType::Bool
    });
    let mut a = ActionDef::new(&name, ty);
    a.enabled = prop_bool(w, id, "Enabled", true);
    let trigger = prop_str(w, id, "Trigger");
    a.trigger = Trigger::from_name(&trigger).unwrap_or_else(|| {
        warnings.push(format!("{context}/{name}: unknown Trigger `{trigger}`"));
        Trigger::Press
    });
    let ms = |p: &str, d: f64| prop_num(w, id, p, d).max(0.0) as f32;
    a.hold_time = ms("HoldTime", 500.0);
    a.tap_time = ms("TapTime", 200.0);
    a.double_tap_window = ms("DoubleTapWindow", 300.0);
    a.repeat_delay = ms("RepeatDelay", 400.0);
    a.repeat_rate = ms("RepeatRate", 100.0).max(1.0);
    a.role = Role::from_name(&prop_str(w, id, "Role"));
    a.display_name = prop_str(w, id, "DisplayName");
    a.rebindable = prop_bool(w, id, "Rebindable", true);
    a.required = prop_bool(w, id, "Required", false);
    a.can_toggle = prop_bool(w, id, "CanToggle", true);
    a.show_on_touch = prop_bool(w, id, "ShowOnTouch", false);
    a.touch_order = prop_num(w, id, "TouchOrder", 0.0) as f32;
    a.instance = Some(id);
    for &child in w.children(id) {
        if w.class_name(child) == Some("InputBinding") {
            a.bindings.push(read_binding(
                w,
                child,
                &format!("{context}/{name}"),
                warnings,
            ));
        }
    }
    a
}

fn read_binding(w: &World, id: InstanceId, path: &str, warnings: &mut Vec<String>) -> BindingDef {
    let mut control = |prop: &str| -> Option<Control> {
        let name = prop_str(w, id, prop);
        if name.is_empty() || name == "None" {
            return None;
        }
        let found = Control::from_name(&name);
        if found.is_none() {
            warnings.push(format!("{path}: unknown control `{name}` in {prop}"));
        }
        found
    };
    let mut b = BindingDef {
        key: control("KeyCode"),
        composite: [
            control("Up"),
            control("Down"),
            control("Left"),
            control("Right"),
            control("Forward"),
            control("Backward"),
        ],
        modifiers: [control("PrimaryModifier"), control("SecondaryModifier")],
        ..BindingDef::default()
    };
    b.scale = prop_num(w, id, "Scale", 1.0) as f32;
    let v2 = prop_vec(w, id, "Vector2Scale", [1.0, 1.0, 0.0]);
    b.vector2_scale = [v2[0], v2[1]];
    b.vector3_scale = prop_vec(w, id, "Vector3Scale", [1.0; 3]);
    b.response_curve = prop_num(w, id, "ResponseCurve", 1.0).max(0.01) as f32;
    let dz = prop_num(w, id, "Deadzone", -1.0);
    b.deadzone = (dz >= 0.0).then_some(dz.min(0.99) as f32);
    b.scale_by_dt = prop_bool(w, id, "ScaleByDeltaTime", false);
    b.clamp_magnitude = prop_bool(w, id, "ClampMagnitudeToOne", true);
    b.pressed_threshold = prop_num(w, id, "PressedThreshold", 0.5) as f32;
    b.released_threshold = prop_num(w, id, "ReleasedThreshold", 0.4) as f32;
    b
}
