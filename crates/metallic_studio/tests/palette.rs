//! The command palette (Ctrl+K) and Insert at cursor (Shift+A), driven by keys in the real
//! editor window.
use eframe::egui::{self, Event, Key, Modifiers};
use egui_kittest::Harness;
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_studio::editor::{EditorState, Gesture};
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::output::OutputPanel;
use metallic_studio::palette::{Group, PaletteMode};

fn editor() -> (EditorUi, Vec<InstanceId>, InstanceId) {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let coins: Vec<InstanceId> = (0..3u8)
        .map(|i| {
            let c = world.create("Part", Some(ws)).unwrap();
            world.set(c, "Name", Value::String("Coin".into())).unwrap();
            world
                .set(
                    c,
                    "Position",
                    Value::Vector3(Vec3::new(f64::from(i) * 6.0, 0.5, 0.0)),
                )
                .unwrap();
            world.set(c, "Anchored", Value::Bool(true)).unwrap();
            c
        })
        .collect();
    let floor = world.create("Part", Some(ws)).unwrap();
    world
        .set(floor, "Name", Value::String("Floor".into()))
        .unwrap();
    world
        .set(floor, "Size", Value::Vector3(Vec3::new(200.0, 1.0, 200.0)))
        .unwrap();
    world
        .set(floor, "Position", Value::Vector3(Vec3::new(0.0, -0.5, 0.0)))
        .unwrap();
    world.set(floor, "Anchored", Value::Bool(true)).unwrap();
    let output = OutputPanel::new();
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Test",
        output.log.sink(),
    );
    state.can_save = false;
    (EditorUi::new(state, output), coins, floor)
}

fn harness(ui: EditorUi) -> Harness<'static, EditorUi> {
    // The panels use the theme's bundled fonts, installed on the first pass (drawn empty).
    let mut themed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, editor: &mut EditorUi| {
                if !themed {
                    metallic_studio::theme::apply(
                        ui.ctx(),
                        metallic_studio::theme::ThemeId::Graphite,
                    );
                    themed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                editor.ui(ui, None);
            },
            ui,
        );
    h.run_steps(3);
    h
}

fn type_text(h: &mut Harness<'static, EditorUi>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

#[test]
fn ctrl_k_finds_objects_and_enter_selects_every_match() {
    let (ui, coins, _) = editor();
    let mut h = harness(ui);
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(2);
    assert_eq!(h.state().palette.mode(), Some(PaletteMode::Search));
    type_text(&mut h, "coin");
    let first = &h.state().palette.rows()[0];
    assert_eq!(first.group, Group::Objects);
    assert!(first.detail.contains("3 objects"), "{}", first.detail);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(!h.state().palette.is_open());
    let mut selected = h.state().state.selection.ids().to_vec();
    selected.sort();
    let mut want = coins.clone();
    want.sort();
    assert_eq!(selected, want);
}

#[test]
fn ctrl_shift_p_lists_commands_and_runs_one() {
    let (ui, coins, _) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.select_only(coins[0]);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run_steps(2);
    assert_eq!(h.state().palette.query(), ">");
    type_text(&mut h, "duplicate");
    assert!(
        h.state()
            .palette
            .rows()
            .iter()
            .all(|r| matches!(r.group, Group::Commands | Group::Litha))
    );
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(
        h.state().state.history.len(),
        1,
        "Duplicate ran as one step"
    );
    assert_eq!(
        h.state().palette.recent().next(),
        Some("edit.duplicate"),
        "remembered for the empty query"
    );
}

#[test]
fn escape_closes_without_touching_the_selection() {
    let (ui, coins, _) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.select_only(coins[1]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(2);
    type_text(&mut h, "floor");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().palette.is_open());
    assert_eq!(h.state().state.selection.ids(), &[coins[1]]);
}

#[test]
fn shift_a_inserts_under_the_cursor() {
    let (ui, _, _) = editor();
    let mut h = harness(ui);
    // Point at the floor away from the coins.
    let rect = h.state().viewport.rect();
    let target = rect.center() + egui::vec2(rect.width() * 0.2, rect.height() * 0.25);
    h.hover_at(target);
    h.run_steps(2);
    let at = h.state().state.insert_point;
    h.key_press_modifiers(Modifiers::SHIFT, Key::A);
    h.run_steps(2);
    assert_eq!(h.state().palette.mode(), Some(PaletteMode::Insert));
    type_text(&mut h, "part");
    h.key_press(Key::Enter);
    h.run_steps(2);
    let s = &h.state().state;
    let id = *s.selection.ids().first().expect("the new part is selected");
    assert_eq!(s.world.class_name(id), Some("Part"));
    assert_eq!(s.world.get(id, "Position").unwrap(), Value::Vector3(at));
    assert_eq!(s.history.len(), 1);
}

#[test]
fn ctrl_enter_asks_litha_and_the_question_waits_for_the_panel() {
    let (ui, _, _) = editor();
    let mut h = harness(ui);
    h.state_mut().open_command_palette();
    h.run_steps(2);
    type_text(&mut h, "why does my coin fall");
    let last = h.state().palette.rows().last().unwrap().clone();
    assert_eq!(last.group, Group::Litha);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(2);
    assert!(!h.state().palette.is_open());
    assert_eq!(
        h.state_mut().take_litha_question().as_deref(),
        Some("why does my coin fall")
    );
}

#[test]
fn the_palette_does_not_open_during_a_gesture() {
    let (mut ui, _, _) = editor();
    for g in [Gesture::Viewport, Gesture::Properties, Gesture::Tool] {
        ui.state.set_gesture(g, true);
        ui.open_command_palette();
        assert!(!ui.palette.is_open(), "{g:?} gesture open");
        ui.state.set_gesture(g, false);
    }
    ui.open_command_palette();
    assert!(ui.palette.is_open());
}
