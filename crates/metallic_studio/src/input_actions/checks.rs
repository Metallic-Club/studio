//! The Checks strip (spec 10.1). The same list is meant to feed the Accessibility check (#182).
//!
//! [`run_checks`] reads the world and returns [`Finding`]s, most severe first. Each rule has a
//! [`CheckId`] so tests and the Accessibility check can ask for one rule by name.

use std::collections::{BTreeMap, HashMap};

use metallic_core::{InstanceId, World};
use metallic_runtime::input::{
    ActionDef, ActionType, BindingDef, ContextDef, Control, Role, SYSTEM_CONTEXT, Trigger,
};

use super::model::{
    DeviceColumn, InputModel, action_paths, binding_device, binding_label, control_label,
};
use super::preview::studio_keeps;
use super::touch::{
    self, DeviceProfile, MAX_BUTTONS, MIN_GAP, MIN_SIZE, SAFE_MARGIN, Violation, WARN_BUTTONS,
};

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Worth knowing.
    Info,
    /// Should be fixed.
    Warning,
    /// Must be fixed.
    Error,
}

impl CheckId {
    /// True for the rules about the touch layout (they open the device preview).
    pub fn is_touch(self) -> bool {
        matches!(
            self,
            CheckId::TouchTooSmall
                | CheckId::TouchTooClose
                | CheckId::TouchOutsideSafeArea
                | CheckId::TooManyTouchButtons
        )
    }
}

impl Level {
    /// Word shown in the strip.
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "Info",
            Level::Warning => "Warning",
            Level::Error => "Error",
        }
    }
}

/// The rules of spec 10.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CheckId {
    /// The same control on two enabled actions of one context.
    DuplicateControl,
    /// An action with no keyboard binding.
    NoKeyboard,
    /// An action with no gamepad binding.
    NoGamepad,
    /// More than five touch buttons (Error above six).
    TooManyTouchButtons,
    /// A touch control under 48 dp.
    TouchTooSmall,
    /// Two touch controls closer than 8 dp.
    TouchTooClose,
    /// A touch control outside the safe area (or within 16 dp of its edge).
    TouchOutsideSafeArea,
    /// A Hold trigger longer than 1 s (Error above 3 s).
    LongHold,
    /// A held action (Run, Aim) that cannot become a toggle.
    NoToggle,
    /// Esc is bound (it opens the Player menu).
    EscBound,
    /// A key Studio keeps during Play (F1 to F12, Ctrl or Alt chords).
    StudioKey,
    /// An action was renamed: players' saved overrides for it no longer apply.
    Renamed,
}

/// One problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Which rule.
    pub id: CheckId,
    /// How bad.
    pub level: Level,
    /// What is wrong, in one sentence.
    pub message: String,
    /// The instances to select when the finding is clicked.
    pub targets: Vec<InstanceId>,
}

/// Action paths as they were when the panel opened or the scene was last saved.
#[derive(Debug, Clone, Default)]
pub struct NameBaseline {
    paths: HashMap<InstanceId, String>,
}

impl NameBaseline {
    /// Remembers every action's `Context/Action` path.
    pub fn capture(world: &World) -> Self {
        Self {
            paths: action_paths(world).into_iter().collect(),
        }
    }

    /// The remembered path of an action.
    pub fn path(&self, id: InstanceId) -> Option<&str> {
        self.paths.get(&id).map(String::as_str)
    }

    /// Keeps remembered paths for instances an undo rebuilt with new ids.
    pub fn remap(&mut self, map: &super::commands::IdMap) {
        for (old, new) in map {
            if let Some(p) = self.paths.remove(old) {
                self.paths.insert(*new, p);
            }
        }
    }
}

/// Bool actions that are held for a long time and should allow toggling.
fn is_long_hold(a: &ActionDef) -> bool {
    if a.ty != ActionType::Bool {
        return false;
    }
    if a.role == Role::Run {
        return true;
    }
    let n = a.name.to_ascii_lowercase();
    ["run", "sprint", "aim", "crouch"].iter().any(|w| n == *w)
}

/// The key a binding is pressed with, plus its modifiers: two bindings conflict when these match.
fn signatures(b: &BindingDef) -> Vec<(Control, Vec<Control>)> {
    let mut mods: Vec<Control> = b.modifiers.iter().flatten().copied().collect();
    mods.sort_by_key(|c| c.name());
    let keys: Vec<Control> = b
        .key
        .into_iter()
        .chain(b.composite.iter().flatten().copied())
        .collect();
    keys.into_iter().map(|k| (k, mods.clone())).collect()
}

fn path_of(ctx: &str, action: &str) -> String {
    format!("{ctx}/{action}")
}

/// Same control on two enabled actions of one RebindGroup (the contexts of a group share keys).
pub(super) fn duplicate_control_findings(contexts: &[ContextDef]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut groups: BTreeMap<&str, Vec<&ContextDef>> = BTreeMap::new();
    for ctx in contexts.iter().filter(|c| c.enabled) {
        groups.entry(&ctx.rebind_group).or_default().push(ctx);
    }
    for group in groups.values() {
        let shared = group.len() > 1;
        let mut users: BTreeMap<(String, String), Vec<(&ContextDef, &ActionDef)>> = BTreeMap::new();
        for &ctx in group {
            for a in ctx.actions.iter().filter(|a| a.enabled) {
                for b in &a.bindings {
                    for (key, mods) in signatures(b) {
                        let mut label: Vec<&str> = mods.iter().map(|&m| control_label(m)).collect();
                        label.push(control_label(key));
                        let entry = users
                            .entry((key.name().to_string(), label.join(" + ")))
                            .or_default();
                        if !entry
                            .iter()
                            .any(|(c, x)| c.name == ctx.name && x.name == a.name)
                        {
                            entry.push((ctx, a));
                        }
                    }
                }
            }
        }
        for ((_, label), actions) in users {
            if actions.len() > 1 {
                let names: Vec<String> = actions
                    .iter()
                    .map(|(c, a)| {
                        if shared {
                            path_of(&c.name, &a.name)
                        } else {
                            a.name.clone()
                        }
                    })
                    .collect();
                let place = if shared {
                    format!("group {}", actions[0].0.rebind_group)
                } else {
                    actions[0].0.name.clone()
                };
                out.push(Finding {
                    id: CheckId::DuplicateControl,
                    level: Level::Error,
                    message: format!("{label} is bound to {} in {place}", names.join(" and ")),
                    targets: actions.iter().filter_map(|(_, a)| a.instance).collect(),
                });
            }
        }
    }
    out
}

/// Runs every rule.
pub fn run_checks(world: &World, baseline: &NameBaseline) -> Vec<Finding> {
    let model = InputModel::read(world);
    let mut out = Vec::new();
    let target = |a: &ActionDef| a.instance.into_iter().collect::<Vec<_>>();

    out.extend(duplicate_control_findings(&model.config.contexts));

    for ctx in &model.config.contexts {
        if !ctx.enabled {
            continue;
        }

        for a in ctx.actions.iter().filter(|a| a.enabled) {
            let path = path_of(&ctx.name, &a.name);
            if a.ty != ActionType::ViewportPosition {
                let has = |d: DeviceColumn| a.bindings.iter().any(|b| binding_device(b) == d);
                if !has(DeviceColumn::KeyboardMouse) {
                    out.push(Finding {
                        id: CheckId::NoKeyboard,
                        level: Level::Warning,
                        message: format!("{path} has no keyboard or mouse binding"),
                        targets: target(a),
                    });
                }
                if !has(DeviceColumn::Gamepad) {
                    out.push(Finding {
                        id: CheckId::NoGamepad,
                        level: Level::Warning,
                        message: format!("{path} has no gamepad binding"),
                        targets: target(a),
                    });
                }
            }
            if a.trigger == Trigger::Hold && a.hold_time > 1000.0 {
                let error = a.hold_time > 3000.0;
                out.push(Finding {
                    id: CheckId::LongHold,
                    level: if error { Level::Error } else { Level::Warning },
                    message: format!(
                        "{path} needs a {:.1} s hold; keep holds under {} s",
                        a.hold_time / 1000.0,
                        if error { 3 } else { 1 }
                    ),
                    targets: target(a),
                });
            }
            if is_long_hold(a) && !a.can_toggle {
                out.push(Finding {
                    id: CheckId::NoToggle,
                    level: Level::Warning,
                    message: format!(
                        "{path} is held for long; allow players to make it a toggle (CanToggle)"
                    ),
                    targets: target(a),
                });
            }
            for b in &a.bindings {
                // The engine's own Menu action is the one thing that owns Esc.
                if b.reads(Control::Escape) && ctx.name != SYSTEM_CONTEXT {
                    out.push(Finding {
                        id: CheckId::EscBound,
                        level: Level::Error,
                        message: format!("{path} uses Esc, which opens the Player menu"),
                        targets: b.instance.or(a.instance).into_iter().collect(),
                    });
                }
                let reserved = b.controls().any(|c| studio_keeps(c, false, false))
                    || b.modifiers.iter().flatten().any(|&m| {
                        matches!(
                            m,
                            Control::LeftControl
                                | Control::RightControl
                                | Control::LeftAlt
                                | Control::RightAlt
                        )
                    });
                if reserved {
                    out.push(Finding {
                        id: CheckId::StudioKey,
                        level: Level::Info,
                        message: format!(
                            "{path}: Studio keeps {} in Play; it works in the Player",
                            binding_label(b)
                        ),
                        targets: b.instance.or(a.instance).into_iter().collect(),
                    });
                }
            }
            if let Some(id) = a.instance
                && let Some(old) = baseline.path(id)
                && old != path
            {
                out.push(Finding {
                    id: CheckId::Renamed,
                    level: Level::Info,
                    message: format!(
                        "{old} is now {path}; players' saved controls for it no longer apply"
                    ),
                    targets: vec![id],
                });
            }
        }
    }

    // Touch.
    let count = touch::touch_button_count(world);
    if count > WARN_BUTTONS {
        out.push(Finding {
            id: CheckId::TooManyTouchButtons,
            level: if count > MAX_BUTTONS {
                Level::Error
            } else {
                Level::Warning
            },
            message: if count > MAX_BUTTONS {
                format!("{count} touch buttons; only {MAX_BUTTONS} are placed")
            } else {
                format!("{count} touch buttons; keep it to {WARN_BUTTONS} or fewer")
            },
            targets: Vec::new(),
        });
    }
    out.extend(touch_findings(world));

    out.sort_by(|a, b| b.level.cmp(&a.level).then(a.id.cmp(&b.id)));
    out
}

/// Where a touch rule fails, the controls involved and the detail text.
type Merged = (Vec<String>, Vec<InstanceId>, String);

/// Touch layout rules on every device preset and orientation, merged per control.
fn touch_findings(world: &World) -> Vec<Finding> {
    let actions = touch::touch_actions(world);
    // (rule, the labels involved) -> (where it fails, targets, detail)
    let mut merged: BTreeMap<(CheckId, String), Merged> = BTreeMap::new();
    for profile in DeviceProfile::all() {
        let placed = touch::layout(&profile, &actions);
        for v in touch::violations(&profile, &placed) {
            let (id, key, ids, detail) = match v {
                Violation::TooSmall { control } => {
                    let p = &placed[control];
                    (
                        CheckId::TouchTooSmall,
                        p.label.clone(),
                        p.id.into_iter().collect::<Vec<_>>(),
                        format!("is {:.0} dp; touch controls need {MIN_SIZE:.0} dp", p.size),
                    )
                }
                Violation::TooClose { a, b, gap } => {
                    let (pa, pb) = (&placed[a], &placed[b]);
                    (
                        CheckId::TouchTooClose,
                        format!("{} and {}", pa.label, pb.label),
                        pa.id.into_iter().chain(pb.id).collect(),
                        if gap < 0.0 {
                            "overlap".to_string()
                        } else {
                            format!("are {gap:.0} dp apart; keep {MIN_GAP:.0} dp")
                        },
                    )
                }
                Violation::OutsideSafeArea { control } => {
                    let p = &placed[control];
                    (
                        CheckId::TouchOutsideSafeArea,
                        p.label.clone(),
                        p.id.into_iter().collect(),
                        format!("is less than {SAFE_MARGIN:.0} dp inside the safe area"),
                    )
                }
            };
            let entry = merged
                .entry((id, key))
                .or_insert_with(|| (Vec::new(), ids, detail));
            entry.0.push(profile.label());
        }
    }
    merged
        .into_iter()
        .map(|((id, key), (screens, targets, detail))| Finding {
            id,
            level: Level::Error,
            message: format!("{key} {detail} ({})", screens.join(", ")),
            targets,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_actions::commands::StepBuilder;
    use crate::input_actions::model::context_snapshot;
    use metallic_core::{UDim2, Value};
    use metallic_runtime::input::{default_character_controls, system_context};

    /// Parkour's controls as scene instances.
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

    fn action(w: &World, ctx: InstanceId, name: &str) -> InstanceId {
        w.find_first_child(ctx, name).unwrap()
    }

    fn fires(w: &World, base: &NameBaseline, id: CheckId) -> bool {
        run_checks(w, base).iter().any(|f| f.id == id)
    }

    fn add_binding(w: &mut World, action: InstanceId, key: &str) -> InstanceId {
        let b = w.create("InputBinding", Some(action)).unwrap();
        w.set(b, "KeyCode", Value::String(key.into())).unwrap();
        b
    }

    #[test]
    fn sample_scene_is_clean() {
        let (w, _) = sample();
        let f = run_checks(&w, &NameBaseline::capture(&w));
        assert!(f.is_empty(), "{f:#?}");
    }

    #[test]
    fn duplicate_control_fires_and_clears() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        let b = add_binding(&mut w, jump, "LeftShift"); // Run already has LeftShift
        let f = run_checks(&w, &base);
        let dup = f
            .iter()
            .find(|f| f.id == CheckId::DuplicateControl)
            .unwrap();
        assert_eq!(dup.level, Level::Error);
        assert_eq!(dup.targets.len(), 2);
        w.destroy(b);
        assert!(!fires(&w, &base, CheckId::DuplicateControl));
        // A chord is a different control: Shift + Space does not clash with Space.
        let run = action(&w, ctx, "Run");
        let chord = add_binding(&mut w, run, "Space");
        w.set(
            chord,
            "PrimaryModifier",
            Value::String("LeftControl".into()),
        )
        .unwrap();
        assert!(!fires(&w, &base, CheckId::DuplicateControl));
        // Disabled actions do not clash.
        let b = add_binding(&mut w, jump, "LeftShift");
        w.set(jump, "Enabled", Value::Bool(false)).unwrap();
        assert!(!fires(&w, &base, CheckId::DuplicateControl));
        w.destroy(b);
    }

    #[test]
    fn duplicate_control_spans_a_rebind_group() {
        let (mut w, ctx) = sample();
        let rs = w.root("ReplicatedStorage").unwrap();
        let group = w.get(ctx, "RebindGroup").unwrap();
        let base = NameBaseline::capture(&w);
        let other = |w: &mut World, g: Option<&Value>, name: &str| {
            let c = w.create("InputContext", Some(rs)).unwrap();
            w.set(c, "Name", Value::String(name.into())).unwrap();
            if let Some(g) = g {
                w.set(c, "RebindGroup", g.clone()).unwrap();
            }
            let a = w.create("InputAction", Some(c)).unwrap();
            w.set(a, "Name", Value::String("Interact".into())).unwrap();
            add_binding(w, a, "E");
            (c, a)
        };
        // Another group never clashes.
        let (c2, _) = other(&mut w, Some(&Value::String("Menu".into())), "MenuKeys");
        let jump = action(&w, ctx, "Jump");
        add_binding(&mut w, jump, "E");
        assert!(!fires(&w, &base, CheckId::DuplicateControl));
        // The same group does, across the two contexts.
        w.set(c2, "RebindGroup", group).unwrap();
        let f = run_checks(&w, &base);
        let dup = f
            .iter()
            .find(|f| f.id == CheckId::DuplicateControl)
            .expect("E is bound twice in one group");
        assert_eq!(dup.targets.len(), 2);
        assert!(dup.message.contains("MenuKeys/Interact"), "{}", dup.message);
    }

    #[test]
    fn missing_devices_fire_and_clear() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let dash = w.create("InputAction", Some(ctx)).unwrap();
        w.set(dash, "Name", Value::String("Dash".into())).unwrap();
        assert!(fires(&w, &base, CheckId::NoKeyboard));
        assert!(fires(&w, &base, CheckId::NoGamepad));
        add_binding(&mut w, dash, "Q");
        assert!(!fires(&w, &base, CheckId::NoKeyboard));
        assert!(fires(&w, &base, CheckId::NoGamepad));
        add_binding(&mut w, dash, "ButtonX");
        assert!(!fires(&w, &base, CheckId::NoGamepad));
    }

    #[test]
    fn too_many_touch_buttons() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let mut added = Vec::new();
        // Jump and Run are already buttons: four more make six (warning), five more seven (error).
        for (i, key) in ["Q", "E", "R", "F", "G"].iter().enumerate() {
            let a = w.create("InputAction", Some(ctx)).unwrap();
            w.set(a, "Name", Value::String(format!("Extra{i}")))
                .unwrap();
            w.set(a, "ShowOnTouch", Value::Bool(true)).unwrap();
            w.set(a, "TouchOrder", Value::Number(10.0 + i as f64))
                .unwrap();
            add_binding(&mut w, a, key);
            add_binding(
                &mut w,
                a,
                ["ButtonX", "ButtonY", "ButtonB", "ButtonR1", "ButtonL1"][i],
            );
            added.push(a);
        }
        let level = |w: &World| {
            run_checks(w, &base)
                .into_iter()
                .find(|f| f.id == CheckId::TooManyTouchButtons)
                .map(|f| f.level)
        };
        assert_eq!(level(&w), Some(Level::Error));
        w.destroy(added[4]);
        assert_eq!(level(&w), Some(Level::Warning));
        w.destroy(added[3]);
        assert_eq!(level(&w), None);
    }

    #[test]
    fn touch_size_gap_and_safe_area_fire_and_clear() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        let run = action(&w, ctx, "Run");
        w.set(jump, "TouchLayout", Value::String("Custom".into()))
            .unwrap();
        // 40 dp, 40 dp in from the bottom-right corner.
        w.set(
            jump,
            "TouchPosition",
            Value::UDim2(UDim2::new(1.0, -40.0, 1.0, -40.0)),
        )
        .unwrap();
        w.set(jump, "TouchSize", Value::Number(40.0)).unwrap();
        assert!(fires(&w, &base, CheckId::TouchTooSmall));
        w.set(jump, "TouchSize", Value::Number(56.0)).unwrap();
        assert!(!fires(&w, &base, CheckId::TouchTooSmall));
        // Pushed to the edge: centre 20 dp from the corner, radius 28.
        w.set(
            jump,
            "TouchPosition",
            Value::UDim2(UDim2::new(1.0, -20.0, 1.0, -20.0)),
        )
        .unwrap();
        assert!(fires(&w, &base, CheckId::TouchOutsideSafeArea));
        // On top of Run.
        w.set(run, "TouchLayout", Value::String("Custom".into()))
            .unwrap();
        w.set(
            run,
            "TouchPosition",
            Value::UDim2(UDim2::new(1.0, -64.0, 1.0, -64.0)),
        )
        .unwrap();
        w.set(
            jump,
            "TouchPosition",
            Value::UDim2(UDim2::new(1.0, -80.0, 1.0, -64.0)),
        )
        .unwrap();
        assert!(fires(&w, &base, CheckId::TouchTooClose));
        for id in [jump, run] {
            w.set(id, "TouchLayout", Value::String("Auto".into()))
                .unwrap();
        }
        let f = run_checks(&w, &base);
        assert!(f.is_empty(), "{f:#?}");
    }

    #[test]
    fn long_hold_and_toggle() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        w.set(jump, "Trigger", Value::String("Hold".into()))
            .unwrap();
        w.set(jump, "HoldTime", Value::Number(1500.0)).unwrap();
        let level = |w: &World| {
            run_checks(w, &base)
                .into_iter()
                .find(|f| f.id == CheckId::LongHold)
                .map(|f| f.level)
        };
        assert_eq!(level(&w), Some(Level::Warning));
        w.set(jump, "HoldTime", Value::Number(3500.0)).unwrap();
        assert_eq!(level(&w), Some(Level::Error));
        w.set(jump, "HoldTime", Value::Number(800.0)).unwrap();
        assert_eq!(level(&w), None);

        let run = action(&w, ctx, "Run");
        w.set(run, "CanToggle", Value::Bool(false)).unwrap();
        assert!(fires(&w, &base, CheckId::NoToggle));
        w.set(run, "CanToggle", Value::Bool(true)).unwrap();
        assert!(!fires(&w, &base, CheckId::NoToggle));
    }

    #[test]
    fn a_scene_with_the_system_menu_is_clean() {
        let (mut w, _) = sample();
        let rs = w.root("ReplicatedStorage").unwrap();
        let mut b = StepBuilder::new(&mut w);
        let sys = b.create(rs, context_snapshot(&system_context())).unwrap();
        let _ = b.finish("system");
        let base = NameBaseline::capture(&w);
        let f = run_checks(&w, &base);
        assert!(f.is_empty(), "{f:#?}");
        // Game actions on Esc are still flagged.
        let ctx = w.find_first_child(rs, "CharacterControls").unwrap();
        let jump = action(&w, ctx, "Jump");
        add_binding(&mut w, jump, "Escape");
        assert!(fires(&w, &base, CheckId::EscBound));
        let _ = sys;
    }

    #[test]
    fn esc_and_studio_keys() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        let esc = add_binding(&mut w, jump, "Escape");
        assert!(fires(&w, &base, CheckId::EscBound));
        w.destroy(esc);
        assert!(!fires(&w, &base, CheckId::EscBound));
        let f5 = add_binding(&mut w, jump, "F5");
        let f = run_checks(&w, &base);
        let info = f.iter().find(|f| f.id == CheckId::StudioKey).unwrap();
        assert_eq!(info.level, Level::Info);
        w.destroy(f5);
        assert!(!fires(&w, &base, CheckId::StudioKey));
        let chord = add_binding(&mut w, jump, "J");
        w.set(chord, "PrimaryModifier", Value::String("LeftAlt".into()))
            .unwrap();
        assert!(fires(&w, &base, CheckId::StudioKey));
    }

    #[test]
    fn rename_fires_and_clears() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        w.set(jump, "Name", Value::String("Hop".into())).unwrap();
        let f = run_checks(&w, &base);
        let r = f.iter().find(|f| f.id == CheckId::Renamed).unwrap();
        assert!(r.message.contains("CharacterControls/Jump"));
        w.set(jump, "Name", Value::String("Jump".into())).unwrap();
        assert!(!fires(&w, &base, CheckId::Renamed));
    }

    #[test]
    fn errors_come_first() {
        let (mut w, ctx) = sample();
        let base = NameBaseline::capture(&w);
        let jump = action(&w, ctx, "Jump");
        add_binding(&mut w, jump, "F2");
        add_binding(&mut w, jump, "Escape");
        let f = run_checks(&w, &base);
        assert_eq!(f[0].level, Level::Error);
        assert_eq!(f.last().unwrap().level, Level::Info);
    }
}
