//! The Checks strip (spec 10.1). The same list is meant to feed the Accessibility check (#182).
//!
//! [`run_checks`] reads the world and returns [`Finding`]s, most severe first. Each rule has a
//! [`CheckId`] so tests and the Accessibility check can ask for one rule by name.

use std::collections::{BTreeMap, HashMap};

use metallic_core::{InstanceId, World};
use metallic_runtime::input::{ActionDef, ActionType, BindingDef, Control, Role, Trigger};

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

/// Runs every rule.
pub fn run_checks(world: &World, baseline: &NameBaseline) -> Vec<Finding> {
    let model = InputModel::read(world);
    let mut out = Vec::new();
    let target = |a: &ActionDef| a.instance.into_iter().collect::<Vec<_>>();

    for ctx in &model.config.contexts {
        if !ctx.enabled {
            continue;
        }
        // Same control on two enabled actions of one context.
        let mut users: BTreeMap<(String, String), Vec<&ActionDef>> = BTreeMap::new();
        for a in ctx.actions.iter().filter(|a| a.enabled) {
            for b in &a.bindings {
                for (key, mods) in signatures(b) {
                    let mut label: Vec<&str> = mods.iter().map(|&m| control_label(m)).collect();
                    label.push(control_label(key));
                    let entry = users
                        .entry((key.name().to_string(), label.join(" + ")))
                        .or_default();
                    if !entry.iter().any(|x| x.name == a.name) {
                        entry.push(a);
                    }
                }
            }
        }
        for ((_, label), actions) in users {
            if actions.len() > 1 {
                let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
                out.push(Finding {
                    id: CheckId::DuplicateControl,
                    level: Level::Error,
                    message: format!(
                        "{label} is bound to {} in {}",
                        names.join(" and "),
                        ctx.name
                    ),
                    targets: actions.iter().filter_map(|a| a.instance).collect(),
                });
            }
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
                if b.reads(Control::Escape) {
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
    use metallic_runtime::input::default_character_controls;

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
