//! Tests for the player's controls (issue #254): override storage, applying overrides, conflicts,
//! hold and toggle, glyphs. No window, no gamepad, no GPU.
use super::glyph::{
    ControlGlyph, Dir, FacePos, GamepadStyle, GlyphArt, MousePart, Side, auto_gamepad_style,
    control_glyph, image_id, slot_glyph,
};
use super::overrides::{
    GamepadGlyphPref, LoadStatus, OverrideStore, PlayerOverrides, Slot, Stick, StickSettings,
    game_id, rfc3339,
};
use super::rebind::{
    self, RebindError, Resolution, SlotKind, SlotRef, accepts, action_groups, assign, clear,
    find_conflict, slot_kind, unbound_actions,
};
use super::*;

const KB: Device = Device::KeyboardMouse;
const PAD: Device = Device::Gamepad(0);
const DT: f32 = 1.0 / 60.0;
const JUMP: &str = "CharacterControls/Jump";
const RUN: &str = "CharacterControls/Run";
const MOVE: &str = "CharacterControls/Move";
const LOOK: &str = "CharacterControls/Look";

fn defaults() -> InputConfig {
    InputConfig::player_defaults()
}

fn slot(path: &str, device: DeviceKind, slot: usize, part: Option<usize>) -> SlotRef {
    SlotRef {
        path: path.into(),
        device,
        slot,
        part,
    }
}

fn kb(path: &str, s: usize) -> SlotRef {
    slot(path, DeviceKind::KeyboardMouse, s, None)
}

/// Jump on E instead of Space (keyboard only).
fn jump_on_e() -> PlayerOverrides {
    let mut o = PlayerOverrides::default();
    assign(&defaults(), &mut o, &kb(JUMP, 0), Control::E, None).unwrap();
    o
}

struct Rig {
    rt: InputRuntime,
    t: f64,
}

impl Rig {
    fn new(o: PlayerOverrides) -> Self {
        let mut rt = InputRuntime::new(&defaults());
        rt.set_overrides(o);
        Rig { rt, t: 10.0 }
    }

    fn press(&mut self, d: Device, c: Control, v: f32) {
        self.rt.push(RawEvent::button(d, c, v, self.t));
        self.frame();
    }

    fn tap(&mut self, d: Device, c: Control) {
        self.press(d, c, 1.0);
        self.press(d, c, 0.0);
    }

    fn frame(&mut self) -> FrameInput {
        self.t += f64::from(DT);
        self.rt.frame(self.t, DT)
    }

    fn id(&self, path: &str) -> ActionId {
        self.rt.find_action(path).unwrap()
    }
}

// ---------------------------------------------------------------------------------------------
// Storage

fn sample() -> PlayerOverrides {
    let mut o = jump_on_e();
    o.action_mut(RUN).toggle = Some(true);
    o.set_stick(
        Stick::Right,
        StickSettings {
            sensitivity: 1.25,
            invert_x: false,
            invert_y: true,
            deadzone: Some(0.1),
        },
    );
    o.assist.replace_holds = true;
    o.assist.hold_time_scale = 1.5;
    o.gamepad_glyphs = GamepadGlyphPref::Positions;
    o
}

#[test]
fn overrides_round_trip_through_json() {
    let o = sample();
    let text = o.to_json("2026-10-30T12:00:00Z");
    let back = PlayerOverrides::from_json(&text).unwrap();
    assert_eq!(back.actions, o.actions);
    assert_eq!(back.stick(Stick::Right), o.stick(Stick::Right));
    assert_eq!(back.assist, o.assist);
    assert_eq!(back.gamepad_glyphs, o.gamepad_glyphs);
    assert_eq!(back.updated.as_deref(), Some("2026-10-30T12:00:00Z"));
    // Written again, the text is the same.
    assert_eq!(back.to_json("2026-10-30T12:00:00Z"), text);
    // Only differences are written: no gamepad bindings, no left stick, no mouse.
    assert!(!text.contains("Gamepad\""), "{text}");
    assert!(!text.contains("Thumbstick1"), "{text}");
    assert!(text.contains("\"key\": \"E\""), "{text}");
}

#[test]
fn unknown_keys_survive_a_round_trip() {
    let text = r#"{
        "version": 2,
        "overrides": {
            "CharacterControls/Jump": {
                "bindings": [
                    { "device": "Keyboard", "key": "E" },
                    { "device": "Hologram", "gesture": "Wave" }
                ],
                "toggle": false,
                "later": { "a": 1 }
            },
            "Gone/Action": { "toggle": true }
        },
        "sticks": { "Thumbstick2": { "invertY": true, "curve": "soft" }, "Gyro": { "on": true } },
        "assist": { "toggleAll": true, "autoAim": 0.5 },
        "glyphs": { "gamepadStyle": "Letters", "keyboardLayout": "azerty" },
        "touch": { "stick": "Fixed", "scale": 1.2 },
        "updated": "2026-10-30T12:00:00Z"
    }"#;
    let o = PlayerOverrides::from_json(text).unwrap();
    assert!(o.stick(Stick::Right).invert_y);
    assert!(o.assist.toggle_all);
    let out = o.to_json("2026-10-31T00:00:00Z");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["touch"]["stick"], "Fixed", "{out}");
    assert_eq!(v["overrides"][JUMP]["later"]["a"], 1);
    assert_eq!(v["overrides"][JUMP]["bindings"][1]["gesture"], "Wave");
    assert_eq!(v["overrides"]["Gone/Action"]["toggle"], true);
    assert_eq!(v["sticks"]["Thumbstick2"]["curve"], "soft");
    assert_eq!(v["sticks"]["Gyro"]["on"], true);
    assert_eq!(v["assist"]["autoAim"], 0.5);
    assert_eq!(v["glyphs"]["keyboardLayout"], "azerty");
    assert_eq!(v["version"], 1);
    // An override of an action that no longer exists is kept but changes nothing.
    let cfg = o.apply(&defaults());
    assert_eq!(cfg.contexts.len(), defaults().contexts.len());
}

#[test]
fn a_wrong_field_is_skipped_and_the_rest_kept() {
    let text = r#"{ "overrides": { "CharacterControls/Run": { "toggle": "yes" },
        "CharacterControls/Jump": { "bindings": [ { "device": "Keyboard", "key": "NotAKey" } ] } },
        "sticks": { "Thumbstick1": { "sensitivity": "fast", "invertX": true } },
        "assist": { "holdTimeScale": 99 } }"#;
    let o = PlayerOverrides::from_json(text).unwrap();
    assert_eq!(o.action(RUN).unwrap().toggle, None);
    // The unreadable binding is kept as unknown data; Jump keeps the creator's keys.
    let cfg = o.apply(&defaults());
    let (_, jump) = cfg.action(JUMP).unwrap();
    assert!(jump.bindings.iter().any(|b| b.key == Some(Control::Space)));
    assert!(o.stick(Stick::Left).invert_x);
    assert_eq!(o.stick(Stick::Left).sensitivity, 1.0);
    assert_eq!(o.assist.hold_time_scale, 3.0, "clamped to the range");
}

fn temp_store() -> (tempfile::TempDir, OverrideStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = OverrideStore::new(dir.path(), &game_id("Parkour"), "local");
    (dir, store)
}

#[test]
fn a_missing_file_gives_the_creators_controls() {
    let (_dir, store) = temp_store();
    let (o, status) = store.load();
    assert_eq!(status, LoadStatus::Missing);
    assert!(o.is_empty());
}

#[test]
fn the_store_round_trips_atomically() {
    let (dir, store) = temp_store();
    assert!(
        store
            .path()
            .ends_with(format!("input/{}/local.json", game_id("Parkour")))
    );
    let o = sample();
    store.save(&o).unwrap();
    let (back, status) = store.load();
    assert_eq!(status, LoadStatus::Loaded);
    assert_eq!(back.actions, o.actions);
    assert_eq!(back.assist, o.assist);
    assert!(back.updated.is_some());
    // No temporary file is left behind.
    let names: Vec<_> = std::fs::read_dir(store.path().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("local.json")]);
    // Nothing differs any more: the file goes away.
    store.save(&PlayerOverrides::default()).unwrap();
    assert!(!store.path().exists());
    drop(dir);
}

#[test]
fn a_corrupt_file_is_moved_aside_and_defaults_apply() {
    let (_dir, store) = temp_store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    for garbage in ["{ not json", "[1, 2, 3]", "\u{0}\u{1}binary"] {
        std::fs::write(store.path(), garbage).unwrap();
        let (o, status) = store.load();
        assert!(o.is_empty(), "{garbage}");
        let LoadStatus::Corrupt { moved_to, .. } = status else {
            panic!("{garbage}: {status:?}");
        };
        let aside = moved_to.expect("moved aside");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), garbage);
        assert!(!store.path().exists());
        // The next save writes a good file.
        store.save(&jump_on_e()).unwrap();
        assert_eq!(store.load().1, LoadStatus::Loaded);
        std::fs::remove_file(store.path()).unwrap();
    }
    // Not UTF-8 at all.
    std::fs::write(store.path(), [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(store.load().1, LoadStatus::Corrupt { .. }));
}

#[test]
fn game_ids_are_safe_folder_names() {
    let id = game_id("Parkour");
    assert!(
        id.starts_with("parkour-") && id.len() == "parkour-".len() + 8,
        "{id}"
    );
    assert_eq!(game_id("Parkour"), id, "stable");
    assert!(game_id("Tower Defense: 2!").starts_with("tower-defense-2-"));
    assert!(game_id("../..").starts_with("game-"));
    assert!(game_id("").starts_with("game-"));
    assert!(game_id(&"x".repeat(200)).len() <= 48 + 9);
    // Names that slug alike still get their own folder.
    assert_ne!(game_id("Tower Defense"), game_id("tower-defense"));
    assert_ne!(game_id("Obby"), game_id("OBBY"));
    // Never a Windows reserved device name, with or without an extension.
    const RESERVED: [&str; 6] = ["con", "nul", "prn", "aux", "com1", "lpt1"];
    for name in ["CON", "nul", "Com1", "LPT1", "aux", "PRN.txt"] {
        let id = game_id(name);
        let stem = id.split('.').next().unwrap();
        assert!(!RESERVED.contains(&stem), "{name} -> {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "{id}"
        );
    }
}

#[test]
fn a_file_without_any_menu_binding_gets_menu_back() {
    let text = r#"{ "overrides": { "Metallic.System/Menu": { "bindings": [
        { "device": "Keyboard", "key": "None" },
        { "device": "Gamepad", "key": "None" } ] } } }"#;
    let o = PlayerOverrides::from_json(text).unwrap();
    assert!(o.action(MENU_ACTION).is_none(), "{o:?}");
    let mut r = Rig::new(o);
    let menu = r.id(MENU_ACTION);
    let before = r.rt.presses(menu);
    r.tap(KB, Control::Escape);
    r.tap(PAD, Control::ButtonStart);
    assert_eq!(r.rt.presses(menu), before + 2);
    // Only the empty device comes back; a real rebind and the toggle are kept.
    let text = r#"{ "overrides": { "Metallic.System/Menu": { "bindings": [
        { "device": "Keyboard", "key": "M" },
        { "device": "Gamepad", "key": "None" } ] } } }"#;
    let o = PlayerOverrides::from_json(text).unwrap();
    let cfg = o.apply(&defaults());
    let (_, menu) = cfg.action(MENU_ACTION).unwrap();
    let keys: Vec<_> = menu.bindings.iter().filter_map(|b| b.key).collect();
    assert_eq!(keys, [Control::M, Control::ButtonStart]);
}

#[test]
fn a_file_keeps_at_most_four_slots_per_device() {
    let keys: Vec<String> = ["E", "F", "G", "H", "J", "K", "L"]
        .iter()
        .map(|k| format!(r#"{{ "device": "Keyboard", "key": "{k}" }}"#))
        .collect();
    let text = format!(
        r#"{{ "overrides": {{ "{JUMP}": {{ "bindings": [ {} ] }} }} }}"#,
        keys.join(", ")
    );
    let o = PlayerOverrides::from_json(&text).unwrap();
    let cfg = o.apply(&defaults());
    let (_, jump) = cfg.action(JUMP).unwrap();
    let kb: Vec<_> = jump
        .bindings
        .iter()
        .filter_map(|b| b.key)
        .filter(|c| !c.is_gamepad())
        .collect();
    assert_eq!(kb, [Control::E, Control::F, Control::G, Control::H]);
}

#[test]
fn an_oversized_file_is_ignored_and_set_aside() {
    let (_dir, store) = temp_store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    // Valid JSON, but padded past the cap.
    let mut text = jump_on_e().to_json("t");
    text.insert_str(1, &" ".repeat(super::overrides::MAX_FILE_BYTES as usize));
    std::fs::write(store.path(), &text).unwrap();
    let (o, status) = store.load();
    assert!(o.is_empty());
    let LoadStatus::Corrupt { moved_to, .. } = status else {
        panic!("{status:?}");
    };
    assert!(moved_to.is_some());
    // Just under the cap still loads.
    let text = jump_on_e().to_json("t");
    std::fs::write(store.path(), &text).unwrap();
    assert_eq!(store.load().1, LoadStatus::Loaded);
}

#[test]
fn timestamps_are_rfc3339_utc() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(rfc3339(1_735_689_599), "2024-12-31T23:59:59Z");
}

// ---------------------------------------------------------------------------------------------
// Applying overrides

#[test]
fn rebinding_jump_changes_play_and_keeps_the_gamepad() {
    let mut r = Rig::new(jump_on_e());
    r.press(KB, Control::Space, 1.0);
    assert!(!r.frame().jump, "Space no longer jumps");
    r.press(KB, Control::Space, 0.0);
    r.press(KB, Control::E, 1.0);
    assert!(r.frame().jump, "E jumps");
    r.press(KB, Control::E, 0.0);
    r.press(PAD, Control::ButtonA, 1.0);
    assert!(r.frame().jump, "the gamepad keeps the creator's A");
}

#[test]
fn a_rebind_survives_a_restart_and_deleting_the_file_restores_defaults() {
    let (_dir, store) = temp_store();
    store.save(&jump_on_e()).unwrap();
    // "Restart": a new runtime reads the file.
    let (o, _) = store.load();
    let mut r = Rig::new(o);
    r.press(KB, Control::E, 1.0);
    assert!(r.frame().jump);
    std::fs::remove_file(store.path()).unwrap();
    let (o, status) = store.load();
    assert_eq!(status, LoadStatus::Missing);
    let mut r = Rig::new(o);
    r.press(KB, Control::Space, 1.0);
    assert!(r.frame().jump, "Space jumps again");
}

#[test]
fn stick_settings_scale_invert_and_dead_zone() {
    let mut o = PlayerOverrides::default();
    o.set_stick(
        Stick::Right,
        StickSettings {
            sensitivity: 2.0,
            invert_x: false,
            invert_y: true,
            deadzone: Some(0.0),
        },
    );
    let mut r = Rig::new(o);
    r.rt.push(RawEvent::vector(PAD, Control::Thumbstick2, [0.0, 0.1], r.t));
    let f = r.frame();
    // Dead zone 0: 0.1 counts. Look y is up; inverted, so orbit pitch (= -look y) is positive.
    let expected = 0.1 * STICK_ORBIT_SPEED * 2.0 * DT;
    assert!((f.orbit[1] - expected).abs() < 1e-5, "{:?}", f.orbit);
    // Settings are clamped.
    let mut o = PlayerOverrides::default();
    o.set_stick(
        Stick::Left,
        StickSettings {
            sensitivity: 9.0,
            deadzone: Some(0.9),
            ..Default::default()
        },
    );
    assert_eq!(o.stick(Stick::Left).sensitivity, 2.0);
    assert_eq!(o.stick(Stick::Left).deadzone, Some(0.4));
}

#[test]
fn mouse_invert_y_flips_look() {
    let mut o = PlayerOverrides::default();
    o.set_stick(
        Stick::Mouse,
        StickSettings {
            invert_y: true,
            ..Default::default()
        },
    );
    let mut r = Rig::new(o);
    r.press(KB, Control::MouseButton2, 1.0);
    r.rt.push(RawEvent::vector(KB, Control::MouseDelta, [0.0, 10.0], r.t));
    let f = r.frame();
    // Normally mouse down looks further down (positive pitch); inverted it is negative.
    assert!(f.orbit[1] < 0.0, "{:?}", f.orbit);
}

// ---------------------------------------------------------------------------------------------
// Conflicts

#[test]
fn a_conflict_offers_swap_replace_and_cancel() {
    let cfg = defaults();
    let target = kb(JUMP, 0);
    let mut o = PlayerOverrides::default();
    let up = slot(MOVE, DeviceKind::KeyboardMouse, 0, Some(0));
    let c = find_conflict(&cfg, &o, &target, Control::W).expect("W is Move's Up");
    assert_eq!(c.other, up);
    assert!(c.can_swap && c.can_replace && !c.fixed);
    // Without a resolution nothing changes (Cancel).
    let err = assign(&cfg, &mut o, &target, Control::W, None).unwrap_err();
    assert!(matches!(err, RebindError::Conflict(_)));
    assert!(o.is_empty(), "cancel changes nothing");

    // Swap: Move's Up gets Space.
    let mut swapped = PlayerOverrides::default();
    assign(
        &cfg,
        &mut swapped,
        &target,
        Control::W,
        Some(Resolution::Swap),
    )
    .unwrap();
    let slots = rebind::display_slots(&cfg, &swapped, MOVE, DeviceKind::KeyboardMouse);
    assert_eq!(slots[0].part(Some(0)), Some(Control::Space));
    let slots = rebind::display_slots(&cfg, &swapped, JUMP, DeviceKind::KeyboardMouse);
    assert_eq!(slots[0], Slot::Single(Some(Control::W)));

    // Replace: Move's Up is unbound, the other parts stay.
    let mut replaced = PlayerOverrides::default();
    assign(
        &cfg,
        &mut replaced,
        &target,
        Control::W,
        Some(Resolution::Replace),
    )
    .unwrap();
    let slots = rebind::display_slots(&cfg, &replaced, MOVE, DeviceKind::KeyboardMouse);
    assert_eq!(
        slots[0],
        Slot::Composite([None, Some(Control::S), Some(Control::A), Some(Control::D)])
    );
    // Move still has keys, so it is not listed as unbound.
    assert!(unbound_actions(&cfg, &replaced, DeviceKind::KeyboardMouse).is_empty());

    // And in play: W jumps, Space walks forward after the swap.
    let mut r = Rig::new(swapped);
    r.press(KB, Control::W, 1.0);
    assert!(r.frame().jump);
    r.press(KB, Control::W, 0.0);
    r.press(KB, Control::Space, 1.0);
    assert_eq!(r.frame().movement, [0.0, 1.0]);
}

#[test]
fn menu_cannot_be_left_unbound() {
    let cfg = defaults();
    let mut o = PlayerOverrides::default();
    let menu_kb = kb(MENU_ACTION, 0);
    assert_eq!(
        clear(&cfg, &mut o, &menu_kb),
        Err(RebindError::Required("Menu".into()))
    );
    // Taking Escape for Jump: Replace would leave Menu without a key, so only Swap is offered.
    let c = find_conflict(&cfg, &o, &kb(JUMP, 0), Control::Escape).unwrap();
    assert_eq!(c.other, menu_kb);
    assert!(c.can_swap && !c.can_replace);
    assert_eq!(
        assign(
            &cfg,
            &mut o,
            &kb(JUMP, 0),
            Control::Escape,
            Some(Resolution::Replace)
        ),
        Err(RebindError::NotAllowed)
    );
    assign(
        &cfg,
        &mut o,
        &kb(JUMP, 0),
        Control::Escape,
        Some(Resolution::Swap),
    )
    .unwrap();
    let menu = rebind::display_slots(&cfg, &o, MENU_ACTION, DeviceKind::KeyboardMouse);
    assert_eq!(menu[0], Slot::Single(Some(Control::Space)));
    // Menu can move to another key, and then the old one is free.
    let mut o = PlayerOverrides::default();
    assign(&cfg, &mut o, &menu_kb, Control::M, None).unwrap();
    assert!(find_conflict(&cfg, &o, &kb(JUMP, 0), Control::Escape).is_none());
    let mut r = Rig::new(o);
    let menu = r.id(MENU_ACTION);
    let before = r.rt.presses(menu);
    r.tap(KB, Control::M);
    assert_eq!(r.rt.presses(menu), before + 1);
    // Menu keeps its gamepad Start too.
    let pad = slot(MENU_ACTION, DeviceKind::Gamepad, 0, None);
    assert!(clear(&cfg, &mut PlayerOverrides::default(), &pad).is_err());
}

#[test]
fn the_system_group_conflicts_with_every_group_and_others_stay_apart() {
    let mut cfg = defaults();
    let mut vehicle = ContextDef::new("Vehicle");
    vehicle.rebind_group = "Driving".into();
    vehicle.actions =
        vec![ActionDef::new("Horn", ActionType::Bool).with_binding(BindingDef::key(Control::H))];
    let mut menu_ui = ContextDef::new("ShopMenu");
    menu_ui.rebind_group = "Menu".into();
    menu_ui.actions = vec![
        ActionDef::new("Buy", ActionType::Bool).with_binding(BindingDef::key(Control::Return)),
    ];
    cfg.contexts.push(vehicle);
    cfg.contexts.push(menu_ui);
    let o = PlayerOverrides::default();
    // Driving vs Menu's Escape: the system context sinks it, so it conflicts.
    let horn = kb("Vehicle/Horn", 0);
    let c = find_conflict(&cfg, &o, &horn, Control::Escape).unwrap();
    assert_eq!(c.other.path, MENU_ACTION);
    // Driving vs Game: no conflict on W.
    assert!(find_conflict(&cfg, &o, &horn, Control::W).is_none());
    // Game vs the shop's Return: different groups, no conflict.
    assert!(find_conflict(&cfg, &o, &kb(JUMP, 0), Control::Return).is_none());
    // Groups list System first.
    let groups = action_groups(&cfg);
    let names: Vec<_> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["System", "Game", "Driving", "Menu"]);
}

#[test]
fn composite_parts_swap_within_the_action() {
    let cfg = defaults();
    let mut o = PlayerOverrides::default();
    let up = slot(MOVE, DeviceKind::KeyboardMouse, 0, Some(0));
    let c = find_conflict(&cfg, &o, &up, Control::A).unwrap();
    assert_eq!(c.other, slot(MOVE, DeviceKind::KeyboardMouse, 0, Some(2)));
    assign(&cfg, &mut o, &up, Control::A, Some(Resolution::Swap)).unwrap();
    let s = rebind::display_slots(&cfg, &o, MOVE, DeviceKind::KeyboardMouse);
    assert_eq!(
        s[0],
        Slot::Composite([
            Some(Control::A),
            Some(Control::S),
            Some(Control::W),
            Some(Control::D)
        ])
    );
    // Swapping back gives the creator's layout, and the override disappears.
    assign(&cfg, &mut o, &up, Control::W, Some(Resolution::Swap)).unwrap();
    assert!(o.is_empty(), "{o:?}");
}

#[test]
fn slots_take_only_controls_of_their_device_and_shape() {
    let cfg = defaults();
    let o = PlayerOverrides::default();
    let pad_move = slot(MOVE, DeviceKind::Gamepad, 0, None);
    assert_eq!(slot_kind(&cfg, &o, &pad_move), Some(SlotKind::Stick));
    assert!(accepts(&cfg, &o, &pad_move, Control::Thumbstick2));
    assert!(!accepts(&cfg, &o, &pad_move, Control::ButtonA));
    let kb_jump = kb(JUMP, 0);
    assert!(accepts(&cfg, &o, &kb_jump, Control::MouseButton1));
    assert!(
        !accepts(&cfg, &o, &kb_jump, Control::ButtonA),
        "a pad button on the keyboard tab"
    );
    assert!(!accepts(&cfg, &o, &kb_jump, Control::MouseWheel));
    assert!(accepts(
        &cfg,
        &o,
        &kb("CharacterControls/Zoom", 1),
        Control::MouseWheel
    ));
    // Mouse look is fixed: its settings are sliders, not a rebind.
    assert_eq!(slot_kind(&cfg, &o, &kb(LOOK, 0)), Some(SlotKind::Fixed));
    // The modifier of mouse look is in use: only Cancel.
    let c = find_conflict(&cfg, &o, &kb_jump, Control::MouseButton2).unwrap();
    assert!(c.fixed && !c.can_swap && !c.can_replace);
}

#[test]
fn a_stick_swap_moves_both_actions() {
    let cfg = defaults();
    let mut o = PlayerOverrides::default();
    let pad_move = slot(MOVE, DeviceKind::Gamepad, 0, None);
    assign(
        &cfg,
        &mut o,
        &pad_move,
        Control::Thumbstick2,
        Some(Resolution::Swap),
    )
    .unwrap();
    let look = rebind::display_slots(&cfg, &o, LOOK, DeviceKind::Gamepad);
    assert_eq!(look[0], Slot::Single(Some(Control::Thumbstick1)));
    let mut r = Rig::new(o);
    r.rt.push(RawEvent::vector(PAD, Control::Thumbstick2, [0.0, 1.0], r.t));
    let f = r.frame();
    assert_eq!(f.movement, [0.0, 1.0], "the right stick walks");
    // The left stick turns the camera with Look's own scale (per second, unclamped).
    r.rt.push(RawEvent::vector(PAD, Control::Thumbstick1, [1.0, 0.0], r.t));
    let f = r.frame();
    assert!(
        (f.orbit[0] - STICK_ORBIT_SPEED * DT).abs() < 1e-5,
        "{:?}",
        f.orbit
    );
}

#[test]
fn an_empty_alternate_slot_takes_a_new_key() {
    let cfg = defaults();
    let mut o = PlayerOverrides::default();
    assign(&cfg, &mut o, &kb(JUMP, 1), Control::F, None).unwrap();
    let mut r = Rig::new(o);
    r.press(KB, Control::F, 1.0);
    assert!(r.frame().jump);
    r.press(KB, Control::F, 0.0);
    r.press(KB, Control::Space, 1.0);
    assert!(r.frame().jump, "the primary key still works");
}

#[test]
fn clearing_every_key_lists_the_action_as_unbound() {
    let cfg = defaults();
    let mut o = PlayerOverrides::default();
    clear(&cfg, &mut o, &kb(JUMP, 0)).unwrap();
    assert_eq!(unbound_actions(&cfg, &o, DeviceKind::KeyboardMouse), [JUMP]);
    assert!(unbound_actions(&cfg, &o, DeviceKind::Gamepad).is_empty());
    let text = o.to_json("t");
    assert!(
        text.contains("\"None\""),
        "the empty slot is stored: {text}"
    );
    let back = PlayerOverrides::from_json(&text).unwrap();
    assert_eq!(
        unbound_actions(&cfg, &back, DeviceKind::KeyboardMouse),
        [JUMP]
    );
    rebind::reset_action(&mut o, JUMP, DeviceKind::KeyboardMouse);
    assert!(o.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Hold and toggle

fn run_toggle() -> PlayerOverrides {
    let mut o = PlayerOverrides::default();
    o.action_mut(RUN).toggle = Some(true);
    o
}

#[test]
fn a_toggle_flips_on_press_and_ignores_release() {
    let mut r = Rig::new(run_toggle());
    let run = r.id(RUN);
    assert!(r.rt.is_toggle(run));
    r.tap(KB, Control::LeftShift);
    assert!(r.frame().run, "on after one tap");
    r.frame();
    assert!(r.frame().run, "stays on");
    r.press(KB, Control::LeftShift, 1.0);
    assert!(!r.frame().run, "the second press turns it off");
    r.press(KB, Control::LeftShift, 0.0);
    assert!(!r.frame().run);
    // A tap shorter than a frame still flips it, and letting go sends no Released.
    r.rt.drain_events().for_each(drop);
    r.rt.push(RawEvent::button(KB, Control::LeftShift, 1.0, r.t));
    r.rt.push(RawEvent::button(KB, Control::LeftShift, 0.0, r.t + 0.001));
    assert!(r.frame().run);
    let events: Vec<_> = r.rt.drain_events().filter(|e| e.action == run).collect();
    assert!(events.iter().all(|e| e.kind != ActionEventKind::Released));
}

#[test]
fn toggle_every_hold_action_skips_jump_and_menu() {
    let mut o = PlayerOverrides::default();
    o.assist.toggle_all = true;
    let r = Rig::new(o.clone());
    assert!(r.rt.is_toggle(r.id(RUN)));
    assert!(!r.rt.is_toggle(r.id(JUMP)));
    assert!(!r.rt.is_toggle(r.id(MENU_ACTION)));
    // The row's own Hold wins over the global switch.
    o.action_mut(RUN).toggle = Some(false);
    let r = Rig::new(o);
    assert!(!r.rt.is_toggle(r.id(RUN)));
}

#[test]
fn toggles_clear_on_death_and_when_switched_off() {
    let mut r = Rig::new(run_toggle());
    r.tap(KB, Control::LeftShift);
    assert!(r.frame().run);
    let t = r.t;
    r.rt.clear_toggles(t);
    assert!(!r.frame().run, "death clears the toggle");
    r.tap(KB, Control::LeftShift);
    assert!(r.frame().run);
    // The player switches back to Hold while it is latched: it lets go.
    r.rt.set_overrides(PlayerOverrides::default());
    assert!(!r.frame().run);
}

#[test]
fn replace_holds_with_presses_and_scale_hold_time() {
    let hold = ActionDef::new("Charge", ActionType::Bool)
        .with_trigger(Trigger::Hold)
        .with_binding(BindingDef::key(Control::Q));
    let mut ctx = ContextDef::new("T");
    ctx.actions = vec![hold];
    let cfg = InputConfig {
        contexts: vec![ctx],
        warnings: Vec::new(),
    };
    let triggered = |o: PlayerOverrides, hold_for: f64| {
        let mut rt = InputRuntime::new(&cfg);
        rt.set_overrides(o);
        rt.push(RawEvent::button(KB, Control::Q, 1.0, 1.0));
        rt.evaluate(1.0, DT);
        rt.evaluate(1.0 + hold_for, DT);
        rt.drain_events()
            .filter(|e| e.kind == ActionEventKind::Triggered)
            .count()
    };
    assert_eq!(triggered(PlayerOverrides::default(), 0.0), 0);
    assert_eq!(triggered(PlayerOverrides::default(), 0.5), 1);
    let mut presses = PlayerOverrides::default();
    presses.assist.replace_holds = true;
    assert_eq!(triggered(presses, 0.0), 1, "fires on the press");
    let mut slow = PlayerOverrides::default();
    slow.assist.hold_time_scale = 2.0;
    assert_eq!(
        triggered(slow.clone(), 0.5),
        0,
        "500 ms is no longer enough"
    );
    assert_eq!(triggered(slow, 1.0), 1);
}

// ---------------------------------------------------------------------------------------------
// Glyphs

#[test]
fn gamepad_glyphs_map_by_position() {
    let l = GamepadStyle::Letters;
    let p = GamepadStyle::Positions;
    let face = |c, s| control_glyph(c, s).art;
    assert_eq!(
        face(Control::ButtonA, l),
        GlyphArt::Face {
            pos: FacePos::South,
            letter: Some("A")
        }
    );
    assert_eq!(
        face(Control::ButtonB, l),
        GlyphArt::Face {
            pos: FacePos::East,
            letter: Some("B")
        }
    );
    assert_eq!(
        face(Control::ButtonX, p),
        GlyphArt::Face {
            pos: FacePos::West,
            letter: None
        }
    );
    assert_eq!(control_glyph(Control::ButtonY, p).text, "Top Button");
    assert_eq!(face(Control::ButtonL1, l), GlyphArt::Bumper(Side::Left));
    assert_eq!(face(Control::ButtonR2, l), GlyphArt::Trigger(Side::Right));
    assert_eq!(
        face(Control::ButtonL3, p),
        GlyphArt::Stick {
            side: Side::Left,
            press: true
        }
    );
    assert_eq!(face(Control::DPadLeft, l), GlyphArt::DPad(Some(Dir::Left)));
    assert_eq!(face(Control::ButtonStart, l), GlyphArt::Center(Side::Right));
    // Only the four face buttons differ between the styles.
    for &c in Control::ALL {
        let differs = control_glyph(c, l) != control_glyph(c, p);
        let is_face = matches!(
            c,
            Control::ButtonA | Control::ButtonB | Control::ButtonX | Control::ButtonY
        );
        assert_eq!(differs, is_face, "{c:?}");
    }
}

#[test]
fn keyboard_and_mouse_glyphs() {
    let g = |c| control_glyph(c, GamepadStyle::Letters);
    assert_eq!(
        g(Control::Space).art,
        GlyphArt::Keycap {
            label: "Space",
            wide: true
        }
    );
    assert_eq!(
        g(Control::W).art,
        GlyphArt::Keycap {
            label: "W",
            wide: false
        }
    );
    assert_eq!(g(Control::Up).art, GlyphArt::Arrow(Dir::Up));
    assert_eq!(g(Control::LeftShift).text, "Left Shift");
    assert_eq!(
        g(Control::MouseButton2).art,
        GlyphArt::Mouse(MousePart::Right)
    );
    assert_eq!(
        g(Control::MouseWheel).art,
        GlyphArt::Mouse(MousePart::Wheel)
    );
}

#[test]
fn glyph_words_name_no_brand() {
    const BRANDS: [&str; 7] = [
        "xbox",
        "playstation",
        "nintendo",
        "steam",
        "sony",
        "microsoft",
        "switch",
    ];
    for style in [GamepadStyle::Letters, GamepadStyle::Positions] {
        for &c in Control::ALL {
            let ControlGlyph { art, text } = control_glyph(c, style);
            assert!(!text.is_empty(), "{c:?}");
            let mut words = text.to_lowercase();
            if let GlyphArt::Keycap { label, .. } = art {
                words.push(' ');
                words.push_str(&label.to_lowercase());
            }
            words.push(' ');
            words.push_str(&image_id(c, style));
            for b in BRANDS {
                assert!(!words.contains(b), "{c:?}: {words}");
            }
        }
    }
}

#[test]
fn slot_glyphs_read_like_the_keyboard() {
    let wasd = Slot::Composite([
        Some(Control::W),
        Some(Control::S),
        Some(Control::A),
        Some(Control::D),
    ]);
    let g = slot_glyph(&wasd, DeviceKind::KeyboardMouse, GamepadStyle::Letters);
    assert_eq!(g.text, "W A S D");
    assert_eq!(g.count, 4);
    assert_eq!(g.image, "metallic://glyph/composite/w+s+a+d");
    let a = slot_glyph(
        &Slot::Single(Some(Control::ButtonA)),
        DeviceKind::Gamepad,
        GamepadStyle::Letters,
    );
    assert_eq!(a.image, "metallic://glyph/gamepad/letters/button_a");
    assert_eq!(a.text, "A");
    let none = slot_glyph(
        &Slot::Single(None),
        DeviceKind::Gamepad,
        GamepadStyle::Letters,
    );
    assert!(none.is_unbound());
    assert_eq!(none.text, "Unbound");
    assert_eq!(
        image_id(Control::LeftShift, GamepadStyle::Letters),
        "metallic://glyph/keyboard/left_shift"
    );
}

#[test]
fn the_auto_style_follows_the_pad_vendor() {
    assert_eq!(auto_gamepad_style(Some(0x054C)), GamepadStyle::Positions);
    assert_eq!(auto_gamepad_style(Some(0x057E)), GamepadStyle::Positions);
    assert_eq!(auto_gamepad_style(Some(0x045E)), GamepadStyle::Letters);
    assert_eq!(auto_gamepad_style(None), GamepadStyle::Letters);
}

#[test]
fn runtime_glyphs_follow_rebinds_style_and_device() {
    let mut r = Rig::new(PlayerOverrides::default());
    let jump = r.id(JUMP);
    assert_eq!(r.rt.glyph(jump, None).unwrap().text, "Space");
    assert_eq!(
        r.rt.glyph(jump, Some(DeviceKind::Gamepad)).unwrap().text,
        "A"
    );
    let first: *const Glyph = r.rt.glyph(jump, None).unwrap();
    let again: *const Glyph = r.rt.glyph(jump, None).unwrap();
    assert_eq!(first, again, "the second lookup reads the cache");
    let changes = r.rt.glyph_changes();
    r.rt.set_overrides(jump_on_e());
    assert!(r.rt.glyph_changes() > changes);
    let jump = r.id(JUMP);
    assert_eq!(r.rt.glyph(jump, None).unwrap().text, "E");
    let changes = r.rt.glyph_changes();
    r.rt.set_gamepad_style(GamepadStyle::Positions);
    assert!(r.rt.glyph_changes() > changes);
    assert_eq!(
        r.rt.glyph(jump, Some(DeviceKind::Gamepad)).unwrap().text,
        "Bottom Button"
    );
    // The last-used device picks the glyph.
    let changes = r.rt.glyph_changes();
    r.press(PAD, Control::ButtonB, 1.0);
    assert!(r.rt.glyph_changes() > changes);
    assert_eq!(r.rt.glyph(jump, None).unwrap().text, "Bottom Button");
    // Toggle badge.
    r.rt.set_overrides(run_toggle());
    let run = r.id(RUN);
    assert!(r.rt.glyph(run, None).unwrap().toggle);
    // Composites and unbound slots.
    let mv = r.id(MOVE);
    let kb_glyphs = r.rt.glyphs(mv, DeviceKind::KeyboardMouse);
    assert_eq!(kb_glyphs.len(), 2);
    assert_eq!(kb_glyphs[0].text, "W A S D");
    let mut o = PlayerOverrides::default();
    clear(&defaults(), &mut o, &kb(JUMP, 0)).unwrap();
    r.rt.set_overrides(o);
    let jump = r.id(JUMP);
    assert!(
        r.rt.glyph(jump, Some(DeviceKind::KeyboardMouse))
            .unwrap()
            .is_unbound()
    );
}

#[test]
fn the_player_defaults_hold_menu_on_escape_and_start() {
    let r = Rig::new(PlayerOverrides::default());
    let menu = r.id(MENU_ACTION);
    let keys: Vec<_> = r.rt.bindings(menu).filter_map(|b| b.key).collect();
    // Plus the touch overlay's menu button (#252), which phones use.
    assert_eq!(
        keys,
        [Control::Escape, Control::ButtonStart, Control::TouchButton7]
    );
    // From the scene too, and the rebind group defaults to Game when the scene names none.
    let mut w = metallic_core::World::new_with_services();
    let rs = w.root("ReplicatedStorage").unwrap();
    let c = w.create("InputContext", Some(rs)).unwrap();
    w.set(c, "Name", metallic_core::Value::String("Gameplay".into()))
        .unwrap();
    let cfg = InputConfig::from_world(&w);
    assert!(cfg.action(MENU_ACTION).is_some());
    assert_eq!(cfg.contexts[0].rebind_group, DEFAULT_GROUP);
}

#[test]
fn menu_presses_are_counted_even_within_one_frame() {
    let mut r = Rig::new(PlayerOverrides::default());
    let menu = r.id(MENU_ACTION);
    let before = r.rt.presses(menu);
    r.rt.push(RawEvent::button(KB, Control::Escape, 1.0, r.t));
    r.rt.push(RawEvent::button(KB, Control::Escape, 0.0, r.t + 0.001));
    r.frame();
    assert_eq!(r.rt.presses(menu), before + 1);
    // Escape is sunk by the system context: a game action on Escape does not see it.
    let mut cfg = defaults();
    cfg.contexts[0].actions.push(
        ActionDef::new("Pause", ActionType::Bool).with_binding(BindingDef::key(Control::Escape)),
    );
    let mut rt = InputRuntime::new(&cfg);
    let pause = rt.find_action("CharacterControls/Pause").unwrap();
    rt.push(RawEvent::button(KB, Control::Escape, 1.0, 1.0));
    rt.evaluate(1.0, DT);
    assert!(!rt.bool(pause));
}
