//! Unit tests for the input action runtime (spec 14): no window, no gamepad, no GPU.
use super::*;
use metallic_core::{Value, World};

const KB: Device = Device::KeyboardMouse;
const PAD: Device = Device::Gamepad(0);
const DT: f32 = 1.0 / 60.0;

/// A runtime plus a clock, so tests read like "press at 10.0 s".
struct Rig {
    rt: InputRuntime,
    t: f64,
}

impl Rig {
    fn new(config: &InputConfig) -> Self {
        Rig {
            rt: InputRuntime::new(config),
            t: 10.0,
        }
    }

    fn key(&mut self, c: Control, v: f32) {
        self.rt.push(RawEvent::button(KB, c, v, self.t));
    }

    fn stick(&mut self, c: Control, x: f32, y: f32) {
        self.rt.push(RawEvent::vector(PAD, c, [x, y], self.t));
    }

    /// Moves the clock to `t` and evaluates.
    fn at(&mut self, t: f64) {
        self.t = t;
        self.rt.evaluate(t, DT);
    }

    /// Evaluates one 60 Hz frame later.
    fn frame(&mut self) {
        self.at(self.t + f64::from(DT));
    }

    fn id(&self, path: &str) -> ActionId {
        self.rt
            .find_action(path)
            .unwrap_or_else(|| panic!("no action {path}"))
    }

    fn events(&mut self) -> Vec<ActionEvent> {
        self.rt.drain_events().collect()
    }

    fn count(&mut self, kind: ActionEventKind) -> usize {
        self.events().iter().filter(|e| e.kind == kind).count()
    }
}

fn config_of(ctxs: Vec<ContextDef>) -> InputConfig {
    InputConfig {
        contexts: ctxs,
        warnings: Vec::new(),
    }
}

fn ctx(name: &str, priority: f64, sink: bool, actions: Vec<ActionDef>) -> ContextDef {
    let mut c = ContextDef::new(name);
    c.priority = priority;
    c.sink = sink;
    c.actions = actions;
    c
}

/// One Bool action `T/A` on key `Q` with a trigger.
fn trigger_rig(trigger: Trigger) -> Rig {
    let a = ActionDef::new("A", ActionType::Bool)
        .with_trigger(trigger)
        .with_binding(BindingDef::key(Control::Q));
    Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]))
}

fn axis_rig(ty: ActionType, binding: BindingDef) -> Rig {
    let a = ActionDef::new("A", ty).with_binding(binding);
    Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]))
}

// ----------------------------------------------------------------------------- dead zone etc.

#[test]
fn dead_zone_zeroes_small_tilt_and_rescales_the_rest() {
    let mut r = axis_rig(
        ActionType::Direction2D,
        BindingDef::key(Control::Thumbstick1),
    );
    let a = r.id("T/A");
    r.stick(Control::Thumbstick1, 0.17, 0.0);
    r.frame();
    assert_eq!(r.rt.axis2(a), [0.0, 0.0]);
    r.stick(Control::Thumbstick1, 1.0, 0.0);
    r.frame();
    assert!((r.rt.axis2(a)[0] - 1.0).abs() < 1e-6);
    r.stick(Control::Thumbstick1, 0.0, 0.59);
    r.frame();
    let y = r.rt.axis2(a)[1];
    assert!(
        (y - 0.5).abs() < 0.01,
        "0.59 rescales to about 0.5, got {y}"
    );
    // Radial: a diagonal is judged by its length, not per axis.
    r.stick(Control::Thumbstick1, 0.12, 0.12);
    r.frame();
    assert_eq!(r.rt.axis2(a), [0.0, 0.0]);
}

#[test]
fn explicit_dead_zone_and_trigger_default() {
    let mut r = axis_rig(
        ActionType::Direction1D,
        BindingDef::key(Control::ButtonR2).with_deadzone(0.5),
    );
    let a = r.id("T/A");
    r.key(Control::ButtonR2, 0.4);
    r.frame();
    assert_eq!(r.rt.axis1(a), 0.0);
    r.key(Control::ButtonR2, 1.0);
    r.frame();
    assert!((r.rt.axis1(a) - 1.0).abs() < 1e-6);
    // The L2 and R2 default is 0.05.
    let mut r = axis_rig(ActionType::Direction1D, BindingDef::key(Control::ButtonL2));
    let a = r.id("T/A");
    r.key(Control::ButtonL2, 0.04);
    r.frame();
    assert_eq!(r.rt.axis1(a), 0.0);
    r.key(Control::ButtonL2, 0.5);
    r.frame();
    assert!(r.rt.axis1(a) > 0.4);
}

#[test]
fn response_curve_and_scale() {
    let mut r = axis_rig(
        ActionType::Direction1D,
        BindingDef::key(Control::ButtonL2)
            .with_deadzone(0.0)
            .with_curve(2.0)
            .with_scale(-4.0),
    );
    let a = r.id("T/A");
    r.key(Control::ButtonL2, 0.5);
    r.frame();
    assert!((r.rt.axis1(a) + 1.0).abs() < 1e-6, "0.5^2 * -4");
}

#[test]
fn scale_by_delta_time_uses_the_frame_time() {
    let mut r = axis_rig(
        ActionType::Direction1D,
        BindingDef::key(Control::ButtonR1)
            .with_scale(12.0)
            .per_second(),
    );
    let a = r.id("T/A");
    r.key(Control::ButtonR1, 1.0);
    r.t += 0.5;
    r.rt.evaluate(r.t, 0.5);
    assert!((r.rt.axis1(a) - 6.0).abs() < 1e-5);
}

// ----------------------------------------------------------------------------- composites

fn wasd() -> BindingDef {
    BindingDef::composite(Control::W, Control::S, Control::A, Control::D)
}

#[test]
fn composite_opposites_cancel_and_diagonals_have_length_one() {
    let mut r = axis_rig(ActionType::Direction2D, wasd());
    let a = r.id("T/A");
    r.key(Control::A, 1.0);
    r.key(Control::D, 1.0);
    r.frame();
    assert_eq!(r.rt.axis2(a), [0.0, 0.0]);
    r.key(Control::A, 0.0);
    r.key(Control::W, 1.0);
    r.frame();
    let v = r.rt.axis2(a);
    // D and W: diagonal of length 1.
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    assert!((len - 1.0).abs() < 1e-6, "{v:?}");
    assert!(v[0] > 0.0 && v[1] > 0.0);
}

#[test]
fn composite_3d_uses_forward_and_backward_for_z() {
    let mut b = wasd();
    b.composite[4] = Some(Control::E);
    b.composite[5] = Some(Control::Q);
    let mut r = axis_rig(ActionType::Direction3D, b);
    let a = r.id("T/A");
    r.key(Control::E, 1.0);
    r.frame();
    assert_eq!(r.rt.axis3(a), [0.0, 0.0, 1.0]);
}

#[test]
fn largest_magnitude_wins_instead_of_adding() {
    let mut cfg = default_character_controls();
    cfg.actions.truncate(1);
    let mut r = Rig::new(&config_of(vec![cfg]));
    let mv = r.id("CharacterControls/Move");
    r.key(Control::W, 1.0);
    r.stick(Control::Thumbstick1, 0.0, 1.0);
    r.frame();
    assert_eq!(r.rt.axis2(mv), [0.0, 1.0], "key and stick do not add up");
    // A half-tilted stick alone walks at half speed (rescaled by the dead zone).
    r.key(Control::W, 0.0);
    r.stick(Control::Thumbstick1, 0.0, 0.59);
    r.frame();
    let y = r.rt.axis2(mv)[1];
    assert!(y > 0.3 && y < 0.8, "{y}");
}

#[test]
fn bool_is_the_or_of_its_bindings() {
    let a = ActionDef::new("Jump", ActionType::Bool)
        .with_binding(BindingDef::key(Control::Space))
        .with_binding(BindingDef::key(Control::ButtonA));
    let mut r = Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]));
    let j = r.id("T/Jump");
    r.key(Control::Space, 1.0);
    r.frame();
    assert!(r.rt.bool(j));
    r.rt.push(RawEvent::button(PAD, Control::ButtonA, 1.0, r.t));
    r.key(Control::Space, 0.0);
    r.frame();
    assert!(r.rt.bool(j), "the pad button still holds it");
    r.rt.push(RawEvent::button(PAD, Control::ButtonA, 0.0, r.t));
    r.frame();
    assert!(!r.rt.bool(j));
}

#[test]
fn analog_trigger_passes_the_pressed_and_released_thresholds() {
    let a = ActionDef::new("Fire", ActionType::Bool)
        .with_binding(BindingDef::key(Control::ButtonR2).with_deadzone(0.0));
    let mut r = Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]));
    let f = r.id("T/Fire");
    for (v, down) in [(0.45, false), (0.55, true), (0.45, true), (0.39, false)] {
        r.rt.push(RawEvent::button(PAD, Control::ButtonR2, v, r.t));
        r.frame();
        assert_eq!(r.rt.bool(f), down, "at {v}");
    }
}

#[test]
fn modifier_binding_beats_the_bare_key() {
    let a = ActionDef::new("A", ActionType::Bool)
        .with_binding(BindingDef::key(Control::W))
        .with_binding(BindingDef::key(Control::W).with_modifier(Control::LeftShift));
    let mut r = Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]));
    let id = r.id("T/A");
    // W alone: the bare binding.
    r.key(Control::W, 1.0);
    r.frame();
    assert!(r.rt.bool(id));
    assert_eq!(r.count(ActionEventKind::Pressed), 1);
    // Shift+W: the modified binding takes over; the action stays down and fires no second press.
    r.key(Control::LeftShift, 1.0);
    r.frame();
    assert!(r.rt.bool(id));
    assert_eq!(r.count(ActionEventKind::Pressed), 0);
    // Shift without W: nothing.
    r.key(Control::W, 0.0);
    r.frame();
    assert!(!r.rt.bool(id));
}

#[test]
fn bare_key_does_not_fire_with_the_modifier_on_a_separate_action_binding() {
    // Shift+W alone must not leave the bare binding contributing: two bindings, one action,
    // the modified one is shadowing, so releasing Shift (W held) keeps the action down via bare W.
    let a = ActionDef::new("A", ActionType::Bool)
        .with_binding(BindingDef::key(Control::W))
        .with_binding(BindingDef::key(Control::W).with_modifier(Control::LeftShift));
    let mut r = Rig::new(&config_of(vec![ctx("T", 0.0, false, vec![a])]));
    let id = r.id("T/A");
    r.key(Control::LeftShift, 1.0);
    r.key(Control::W, 1.0);
    r.frame();
    assert!(r.rt.bool(id));
    r.key(Control::LeftShift, 0.0);
    r.frame();
    assert!(r.rt.bool(id));
}

#[test]
fn mouse_look_needs_the_right_button() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.rt.push(RawEvent::vector(
        KB,
        Control::MouseDelta,
        [100.0, 50.0],
        r.t,
    ));
    r.frame();
    assert_eq!(r.rt.frame_input().orbit, [0.0, 0.0]);
    r.key(Control::MouseButton2, 1.0);
    r.frame();
    assert!(r.rt.wants_mouse_delta());
    r.rt.push(RawEvent::vector(
        KB,
        Control::MouseDelta,
        [100.0, 50.0],
        r.t,
    ));
    r.frame();
    let f = r.rt.frame_input();
    assert!(
        (f.orbit[0] - 0.6).abs() < 1e-5 && (f.orbit[1] - 0.3).abs() < 1e-5,
        "{f:?}"
    );
    // Cleared after the frame.
    r.frame();
    assert_eq!(r.rt.frame_input().orbit, [0.0, 0.0]);
    r.key(Control::MouseButton2, 0.0);
    r.frame();
    assert!(!r.rt.wants_mouse_delta());
}

#[test]
fn wheel_zoom_matches_the_old_numbers() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.rt.push(RawEvent::wheel(KB, 1.0, r.t));
    r.frame();
    assert_eq!(
        r.rt.frame_input().zoom,
        -2.0,
        "a notch up zooms in by 2 studs"
    );
    r.frame();
    assert_eq!(r.rt.frame_input().zoom, 0.0);
}

#[test]
fn right_stick_turns_at_three_radians_per_second() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.stick(Control::Thumbstick2, 1.0, 0.0);
    r.t += 0.1;
    r.rt.evaluate(r.t, 0.1);
    let f = r.rt.frame_input();
    assert!((f.orbit[0] - 0.3).abs() < 1e-5, "{f:?}");
    r.stick(Control::Thumbstick2, 0.0, 1.0);
    r.t += 0.1;
    r.rt.evaluate(r.t, 0.1);
    assert!(
        (r.rt.frame_input().orbit[1] + 0.3).abs() < 1e-5,
        "stick up looks up"
    );
}

#[test]
fn character_controls_keyboard_and_pad_feed_the_frame_input() {
    let mut r = Rig::new(&InputConfig::character_controls());
    assert_eq!(r.rt.frame_input(), FrameInput::default());
    r.key(Control::W, 1.0);
    r.key(Control::D, 1.0);
    r.key(Control::Space, 1.0);
    r.key(Control::LeftShift, 1.0);
    r.frame();
    let f = r.rt.frame_input();
    assert!(f.movement[0] > 0.0 && f.movement[1] > 0.0);
    assert!(f.jump && f.run);
    r.key(Control::W, 0.0);
    r.key(Control::D, 0.0);
    r.key(Control::Space, 0.0);
    r.key(Control::LeftShift, 0.0);
    r.rt.push(RawEvent::button(PAD, Control::ButtonA, 1.0, r.t));
    r.rt.push(RawEvent::button(PAD, Control::ButtonL3, 1.0, r.t));
    r.rt.push(RawEvent::button(PAD, Control::DPadUp, 1.0, r.t));
    r.frame();
    let f = r.rt.frame_input();
    assert_eq!(f.movement, [0.0, 1.0]);
    assert!(f.jump && f.run);
}

// ----------------------------------------------------------------------------- triggers

fn press_at(r: &mut Rig, t: f64, down: bool) {
    r.rt.push(RawEvent::button(
        KB,
        Control::Q,
        if down { 1.0 } else { 0.0 },
        t,
    ));
    r.at(t);
}

#[test]
fn press_trigger_fires_on_the_press_edge_only() {
    let mut r = trigger_rig(Trigger::Press);
    press_at(&mut r, 10.0, true);
    assert_eq!(r.count(ActionEventKind::Triggered), 1);
    r.at(11.0);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
    press_at(&mut r, 11.5, false);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
}

#[test]
fn release_trigger_fires_on_the_release_edge() {
    let mut r = trigger_rig(Trigger::Release);
    press_at(&mut r, 10.0, true);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
    press_at(&mut r, 10.7, false);
    assert_eq!(r.count(ActionEventKind::Triggered), 1);
}

#[test]
fn hold_fires_once_at_500_ms_and_not_at_499() {
    let mut r = trigger_rig(Trigger::Hold);
    let a = r.id("T/A");
    press_at(&mut r, 10.0, true);
    r.at(10.499);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
    assert!(r.rt.hold_progress(a) > 0.99 && r.rt.hold_progress(a) < 1.0);
    r.at(10.5);
    assert_eq!(r.count(ActionEventKind::Triggered), 1);
    r.at(12.0);
    assert_eq!(r.count(ActionEventKind::Triggered), 0, "once per press");
    assert_eq!(r.rt.hold_progress(a), 1.0);
    press_at(&mut r, 12.1, false);
    assert_eq!(r.rt.hold_progress(a), 0.0);
    // A shorter press never fires.
    press_at(&mut r, 13.0, true);
    press_at(&mut r, 13.499, false);
    r.at(14.0);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
}

#[test]
fn hold_fires_even_when_the_frame_is_long() {
    let mut r = trigger_rig(Trigger::Hold);
    press_at(&mut r, 10.0, true);
    r.at(10.9);
    let ev = r.events();
    let fired: Vec<_> = ev
        .iter()
        .filter(|e| e.kind == ActionEventKind::Triggered)
        .collect();
    assert_eq!(fired.len(), 1);
    assert!(
        (fired[0].time - 10.5).abs() < 1e-9,
        "stamped with the moment it was due"
    );
}

#[test]
fn tap_fires_on_release_within_200_ms() {
    for (ms, fires) in [(199.0, true), (201.0, false)] {
        let mut r = trigger_rig(Trigger::Tap);
        press_at(&mut r, 10.0, true);
        assert_eq!(r.count(ActionEventKind::Triggered), 0);
        press_at(&mut r, 10.0 + ms / 1000.0, false);
        assert_eq!(
            r.count(ActionEventKind::Triggered),
            usize::from(fires),
            "{ms} ms"
        );
    }
}

#[test]
fn double_tap_needs_two_quick_taps_within_the_window() {
    for (gap, fires) in [(299.0, true), (301.0, false)] {
        let mut r = trigger_rig(Trigger::DoubleTap);
        press_at(&mut r, 10.0, true);
        press_at(&mut r, 10.05, false);
        assert_eq!(r.count(ActionEventKind::Triggered), 0);
        let second = 10.05 + gap / 1000.0;
        press_at(&mut r, second, true);
        press_at(&mut r, second + 0.05, false);
        assert_eq!(
            r.count(ActionEventKind::Triggered),
            usize::from(fires),
            "gap {gap} ms"
        );
    }
    // A long second press is no tap.
    let mut r = trigger_rig(Trigger::DoubleTap);
    press_at(&mut r, 10.0, true);
    press_at(&mut r, 10.05, false);
    press_at(&mut r, 10.1, true);
    press_at(&mut r, 10.5, false);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
    // A third tap right after a double tap starts over.
    let mut r = trigger_rig(Trigger::DoubleTap);
    for (i, t) in [10.0, 10.1, 10.2].into_iter().enumerate() {
        press_at(&mut r, t, true);
        press_at(&mut r, t + 0.05, false);
        assert_eq!(
            r.count(ActionEventKind::Triggered),
            usize::from(i == 1),
            "tap {i}"
        );
    }
}

#[test]
fn repeat_fires_on_press_then_at_400_500_600_ms() {
    let mut r = trigger_rig(Trigger::Repeat);
    press_at(&mut r, 10.0, true);
    assert_eq!(r.count(ActionEventKind::Triggered), 1, "the press itself");
    r.at(10.399);
    assert_eq!(r.count(ActionEventKind::Triggered), 0);
    for (t, n) in [(10.4, 1), (10.499, 0), (10.5, 1), (10.599, 0), (10.6, 1)] {
        r.at(t);
        assert_eq!(r.count(ActionEventKind::Triggered), n, "at {t}");
    }
    press_at(&mut r, 10.65, false);
    r.at(11.0);
    assert_eq!(
        r.count(ActionEventKind::Triggered),
        0,
        "released: no more repeats"
    );
}

#[test]
fn repeat_after_a_long_pause_does_not_burst() {
    let mut r = trigger_rig(Trigger::Repeat);
    press_at(&mut r, 10.0, true);
    r.events();
    r.at(70.0);
    let n = r.count(ActionEventKind::Triggered);
    assert!(n <= 32, "{n}");
}

#[test]
fn a_tap_shorter_than_a_frame_is_not_lost() {
    let mut r = trigger_rig(Trigger::Tap);
    let a = r.id("T/A");
    r.rt.push(RawEvent::button(KB, Control::Q, 1.0, 10.001));
    r.rt.push(RawEvent::button(KB, Control::Q, 0.0, 10.005));
    r.at(10.016);
    let ev = r.events();
    assert!(!r.rt.bool(a));
    for kind in [
        ActionEventKind::Pressed,
        ActionEventKind::Released,
        ActionEventKind::Triggered,
    ] {
        assert_eq!(ev.iter().filter(|e| e.kind == kind).count(), 1, "{kind:?}");
    }
}

#[test]
fn pressed_and_released_follow_the_raw_state_whatever_the_trigger() {
    let mut r = trigger_rig(Trigger::Hold);
    press_at(&mut r, 10.0, true);
    press_at(&mut r, 10.1, false);
    let ev = r.events();
    assert_eq!(
        ev.iter()
            .filter(|e| e.kind == ActionEventKind::Pressed)
            .count(),
        1
    );
    assert_eq!(
        ev.iter()
            .filter(|e| e.kind == ActionEventKind::Released)
            .count(),
        1
    );
}

#[test]
fn vector_actions_announce_changes() {
    let mut r = axis_rig(ActionType::Direction2D, wasd());
    r.frame();
    assert!(r.events().is_empty());
    r.key(Control::W, 1.0);
    r.frame();
    let ev = r.events();
    assert!(
        ev.iter().any(|e| e.kind == ActionEventKind::StateChanged
            && e.value == ActionValue::Axis2([0.0, 1.0]))
    );
    assert!(ev.iter().any(|e| e.kind == ActionEventKind::Triggered));
    r.frame();
    assert!(r.events().is_empty(), "no change, no event");
}

// ----------------------------------------------------------------------------- priority, sink

fn vehicle_config(sink: bool) -> InputConfig {
    let drive = ActionDef::new("Drive", ActionType::Direction1D).with_binding({
        let mut b = BindingDef::composite(Control::W, Control::S, Control::A, Control::A);
        b.composite[2] = None;
        b.composite[3] = None;
        b
    });
    let honk = ActionDef::new("Honk", ActionType::Bool).with_binding(BindingDef::key(Control::W));
    let mut cc = default_character_controls();
    cc.actions.truncate(1);
    config_of(vec![cc, ctx("Vehicle", 10.0, sink, vec![drive, honk])])
}

#[test]
fn without_sink_both_contexts_see_the_key() {
    let mut r = Rig::new(&vehicle_config(false));
    r.key(Control::W, 1.0);
    r.frame();
    assert_eq!(r.rt.axis2(r.id("CharacterControls/Move")), [0.0, 1.0]);
    assert_eq!(r.rt.axis1(r.id("Vehicle/Drive")), 1.0);
}

#[test]
fn a_sinking_context_takes_its_keys_and_lets_the_others_through() {
    let mut r = Rig::new(&vehicle_config(true));
    let mv = r.id("CharacterControls/Move");
    r.key(Control::W, 1.0);
    r.key(Control::S, 1.0);
    r.key(Control::D, 1.0);
    r.frame();
    assert_eq!(
        r.rt.axis1(r.id("Vehicle/Drive")),
        0.0,
        "W and S cancel in the vehicle"
    );
    // W and S are sunk; D still walks.
    assert_eq!(r.rt.axis2(mv), [1.0, 0.0]);
    // A key only the vehicle uses.
    r.key(Control::S, 0.0);
    r.frame();
    assert_eq!(r.rt.axis1(r.id("Vehicle/Drive")), 1.0);
    assert_eq!(
        r.rt.axis2(mv),
        [1.0, 0.0],
        "W is sunk: the walk does not go forward"
    );
}

#[test]
fn disabling_the_sinking_context_hands_the_keys_back() {
    let mut cfg = vehicle_config(true);
    let mut r = Rig::new(&cfg);
    r.key(Control::W, 1.0);
    r.frame();
    assert_eq!(r.rt.axis2(r.id("CharacterControls/Move")), [0.0, 0.0]);
    cfg.contexts[1].enabled = false;
    r.rt.load(&cfg);
    r.frame();
    assert_eq!(r.rt.axis2(r.id("CharacterControls/Move")), [0.0, 1.0]);
    assert_eq!(
        r.rt.axis1(r.id("Vehicle/Drive")),
        0.0,
        "disabled actions read zero"
    );
}

#[test]
fn a_lower_priority_context_cannot_sink_a_higher_one() {
    // The sinking context is below: the higher one still reads W.
    let drive = ActionDef::new("Honk", ActionType::Bool).with_binding(BindingDef::key(Control::W));
    let cc = default_character_controls();
    let mut r = Rig::new(&config_of(vec![ctx("High", 5.0, false, vec![drive]), {
        let mut c = cc;
        c.sink = true;
        c.priority = -1.0;
        c
    }]));
    r.key(Control::W, 1.0);
    r.frame();
    assert!(r.rt.bool(r.id("High/Honk")));
    assert_eq!(r.rt.axis2(r.id("CharacterControls/Move")), [0.0, 1.0]);
}

#[test]
fn equal_priority_asks_the_most_recent_context_first() {
    let mk = |name: &str, order: u64| {
        let a = ActionDef::new("A", ActionType::Bool).with_binding(BindingDef::key(Control::W));
        let mut c = ctx(name, 0.0, name == "New", vec![a]);
        c.order = order;
        c
    };
    let mut r = Rig::new(&config_of(vec![mk("Old", 1), mk("New", 2)]));
    r.key(Control::W, 1.0);
    r.frame();
    assert!(r.rt.bool(r.id("New/A")));
    assert!(!r.rt.bool(r.id("Old/A")), "New sinks W and is asked first");
}

#[test]
fn gui_capture_hides_pointer_and_keyboard_controls() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.key(Control::W, 1.0);
    r.frame();
    assert_eq!(r.rt.frame_input().movement, [0.0, 1.0]);
    r.rt.set_gui_capture(false, true);
    r.frame();
    assert_eq!(
        r.rt.frame_input().movement,
        [0.0, 0.0],
        "a TextBox has focus"
    );
    r.rt.push(RawEvent::button(PAD, Control::ButtonA, 1.0, r.t));
    r.frame();
    assert!(r.rt.frame_input().jump, "pad buttons still reach the game");
    r.rt.set_gui_capture(false, false);
    r.frame();
    assert_eq!(r.rt.frame_input().movement, [0.0, 1.0]);
}

#[test]
fn release_all_ends_every_held_action() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.key(Control::W, 1.0);
    r.key(Control::Space, 1.0);
    r.stick(Control::Thumbstick1, 1.0, 0.0);
    r.frame();
    r.rt.release_all(r.t);
    r.frame();
    assert_eq!(r.rt.frame_input(), FrameInput::default());
}

// ----------------------------------------------------------------------------- last device

fn device_rig() -> Rig {
    Rig::new(&InputConfig::character_controls())
}

#[test]
fn drift_below_half_does_not_switch_devices() {
    let mut r = device_rig();
    r.stick(Control::Thumbstick1, 0.4, 0.0);
    r.at(11.0);
    assert_eq!(r.rt.last_device(), DeviceKind::KeyboardMouse);
    r.stick(Control::Thumbstick1, 0.6, 0.0);
    r.at(12.0);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
}

#[test]
fn the_current_device_must_be_quiet_for_300_ms() {
    let mut r = device_rig();
    r.rt.push(RawEvent::button(KB, Control::E, 1.0, 10.0));
    r.rt.push(RawEvent::button(KB, Control::E, 0.0, 10.01));
    r.at(10.02);
    let before = r.rt.device_changes();
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 10.299));
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 0.0, 10.3));
    r.at(10.31);
    assert_eq!(
        r.rt.last_device(),
        DeviceKind::KeyboardMouse,
        "only 289 ms since the last key"
    );
    assert_eq!(r.rt.device_changes(), before);
    // The key's last meaningful input was at 10.0; 300 ms later it is quiet.
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 10.3));
    r.at(10.32);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
}

#[test]
fn a_held_key_keeps_its_device_current() {
    let mut r = device_rig();
    r.key(Control::W, 1.0);
    r.at(10.0);
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 12.0));
    r.at(12.0);
    assert_eq!(r.rt.last_device(), DeviceKind::KeyboardMouse);
    r.key(Control::W, 0.0);
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 0.0, 12.5));
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 12.6));
    r.at(12.7);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
}

#[test]
fn switches_are_at_least_500_ms_apart() {
    let mut r = device_rig();
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 10.0));
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 0.0, 10.05));
    r.at(10.06);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
    // 400 ms after the switch the pad has been quiet for more than 300 ms, but 500 ms have not passed.
    r.rt.push(RawEvent::button(KB, Control::E, 1.0, 10.4));
    r.rt.push(RawEvent::button(KB, Control::E, 0.0, 10.41));
    r.at(10.42);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
    r.rt.push(RawEvent::button(KB, Control::E, 1.0, 10.5));
    r.at(10.51);
    assert_eq!(r.rt.last_device(), DeviceKind::KeyboardMouse);
}

#[test]
fn mouse_movement_counts_from_12_px_within_200_ms() {
    let mut r = device_rig();
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 1.0, 9.0));
    r.rt.push(RawEvent::button(PAD, Control::ButtonB, 0.0, 9.01));
    r.at(9.5);
    assert_eq!(r.rt.last_device(), DeviceKind::Gamepad);
    for i in 0..4 {
        r.rt.push(RawEvent::vector(
            KB,
            Control::MouseDelta,
            [2.0, 0.0],
            10.0 + 0.1 * f64::from(i),
        ));
    }
    r.at(10.4);
    assert_eq!(
        r.rt.last_device(),
        DeviceKind::Gamepad,
        "8 px spread over 300 ms"
    );
    r.rt.push(RawEvent::vector(KB, Control::MouseDelta, [7.0, 0.0], 11.0));
    r.rt.push(RawEvent::vector(KB, Control::MouseDelta, [6.0, 0.0], 11.1));
    r.at(11.2);
    assert_eq!(r.rt.last_device(), DeviceKind::KeyboardMouse);
}

#[test]
fn any_finger_down_switches_to_touch() {
    let mut r = device_rig();
    r.rt.push(RawEvent::button(
        Device::Touch,
        Control::TouchButton,
        1.0,
        11.0,
    ));
    r.at(11.1);
    assert_eq!(r.rt.last_device(), DeviceKind::Touch);
}

// ----------------------------------------------------------------------------- queue, scene, script

#[test]
fn full_queue_merges_mouse_moves_and_keeps_every_button_edge() {
    let mut q = InputQueue::new();
    for i in 0..QUEUE_CAPACITY {
        q.push(RawEvent::vector(
            KB,
            Control::MouseDelta,
            [1.0, 0.0],
            i as f64,
        ));
    }
    assert_eq!(q.len(), QUEUE_CAPACITY);
    q.push(RawEvent::vector(
        KB,
        Control::MouseDelta,
        [5.0, 0.0],
        2000.0,
    ));
    assert_eq!(q.len(), QUEUE_CAPACITY, "merged");
    q.push(RawEvent::button(KB, Control::Space, 1.0, 2001.0));
    assert_eq!(
        q.len(),
        QUEUE_CAPACITY,
        "the edge took the place of the oldest move"
    );
    let mut sum = 0.0;
    let mut edges = 0;
    while let Some(e) = q.pop() {
        match e.control {
            Control::MouseDelta => sum += e.pos.map_or(0.0, |p| p[0]),
            Control::Space => edges += 1,
            _ => {}
        }
    }
    assert_eq!(edges, 1);
    assert_eq!(
        sum,
        1023.0 + 5.0,
        "one move was dropped for the edge, the 5 px merged"
    );
    // Edges alone grow the queue instead of being dropped.
    for i in 0..QUEUE_CAPACITY + 10 {
        q.push(RawEvent::button(
            KB,
            Control::Space,
            (i % 2) as f32,
            i as f64,
        ));
    }
    assert_eq!(q.len(), QUEUE_CAPACITY + 10);
}

#[test]
fn non_finite_events_are_dropped() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.rt.push(RawEvent::button(KB, Control::W, f32::NAN, r.t));
    r.rt.push(RawEvent::vector(
        PAD,
        Control::Thumbstick1,
        [f32::INFINITY, 0.0],
        r.t,
    ));
    r.frame();
    assert_eq!(r.rt.frame_input(), FrameInput::default());
}

#[test]
fn control_names_round_trip() {
    for &c in Control::ALL {
        assert_eq!(Control::from_name(c.name()), Some(c));
    }
    assert_eq!(Control::from_name("None"), None);
    assert_eq!(Control::from_name("w"), None);
    assert!(Control::ALL.len() < 256);
}

fn world_with_input() -> (World, metallic_core::InstanceId) {
    let mut w = World::new_with_services();
    let rs = w.root("ReplicatedStorage").unwrap();
    let ctx = w.create("InputContext", Some(rs)).unwrap();
    w.set(ctx, "Name", Value::String("Gameplay".into()))
        .unwrap();
    (w, ctx)
}

fn add_action(w: &mut World, ctx: metallic_core::InstanceId, name: &str, key: &str) {
    let a = w.create("InputAction", Some(ctx)).unwrap();
    w.set(a, "Name", Value::String(name.into())).unwrap();
    let b = w.create("InputBinding", Some(a)).unwrap();
    w.set(b, "KeyCode", Value::String(key.into())).unwrap();
}

#[test]
fn scene_without_character_controls_gets_the_built_in_one() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "F");
    let mut r = Rig {
        rt: InputRuntime::from_world(&w),
        t: 10.0,
    };
    assert!(r.rt.find_action("Gameplay/Dash").is_some());
    assert!(r.rt.find_action("CharacterControls/Move").is_some());
    r.key(Control::F, 1.0);
    r.frame();
    assert!(r.rt.bool(r.id("Gameplay/Dash")));
}

#[test]
fn a_scene_character_controls_context_wins_entirely() {
    let (mut w, _) = world_with_input();
    let rs = w.root("ReplicatedStorage").unwrap();
    let cc = w.create("InputContext", Some(rs)).unwrap();
    w.set(cc, "Name", Value::String("CharacterControls".into()))
        .unwrap();
    add_action(&mut w, cc, "Jump", "J");
    let mut r = Rig {
        rt: InputRuntime::from_world(&w),
        t: 10.0,
    };
    assert!(
        r.rt.find_action("CharacterControls/Move").is_none(),
        "no per-action fallback"
    );
    r.key(Control::Space, 1.0);
    r.key(Control::W, 1.0);
    r.frame();
    assert_eq!(r.rt.frame_input(), FrameInput::default());
    r.key(Control::J, 1.0);
    r.frame();
    assert!(r.rt.frame_input().jump);
}

#[test]
fn disabled_context_and_action_read_zero() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "F");
    add_action(&mut w, ctx, "Roll", "G");
    let roll = w.find_first_child(ctx, "Roll").unwrap();
    w.set(roll, "Enabled", Value::Bool(false)).unwrap();
    let mut r = Rig {
        rt: InputRuntime::from_world(&w),
        t: 10.0,
    };
    r.key(Control::F, 1.0);
    r.key(Control::G, 1.0);
    r.frame();
    assert!(r.rt.bool(r.id("Gameplay/Dash")));
    assert!(!r.rt.bool(r.id("Gameplay/Roll")));
    w.set(ctx, "Enabled", Value::Bool(false)).unwrap();
    assert!(r.rt.sync(&w));
    r.frame();
    assert!(!r.rt.bool(r.id("Gameplay/Dash")));
}

#[test]
fn sync_reloads_only_when_an_input_instance_changed() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "F");
    let mut rt = InputRuntime::from_world(&w);
    assert!(!rt.sync(&w), "nothing changed");
    let ws = w.root("Workspace").unwrap();
    w.create("Part", Some(ws)).unwrap();
    assert!(!rt.sync(&w), "a part is not input");
    let dash = w.find_first_child(ctx, "Dash").unwrap();
    let bind = w.children(dash)[0];
    w.set(bind, "KeyCode", Value::String("G".into())).unwrap();
    assert!(rt.sync(&w));
    let mut r = Rig { rt, t: 10.0 };
    r.key(Control::G, 1.0);
    r.frame();
    assert!(r.rt.bool(r.id("Gameplay/Dash")));
}

#[test]
fn a_reload_keeps_held_actions_held() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "F");
    let mut r = Rig {
        rt: InputRuntime::from_world(&w),
        t: 10.0,
    };
    r.key(Control::F, 1.0);
    r.frame();
    assert_eq!(r.count(ActionEventKind::Pressed), 1);
    w.set(ctx, "Priority", Value::Number(3.0)).unwrap();
    assert!(r.rt.sync(&w));
    r.frame();
    assert_eq!(r.count(ActionEventKind::Pressed), 0, "no second press");
    assert!(r.rt.bool(r.id("Gameplay/Dash")));
}

#[test]
fn unknown_control_names_are_reported_and_ignored() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "Fnord");
    let rt = InputRuntime::from_world(&w);
    assert_eq!(rt.warnings().len(), 1);
    assert!(rt.warnings()[0].contains("Gameplay/Dash"));
}

#[test]
fn input_script_parses_and_replays_by_frame() {
    let text = "# comment\n\n30 keyboard Space down\n36 keyboard Space up\n40 gamepad0 Thumbstick1 axis 0.0 1.0\n\
                50 keyboard MouseDelta delta 3 4\n60 keyboard MouseWheel delta 1\n";
    let mut s = ScriptedSource::parse(text, 1.0 / 60.0).unwrap();
    assert_eq!(s.last_frame(), 60);
    let mut out = Vec::new();
    for _ in 0..31 {
        s.poll(&mut out);
    }
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].control, Control::Space);
    assert!((out[0].time - 0.5).abs() < 1e-9);
    out.clear();
    for _ in 31..61 {
        s.poll(&mut out);
    }
    assert_eq!(out.len(), 4);
    assert_eq!(out[1].pos, Some([0.0, 1.0]));
    assert_eq!(out[1].device, PAD);
    assert_eq!(out[2].pos, Some([3.0, 4.0]));
    assert_eq!(out[3].value, 1.0);
}

#[test]
fn input_script_errors_name_the_line() {
    for (text, want) in [
        ("1 keyboard Space\n", "line 1"),
        ("x keyboard Space down\n", "bad frame"),
        ("1 joystick Space down\n", "unknown device"),
        ("1 keyboard Spacebar down\n", "unknown control"),
        ("1 keyboard Space hop\n", "unknown state"),
        ("\n1 gamepad Thumbstick1 axis\n", "line 2"),
        ("1 gamepad4 ButtonA down\n", "unknown device"),
    ] {
        let err = ScriptedSource::parse(text, 0.016).unwrap_err();
        assert!(err.contains(want), "{text:?}: {err}");
    }
}

#[test]
fn a_script_drives_the_character() {
    let text = "1 keyboard W down\n2 keyboard Space down\n10 keyboard W up\n10 keyboard Space up\n";
    let mut s = ScriptedSource::parse(text, f64::from(DT)).unwrap();
    let mut rt = InputRuntime::from_world(&World::new_with_services());
    let mut walked = 0;
    for frame in 0..20 {
        rt.poll_source(&mut s);
        let f = rt.frame(f64::from(frame) * f64::from(DT), DT);
        if f.movement == [0.0, 1.0] && f.jump {
            walked += 1;
        }
    }
    assert_eq!(walked, 8, "frames 2 to 9");
}

#[test]
fn destroying_a_lone_binding_stops_it() {
    let (mut w, ctx) = world_with_input();
    add_action(&mut w, ctx, "Dash", "F");
    let mut rt = InputRuntime::from_world(&w);
    let dash = w.find_first_child(ctx, "Dash").unwrap();
    let bind = w.children(dash)[0];
    w.destroy(bind);
    assert!(rt.sync(&w), "a destroyed binding reloads the runtime");
    let mut r = Rig { rt, t: 10.0 };
    r.key(Control::F, 1.0);
    r.frame();
    assert!(
        !r.rt.bool(r.id("Gameplay/Dash")),
        "the deleted binding no longer fires"
    );
}

#[test]
fn undrained_events_are_capped_and_the_newest_are_kept() {
    let mut r = trigger_rig(Trigger::Press);
    for i in 0..10_000 {
        press_at(&mut r, 10.0 + f64::from(i) * 0.01, true);
        press_at(&mut r, 10.005 + f64::from(i) * 0.01, false);
    }
    assert!(r.rt.dropped_events() > 0);
    let ev = r.events();
    assert!(ev.len() <= 1024);
    assert!(
        ev.last().is_some_and(|e| e.time > 109.0),
        "the newest events survive"
    );
}

#[test]
fn luau_key_codes_match_the_controls() {
    // `Enum.KeyCode` (metallic_script) lists `Unknown` and then every control, in this order.
    let controls: Vec<&str> = Control::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(metallic_script::KEY_CODES[0], "Unknown");
    assert_eq!(&metallic_script::KEY_CODES[1..], controls.as_slice());
}
