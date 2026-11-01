//! Rebinding for the player's Controls page (docs/design/input-actions.md sections 8.1 to 8.3):
//! which actions show and in which groups, what a slot accepts while listening, conflicts with
//! their Swap / Replace / Cancel choices, and the rules for `Required` actions and `Menu`.
//!
//! Everything here is pure: it reads the creator's [`InputConfig`] and edits
//! [`PlayerOverrides`]; the runtime applies the result with
//! [`InputRuntime::set_overrides`](super::InputRuntime::set_overrides).
//!
//! A conflict is the same control on two enabled actions in one `RebindGroup`. Actions of the
//! built-in `Metallic.System` context (`Menu`) are checked against every group, because that
//! context sinks its controls above every game context. Composite parts count one by one, so
//! binding Jump to `W` conflicts with `Move`'s Up part, and a swap gives `Move` the old key.
use super::config::{
    ActionDef, ActionType, ContextDef, InputConfig, Role, SYSTEM_CONTEXT, SYSTEM_GROUP,
};
use super::control::{Control, ControlKind, DeviceKind};
use super::overrides::{PlayerOverrides, Slot, Stick, binding_device, control_device};

/// Slots shown per action and device: Primary and Alternate.
pub const MIN_SLOTS: usize = 2;
/// Most slots the page shows for one action and device.
pub const MAX_SLOTS: usize = 4;
/// Seconds a slot listens before it gives up (spec 8.2).
pub const LISTEN_TIMEOUT: f64 = 8.0;
/// Seconds the "Undo" toast stays after a reset (spec 8.1).
pub const UNDO_SECONDS: f64 = 10.0;

/// One part of one slot of one action on one device.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlotRef {
    /// The action path, `Context/Action`.
    pub path: String,
    /// The device tab.
    pub device: DeviceKind,
    /// Slot index: 0 Primary, 1 Alternate, ...
    pub slot: usize,
    /// The part of a composite (0 Up, 1 Down, 2 Left, 3 Right); `None` for a single control.
    pub part: Option<usize>,
}

/// What a slot (part) takes while listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A key, mouse button or gamepad button.
    Button,
    /// A key, a button or the mouse wheel (one-axis actions).
    ButtonOrWheel,
    /// A stick.
    Stick,
    /// A four-key composite; each part takes a button.
    Composite,
    /// Mouse movement or a pointer position: shown, not rebindable (its sensitivity and inversion
    /// are settings instead).
    Fixed,
}

/// One action row on the page.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionRow {
    /// `Context/Action`.
    pub path: String,
    /// `DisplayName`, else the name.
    pub name: String,
    /// Value type.
    pub ty: ActionType,
    /// Role.
    pub role: Role,
    /// The row has a Hold/Toggle switch.
    pub can_toggle: bool,
    /// Cannot be left unbound on a device that has bindings.
    pub required: bool,
    /// The action and its context are enabled.
    pub enabled: bool,
}

/// Rows of one `RebindGroup`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionGroup {
    /// The group name (`System`, `Game`, ...).
    pub name: String,
    /// The rows, by context priority then tree order.
    pub rows: Vec<ActionRow>,
}

/// The rows of the page grouped by `RebindGroup`: the system group first, then groups by their
/// highest context priority, ties by tree order. Only `Rebindable` actions show.
pub fn action_groups(config: &InputConfig) -> Vec<ActionGroup> {
    let mut contexts: Vec<&ContextDef> = config.contexts.iter().collect();
    contexts.sort_by(|a, b| {
        let sys = |c: &ContextDef| c.name != SYSTEM_CONTEXT;
        sys(a)
            .cmp(&sys(b))
            .then(b.priority.total_cmp(&a.priority))
            .then(a.order.cmp(&b.order))
    });
    let mut groups: Vec<ActionGroup> = Vec::new();
    for ctx in contexts {
        let rows = ctx
            .actions
            .iter()
            .filter(|a| a.rebindable)
            .map(|a| ActionRow {
                path: format!("{}/{}", ctx.name, a.name),
                name: if a.display_name.is_empty() {
                    a.name.clone()
                } else {
                    a.display_name.clone()
                },
                ty: a.ty,
                role: a.role,
                can_toggle: a.ty == ActionType::Bool && a.can_toggle,
                required: a.required,
                enabled: ctx.enabled && a.enabled,
            });
        match groups.iter_mut().find(|g| g.name == ctx.rebind_group) {
            Some(g) => g.rows.extend(rows),
            None => groups.push(ActionGroup {
                name: ctx.rebind_group.clone(),
                rows: rows.collect(),
            }),
        }
    }
    groups.retain(|g| !g.rows.is_empty());
    groups
}

/// The empty slot an action gets on a device when the creator left one free.
pub fn default_slot(ty: ActionType, device: DeviceKind) -> Slot {
    match (ty, device) {
        (ActionType::Direction2D | ActionType::Direction3D, DeviceKind::KeyboardMouse) => {
            Slot::Composite([None; 4])
        }
        _ => Slot::Single(None),
    }
}

/// The slots the page shows for an action on a device: the player's or the creator's, padded
/// with empty ones to [`MIN_SLOTS`], at most [`MAX_SLOTS`].
pub fn display_slots(
    config: &InputConfig,
    o: &PlayerOverrides,
    path: &str,
    device: DeviceKind,
) -> Vec<Slot> {
    let Some((_, action)) = config.action(path) else {
        return Vec::new();
    };
    let mut slots = o.slots(path, action, device);
    slots.truncate(MAX_SLOTS);
    while slots.len() < MIN_SLOTS {
        slots.push(default_slot(action.ty, device));
    }
    slots
}

/// What a slot part takes; `None` when the slot or action does not exist.
pub fn slot_kind(config: &InputConfig, o: &PlayerOverrides, target: &SlotRef) -> Option<SlotKind> {
    let (_, action) = config.action(&target.path)?;
    let slots = display_slots(config, o, &target.path, target.device);
    let slot = *slots.get(target.slot)?;
    Some(match (slot, target.part) {
        (Slot::Composite(_), Some(i)) if i < 4 => SlotKind::Button,
        (Slot::Composite(_), _) => SlotKind::Composite,
        (Slot::Single(_), Some(_)) => return None,
        (Slot::Single(Some(c)), None)
            if matches!(c.kind(), ControlKind::Delta | ControlKind::Position) =>
        {
            SlotKind::Fixed
        }
        (Slot::Single(_), None) => match action.ty {
            ActionType::Bool => SlotKind::Button,
            ActionType::Direction1D => SlotKind::ButtonOrWheel,
            ActionType::Direction2D | ActionType::Direction3D => SlotKind::Stick,
            ActionType::ViewportPosition => SlotKind::Fixed,
        },
    })
}

/// True when a slot of `kind` on `device` takes `control`.
pub fn kind_accepts(kind: SlotKind, device: DeviceKind, control: Control) -> bool {
    if control_device(control) != device {
        return false;
    }
    match kind {
        SlotKind::Button => control.kind() == ControlKind::Button,
        SlotKind::ButtonOrWheel => {
            matches!(control.kind(), ControlKind::Button | ControlKind::Wheel)
        }
        SlotKind::Stick => control.kind() == ControlKind::Stick,
        SlotKind::Composite | SlotKind::Fixed => false,
    }
}

/// True when listening on `target` would take `control`.
pub fn accepts(
    config: &InputConfig,
    o: &PlayerOverrides,
    target: &SlotRef,
    control: Control,
) -> bool {
    slot_kind(config, o, target).is_some_and(|k| kind_accepts(k, target.device, control))
}

/// Another use of the control a slot is about to take.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    /// The slot part that holds the control now.
    pub other: SlotRef,
    /// That action's display name.
    pub other_name: String,
    /// The control is a modifier or a fixed binding of the other action: only Cancel.
    pub fixed: bool,
    /// Swap is possible: the other slot can take the control this slot had.
    pub can_swap: bool,
    /// Replace is possible: the other action may lose the control.
    pub can_replace: bool,
}

/// How the player settled a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The other action gets the control this slot had (the default choice).
    Swap,
    /// The other action loses the control; its slot shows "Unbound".
    Replace,
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum RebindError {
    /// No such action or slot.
    Unknown,
    /// The slot does not take that control (wrong device or shape).
    Rejected,
    /// The slot is fixed (mouse movement).
    Fixed,
    /// The control is in use; settle it with a [`Resolution`].
    Conflict(Conflict),
    /// This resolution is not possible for the conflict.
    NotAllowed,
    /// The action (named) would have no binding on the device, which `Required` and `Menu`
    /// forbid.
    Required(String),
}

fn groups_meet(a: &ContextDef, b: &ContextDef) -> bool {
    a.rebind_group == b.rebind_group
        || a.rebind_group == SYSTEM_GROUP
        || b.rebind_group == SYSTEM_GROUP
}

fn display_name(a: &ActionDef) -> String {
    if a.display_name.is_empty() {
        a.name.clone()
    } else {
        a.display_name.clone()
    }
}

/// True when the slots hold any control.
fn any_control(slots: &[Slot]) -> bool {
    slots.iter().any(|s| !s.is_empty())
}

/// True when `action` must keep a binding on `device` after its slots become `after`.
fn violates_required(action: &ActionDef, device: DeviceKind, after: &[Slot]) -> bool {
    let had_any = action
        .bindings
        .iter()
        .any(|b| binding_device(b) == Some(device));
    (action.required || action.role == Role::Menu) && had_any && !any_control(after)
}

/// The first other use of `control` that `target` would conflict with, if any.
pub fn find_conflict(
    config: &InputConfig,
    o: &PlayerOverrides,
    target: &SlotRef,
    control: Control,
) -> Option<Conflict> {
    let (target_ctx, _) = config.action(&target.path)?;
    let device = target.device;
    let old = display_slots(config, o, &target.path, device)
        .get(target.slot)
        .and_then(|s| s.part(target.part));
    for ctx in &config.contexts {
        if !ctx.enabled || !groups_meet(target_ctx, ctx) {
            continue;
        }
        for action in ctx.actions.iter().filter(|a| a.enabled) {
            let path = format!("{}/{}", ctx.name, action.name);
            // A modifier of the creator's binding (right mouse for camera turning) is fixed.
            let modifier = action
                .bindings
                .iter()
                .filter(|b| binding_device(b) == Some(device))
                .any(|b| b.modifiers.contains(&Some(control)));
            if modifier && path != target.path {
                return Some(Conflict {
                    other: SlotRef {
                        path,
                        device,
                        slot: 0,
                        part: None,
                    },
                    other_name: display_name(action),
                    fixed: true,
                    can_swap: false,
                    can_replace: false,
                });
            }
            let slots = display_slots(config, o, &path, device);
            for (si, slot) in slots.iter().enumerate() {
                let parts: &[Option<usize>] = match slot {
                    Slot::Single(_) => &[None],
                    Slot::Composite(_) => &[Some(0), Some(1), Some(2), Some(3)],
                };
                for &part in parts {
                    let here = SlotRef {
                        path: path.clone(),
                        device,
                        slot: si,
                        part,
                    };
                    if slot.part(part) != Some(control) || here == *target {
                        continue;
                    }
                    let other_kind = slot_kind(config, o, &here);
                    let fixed = other_kind == Some(SlotKind::Fixed);
                    let mut after_replace = slots.clone();
                    after_replace[si].set_part(part, None);
                    let same_action = path == target.path;
                    let replace_ok = !fixed
                        && (same_action || !violates_required(action, device, &after_replace));
                    let swap_ok = !fixed
                        && match old {
                            Some(old) => other_kind.is_some_and(|k| kind_accepts(k, device, old)),
                            None => replace_ok,
                        };
                    return Some(Conflict {
                        other: here,
                        other_name: display_name(action),
                        fixed,
                        can_swap: swap_ok,
                        can_replace: replace_ok,
                    });
                }
            }
        }
    }
    None
}

/// Stores `slots` as the player's slots for `path` on `device`; slots equal to the creator's
/// (ignoring empty slots at the end) remove the override instead.
fn store_slots(
    config: &InputConfig,
    o: &mut PlayerOverrides,
    path: &str,
    device: DeviceKind,
    mut slots: Vec<Slot>,
) {
    let Some((_, action)) = config.action(path) else {
        return;
    };
    let trim = |s: &mut Vec<Slot>| {
        while s.last().is_some_and(Slot::is_empty) {
            s.pop();
        }
    };
    trim(&mut slots);
    let mut creator = super::overrides::creator_slots(action, device);
    trim(&mut creator);
    let entry = o.action_mut(path);
    entry.set_slots(device, (slots != creator).then_some(slots));
    o.prune();
}

/// Binds `control` to `target`. When the control is in use the call fails with
/// [`RebindError::Conflict`] unless `resolution` settles it; Cancel is simply not calling again.
pub fn assign(
    config: &InputConfig,
    o: &mut PlayerOverrides,
    target: &SlotRef,
    control: Control,
    resolution: Option<Resolution>,
) -> Result<(), RebindError> {
    let kind = slot_kind(config, o, target).ok_or(RebindError::Unknown)?;
    if kind == SlotKind::Fixed {
        return Err(RebindError::Fixed);
    }
    if !kind_accepts(kind, target.device, control) {
        return Err(RebindError::Rejected);
    }
    let device = target.device;
    let mut slots = display_slots(config, o, &target.path, device);
    let old = slots[target.slot].part(target.part);
    if old == Some(control) {
        return Ok(());
    }
    let conflict = find_conflict(config, o, target, control);
    let mut other_slots = None;
    if let Some(c) = &conflict {
        let allowed = match resolution {
            None => return Err(RebindError::Conflict(c.clone())),
            Some(Resolution::Swap) => c.can_swap,
            Some(Resolution::Replace) => c.can_replace,
        };
        if !allowed {
            return Err(RebindError::NotAllowed);
        }
        let give = match resolution {
            Some(Resolution::Swap) => old,
            _ => None,
        };
        if c.other.path == target.path {
            slots[c.other.slot].set_part(c.other.part, give);
        } else {
            let mut s = display_slots(config, o, &c.other.path, device);
            s[c.other.slot].set_part(c.other.part, give);
            other_slots = Some((c.other.path.clone(), s));
        }
    }
    slots[target.slot].set_part(target.part, Some(control));
    store_slots(config, o, &target.path, device, slots);
    if let Some((path, s)) = other_slots {
        store_slots(config, o, &path, device, s);
    }
    Ok(())
}

/// Empties `target` (Backspace while listening, or the clear button). `Required` actions and
/// `Menu` keep their last binding on a device that had any.
pub fn clear(
    config: &InputConfig,
    o: &mut PlayerOverrides,
    target: &SlotRef,
) -> Result<(), RebindError> {
    let (_, action) = config.action(&target.path).ok_or(RebindError::Unknown)?;
    match slot_kind(config, o, target).ok_or(RebindError::Unknown)? {
        SlotKind::Fixed => return Err(RebindError::Fixed),
        SlotKind::Composite => return Err(RebindError::Rejected),
        _ => {}
    }
    let mut slots = display_slots(config, o, &target.path, target.device);
    slots[target.slot].set_part(target.part, None);
    if violates_required(action, target.device, &slots) {
        return Err(RebindError::Required(display_name(action)));
    }
    store_slots(config, o, &target.path, target.device, slots);
    Ok(())
}

/// Swaps the first two keyboard composites of a direction action: WASD and the arrows.
pub fn swap_composites(config: &InputConfig, o: &mut PlayerOverrides, path: &str) -> bool {
    let device = DeviceKind::KeyboardMouse;
    let mut slots = display_slots(config, o, path, device);
    let found: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Slot::Composite(_)))
        .map(|(i, _)| i)
        .take(2)
        .collect();
    let [a, b] = found[..] else {
        return false;
    };
    slots.swap(a, b);
    store_slots(config, o, path, device, slots);
    true
}

/// The row's reset: the creator's bindings on `device` and the default Hold/Toggle.
pub fn reset_action(o: &mut PlayerOverrides, path: &str, device: DeviceKind) {
    if let Some(a) = o.actions.get_mut(path) {
        a.set_slots(device, None);
        a.toggle = None;
    }
    o.prune();
}

/// "Reset this tab": every action's bindings on `device`, plus that device's stick or mouse
/// settings (and the glyph style for the gamepad).
pub fn reset_device(o: &mut PlayerOverrides, device: DeviceKind) {
    for a in o.actions.values_mut() {
        a.set_slots(device, None);
    }
    o.prune();
    let sticks: &[Stick] = match device {
        DeviceKind::KeyboardMouse => &[Stick::Mouse],
        DeviceKind::Gamepad => &[Stick::Left, Stick::Right],
        DeviceKind::Touch => &[],
    };
    for &s in sticks {
        o.set_stick(s, Default::default());
    }
    if device == DeviceKind::Gamepad {
        o.gamepad_glyphs = Default::default();
    }
}

/// Paths of enabled, rebindable actions that the creator bound on `device` and that now have no
/// binding there (the banner "2 actions have no binding").
pub fn unbound_actions(
    config: &InputConfig,
    o: &PlayerOverrides,
    device: DeviceKind,
) -> Vec<String> {
    let mut out = Vec::new();
    for ctx in config.contexts.iter().filter(|c| c.enabled) {
        for a in ctx.actions.iter().filter(|a| a.enabled && a.rebindable) {
            let had = a.bindings.iter().any(|b| binding_device(b) == Some(device));
            let path = format!("{}/{}", ctx.name, a.name);
            if had && !any_control(&o.slots(&path, a, device)) {
                out.push(path);
            }
        }
    }
    out
}
