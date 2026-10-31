//! Tests of the Controls page state: keyboard and gamepad navigation, listening, conflicts,
//! resets and undo. No window and no GPU.
use super::*;
use metallic_runtime::input::MENU_ACTION;

const JUMP: &str = "CharacterControls/Jump";
const RUN: &str = "CharacterControls/Run";

fn page() -> ControlsPage {
    let mut p = ControlsPage::new(
        InputConfig::player_defaults(),
        PlayerOverrides::default(),
        GamepadStyle::Letters,
    );
    p.tick(1.0);
    p
}

/// Moves the focus with the arrow keys until `want` is focused.
fn focus(p: &mut ControlsPage, want: &Item) {
    p.nav(Nav::Up);
    for _ in 0..60 {
        p.nav(Nav::Up);
    }
    // Left and Right change sliders and the glyph style: those rows have one cell.
    let moves = |p: &ControlsPage| {
        !matches!(
            p.focused_item(),
            Some(Item::Slider(_)) | Some(Item::GlyphStyle) | Some(Item::StickMode)
        )
    };
    for _ in 0..80 {
        for _ in 0..8 {
            if moves(p) {
                p.nav(Nav::Left);
            }
        }
        for _ in 0..8 {
            if p.focused_item() == Some(want) {
                return;
            }
            if moves(p) {
                p.nav(Nav::Right);
            }
        }
        if p.focused_item() == Some(want) {
            return;
        }
        p.nav(Nav::Down);
    }
    panic!("cannot reach {want:?}; focus {:?}", p.focused_item());
}

fn kb_slot(path: &str, slot: usize) -> Item {
    Item::Slot(SlotRef {
        path: path.into(),
        device: DeviceKind::KeyboardMouse,
        slot,
        part: None,
    })
}

fn keys(p: &ControlsPage, path: &str) -> Vec<Slot> {
    rebind::display_slots(p.base(), p.overrides(), path, DeviceKind::KeyboardMouse)
}

#[test]
fn rows_list_groups_with_system_first() {
    let p = page();
    let groups: Vec<_> = p
        .rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Group(g) => Some(g.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(groups, ["System", "Game"]);
    assert!(p.focused_item().is_some(), "something has focus at once");
}

#[test]
fn rebinding_with_the_keyboard_only() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    p.nav(Nav::Accept);
    assert!(p.is_listening());
    let out = p.listen(Control::F, true);
    assert!(out.changed);
    assert!(!p.is_listening());
    assert_eq!(keys(&p, JUMP)[0], Slot::Single(Some(Control::F)));
    assert_eq!(p.focused_item(), Some(&kb_slot(JUMP, 0)), "focus stays");
}

#[test]
fn listening_cancels_on_escape_b_and_timeout() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    p.nav(Nav::Accept);
    assert!(!p.listen(Control::Escape, true).changed);
    assert!(!p.is_listening());
    p.nav(Nav::Accept);
    p.listen(Control::ButtonB, true);
    assert!(!p.is_listening(), "B cancels on the keyboard tab");
    p.nav(Nav::Accept);
    p.tick(1.0 + LISTEN_TIMEOUT);
    assert!(!p.is_listening());
    assert!(p.notice().is_some());
    assert!(p.overrides().is_empty());
}

#[test]
fn the_wrong_device_is_refused_with_a_notice() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    p.nav(Nav::Accept);
    assert!(!p.listen(Control::ButtonX, true).changed);
    assert!(p.is_listening(), "still waiting");
    assert_eq!(p.notice(), Some("This tab takes keys and mouse buttons"));
}

#[test]
fn a_modifier_binds_on_release() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    p.nav(Nav::Accept);
    assert!(!p.listen(Control::LeftAlt, true).changed);
    assert!(p.is_listening());
    assert!(p.listen(Control::LeftAlt, false).changed);
    assert_eq!(keys(&p, JUMP)[0], Slot::Single(Some(Control::LeftAlt)));
}

#[test]
fn a_conflict_opens_the_dialog_and_cancel_changes_nothing() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    let pick_shift = |p: &mut ControlsPage| {
        p.nav(Nav::Accept);
        p.listen(Control::LeftShift, true);
        p.listen(Control::LeftShift, false);
    };
    pick_shift(&mut p);
    assert!(matches!(p.mode(), Mode::Conflict { .. }), "{:?}", p.mode());
    assert_eq!(
        p.conflict_choices(),
        [Choice::Swap, Choice::Replace, Choice::Cancel]
    );
    // Right twice: Cancel.
    p.nav(Nav::Right);
    p.nav(Nav::Right);
    assert!(!p.nav(Nav::Accept).changed);
    assert!(p.overrides().is_empty());
    // Again, and Swap (the first button): Run gets Space.
    pick_shift(&mut p);
    assert!(p.nav(Nav::Accept).changed);
    assert_eq!(keys(&p, RUN)[0], Slot::Single(Some(Control::Space)));
    assert_eq!(keys(&p, JUMP)[0], Slot::Single(Some(Control::LeftShift)));
}

#[test]
fn menu_on_the_keyboard_tab_takes_escape_and_cannot_be_cleared() {
    let mut p = page();
    let menu = kb_slot(MENU_ACTION, 0);
    focus(&mut p, &menu);
    assert!(!p.nav(Nav::Clear).changed);
    assert!(p.notice().is_some(), "says why");
    p.nav(Nav::Accept);
    assert!(p.listen(Control::M, true).changed);
    p.nav(Nav::Accept);
    // Escape binds Menu back instead of cancelling.
    assert!(p.listen(Control::Escape, true).changed);
    assert!(p.overrides().is_empty());
}

#[test]
fn reset_all_confirms_and_undo_restores() {
    let mut p = page();
    focus(&mut p, &kb_slot(JUMP, 0));
    p.nav(Nav::Accept);
    p.listen(Control::F, true);
    let edited = p.overrides().clone();
    focus(&mut p, &Item::ResetAll);
    p.nav(Nav::Accept);
    assert!(matches!(p.mode(), Mode::Confirm { yes: false, .. }));
    // Cancel is focused first: Accept changes nothing.
    assert!(!p.nav(Nav::Accept).changed);
    assert_eq!(p.overrides(), &edited);
    p.nav(Nav::Accept);
    p.nav(Nav::Left);
    assert!(p.nav(Nav::Accept).changed);
    assert!(p.overrides().is_empty());
    assert!(p.toast().is_some());
    // R (or Y) while the toast shows: undo.
    assert!(p.nav(Nav::Reset).changed);
    assert_eq!(p.overrides(), &edited);
    // The toast lasts 10 s.
    focus(&mut p, &Item::ResetAll);
    p.nav(Nav::Accept);
    p.nav(Nav::Left);
    p.nav(Nav::Accept);
    assert!(p.toast().is_some());
    p.tick(1.0 + UNDO_SECONDS + 0.1);
    assert!(p.toast().is_none());
}

#[test]
fn tabs_cycle_and_the_hold_switch_toggles() {
    let mut p = page();
    p.nav(Nav::NextTab);
    assert_eq!(p.tab(), Tab::Gamepad);
    p.nav(Nav::NextTab);
    assert_eq!(p.tab(), Tab::Assist);
    p.nav(Nav::NextTab);
    assert_eq!(p.tab(), Tab::Touch);
    p.nav(Nav::NextTab);
    assert_eq!(p.tab(), Tab::Keyboard);
    p.nav(Nav::PrevTab);
    assert_eq!(p.tab(), Tab::Touch);
    p.set_tab(Tab::Keyboard);
    let hold = Item::Hold(RUN.into());
    focus(&mut p, &hold);
    assert!(p.nav(Nav::Accept).changed);
    assert!(p.toggles(RUN));
    assert!(p.nav(Nav::Accept).changed);
    assert!(!p.toggles(RUN));
    assert!(p.overrides().is_empty(), "the default leaves no override");
}

#[test]
fn sliders_step_and_clamp() {
    let mut p = page();
    p.set_tab(Tab::Gamepad);
    let id = SliderId::Sensitivity(Stick::Right);
    focus(&mut p, &Item::Slider(id));
    assert!(p.nav(Nav::Right).changed);
    let (v, _, _) = p.slider(id);
    assert!((v - 1.05).abs() < 1e-4, "{v}");
    for _ in 0..100 {
        p.nav(Nav::Right);
    }
    let (v, (_, hi), _) = p.slider(id);
    assert_eq!(v, hi);
    assert!(!p.nav(Nav::Right).changed, "at the top");
}

#[test]
fn overrides_for_unknown_actions_never_break_the_rows() {
    // A file from another game version: toggles for actions this game does not have.
    let mut o = PlayerOverrides::default();
    o.action_mut("Gone/Action").toggle = Some(true);
    o.assist.toggle_all = true;
    let mut p = ControlsPage::new(InputConfig::player_defaults(), o, GamepadStyle::Letters);
    for tab in [Tab::Keyboard, Tab::Gamepad, Tab::Assist] {
        p.set_tab(tab);
        assert!(!p.rows().is_empty());
    }
    assert!(p.toggles(RUN), "toggle every hold applies to Run");
    assert!(!p.toggles("Gone/Action") || p.base().action("Gone/Action").is_none());
}

/// #333: the Touch tab, by keyboard or gamepad only: stick mode, opacity and button size.
#[test]
fn the_touch_tab_sets_stick_opacity_and_size() {
    use metallic_runtime::input::touch::StickMode;
    let mut p = page();
    for _ in 0..3 {
        p.nav(Nav::NextTab);
    }
    assert_eq!(p.tab(), Tab::Touch);
    let settings: Vec<_> = p
        .rows()
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::Setting(name, _) => Some(name),
            _ => None,
        })
        .collect();
    assert_eq!(
        settings,
        ["Stick", "Opacity", "Button size", "Large controls"]
    );
    // Stick: Right picks Fixed, Accept flips back, Left keeps Floating.
    focus(&mut p, &Item::StickMode);
    assert!(p.nav(Nav::Right).changed);
    assert_eq!(p.touch().stick_mode, StickMode::Fixed);
    assert!(!p.nav(Nav::Right).changed, "already Fixed");
    assert!(p.nav(Nav::Accept).changed);
    assert_eq!(p.touch().stick_mode, StickMode::Floating);
    assert!(p.nav(Nav::Right).changed);
    // Opacity: 5% steps, 25% to 100%.
    focus(&mut p, &Item::Slider(SliderId::TouchOpacity));
    assert!(!p.nav(Nav::Right).changed, "starts at 100%");
    assert!(p.nav(Nav::Left).changed);
    let (v, _, text) = p.slider(SliderId::TouchOpacity);
    assert!((v - 0.95).abs() < 1e-4 && text == "95%", "{v} {text}");
    for _ in 0..40 {
        p.nav(Nav::Left);
    }
    assert_eq!(p.touch().opacity, 0.25, "clamped");
    // Button size: 10% steps, 80% to 150%.
    focus(&mut p, &Item::Slider(SliderId::TouchSize));
    assert!(p.nav(Nav::Right).changed);
    assert!((p.touch().size - 1.1).abs() < 1e-4);
    for _ in 0..20 {
        p.nav(Nav::Right);
    }
    assert_eq!(p.touch().size, 1.5, "clamped");
    // Large controls.
    focus(&mut p, &Item::Switch(SwitchId::LargeControls));
    assert!(p.nav(Nav::Accept).changed);
    assert!(p.touch().large_controls);
    // Saved with the other overrides.
    let back = PlayerOverrides::from_json(&p.overrides().to_json("x")).unwrap();
    assert_eq!(back.touch, p.touch());
    // Reset this tab puts the touch settings back and nothing else.
    focus(&mut p, &Item::ResetTab);
    p.nav(Nav::Accept);
    p.nav(Nav::Left);
    assert!(p.nav(Nav::Accept).changed);
    assert!(p.overrides().is_empty());
    assert_eq!(p.toast().map(|t| t.text.as_str()), Some("Touch reset"));
}
