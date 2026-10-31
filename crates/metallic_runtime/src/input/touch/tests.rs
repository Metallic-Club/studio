//! Touch controls: layout rule, stick maths, finger routing (spec 7, 14).
use super::layout::{MIN_CONTROL, MIN_GAP, SAFE_MARGIN, dist};
use super::*;
use crate::input::{
    ActionDef, ActionEvent, ActionEventKind, ActionType, BindingDef, ContextDef, Control,
    DeviceKind, InputConfig, InputRuntime, Trigger,
};

const DT: f32 = 1.0 / 60.0;

fn config_of(ctxs: Vec<ContextDef>) -> InputConfig {
    InputConfig {
        contexts: ctxs,
        warnings: Vec::new(),
    }
}

fn button_action(name: &str, order: f32) -> ActionDef {
    let mut a = ActionDef::new(name, ActionType::Bool).with_binding(BindingDef::key(Control::Q));
    a.show_on_touch = true;
    a.touch_order = order;
    a
}

/// `CharacterControls` (stick, look pad, Jump, Run) plus `extra` more buttons in context `Game`.
fn config_with_buttons(extra: usize) -> InputConfig {
    let mut game = ContextDef::new("Game");
    game.actions = (0..extra)
        .map(|i| button_action(&format!("B{i}"), 2.0 + i as f32))
        .collect();
    config_of(vec![crate::input::default_character_controls(), game])
}

/// A config with exactly `n` touch buttons (Jump, Run, then extras) and the stick.
fn config_n_buttons(n: usize) -> InputConfig {
    let mut c = config_with_buttons(n.saturating_sub(2));
    if n < 2 {
        c.contexts[0].actions.retain(|a| a.name != "Run");
    }
    if n == 0 {
        c.contexts[0].actions.retain(|a| a.name != "Jump");
    }
    c
}

fn set_of(config: &InputConfig) -> TouchSet {
    InputRuntime::new(config).touch_set().clone()
}

/// Checks the rules of spec 7.2 on a layout.
fn check_rules(l: &TouchLayout, what: &str) {
    let area = l.safe.inset(SAFE_MARGIN);
    let circles = l.circles();
    for (i, &(c, r)) in circles.iter().enumerate() {
        assert!(
            2.0 * r >= MIN_CONTROL - 1e-3,
            "{what}: control {i} is {} dp",
            2.0 * r
        );
        let rect = Rect::around(c, r);
        assert!(
            area.contains_rect(&rect, 1e-3),
            "{what}: control {i} {rect:?} leaves the safe area minus 16 dp {area:?}"
        );
        for (j, &(o, or)) in circles.iter().enumerate().skip(i + 1) {
            let gap = dist(c, o) - r - or;
            assert!(
                gap >= MIN_GAP - 1e-3,
                "{what}: controls {i} and {j} are {gap:.2} dp apart"
            );
        }
    }
}

// ----------------------------------------------------------------------------- touch set

#[test]
fn default_controls_get_the_stick_the_look_pad_and_two_buttons() {
    let set = set_of(&InputConfig::character_controls());
    assert_eq!(set.stick.as_ref().unwrap().path, "CharacterControls/Move");
    assert_eq!(set.look.as_ref().unwrap().path, "CharacterControls/Look");
    let names: Vec<&str> = set.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(names, ["Jump", "Run"], "TouchOrder 0 then 1");
    assert_eq!(set.buttons[0].icon, TouchIcon::Jump);
    assert_eq!(set.buttons[1].slot, 1);
    assert!(set.left_out.is_empty());
}

#[test]
fn buttons_sort_by_touch_order_and_stop_at_six() {
    let mut game = ContextDef::new("Game");
    game.actions = (0..8)
        .map(|i| button_action(&format!("B{i}"), 8.0 - i as f32))
        .collect();
    game.actions[3].touch_label = "Use".into();
    let set = set_of(&config_of(vec![game]));
    let names: Vec<&str> = set.buttons.iter().map(|b| b.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "Game/B7", "Game/B6", "Game/B5", "Game/B4", "Game/B3", "Game/B2"
        ]
    );
    assert_eq!(set.buttons[4].label, "Use", "TouchLabel wins over the name");
    assert_eq!(set.left_out, ["Game/B1", "Game/B0"]);
}

#[test]
fn touch_slot_and_priority_decide_the_stick() {
    let mut steer = ActionDef::new("Steer", ActionType::Direction2D);
    steer.show_on_touch = true;
    let mut car = ContextDef::new("Car");
    car.priority = 100.0;
    car.actions = vec![steer];
    let set = set_of(&config_of(vec![
        crate::input::default_character_controls(),
        car,
    ]));
    assert_eq!(
        set.stick.as_ref().unwrap().path,
        "Car/Steer",
        "the higher context wins"
    );
    assert_eq!(set.look.as_ref().unwrap().path, "CharacterControls/Look");
    assert_eq!(
        set.left_out,
        ["CharacterControls/Move"],
        "Move never becomes the look pad"
    );
}

#[test]
fn a_menu_action_gets_the_menu_button() {
    let mut menu = button_action("Menu", 0.0).with_role(crate::input::Role::Menu);
    menu.display_name = "Pause".into();
    let mut sys = ContextDef::new("Metallic.System");
    sys.actions = vec![menu];
    let set = set_of(&config_of(vec![sys]));
    let m = set.menu.as_ref().unwrap();
    assert_eq!(
        (m.slot, m.label.as_str(), m.icon),
        (MENU_SLOT, "Pause", TouchIcon::Menu)
    );
    assert!(set.buttons.is_empty());
    let l = layout(&DeviceProfile::phone(), &set);
    let placed = l.menu.unwrap();
    assert_eq!(placed.radius, 24.0);
    assert_eq!(
        placed.center,
        [422.0, 16.0 + 24.0],
        "top centre of the safe area, 16 dp down"
    );
}

// ----------------------------------------------------------------------------- layout

#[test]
fn phone_landscape_matches_the_mockup() {
    // docs/design/input-actions/touch-layout.html section 2.
    let set = set_of(&config_n_buttons(6));
    let l = layout(&DeviceProfile::phone(), &set);
    let corner = |i: usize| {
        let r = l.buttons[i].rect();
        [
            (r.x * 10.0).round() / 10.0,
            (r.y * 10.0).round() / 10.0,
            r.w,
        ]
    };
    assert_eq!(corner(0), [700.0, 274.0, 72.0], "primary");
    assert_eq!(corner(1), [624.0, 282.0, 56.0], "S1");
    assert_eq!(corner(2), [654.0, 217.7, 56.0], "S2");
    assert_eq!(corner(3), [722.6, 199.3, 56.0], "S3");
    assert_eq!(corner(4), [556.6, 268.8, 56.0], "S4");
    assert_eq!(corner(5), [591.6, 184.3, 56.0], "S5");
    assert_eq!(
        l.buttons.len(),
        6,
        "six buttons at most: the primary and S1 to S5"
    );
    assert!(l.buttons[0].primary && !l.buttons[1].primary);
    let stick = l.stick.unwrap();
    assert_eq!((stick.center, stick.radius), ([132.0, 286.0], 60.0));
    let z = l.stick_zone;
    assert!((z.x, z.w, z.right()) == (48.0, 374.0, 422.0), "{z:?}");
    assert!(
        (z.y - 55.5).abs() < 1e-3 && (z.bottom() - 370.0).abs() < 1e-3,
        "{z:?}"
    );
    check_rules(&l, "phone");
}

#[test]
fn tablet_uses_the_larger_sizes() {
    let set = set_of(&config_n_buttons(4));
    let l = layout(&DeviceProfile::tablet(), &set);
    // Spec 7.2: centre at safe-area right - 24 - size/2, bottom - 24 - size/2.
    assert_eq!(l.buttons[0].rect(), Rect::new(1072.0, 692.0, 84.0, 84.0));
    assert_eq!(l.buttons[1].radius, 32.0);
    let ring1 = dist(l.buttons[0].center, l.buttons[1].center);
    assert!((ring1 - 84.0 * 1.167).abs() < 1e-3, "{ring1}");
    assert_eq!(l.stick.unwrap().radius, 70.0);
}

/// Spec 14: the three presets, both orientations, 1 to 6 buttons, Standard and Large.
#[test]
fn layout_keeps_every_rule_on_every_preset() {
    let presets = [
        ("small phone", DeviceProfile::small_phone()),
        ("phone", DeviceProfile::phone()),
        ("tablet", DeviceProfile::tablet()),
    ];
    for (name, landscape) in presets {
        for (orient, profile) in [("landscape", landscape), ("portrait", landscape.portrait())] {
            for (large, scale) in [(false, 1.0), (true, 1.0), (false, 0.8), (false, 1.5)] {
                for n in 1..=6 {
                    let set = set_of(&config_n_buttons(n));
                    let p = profile.with_large(large).with_scale(scale);
                    let l = layout(&p, &set);
                    let what = format!("{name} {orient} large={large} x{scale} buttons={n}");
                    check_rules(&l, &what);
                    assert!(l.stick.is_some(), "{what}");
                    if scale != 1.0 {
                        // Bigger controls may leave some out (spec 7.2); smaller ones all fit.
                        assert!(!l.buttons.is_empty(), "{what}: the primary fits");
                        assert_eq!(l.buttons.len() + l.unplaced.len(), n, "{what}");
                        if scale < 1.0 {
                            assert_eq!(l.buttons.len(), n, "{what}: all placed");
                        }
                    } else if large {
                        let min = if p.tablet { 151.0 } else { 94.0 };
                        for (_, r) in l.circles() {
                            assert!(2.0 * r >= min - 1e-3, "{what}: {} dp", 2.0 * r);
                        }
                        assert!(!l.buttons.is_empty(), "{what}: the primary fits");
                        assert_eq!(l.buttons.len() + l.unplaced.len(), n, "{what}");
                    } else {
                        assert_eq!(l.buttons.len(), n, "{what}: all placed, {:?}", l.unplaced);
                    }
                }
            }
        }
    }
}

#[test]
fn layout_stays_inside_odd_screens_and_insets() {
    let set = set_of(&config_n_buttons(6));
    for size in [
        [480.0, 320.0],
        [1920.0, 1080.0],
        [2560.0, 1600.0],
        [700.0, 700.0],
    ] {
        for insets in [[0.0; 4], [44.0, 0.0, 44.0, 21.0], [0.0, 59.0, 0.0, 34.0]] {
            let p = DeviceProfile::for_screen(size, insets);
            check_rules(&layout(&p, &set), &format!("{size:?} {insets:?}"));
        }
    }
    // Degenerate screens never panic and place nothing outside.
    for size in [[0.0, 0.0], [10.0, 10.0], [f32::NAN, 300.0]] {
        let l = layout(&DeviceProfile::for_screen(size, [0.0; 4]), &set);
        let area = l.safe.inset(SAFE_MARGIN);
        for b in &l.buttons {
            assert!(area.contains_rect(&b.rect(), 1e-3) || l.safe.w < 2.0 * SAFE_MARGIN);
        }
    }
}

#[test]
fn custom_layout_reads_position_and_size() {
    let mut c = config_n_buttons(2);
    c.contexts[0].actions[1].touch_custom = Some(crate::input::TouchPlacement {
        position: [0.5, -40.0, 0.0, 30.0],
        size: 80.0,
    });
    let set = set_of(&c);
    let l = layout(&DeviceProfile::phone(), &set);
    let jump = l
        .buttons
        .iter()
        .find(|b| set.buttons[b.index].label == "Jump")
        .unwrap();
    // Safe area x 48, width 748: 48 + 374 - 40 = 382 left, 0 + 30 = 30 top; 80 dp wide.
    assert_eq!(jump.rect(), Rect::new(382.0, 30.0, 80.0, 80.0));
    // The automatic Run button becomes the primary in the corner.
    let run = l
        .buttons
        .iter()
        .find(|b| set.buttons[b.index].label == "Run")
        .unwrap();
    assert!(run.primary);
    // A custom control below 48 dp grows to 48 and is kept inside the margin.
    c.contexts[0].actions[1].touch_custom = Some(crate::input::TouchPlacement {
        position: [1.0, 0.0, 1.0, 0.0],
        size: 10.0,
    });
    let set = set_of(&c);
    let l = layout(&DeviceProfile::phone(), &set);
    let jump = l
        .buttons
        .iter()
        .find(|b| set.buttons[b.index].label == "Jump")
        .unwrap();
    assert_eq!(jump.radius, 24.0);
    assert!(l.safe.inset(SAFE_MARGIN).contains_rect(&jump.rect(), 1e-3));
}

// ----------------------------------------------------------------------------- stick maths

#[test]
fn floating_stick_follows_past_its_radius() {
    let mut s = Stick::begin(StickMode::Floating, [0.0, 0.0], 60.0, [200.0, 300.0]);
    assert_eq!(s.base, [200.0, 300.0], "the base appears under the thumb");
    assert_eq!(s.value(), [0.0, 0.0]);
    s.drag([230.0, 300.0]);
    assert_eq!(s.value(), [0.5, 0.0]);
    s.drag([200.0, 240.0]);
    assert_eq!(s.value(), [0.0, 1.0], "screen up is y up");
    s.drag([200.0, 200.0]);
    assert_eq!(s.base, [200.0, 260.0], "past R the base follows");
    assert_eq!(s.knob(), [200.0, 200.0]);
    assert_eq!(s.value(), [0.0, 1.0]);
    s.drag([200.0, 260.0]);
    assert_eq!(s.value(), [0.0, 0.0], "back at the new centre");
}

#[test]
fn fixed_stick_stays_and_clamps_the_knob() {
    let mut s = Stick::begin(StickMode::Fixed, [100.0, 100.0], 60.0, [130.0, 100.0]);
    assert_eq!(s.base, [100.0, 100.0]);
    assert_eq!(s.value(), [0.5, 0.0]);
    s.drag([400.0, 100.0]);
    assert_eq!(s.base, [100.0, 100.0], "a fixed base never moves");
    assert_eq!(s.knob(), [160.0, 100.0]);
    assert_eq!(s.value(), [1.0, 0.0]);
    let diag = Stick::begin(StickMode::Fixed, [0.0, 0.0], 60.0, [600.0, -600.0]);
    let v = diag.value();
    assert!((v[0] - v[1]).abs() < 1e-6 && ((v[0] * v[0] + v[1] * v[1]).sqrt() - 1.0).abs() < 1e-6);
}

/// The stick's dead zone is 0.15 R, radial and rescaled so the output starts from 0.
#[test]
fn stick_dead_zone_is_fifteen_percent_of_the_radius() {
    let mut rt = InputRuntime::new(&InputConfig::character_controls());
    let mut tc = TouchControls::new(DeviceProfile::phone().with_stick_mode(StickMode::Fixed));
    tc.sync(&mut rt, 0.0);
    let rest = tc.layout().stick.unwrap();
    let mv = rt.find_action("CharacterControls/Move").unwrap();
    let mut t = 1.0;
    let mut at = |rt: &mut InputRuntime, tc: &mut TouchControls, phase, dx: f32| {
        t += f64::from(DT);
        tc.touch(
            rt,
            1,
            phase,
            [rest.center[0] + dx * rest.radius, rest.center[1]],
            t,
        );
        rt.evaluate(t, DT);
        rt.axis2(mv)[0]
    };
    assert_eq!(
        at(&mut rt, &mut tc, TouchPhase::Down, 0.1),
        0.0,
        "inside the dead zone"
    );
    assert_eq!(at(&mut rt, &mut tc, TouchPhase::Move, 0.149), 0.0);
    let half = at(&mut rt, &mut tc, TouchPhase::Move, 0.575);
    assert!((half - 0.5).abs() < 1e-5, "rescaled: {half}");
    assert!((at(&mut rt, &mut tc, TouchPhase::Move, 1.0) - 1.0).abs() < 1e-6);
    assert!(
        (at(&mut rt, &mut tc, TouchPhase::Move, 3.0) - 1.0).abs() < 1e-6,
        "clamped"
    );
    assert_eq!(at(&mut rt, &mut tc, TouchPhase::Up, 3.0), 0.0, "lifted");
}

// ----------------------------------------------------------------------------- routing

struct Rig {
    rt: InputRuntime,
    tc: TouchControls,
    t: f64,
}

impl Rig {
    fn new(config: &InputConfig) -> Rig {
        let mut rt = InputRuntime::new(config);
        let mut tc = TouchControls::new(DeviceProfile::phone());
        tc.sync(&mut rt, 0.0);
        Rig { rt, tc, t: 1.0 }
    }

    fn touch(&mut self, id: u64, phase: TouchPhase, at: [f32; 2]) {
        self.t += f64::from(DT);
        self.tc.touch(&mut self.rt, id, phase, at, self.t);
        self.rt.evaluate(self.t, DT);
    }

    /// A new config mid-touch, the way a Player frame sees it: load, overlay sync, evaluate.
    fn relayout(&mut self, config: &InputConfig) {
        self.t += f64::from(DT);
        self.rt.load(config);
        assert!(self.tc.sync(&mut self.rt, self.t), "a new touch set");
        self.rt.evaluate(self.t, DT);
    }

    fn button(&self, label: &str) -> PlacedButton {
        let set = self.tc.set();
        *self
            .tc
            .layout()
            .buttons
            .iter()
            .find(|b| set.buttons[b.index].label == label)
            .unwrap()
    }

    fn events(&mut self, path: &str) -> Vec<ActionEventKind> {
        let id = self.rt.find_action(path).unwrap();
        let events: Vec<ActionEvent> = self.rt.drain_events().collect();
        events
            .iter()
            .filter(|e| e.action == id)
            .map(|e| e.kind)
            .collect()
    }
}

#[test]
fn a_touch_press_fires_the_action_on_touch_down() {
    use ActionEventKind::*;
    let mut r = Rig::new(&InputConfig::character_controls());
    let jump = r.button("Jump");
    r.touch(4, TouchPhase::Down, jump.center);
    assert_eq!(
        r.events("CharacterControls/Jump"),
        [Pressed, StateChanged, Triggered]
    );
    assert!(r.rt.frame_input().jump);
    assert!(r.tc.is_pressed(jump.slot));
    assert_eq!(
        r.rt.last_device(),
        DeviceKind::Touch,
        "a finger down switches to touch"
    );
    r.touch(4, TouchPhase::Move, [jump.center[0] + 20.0, jump.center[1]]);
    assert!(r.rt.frame_input().jump, "small slides keep the press");
    r.touch(4, TouchPhase::Up, jump.center);
    assert_eq!(r.events("CharacterControls/Jump"), [Released, StateChanged]);
    assert!(!r.rt.frame_input().jump);
}

#[test]
fn sliding_off_a_button_cancels_without_triggered() {
    use ActionEventKind::*;
    let mut fire = button_action("Fire", 0.0).with_trigger(Trigger::Release);
    fire.bindings.clear();
    let mut game = ContextDef::new("Game");
    game.actions = vec![fire];
    let mut r = Rig::new(&config_of(vec![game]));
    let b = r.button("Fire");
    // A normal tap fires on release.
    r.touch(1, TouchPhase::Down, b.center);
    r.touch(1, TouchPhase::Up, b.center);
    assert_eq!(
        r.events("Game/Fire"),
        [Pressed, StateChanged, Released, StateChanged, Triggered]
    );
    // Sliding more than 2 radii away releases without Triggered.
    r.touch(1, TouchPhase::Down, b.center);
    r.touch(
        1,
        TouchPhase::Move,
        [b.center[0] - 2.1 * b.radius, b.center[1]],
    );
    assert_eq!(
        r.events("Game/Fire"),
        [Pressed, StateChanged, Released, StateChanged]
    );
    assert!(!r.tc.is_pressed(b.slot));
    r.touch(1, TouchPhase::Move, b.center);
    r.touch(1, TouchPhase::Up, b.center);
    assert!(
        r.events("Game/Fire").is_empty(),
        "the finger stays captured and does nothing"
    );
    // A system cancel also ends the press without Triggered.
    r.touch(2, TouchPhase::Down, b.center);
    r.touch(2, TouchPhase::Cancel, b.center);
    assert_eq!(
        r.events("Game/Fire"),
        [Pressed, StateChanged, Released, StateChanged]
    );
}

#[test]
fn two_fingers_on_one_button_release_with_the_last() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let jump = r.button("Jump");
    r.touch(1, TouchPhase::Down, jump.center);
    r.touch(2, TouchPhase::Down, [jump.center[0] + 5.0, jump.center[1]]);
    r.touch(1, TouchPhase::Up, jump.center);
    assert!(r.rt.frame_input().jump);
    r.touch(2, TouchPhase::Up, jump.center);
    assert!(!r.rt.frame_input().jump);
}

#[test]
fn the_space_around_a_button_does_nothing_and_does_not_start_the_stick() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let run = r.button("Run");
    let near = [run.center[0] - run.radius - 4.0, run.center[1]];
    r.touch(1, TouchPhase::Down, near);
    assert_eq!(r.tc.capture_of(1), Some(Capture::Ignored));
    r.touch(1, TouchPhase::Move, [near[0] - 50.0, near[1]]);
    assert!(r.tc.held_stick().is_none());
    assert_eq!(r.rt.frame_input(), crate::input::FrameInput::default());
}

#[test]
fn stick_and_buttons_work_together_and_the_look_pad_turns_the_camera() {
    let mut r = Rig::new(&InputConfig::character_controls());
    // Left half: the stick appears under the thumb.
    r.touch(1, TouchPhase::Down, [200.0, 300.0]);
    assert_eq!(r.tc.capture_of(1), Some(Capture::Stick));
    r.touch(1, TouchPhase::Move, [200.0, 240.0]);
    assert_eq!(r.rt.frame_input().movement, [0.0, 1.0], "full forward");
    // A second finger on Run while the stick is held.
    let run = r.button("Run");
    r.touch(2, TouchPhase::Down, run.center);
    let f = r.rt.frame_input();
    assert!(f.run && f.movement == [0.0, 1.0]);
    // A third finger on the right half drags the camera: 0.01 rad per dp.
    r.touch(3, TouchPhase::Down, [500.0, 100.0]);
    assert_eq!(r.tc.capture_of(3), Some(Capture::Look));
    r.touch(3, TouchPhase::Move, [530.0, 90.0]);
    let orbit = r.rt.frame_input().orbit;
    assert!(
        (orbit[0] - 0.3).abs() < 1e-5 && (orbit[1] + 0.1).abs() < 1e-5,
        "{orbit:?}"
    );
    r.touch(3, TouchPhase::Move, [530.0, 90.0]);
    assert_eq!(
        r.rt.frame_input().orbit,
        [0.0, 0.0],
        "a delta lasts one frame"
    );
    // A second finger in the stick zone does not steal the stick.
    r.touch(4, TouchPhase::Down, [150.0, 320.0]);
    assert_eq!(r.tc.capture_of(4), Some(Capture::Ignored));
    r.touch(1, TouchPhase::Up, [200.0, 240.0]);
    assert_eq!(r.rt.frame_input().movement, [0.0, 0.0]);
    assert!(r.tc.touched_once());
}

#[test]
fn without_a_look_action_the_right_half_is_a_pointer() {
    let mut c = InputConfig::character_controls();
    c.contexts[0].actions.retain(|a| a.name != "Look");
    let mut click = ActionDef::new("Click", ActionType::Bool)
        .with_binding(BindingDef::key(Control::MouseButton1));
    click.show_on_touch = false;
    let pos = ActionDef::new("Where", ActionType::ViewportPosition)
        .with_binding(BindingDef::key(Control::MousePosition));
    let mut g = ContextDef::new("Game");
    g.actions = vec![click, pos];
    c.contexts.push(g);
    let mut r = Rig::new(&c);
    r.tc.set_px_per_dp(2.0);
    r.touch(1, TouchPhase::Down, [500.0, 100.0]);
    assert_eq!(r.tc.capture_of(1), Some(Capture::Pointer));
    let click = r.rt.find_action("Game/Click").unwrap();
    let pos = r.rt.find_action("Game/Where").unwrap();
    assert!(r.rt.bool(click), "a tap acts as MouseButton1");
    assert_eq!(r.rt.axis2(pos), [1000.0, 200.0], "in pixels");
    r.touch(1, TouchPhase::Up, [500.0, 100.0]);
    assert!(!r.rt.bool(click));
}

#[test]
fn the_menu_button_acts_on_release() {
    use ActionEventKind::*;
    let menu = button_action("Menu", 0.0).with_role(crate::input::Role::Menu);
    let mut sys = ContextDef::new("Sys");
    sys.actions = vec![menu];
    let mut r = Rig::new(&config_of(vec![sys]));
    let m = r.tc.layout().menu.unwrap();
    r.touch(1, TouchPhase::Down, m.center);
    assert!(r.events("Sys/Menu").is_empty(), "nothing on down");
    assert!(r.tc.is_pressed(MENU_SLOT));
    r.touch(1, TouchPhase::Up, m.center);
    assert_eq!(
        r.events("Sys/Menu"),
        [Pressed, StateChanged, Triggered, Released, StateChanged]
    );
    r.touch(1, TouchPhase::Down, m.center);
    r.touch(
        1,
        TouchPhase::Move,
        [m.center[0] + 3.0 * m.radius, m.center[1]],
    );
    r.touch(
        1,
        TouchPhase::Up,
        [m.center[0] + 3.0 * m.radius, m.center[1]],
    );
    assert!(r.events("Sys/Menu").is_empty(), "slid off: cancelled");
}

#[test]
fn ten_fingers_at_most_and_a_removed_button_lets_its_finger_go() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let jump = r.button("Jump");
    for id in 0..12 {
        r.touch(id, TouchPhase::Down, [500.0 + id as f32, 50.0]);
    }
    assert_eq!(r.tc.finger_count(), MAX_FINGERS);
    r.tc.release_all(&mut r.rt, r.t);
    assert_eq!(r.tc.finger_count(), 0);
    r.touch(1, TouchPhase::Down, jump.center);
    assert!(r.rt.frame_input().jump);
    // Disabling Jump changes the touch set: its finger holds nothing from then on.
    let mut c = InputConfig::character_controls();
    c.contexts[0].actions[1].enabled = false;
    r.relayout(&c);
    assert!(!r.rt.frame_input().jump);
    assert_eq!(r.tc.set().buttons.len(), 1);
    assert_eq!(r.tc.capture_of(1), Some(Capture::Ignored));
    assert!(!r.tc.sync(&mut r.rt, r.t), "nothing changed since");
}

/// Two Release-trigger buttons, `Fire` first and `Alt` second, in context `Game`.
fn fire_and_alt(fire_order: f32, fire_label: &str) -> InputConfig {
    let mut fire = button_action("Fire", fire_order).with_trigger(Trigger::Release);
    fire.touch_label = fire_label.into();
    let alt = button_action("Alt", 1.0).with_trigger(Trigger::Release);
    let mut game = ContextDef::new("Game");
    game.actions = vec![fire, alt];
    config_of(vec![crate::input::default_character_controls(), game])
}

#[test]
fn a_new_title_keeps_held_fingers() {
    use ActionEventKind::*;
    let mut r = Rig::new(&fire_and_alt(0.0, ""));
    let fire = r.button("Fire");
    r.touch(1, TouchPhase::Down, fire.center);
    r.touch(2, TouchPhase::Down, [200.0, 300.0]);
    r.touch(2, TouchPhase::Move, [200.0, 240.0]);
    assert_eq!(r.events("Game/Fire"), [Pressed, StateChanged]);
    // SetTitle / a TouchLabel edit: a new touch set and a new layout.
    r.relayout(&fire_and_alt(0.0, "Shoot"));
    assert!(r.button("Shoot").center == fire.center);
    assert!(r.events("Game/Fire").is_empty(), "still held, nothing new");
    assert_eq!(r.rt.frame_input().movement, [0.0, 1.0], "the stick too");
    r.touch(2, TouchPhase::Move, [260.0, 300.0]);
    assert_eq!(
        r.rt.frame_input().movement,
        [1.0, 0.0],
        "and it still moves"
    );
    r.touch(1, TouchPhase::Up, fire.center);
    assert_eq!(
        r.events("Game/Fire"),
        [Released, StateChanged, Triggered],
        "the lift still fires"
    );
    r.touch(2, TouchPhase::Up, [260.0, 300.0]);
    assert_eq!(r.rt.frame_input().movement, [0.0, 0.0]);
    assert_eq!(r.tc.finger_count(), 0);
}

#[test]
fn a_held_button_follows_its_action_to_another_slot() {
    use ActionEventKind::*;
    let mut r = Rig::new(&fire_and_alt(0.0, ""));
    let fire = r.button("Fire");
    r.touch(1, TouchPhase::Down, fire.center);
    r.events("Game/Fire");
    // Fire now sorts after Alt: Alt takes Fire's control (TouchButton3) and Fire moves.
    r.relayout(&fire_and_alt(5.0, ""));
    let moved = r.button("Fire");
    assert_ne!(moved.slot, fire.slot);
    assert_eq!(
        r.tc.capture_of(1),
        Some(Capture::Button {
            slot: moved.slot,
            live: true
        })
    );
    assert!(r.tc.is_pressed(moved.slot) && !r.tc.is_pressed(fire.slot));
    assert!(r.events("Game/Fire").is_empty(), "Fire stays down");
    let alt = r.rt.find_action("Game/Alt").unwrap();
    assert!(!r.rt.bool(alt), "the control Fire left does not press Alt");
    r.touch(1, TouchPhase::Up, fire.center);
    let events: Vec<ActionEvent> = r.rt.drain_events().collect();
    let kinds = |id| -> Vec<ActionEventKind> {
        events
            .iter()
            .filter(|e| e.action == id)
            .map(|e| e.kind)
            .collect()
    };
    let fire_id = r.rt.find_action("Game/Fire").unwrap();
    assert_eq!(kinds(fire_id), [Released, StateChanged, Triggered]);
    assert!(kinds(alt).is_empty());
}

#[test]
fn a_held_button_that_goes_away_is_cancelled() {
    use ActionEventKind::*;
    let mut r = Rig::new(&fire_and_alt(0.0, ""));
    let fire = r.button("Fire");
    r.touch(1, TouchPhase::Down, fire.center);
    r.events("Game/Fire");
    let mut c = fire_and_alt(0.0, "");
    c.contexts[1].actions[0].show_on_touch = false;
    r.relayout(&c);
    assert_eq!(
        r.events("Game/Fire"),
        [Released, StateChanged],
        "no Triggered"
    );
    assert_eq!(r.tc.capture_of(1), Some(Capture::Ignored));
    r.touch(1, TouchPhase::Up, fire.center);
    assert!(r.events("Game/Fire").is_empty());
}

/// #333: a held button whose action changes its role is let go as a cancel, whether it stays a
/// button (another icon) or becomes the menu button, and nothing stays pressed.
#[test]
fn a_role_change_cancels_a_held_press() {
    use crate::input::Role;
    use ActionEventKind::*;
    for role in [Role::Menu, Role::Interact] {
        let mut r = Rig::new(&fire_and_alt(0.0, ""));
        let fire = r.button("Fire");
        r.touch(1, TouchPhase::Down, fire.center);
        r.events("Game/Fire");
        let mut c = fire_and_alt(0.0, "");
        c.contexts[1].actions[0].role = role;
        r.relayout(&c);
        assert_eq!(
            r.events("Game/Fire"),
            [Released, StateChanged],
            "{role:?}: cancelled, no Triggered"
        );
        let id = r.rt.find_action("Game/Fire").unwrap();
        assert!(!r.rt.bool(id), "{role:?}: not stuck");
        assert_eq!(r.tc.capture_of(1), Some(Capture::Ignored));
        assert!((0..TOUCH_BUTTON_SLOTS).all(|s| !r.tc.is_pressed(s)));
        r.touch(1, TouchPhase::Up, fire.center);
        assert!(
            r.events("Game/Fire").is_empty(),
            "{role:?}: the lift does nothing"
        );
        // The action works again from a new touch.
        let b = match role {
            Role::Menu => r.tc.layout().menu.unwrap(),
            _ => r.button("Fire"),
        };
        r.touch(2, TouchPhase::Down, b.center);
        r.touch(2, TouchPhase::Up, b.center);
        assert!(
            r.events("Game/Fire").contains(&Pressed),
            "{role:?}: a new tap presses it"
        );
        assert!(!r.rt.bool(id));
    }
}

#[test]
fn a_stick_finger_sliding_over_a_button_keeps_the_stick() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let jump = r.button("Jump");
    r.touch(1, TouchPhase::Down, [200.0, 300.0]);
    r.touch(1, TouchPhase::Move, jump.center);
    assert_eq!(r.tc.capture_of(1), Some(Capture::Stick));
    let f = r.rt.frame_input();
    assert!(!f.jump, "the button is not pressed");
    assert!(
        f.movement[0] > 0.99,
        "the stick still steers: {:?}",
        f.movement
    );
    r.touch(1, TouchPhase::Up, jump.center);
    assert!(!r.rt.frame_input().jump);
}

#[test]
fn a_cancel_on_the_stick_centres_it() {
    let mut r = Rig::new(&InputConfig::character_controls());
    r.touch(1, TouchPhase::Down, [200.0, 300.0]);
    r.touch(1, TouchPhase::Move, [200.0, 240.0]);
    assert_eq!(r.rt.frame_input().movement, [0.0, 1.0]);
    r.touch(1, TouchPhase::Cancel, [200.0, 240.0]);
    assert_eq!(r.rt.frame_input().movement, [0.0, 0.0]);
    assert!(r.tc.held_stick().is_none());
    assert_eq!(r.tc.finger_count(), 0);
    r.touch(2, TouchPhase::Down, [150.0, 300.0]);
    assert_eq!(r.tc.capture_of(2), Some(Capture::Stick), "free again");
}

#[test]
fn focus_loss_releases_everything_cleanly() {
    use ActionEventKind::*;
    let mut r = Rig::new(&fire_and_alt(0.0, ""));
    let fire = r.button("Fire");
    r.touch(1, TouchPhase::Down, fire.center);
    r.touch(2, TouchPhase::Down, [200.0, 300.0]);
    r.touch(2, TouchPhase::Move, [200.0, 240.0]);
    r.touch(3, TouchPhase::Down, [500.0, 100.0]);
    r.events("Game/Fire");
    r.tc.release_all(&mut r.rt, r.t);
    r.rt.evaluate(r.t, DT);
    assert_eq!(
        r.events("Game/Fire"),
        [Released, StateChanged],
        "no Triggered"
    );
    let f = r.rt.frame_input();
    assert_eq!(f.movement, [0.0, 0.0]);
    assert_eq!(r.tc.finger_count(), 0);
    assert!(r.tc.held_stick().is_none() && !r.tc.is_pressed(fire.slot));
    // The lost fingers' later events do nothing.
    r.touch(1, TouchPhase::Up, fire.center);
    r.touch(2, TouchPhase::Move, [260.0, 300.0]);
    assert!(r.events("Game/Fire").is_empty());
    assert_eq!(r.rt.frame_input().movement, [0.0, 0.0]);
    // And new touches work.
    r.touch(4, TouchPhase::Down, fire.center);
    assert_eq!(r.events("Game/Fire"), [Pressed, StateChanged]);
}

#[test]
fn up_or_cancel_without_a_position_still_lifts() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let jump = r.button("Jump");
    r.touch(1, TouchPhase::Down, jump.center);
    r.touch(1, TouchPhase::Up, [f32::NAN, 0.0]);
    assert!(!r.rt.frame_input().jump);
    assert_eq!(r.tc.finger_count(), 0);
    r.touch(2, TouchPhase::Down, [200.0, 300.0]);
    r.touch(2, TouchPhase::Move, [200.0, 240.0]);
    r.touch(2, TouchPhase::Move, [f32::INFINITY, 240.0]);
    assert_eq!(
        r.rt.frame_input().movement,
        [0.0, 1.0],
        "a bad move is ignored"
    );
    r.t += f64::from(DT);
    r.tc.touch(&mut r.rt, 2, TouchPhase::Cancel, [f32::NAN; 2], f64::NAN);
    r.rt.evaluate(r.t, DT);
    assert_eq!(r.rt.frame_input().movement, [0.0, 0.0]);
    assert_eq!(r.tc.finger_count(), 0);
}

#[test]
fn a_reused_finger_id_ends_the_old_touch_first() {
    let mut r = Rig::new(&InputConfig::character_controls());
    let (jump, run) = (r.button("Jump"), r.button("Run"));
    r.touch(1, TouchPhase::Down, jump.center);
    // Its Up was lost: the same id lands on Run.
    r.touch(1, TouchPhase::Down, run.center);
    let f = r.rt.frame_input();
    assert!(!f.jump && f.run);
    assert_eq!(r.tc.finger_count(), 1);
    assert!(!r.tc.is_pressed(jump.slot));
    r.touch(1, TouchPhase::Up, run.center);
    assert!(!r.rt.frame_input().run);
}

#[test]
fn every_pointer_finger_holds_the_mouse_button() {
    let mut c = InputConfig::character_controls();
    c.contexts[0].actions.retain(|a| a.name != "Look");
    let click = ActionDef::new("Click", ActionType::Bool)
        .with_binding(BindingDef::key(Control::MouseButton1));
    let mut g = ContextDef::new("Game");
    g.actions = vec![click];
    c.contexts.push(g);
    let mut r = Rig::new(&c);
    let click = r.rt.find_action("Game/Click").unwrap();
    r.touch(1, TouchPhase::Down, [500.0, 100.0]);
    r.touch(2, TouchPhase::Down, [600.0, 120.0]);
    r.touch(1, TouchPhase::Up, [500.0, 100.0]);
    assert!(r.rt.bool(click), "the second finger still holds it");
    r.touch(2, TouchPhase::Up, [600.0, 120.0]);
    assert!(!r.rt.bool(click));
}

#[test]
fn forced_device_holds_touch() {
    let mut rt = InputRuntime::new(&InputConfig::character_controls());
    rt.set_forced_device(Some(DeviceKind::Touch));
    assert_eq!(rt.last_device(), DeviceKind::Touch);
    rt.push(crate::input::RawEvent::button(
        crate::input::Device::KeyboardMouse,
        Control::Space,
        1.0,
        5.0,
    ));
    rt.evaluate(5.0, DT);
    assert_eq!(rt.last_device(), DeviceKind::Touch);
    rt.set_forced_device(None);
}

#[test]
fn paint_colours_are_the_visual_language_tokens() {
    let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).unwrap();
    for (name, rgb) in [
        ("g-750", paint::FILL),
        ("v-800", paint::PRIMARY_FILL),
        ("g-300", paint::SILVER),
        ("g-200", paint::SILVER_LIGHT),
        ("g-500", paint::KNOB_RIM),
        ("v-300", paint::VIOLET_LINE),
        ("v-400", paint::VIOLET),
        ("v-700", paint::VIOLET_RIM),
        ("text-1", paint::TEXT),
        ("bg-deep", paint::PILL),
        ("g-400", paint::GUIDE),
    ] {
        let c = metallic_tokens::color(&set, name).unwrap_or_else(|| panic!("token {name}"));
        assert_eq!([c[0], c[1], c[2]], rgb, "{name}");
    }
}

#[test]
fn paint_draws_without_panicking() {
    let mut r = Rig::new(&config_n_buttons(6));
    r.touch(1, TouchPhase::Down, [200.0, 300.0]);
    let jump = r.button("Jump");
    r.touch(2, TouchPhase::Down, jump.center);
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    let painter = ctx.layer_painter(egui::LayerId::background());
    let style = OverlayStyle {
        show_safe_area: true,
        ..OverlayStyle::default()
    };
    paint(&painter, &r.tc, &style, &|_| None);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(out.shapes.len() > 10, "{} shapes", out.shapes.len());
}

// ----------------------------------------------------------------------------- with #254

#[test]
fn touch_controls_have_touch_glyphs_and_the_touch_device() {
    use crate::input::glyph::{GamepadStyle, GlyphArt, control_glyph, image_id, slot_glyph};
    use crate::input::overrides::control_device;
    for slot in 0..TOUCH_BUTTON_SLOTS {
        let c = Control::touch_button(slot).unwrap();
        assert!(c.is_touch());
        assert_eq!(control_device(c), DeviceKind::Touch, "{c:?}");
        let g = control_glyph(c, GamepadStyle::Letters);
        assert!(matches!(g.art, GlyphArt::Touch), "{c:?}");
        assert!(!g.text.is_empty());
        assert!(
            image_id(c, GamepadStyle::Letters).contains("/touch/"),
            "{c:?}"
        );
    }
    let empty = slot_glyph(
        &crate::input::Slot::Single(None),
        DeviceKind::Touch,
        GamepadStyle::Letters,
    );
    assert_eq!(empty.count, 0, "an empty slot is unbound, not a panic");
}

#[test]
fn the_menu_button_opens_the_player_menu() {
    use crate::input::MENU_ACTION;
    let mut r = Rig::new(&InputConfig::player_defaults());
    assert_eq!(
        r.tc.set().menu.as_ref().map(|m| m.path.as_str()),
        Some(MENU_ACTION),
        "the built-in Menu action has the menu button"
    );
    let m = r.tc.layout().menu.unwrap();
    let menu = r.rt.find_action(MENU_ACTION).unwrap();
    let before = r.rt.presses(menu);
    r.touch(1, TouchPhase::Down, m.center);
    r.touch(1, TouchPhase::Up, m.center);
    assert_eq!(r.rt.presses(menu), before + 1, "one press: the menu opens");
}

#[test]
fn touch_settings_are_saved_with_the_overrides() {
    use crate::input::PlayerOverrides;
    use crate::input::overrides::{TOUCH_OPACITY_RANGE, TouchSettings};
    let o = PlayerOverrides::from_json(
        r#"{"version":1,"touch":{"stick":"Fixed","opacity":0.1,"size":9,"largeControls":true,"later":1}}"#,
    )
    .unwrap();
    assert_eq!(o.touch.stick_mode, StickMode::Fixed);
    assert_eq!(o.touch.opacity, TOUCH_OPACITY_RANGE.0, "clamped");
    assert!(o.touch.large_controls);
    assert_eq!(o.touch.size, 1.5, "clamped");
    let text = o.to_json("2026-10-30T00:00:00Z");
    assert!(
        text.contains("\"stick\": \"Fixed\"")
            && text.contains("\"largeControls\": true")
            && text.contains("\"size\": 1.5")
            && text.contains("\"later\""),
        "{text}"
    );
    let mut p = PlayerOverrides::default();
    assert!(p.is_empty());
    p.set_touch(TouchSettings {
        stick_mode: StickMode::Floating,
        opacity: 0.6,
        ..TouchSettings::default()
    });
    assert!(!p.is_empty());
    let back = PlayerOverrides::from_json(&p.to_json("x")).unwrap();
    assert!((back.touch.opacity - 0.6).abs() < 1e-6);
    p.reset_known();
    assert!(p.is_empty());
}
