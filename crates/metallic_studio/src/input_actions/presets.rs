//! Role presets.
//!
//! Two uses of an action's `Role`:
//!
//! - **Add from role**: a new action with the role's type, name and default bindings for keyboard
//!   and gamepad (for Move, Look, Jump, Run and Zoom these are the built-in `CharacterControls`
//!   bindings, so a game that adds them gets what Parkour has).
//! - **Binding presets**: replace the bindings of one device for every action that has a role,
//!   following the one-handed layouts of spec 11.3 or the defaults. Actions without a role, or
//!   whose role the preset does not cover, are left unchanged and listed.
//!
//! Both are one undo step.

use metallic_core::{CoreError, InstanceId, Value, World};
use metallic_runtime::input::{
    ActionType, BindingDef, Control, PAD_ZOOM_SPEED, Role, STICK_ORBIT_SPEED,
    default_character_controls,
};

use super::commands::{Snapshot, StepBuilder};
use super::model::{
    DeviceColumn, InputModel, action_type_name, binding_device, binding_snapshot, role_name,
    unique_child_name,
};

/// A binding preset for one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingPreset {
    /// The built-in keyboard and mouse bindings.
    KeyboardDefault,
    /// Keyboard, left hand (W A S D side).
    KeyboardLeftHand,
    /// Keyboard, right hand (arrows and numpad side).
    KeyboardRightHand,
    /// The built-in gamepad bindings.
    GamepadDefault,
    /// Gamepad, left hand.
    GamepadLeftHand,
    /// Gamepad, right hand.
    GamepadRightHand,
}

impl BindingPreset {
    /// Every preset, in menu order.
    pub const ALL: [BindingPreset; 6] = [
        BindingPreset::KeyboardDefault,
        BindingPreset::KeyboardLeftHand,
        BindingPreset::KeyboardRightHand,
        BindingPreset::GamepadDefault,
        BindingPreset::GamepadLeftHand,
        BindingPreset::GamepadRightHand,
    ];

    /// Menu text.
    pub fn label(self) -> &'static str {
        match self {
            BindingPreset::KeyboardDefault => "Keyboard: default",
            BindingPreset::KeyboardLeftHand => "Keyboard: left hand",
            BindingPreset::KeyboardRightHand => "Keyboard: right hand",
            BindingPreset::GamepadDefault => "Gamepad: default",
            BindingPreset::GamepadLeftHand => "Gamepad: left hand",
            BindingPreset::GamepadRightHand => "Gamepad: right hand",
        }
    }

    /// The device whose bindings the preset replaces.
    pub fn device(self) -> DeviceColumn {
        match self {
            BindingPreset::KeyboardDefault
            | BindingPreset::KeyboardLeftHand
            | BindingPreset::KeyboardRightHand => DeviceColumn::KeyboardMouse,
            _ => DeviceColumn::Gamepad,
        }
    }

    /// The bindings for a role; `None` leaves actions of that role unchanged.
    pub fn bindings(self, role: Role) -> Option<Vec<BindingDef>> {
        use BindingPreset as P;
        use Control as C;
        let turn = |up, down, left, right| {
            BindingDef::composite(up, down, left, right)
                .with_scale(STICK_ORBIT_SPEED)
                .per_second()
        };
        let zoom = |out: Control, inn: Control| {
            vec![
                BindingDef::key(out).with_scale(PAD_ZOOM_SPEED).per_second(),
                BindingDef::key(inn)
                    .with_scale(-PAD_ZOOM_SPEED)
                    .per_second(),
            ]
        };
        let v = match (self, role) {
            (P::KeyboardDefault | P::GamepadDefault, r) => {
                let device = self.device();
                let found: Vec<BindingDef> = default_bindings(r)
                    .into_iter()
                    .filter(|b| binding_device(b) == device)
                    .collect();
                if found.is_empty() {
                    return None;
                }
                found
            }
            (P::KeyboardLeftHand, Role::Move) => {
                vec![BindingDef::composite(C::W, C::S, C::A, C::D)]
            }
            // Q/E turn, R/F pitch (y is up, so R looks up).
            (P::KeyboardLeftHand, Role::Look) => vec![turn(C::R, C::F, C::Q, C::E)],
            (P::KeyboardLeftHand, Role::Jump) => vec![BindingDef::key(C::Space)],
            (P::KeyboardLeftHand, Role::Run) => vec![BindingDef::key(C::LeftShift)],
            (P::KeyboardLeftHand, Role::Zoom) => zoom(C::Z, C::X),
            (P::KeyboardRightHand, Role::Move) => {
                vec![BindingDef::composite(C::Up, C::Down, C::Left, C::Right)]
            }
            // The control set has no numpad keys yet; the keys right of the letters stand in.
            (P::KeyboardRightHand, Role::Look) => {
                vec![turn(C::LeftBracket, C::Quote, C::Semicolon, C::BackSlash)]
            }
            (P::KeyboardRightHand, Role::Jump) => vec![BindingDef::key(C::RightControl)],
            (P::KeyboardRightHand, Role::Run) => vec![BindingDef::key(C::RightShift)],
            (P::KeyboardRightHand, Role::Zoom) => zoom(C::PageDown, C::PageUp),
            (P::GamepadLeftHand, Role::Move) => vec![BindingDef::key(C::Thumbstick1)],
            (P::GamepadLeftHand, Role::Look) => {
                vec![turn(C::DPadUp, C::DPadDown, C::DPadLeft, C::DPadRight)]
            }
            (P::GamepadLeftHand, Role::Jump) => vec![BindingDef::key(C::ButtonL1)],
            (P::GamepadLeftHand, Role::Run) => vec![BindingDef::key(C::ButtonL2)],
            (P::GamepadRightHand, Role::Move) => vec![BindingDef::key(C::Thumbstick2)],
            // Face buttons: X and B turn, Y and A pitch.
            (P::GamepadRightHand, Role::Look) => {
                vec![turn(C::ButtonY, C::ButtonA, C::ButtonX, C::ButtonB)]
            }
            (P::GamepadRightHand, Role::Jump) => vec![BindingDef::key(C::ButtonR1)],
            (P::GamepadRightHand, Role::Run) => vec![BindingDef::key(C::ButtonR2)],
            _ => return None,
        };
        Some(v)
    }
}

/// Default bindings (all devices) for a role. Move, Look, Jump, Run and Zoom come from the
/// built-in `CharacterControls`.
pub fn default_bindings(role: Role) -> Vec<BindingDef> {
    use Control as C;
    if let Some(a) = default_character_controls()
        .actions
        .into_iter()
        .find(|a| a.role == role && role != Role::None)
    {
        return a.bindings;
    }
    match role {
        Role::Primary => vec![
            BindingDef::key(C::MouseButton1),
            BindingDef::key(C::ButtonR2),
        ],
        Role::Secondary => vec![
            BindingDef::key(C::MouseButton2),
            BindingDef::key(C::ButtonL2),
        ],
        Role::Interact => vec![BindingDef::key(C::E), BindingDef::key(C::ButtonX)],
        // Esc and Start belong to the built-in Metallic.System menu; a game's own menu uses Tab and Select.
        Role::Menu => vec![BindingDef::key(C::Tab), BindingDef::key(C::ButtonSelect)],
        _ => Vec::new(),
    }
}

/// The type an action of this role has.
pub fn role_type(role: Role) -> ActionType {
    match role {
        Role::Move | Role::Look => ActionType::Direction2D,
        Role::Zoom => ActionType::Direction1D,
        _ => ActionType::Bool,
    }
}

/// A new action for `role`, with its default bindings. Roles a player taps get `ShowOnTouch`.
pub fn role_action(role: Role, name: &str) -> Snapshot {
    let mut s = Snapshot::new("InputAction")
        .with("Name", Value::String(name.to_string()))
        .with(
            "Type",
            Value::String(action_type_name(role_type(role)).into()),
        )
        .with("Role", Value::String(role_name(role).into()));
    if matches!(
        role,
        Role::Move | Role::Jump | Role::Run | Role::Primary | Role::Secondary | Role::Interact
    ) {
        s = s.with("ShowOnTouch", Value::Bool(true));
    }
    if matches!(role, Role::Menu) {
        s = s.with("Required", Value::Bool(true));
    }
    for b in default_bindings(role) {
        s = s.child(binding_snapshot(&b));
    }
    s
}

/// Adds an action for `role` to `context`; returns it.
pub fn add_role_action(
    b: &mut StepBuilder<'_>,
    context: InstanceId,
    role: Role,
) -> Result<InstanceId, CoreError> {
    let base = if role == Role::None {
        "Action"
    } else {
        role_name(role)
    };
    let name = unique_child_name(b.world(), context, base);
    b.create(context, role_action(role, &name))
}

/// What applying a preset did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetReport {
    /// Actions whose bindings were replaced (`Context/Action`).
    pub changed: Vec<String>,
    /// Actions left as they were (no role, or the preset has nothing for the role).
    pub unchanged: Vec<String>,
}

/// Replaces the preset's device bindings of every scene action with a role.
pub fn apply_preset(
    b: &mut StepBuilder<'_>,
    preset: BindingPreset,
) -> Result<PresetReport, CoreError> {
    let model = InputModel::read(b.world());
    let mut report = PresetReport::default();
    let device = preset.device();
    let mut plan: Vec<(InstanceId, Vec<InstanceId>, Vec<BindingDef>)> = Vec::new();
    for (ctx, a) in model.actions() {
        let Some(id) = a.instance else { continue };
        let path = format!("{}/{}", ctx.name, a.name);
        match preset.bindings(a.role).filter(|_| a.role != Role::None) {
            Some(new) if role_type(a.role) == a.ty => {
                let old: Vec<InstanceId> = a
                    .bindings
                    .iter()
                    .filter(|x| binding_device(x) == device)
                    .filter_map(|x| x.instance)
                    .collect();
                plan.push((id, old, new));
                report.changed.push(path);
            }
            _ => report.unchanged.push(path),
        }
    }
    for (action, old, new) in plan {
        for id in old {
            b.destroy(id)?;
        }
        for def in new {
            b.create(action, binding_snapshot(&def))?;
        }
    }
    Ok(report)
}

/// The scene's actions that a preset would change and leave alone, without changing anything.
pub fn preview_preset(world: &World, preset: BindingPreset) -> PresetReport {
    let mut copy = world.deep_clone();
    let mut b = StepBuilder::new(&mut copy);
    apply_preset(&mut b, preset).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_actions::commands::{CommandSink, History};
    use crate::input_actions::model::{binding_label, context_snapshot};

    fn sample() -> (World, InstanceId) {
        let mut w = World::new_with_services();
        let rs = w.root("ReplicatedStorage").unwrap();
        let mut b = StepBuilder::new(&mut w);
        let ctx = b
            .create(rs, context_snapshot(&default_character_controls()))
            .unwrap();
        let _ = b.finish("sample");
        (w, ctx)
    }

    fn labels(w: &World, ctx: InstanceId, action: &str, device: DeviceColumn) -> Vec<String> {
        let model = InputModel::read(w);
        let id = w.find_first_child(ctx, action).unwrap();
        let (_, a) = model.action(id).unwrap();
        a.bindings
            .iter()
            .filter(|b| binding_device(b) == device)
            .map(binding_label)
            .collect()
    }

    #[test]
    fn every_preset_covers_move_jump_and_run() {
        for p in BindingPreset::ALL {
            for role in [Role::Move, Role::Jump, Role::Run, Role::Look] {
                let b = p.bindings(role).unwrap_or_else(|| panic!("{p:?} {role:?}"));
                assert!(
                    b.iter().all(|x| binding_device(x) == p.device()),
                    "{p:?} {role:?}"
                );
            }
        }
    }

    #[test]
    fn gamepad_right_hand_replaces_only_gamepad_bindings_in_one_step() {
        let (mut w, ctx) = sample();
        let before_kb = labels(&w, ctx, "Jump", DeviceColumn::KeyboardMouse);
        let mut h = History::new();
        let mut b = StepBuilder::new(&mut w);
        let report = apply_preset(&mut b, BindingPreset::GamepadRightHand).unwrap();
        h.record(b.finish("Preset").unwrap());
        assert_eq!(labels(&w, ctx, "Jump", DeviceColumn::Gamepad), ["R1"]);
        assert_eq!(
            labels(&w, ctx, "Move", DeviceColumn::Gamepad),
            ["Right stick"]
        );
        assert_eq!(
            labels(&w, ctx, "Jump", DeviceColumn::KeyboardMouse),
            before_kb
        );
        // Zoom has no one-handed gamepad binding: unchanged and listed.
        assert!(
            report
                .unchanged
                .contains(&"CharacterControls/Zoom".to_string())
        );
        assert_eq!(labels(&w, ctx, "Zoom", DeviceColumn::Gamepad), ["R1", "L1"]);

        assert_eq!(h.len(), 1);
        h.undo(&mut w);
        assert_eq!(labels(&w, ctx, "Jump", DeviceColumn::Gamepad), ["A"]);
        assert_eq!(
            labels(&w, ctx, "Move", DeviceColumn::Gamepad),
            ["Left stick", "D-pad"]
        );
    }

    #[test]
    fn keyboard_default_restores_after_left_hand() {
        let (mut w, ctx) = sample();
        let original = labels(&w, ctx, "Move", DeviceColumn::KeyboardMouse);
        let mut b = StepBuilder::new(&mut w);
        apply_preset(&mut b, BindingPreset::KeyboardRightHand).unwrap();
        let _ = b.finish("x");
        assert_eq!(
            labels(&w, ctx, "Move", DeviceColumn::KeyboardMouse),
            ["Up Left Down Right"]
        );
        assert_eq!(
            labels(&w, ctx, "Jump", DeviceColumn::KeyboardMouse),
            ["R Ctrl"]
        );
        let mut b = StepBuilder::new(&mut w);
        apply_preset(&mut b, BindingPreset::KeyboardDefault).unwrap();
        let _ = b.finish("x");
        assert_eq!(
            labels(&w, ctx, "Move", DeviceColumn::KeyboardMouse),
            original
        );
    }

    #[test]
    fn actions_without_a_role_are_left_alone() {
        let (mut w, ctx) = sample();
        let dash = w.create("InputAction", Some(ctx)).unwrap();
        w.set(dash, "Name", Value::String("Dash".into())).unwrap();
        let k = w.create("InputBinding", Some(dash)).unwrap();
        w.set(k, "KeyCode", Value::String("ButtonX".into()))
            .unwrap();
        let report = preview_preset(&w, BindingPreset::GamepadLeftHand);
        assert!(
            report
                .unchanged
                .contains(&"CharacterControls/Dash".to_string())
        );
        assert!(
            report
                .changed
                .contains(&"CharacterControls/Jump".to_string())
        );
        // The preview changed nothing.
        assert_eq!(labels(&w, ctx, "Jump", DeviceColumn::Gamepad), ["A"]);
    }

    #[test]
    fn add_from_role() {
        let (mut w, ctx) = sample();
        let mut b = StepBuilder::new(&mut w);
        let id = add_role_action(&mut b, ctx, Role::Jump).unwrap();
        let _ = b.finish("x");
        // "Jump" is taken, so the new one is Jump2, with Space and A.
        assert_eq!(super::super::model::prop_string(&w, id, "Name"), "Jump2");
        assert_eq!(
            labels(&w, ctx, "Jump2", DeviceColumn::KeyboardMouse),
            ["Space"]
        );
        let mut b = StepBuilder::new(&mut w);
        let id = add_role_action(&mut b, ctx, Role::Interact).unwrap();
        let _ = b.finish("x");
        let model = InputModel::read(&w);
        let (_, a) = model.action(id).unwrap();
        assert_eq!(a.ty, ActionType::Bool);
        assert!(a.show_on_touch);
        assert_eq!(labels(&w, ctx, "Interact", DeviceColumn::Gamepad), ["X"]);
    }
}
