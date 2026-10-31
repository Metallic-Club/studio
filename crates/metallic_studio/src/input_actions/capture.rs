//! Binding capture: "Press a key, button or move a stick" (spec 8.2, used by the panel's Listen).
//!
//! A [`Listener`] is fed what the devices do, one frame of [`CaptureInput`]s at a time, and
//! answers with a [`CaptureOutcome`]. The rules:
//!
//! - The first control that crosses its press threshold binds. If several cross in the same
//!   frame, the largest magnitude wins (a stick bumped while a trigger is pulled).
//! - Controls that were already down when listening started are ignored until they are released,
//!   so the Enter or Space that pressed "Listen" does not bind itself.
//! - Esc cancels and Backspace clears the slot. When the slot is the `Menu` action, Esc binds
//!   instead and the gamepad's B button cancels.
//! - A modifier (Shift, Ctrl, Alt) held while another key goes down makes a chord, if the slot
//!   allows chords. A modifier pressed and released on its own binds the modifier itself (Studio
//!   rule: creators bind Run to Shift; the Player's Controls page may refuse lone modifiers).
//! - Mouse movement and the pointer position never bind by being moved: they bind only through
//!   the explicit device picker. The wheel binds on its first notch.
//! - After [`LISTEN_TIMEOUT`] seconds without a result the listener gives up.

use metallic_core::{CoreError, InstanceId, Value, World};
use metallic_runtime::input::{Control, ControlKind};

use super::commands::{Snapshot, StepBuilder};
use super::model::{InputModel, prop_string};

/// Seconds a listener waits before giving up.
pub const LISTEN_TIMEOUT: f64 = 8.0;
/// Magnitude a stick or analog trigger must pass to bind.
pub const PRESS_THRESHOLD: f32 = 0.5;

/// What a device did during one frame, as the listener needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptureInput {
    /// A key or button went down (`value` 0 to 1; analog triggers pass their pull).
    Press(Control, f32),
    /// A key or button went up.
    Release(Control),
    /// A stick moved; `magnitude` is the tilt 0 to 1.
    Axis(Control, f32),
    /// The wheel turned (notches).
    Wheel(f32),
}

/// What was captured: a control and the modifiers held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Captured {
    /// The control that binds (`KeyCode`).
    pub control: Control,
    /// `PrimaryModifier` and `SecondaryModifier`.
    pub modifiers: [Option<Control>; 2],
}

impl Captured {
    /// A capture without modifiers.
    pub fn plain(control: Control) -> Self {
        Self {
            control,
            modifiers: [None; 2],
        }
    }
}

/// The result of feeding a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// Still listening.
    Listening,
    /// A control was captured.
    Bound(Captured),
    /// The player cancelled (Esc, or B on the Menu slot).
    Cancelled,
    /// The slot should be emptied (Backspace).
    Cleared,
    /// Nothing happened within [`LISTEN_TIMEOUT`].
    TimedOut,
}

/// True for Shift, Ctrl and Alt.
pub fn is_modifier(c: Control) -> bool {
    matches!(
        c,
        Control::LeftShift
            | Control::RightShift
            | Control::LeftControl
            | Control::RightControl
            | Control::LeftAlt
            | Control::RightAlt
    )
}

/// Listens for one binding.
#[derive(Debug, Clone)]
pub struct Listener {
    started: f64,
    menu_slot: bool,
    allow_chords: bool,
    /// Down when listening began; ignored until released.
    stale: Vec<Control>,
    /// Modifiers held now, in press order, and whether another key went down while they were held.
    held_modifiers: Vec<Control>,
    modifier_used: bool,
}

impl Listener {
    /// Starts listening at time `now` (seconds). `already_down` are the controls held right now.
    pub fn new(now: f64, already_down: &[Control]) -> Self {
        Self {
            started: now,
            menu_slot: false,
            allow_chords: true,
            stale: already_down.to_vec(),
            held_modifiers: Vec::new(),
            modifier_used: false,
        }
    }

    /// The slot belongs to the `Menu` action: Esc binds, gamepad B cancels.
    pub fn for_menu(mut self, menu: bool) -> Self {
        self.menu_slot = menu;
        self
    }

    /// Whether held modifiers turn into a chord.
    pub fn allow_chords(mut self, allow: bool) -> Self {
        self.allow_chords = allow;
        self
    }

    /// Seconds left before the listener gives up.
    pub fn remaining(&self, now: f64) -> f64 {
        (LISTEN_TIMEOUT - (now - self.started)).max(0.0)
    }

    fn chord(&self, control: Control) -> Captured {
        let mut modifiers = [None; 2];
        if self.allow_chords {
            for (slot, &m) in modifiers
                .iter_mut()
                .zip(self.held_modifiers.iter().filter(|&&m| m != control))
            {
                *slot = Some(m);
            }
        }
        Captured { control, modifiers }
    }

    /// Feeds one frame of input at time `now`.
    pub fn feed(&mut self, now: f64, inputs: &[CaptureInput]) -> CaptureOutcome {
        // Releases first: they free stale controls and may complete a lone modifier.
        let mut lone_modifier = None;
        for input in inputs {
            if let CaptureInput::Release(c) = *input {
                if let Some(i) = self.stale.iter().position(|&s| s == c) {
                    self.stale.remove(i);
                    continue;
                }
                if let Some(i) = self.held_modifiers.iter().position(|&m| m == c) {
                    self.held_modifiers.remove(i);
                    if !self.modifier_used && lone_modifier.is_none() {
                        lone_modifier = Some(c);
                    }
                    if self.held_modifiers.is_empty() {
                        self.modifier_used = false;
                    }
                }
            }
        }

        // Then the strongest new press.
        let mut best: Option<(Control, f32)> = None;
        for input in inputs {
            let (c, magnitude) = match *input {
                CaptureInput::Press(c, v) => (
                    c,
                    if c.kind() == ControlKind::Button && v <= 0.0 {
                        1.0
                    } else {
                        v
                    },
                ),
                CaptureInput::Axis(c, m) => (c, m),
                CaptureInput::Wheel(n) if n != 0.0 => (Control::MouseWheel, 1.0),
                _ => continue,
            };
            // NaN fails the threshold test too: a broken stick never binds.
            if self.stale.contains(&c) || magnitude.is_nan() || magnitude < PRESS_THRESHOLD {
                continue;
            }
            if matches!(c, Control::MouseDelta | Control::MousePosition) {
                continue;
            }
            if best.is_none_or(|(_, m)| magnitude > m) {
                best = Some((c, magnitude));
            }
        }

        if let Some((c, _)) = best {
            match c {
                Control::Escape if !self.menu_slot => return CaptureOutcome::Cancelled,
                Control::ButtonB if self.menu_slot => return CaptureOutcome::Cancelled,
                Control::Backspace => return CaptureOutcome::Cleared,
                c if is_modifier(c) => {
                    if !self.held_modifiers.contains(&c) {
                        self.held_modifiers.push(c);
                    }
                }
                c => {
                    self.modifier_used = true;
                    return CaptureOutcome::Bound(self.chord(c));
                }
            }
        }
        if let Some(m) = lone_modifier {
            return CaptureOutcome::Bound(Captured::plain(m));
        }
        if now - self.started >= LISTEN_TIMEOUT {
            return CaptureOutcome::TimedOut;
        }
        CaptureOutcome::Listening
    }
}

// ---------------------------------------------------------------------------------------------
// Conflicts (spec 8.3): the same control on two enabled actions of one RebindGroup.

/// Another action that already uses a captured control.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    /// `Context/Action` of the other action.
    pub path: String,
    /// Its `InputBinding` that reads the control.
    pub binding: InstanceId,
    /// Which property of that binding holds the control (`KeyCode`, `Up`, ...).
    pub prop: &'static str,
}

/// How the creator settles a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The other action gets the control this slot had (only when a binding is being replaced).
    Swap,
    /// The other action loses the control.
    Replace,
    /// Both keep it (the Checks strip keeps showing the error).
    KeepBoth,
}

const CONTROL_PROPS: [&str; 7] = [
    "KeyCode", "Up", "Down", "Left", "Right", "Forward", "Backward",
];

fn binding_modifiers(world: &World, id: InstanceId) -> Vec<Control> {
    let mut m: Vec<Control> = ["PrimaryModifier", "SecondaryModifier"]
        .iter()
        .filter_map(|p| Control::from_name(&prop_string(world, id, p)))
        .collect();
    m.sort_by_key(|c| c.name());
    m
}

/// Bindings of other enabled actions in the same `RebindGroup` that read `captured` with the
/// same modifiers. `action` is the action being bound; `replacing` the binding being replaced.
pub fn conflicts_for(
    world: &World,
    action: InstanceId,
    replacing: Option<InstanceId>,
    captured: &Captured,
) -> Vec<Conflict> {
    let model = InputModel::read(world);
    let Some((own_ctx, _)) = model.action(action) else {
        return Vec::new();
    };
    let group = own_ctx.rebind_group.clone();
    let mut mods: Vec<Control> = captured.modifiers.iter().flatten().copied().collect();
    mods.sort_by_key(|c| c.name());
    let mut out = Vec::new();
    for (ctx, a) in model.actions() {
        if !ctx.enabled || !a.enabled || ctx.rebind_group != group || a.instance == Some(action) {
            continue;
        }
        for b in &a.bindings {
            let Some(bid) = b.instance else { continue };
            if Some(bid) == replacing || binding_modifiers(world, bid) != mods {
                continue;
            }
            for prop in CONTROL_PROPS {
                if Control::from_name(&prop_string(world, bid, prop)) == Some(captured.control) {
                    out.push(Conflict {
                        path: format!("{}/{}", ctx.name, a.name),
                        binding: bid,
                        prop,
                    });
                }
            }
        }
    }
    out
}

/// What a binding's slot reads now: the control in `prop` and, for `KeyCode`, its two modifiers.
/// Read it before overwriting the slot, to hand to [`resolve_conflicts`] as `old`.
pub fn captured_of(world: &World, binding: InstanceId, prop: &str) -> Option<Captured> {
    let control = Control::from_name(&prop_string(world, binding, prop))?;
    let mut modifiers = [None; 2];
    if prop == "KeyCode" {
        for (slot, p) in modifiers
            .iter_mut()
            .zip(["PrimaryModifier", "SecondaryModifier"])
        {
            *slot = Control::from_name(&prop_string(world, binding, p));
        }
    }
    Some(Captured { control, modifiers })
}

/// Settles conflicts inside an undo step. `old` is what the slot had before (for Swap): the other
/// binding gets that control together with its modifiers.
pub fn resolve_conflicts(
    b: &mut StepBuilder<'_>,
    conflicts: &[Conflict],
    resolution: Resolution,
    old: Option<Captured>,
) -> Result<(), CoreError> {
    for c in conflicts {
        match resolution {
            Resolution::KeepBoth => {}
            Resolution::Swap if old.is_some() => {
                let old = old.expect("checked");
                b.set(c.binding, c.prop, Value::String(old.control.name().into()))?;
                if c.prop == "KeyCode" {
                    for (i, prop) in ["PrimaryModifier", "SecondaryModifier"].iter().enumerate() {
                        let name = old.modifiers[i].map_or("None", Control::name);
                        b.set(c.binding, prop, Value::String(name.into()))?;
                    }
                }
            }
            Resolution::Swap | Resolution::Replace => {
                // A single-key binding goes away; a composite only loses that direction.
                if c.prop == "KeyCode" {
                    if b.world().is_alive(c.binding) {
                        b.destroy(c.binding)?;
                    }
                } else {
                    b.set(c.binding, c.prop, Value::String("None".into()))?;
                }
            }
        }
    }
    Ok(())
}

/// Writes a capture into a binding: `KeyCode` and both modifiers.
pub fn write_capture(
    b: &mut StepBuilder<'_>,
    binding: InstanceId,
    captured: &Captured,
) -> Result<(), CoreError> {
    b.set(
        binding,
        "KeyCode",
        Value::String(captured.control.name().into()),
    )?;
    for (i, prop) in ["PrimaryModifier", "SecondaryModifier"].iter().enumerate() {
        let name = captured.modifiers[i].map_or("None", Control::name);
        b.set(binding, prop, Value::String(name.into()))?;
    }
    Ok(())
}

/// A new binding snapshot for a capture.
pub fn capture_snapshot(captured: &Captured) -> Snapshot {
    let mut s = Snapshot::new("InputBinding")
        .with("KeyCode", Value::String(captured.control.name().into()));
    for (i, prop) in ["PrimaryModifier", "SecondaryModifier"].iter().enumerate() {
        if let Some(m) = captured.modifiers[i] {
            s = s.with(prop, Value::String(m.name().into()));
        }
    }
    s
}

/// A four-key composite (Up, Down, Left, Right) for a Direction2D action.
pub fn composite_snapshot(keys: [Control; 4]) -> Snapshot {
    let mut s = Snapshot::new("InputBinding");
    for (prop, k) in ["Up", "Down", "Left", "Right"].iter().zip(keys) {
        s = s.with(prop, Value::String(k.name().into()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use CaptureInput::*;
    use Control as C;

    #[test]
    fn first_key_binds() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(l.feed(0.1, &[]), CaptureOutcome::Listening);
        assert_eq!(
            l.feed(0.2, &[Press(C::E, 1.0)]),
            CaptureOutcome::Bound(Captured::plain(C::E))
        );
    }

    #[test]
    fn key_that_started_listening_is_ignored_until_released() {
        let mut l = Listener::new(0.0, &[C::Return]);
        assert_eq!(
            l.feed(0.1, &[Press(C::Return, 1.0)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.2, &[Release(C::Return)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.3, &[Press(C::Return, 1.0)]),
            CaptureOutcome::Bound(Captured::plain(C::Return))
        );
    }

    #[test]
    fn esc_cancels_backspace_clears() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(0.1, &[Press(C::Escape, 1.0)]),
            CaptureOutcome::Cancelled
        );
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(0.1, &[Press(C::Backspace, 1.0)]),
            CaptureOutcome::Cleared
        );
    }

    #[test]
    fn menu_slot_binds_esc_and_cancels_on_b() {
        let mut l = Listener::new(0.0, &[]).for_menu(true);
        assert_eq!(
            l.feed(0.1, &[Press(C::Escape, 1.0)]),
            CaptureOutcome::Bound(Captured::plain(C::Escape))
        );
        let mut l = Listener::new(0.0, &[]).for_menu(true);
        assert_eq!(
            l.feed(0.1, &[Press(C::ButtonB, 1.0)]),
            CaptureOutcome::Cancelled
        );
    }

    #[test]
    fn largest_magnitude_wins_and_drift_is_ignored() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(0.1, &[Axis(C::Thumbstick1, 0.3)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.2, &[Axis(C::Thumbstick1, 0.6), Press(C::ButtonR2, 0.9)]),
            CaptureOutcome::Bound(Captured::plain(C::ButtonR2))
        );
    }

    #[test]
    fn mouse_motion_never_binds_wheel_does() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(
                0.1,
                &[Axis(C::MouseDelta, 1.0), Axis(C::MousePosition, 1.0)]
            ),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.2, &[Wheel(-1.0)]),
            CaptureOutcome::Bound(Captured::plain(C::MouseWheel))
        );
    }

    #[test]
    fn chords_and_lone_modifiers() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(0.1, &[Press(C::LeftShift, 1.0)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.2, &[Press(C::W, 1.0)]),
            CaptureOutcome::Bound(Captured {
                control: C::W,
                modifiers: [Some(C::LeftShift), None]
            })
        );
        let mut l = Listener::new(0.0, &[]).allow_chords(false);
        l.feed(0.1, &[Press(C::LeftControl, 1.0)]);
        assert_eq!(
            l.feed(0.2, &[Press(C::Z, 1.0)]),
            CaptureOutcome::Bound(Captured::plain(C::Z))
        );
        let mut l = Listener::new(0.0, &[]);
        l.feed(0.1, &[Press(C::LeftShift, 1.0)]);
        assert_eq!(
            l.feed(0.3, &[Release(C::LeftShift)]),
            CaptureOutcome::Bound(Captured::plain(C::LeftShift))
        );
    }

    #[test]
    fn times_out_after_eight_seconds() {
        let mut l = Listener::new(10.0, &[]);
        assert_eq!(l.feed(17.9, &[]), CaptureOutcome::Listening);
        assert!((l.remaining(17.0) - 1.0).abs() < 1e-9);
        assert_eq!(l.feed(18.0, &[]), CaptureOutcome::TimedOut);
    }

    // --- conflicts ---------------------------------------------------------------------------

    use crate::input_actions::commands::{CommandSink, History};
    use crate::input_actions::model::context_snapshot;
    use metallic_runtime::input::default_character_controls;

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

    fn keys_of(w: &World, action: InstanceId) -> Vec<String> {
        w.children(action)
            .iter()
            .map(|&b| {
                CONTROL_PROPS
                    .iter()
                    .map(|p| prop_string(w, b, p))
                    .filter(|s| s != "None")
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn conflict_found_in_the_same_group_only() {
        let (mut w, ctx) = sample();
        let jump = w.find_first_child(ctx, "Jump").unwrap();
        let w_key = Captured::plain(C::W);
        let c = conflicts_for(&w, jump, None, &w_key);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, "CharacterControls/Move");
        assert_eq!(c[0].prop, "Up");
        // Shift + W is a different control.
        let chord = Captured {
            control: C::W,
            modifiers: [Some(C::LeftShift), None],
        };
        assert!(conflicts_for(&w, jump, None, &chord).is_empty());
        // Another rebind group never conflicts.
        let rs = w.root("ReplicatedStorage").unwrap();
        let menu = w.create("InputContext", Some(rs)).unwrap();
        w.set(menu, "RebindGroup", Value::String("Menu".into()))
            .unwrap();
        let open = w.create("InputAction", Some(menu)).unwrap();
        assert!(conflicts_for(&w, open, None, &w_key).is_empty());
    }

    #[test]
    fn replace_swap_and_keep_in_one_undo_step() {
        let (mut w, ctx) = sample();
        let jump = w.find_first_child(ctx, "Jump").unwrap();
        let run = w.find_first_child(ctx, "Run").unwrap();
        let space = w.children(jump)[0];
        let mut h = History::new();

        // Bind Jump's Space slot to LeftShift and swap: Run gets Space.
        let cap = Captured::plain(C::LeftShift);
        let conflicts = conflicts_for(&w, jump, Some(space), &cap);
        assert_eq!(conflicts.len(), 1);
        let mut b = StepBuilder::new(&mut w);
        resolve_conflicts(
            &mut b,
            &conflicts,
            Resolution::Swap,
            Some(Captured::plain(C::Space)),
        )
        .unwrap();
        write_capture(&mut b, space, &cap).unwrap();
        h.record(b.finish("Bind").unwrap());
        assert_eq!(keys_of(&w, jump), ["LeftShift", "ButtonA"]);
        assert_eq!(keys_of(&w, run), ["Space", "RightShift", "ButtonL3"]);
        h.undo(&mut w);
        assert_eq!(keys_of(&w, jump), ["Space", "ButtonA"]);
        assert_eq!(keys_of(&w, run), ["LeftShift", "RightShift", "ButtonL3"]);

        // A new Jump binding on W with Replace: Move loses only its Up key.
        let cap = Captured::plain(C::W);
        let mv = w.find_first_child(ctx, "Move").unwrap();
        let conflicts = conflicts_for(&w, jump, None, &cap);
        let mut b = StepBuilder::new(&mut w);
        resolve_conflicts(&mut b, &conflicts, Resolution::Replace, None).unwrap();
        b.create(jump, capture_snapshot(&cap)).unwrap();
        h.record(b.finish("Bind").unwrap());
        assert_eq!(keys_of(&w, mv)[0], "S A D");
        assert_eq!(keys_of(&w, jump).last().unwrap(), "W");
        h.undo(&mut w);
        assert_eq!(keys_of(&w, mv)[0], "W S A D");
        assert_eq!(keys_of(&w, jump).len(), 2);

        // Replace on a single-key binding removes that binding.
        let cap = Captured::plain(C::RightShift);
        let conflicts = conflicts_for(&w, jump, None, &cap);
        let mut b = StepBuilder::new(&mut w);
        resolve_conflicts(&mut b, &conflicts, Resolution::Replace, None).unwrap();
        let _ = b.finish("x");
        assert_eq!(keys_of(&w, run), ["LeftShift", "ButtonL3"]);

        // Keep both changes nothing.
        let conflicts = conflicts_for(&w, jump, None, &Captured::plain(C::ButtonL3));
        let mut b = StepBuilder::new(&mut w);
        resolve_conflicts(&mut b, &conflicts, Resolution::KeepBoth, None).unwrap();
        assert!(b.finish("x").is_none());
    }

    #[test]
    fn nan_stick_magnitude_never_binds() {
        let mut l = Listener::new(0.0, &[]);
        assert_eq!(
            l.feed(0.1, &[Axis(C::Thumbstick1, f32::NAN)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(0.2, &[Press(C::ButtonA, f32::NAN)]),
            CaptureOutcome::Listening
        );
        assert_eq!(
            l.feed(
                0.3,
                &[Axis(C::Thumbstick1, f32::NAN), Axis(C::Thumbstick2, 0.7)]
            ),
            CaptureOutcome::Bound(Captured::plain(C::Thumbstick2))
        );
    }

    #[test]
    fn swap_gives_the_other_binding_the_old_control_with_its_modifiers() {
        let mut w = World::new_with_services();
        let rs = w.root("ReplicatedStorage").unwrap();
        let ctx = w.create("InputContext", Some(rs)).unwrap();
        let bind = |w: &mut World, name: &str, key: &str, modifier: &str| {
            let a = w.create("InputAction", Some(ctx)).unwrap();
            w.set(a, "Name", Value::String(name.into())).unwrap();
            let b = w.create("InputBinding", Some(a)).unwrap();
            w.set(b, "KeyCode", Value::String(key.into())).unwrap();
            w.set(b, "PrimaryModifier", Value::String(modifier.into()))
                .unwrap();
            (a, b)
        };
        let (a, slot) = bind(&mut w, "A", "E", "LeftAlt");
        let (_, other) = bind(&mut w, "B", "F", "LeftControl");

        let cap = Captured {
            control: C::F,
            modifiers: [Some(C::LeftControl), None],
        };
        let old = captured_of(&w, slot, "KeyCode");
        assert_eq!(
            old,
            Some(Captured {
                control: C::E,
                modifiers: [Some(C::LeftAlt), None]
            })
        );
        let conflicts = conflicts_for(&w, a, Some(slot), &cap);
        assert_eq!(conflicts.len(), 1);
        let mut b = StepBuilder::new(&mut w);
        resolve_conflicts(&mut b, &conflicts, Resolution::Swap, old).unwrap();
        write_capture(&mut b, slot, &cap).unwrap();
        let _ = b.finish("Bind");
        assert_eq!(prop_string(&w, other, "KeyCode"), "E");
        assert_eq!(prop_string(&w, other, "PrimaryModifier"), "LeftAlt");
        assert_eq!(prop_string(&w, other, "SecondaryModifier"), "None");
    }
}
