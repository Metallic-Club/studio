//! Scripted Studio checks: the editor window driven by pointer and key events (no GPU needed;
//! the viewport falls back to a flat background but picking and gizmos work the same).
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_render::glam::DVec3;
use metallic_studio::editor::{EditorState, Tool};
use metallic_studio::editor_ui::{EditorExit, EditorUi};
use metallic_studio::output::OutputPanel;

fn editor() -> (EditorUi, InstanceId, InstanceId) {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(ws)).unwrap();
    let part = world.create("Part", Some(model)).unwrap();
    world
        .set(part, "Size", Value::Vector3(Vec3::new(4.0, 4.0, 4.0)))
        .unwrap();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    let output = OutputPanel::new();
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Test",
        output.log.sink(),
    );
    state.can_save = false;
    (EditorUi::new(state, output), model, part)
}

fn harness(ui: EditorUi) -> Harness<'static, EditorUi> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            |ui, editor: &mut EditorUi| {
                editor.ui(ui, None);
            },
            ui,
        );
    h.run_steps(3);
    h
}

fn press(h: &mut Harness<'static, EditorUi>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
}

fn click(h: &mut Harness<'static, EditorUi>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(2);
    press(h, pos, true);
    press(h, pos, false);
}

#[test]
fn click_selects_model_first_then_the_part() {
    let (ui, model, part) = editor();
    let mut h = harness(ui);
    let at = h
        .state()
        .viewport
        .screen_pos(DVec3::new(0.0, 0.0, 1.9))
        .unwrap();
    // Select tool so no gizmo sits over the part.
    h.state_mut().state.tool = Tool::Select;
    click(&mut h, at);
    assert_eq!(h.state().state.selection.ids(), &[model]);
    click(&mut h, at);
    assert_eq!(h.state().state.selection.ids(), &[part]);
    // Clicking empty space clears.
    let empty = h.state().viewport.rect().left_top() + egui::vec2(10.0, 10.0);
    click(&mut h, empty);
    assert!(h.state().state.selection.is_empty());
}

#[test]
fn gizmo_drag_moves_in_whole_studs_as_one_undo_step() {
    let (ui, _model, part) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.select_only(part);
    h.state_mut().state.tool = Tool::Move;
    h.run_steps(2);
    // Grab the Y arrow halfway along, drag up.
    let ui = h.state();
    let origin = ui.viewport.screen_pos(DVec3::ZERO).unwrap();
    let up = ui.viewport.screen_pos(DVec3::new(0.0, 1.0, 0.0)).unwrap();
    let px_per_stud = (origin.y - up.y).abs();
    let arrow_len_px = 90.0;
    let grab = origin - egui::vec2(0.0, arrow_len_px * 0.6);
    h.hover_at(grab);
    h.run_steps(2);
    press(&mut h, grab, true);
    let target = grab - egui::vec2(0.0, px_per_stud * 3.3);
    h.hover_at(target);
    h.run_steps(2);
    press(&mut h, target, false);
    let s = &h.state().state;
    let Value::Vector3(p) = s.world.get(part, "Position").unwrap() else {
        panic!()
    };
    assert!((p.y - 3.0).abs() < 1e-9, "snapped to 3 studs, got {p:?}");
    assert_eq!(p.x, 0.0);
    assert_eq!(s.history.len(), 1);
    assert_eq!(
        s.history.undo_name(&s.world).as_deref(),
        Some("Move 1 part")
    );
    // Ctrl+Z through the keymap.
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(
        h.state().state.world.get(part, "Position").unwrap(),
        Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
    );
}

#[test]
fn escape_cancels_a_drag() {
    let (ui, _model, part) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.select_only(part);
    h.run_steps(2);
    let origin = h.state().viewport.screen_pos(DVec3::ZERO).unwrap();
    let grab = origin - egui::vec2(0.0, 54.0);
    h.hover_at(grab);
    h.run_steps(2);
    press(&mut h, grab, true);
    h.hover_at(grab - egui::vec2(0.0, 120.0));
    h.run_steps(2);
    assert!(h.state().viewport.is_dragging());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    press(&mut h, grab - egui::vec2(0.0, 120.0), false);
    let s = &h.state().state;
    assert_eq!(
        s.world.get(part, "Position").unwrap(),
        Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
    );
    assert!(s.history.is_empty());
}

#[test]
fn delete_key_and_tool_keys_work_over_the_viewport() {
    let (ui, model, _part) = editor();
    let mut h = harness(ui);
    h.state_mut().state.selection.select_only(model);
    let centre = h.state().viewport.rect().center();
    h.hover_at(centre);
    h.run_steps(2);
    h.key_press(egui::Key::E);
    h.run_steps(2);
    assert_eq!(h.state().state.tool, Tool::Rotate);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(!h.state().state.is_live(model));
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(h.state().state.is_live(model));
}

#[test]
fn alt_click_cycles_to_the_part_behind() {
    let (mut ui, model, front) = editor();
    let back = ui.state.world.create("Part", Some(model)).unwrap();
    ui.state
        .world
        .set(back, "Position", Value::Vector3(Vec3::new(0.0, 0.0, -10.0)))
        .unwrap();
    ui.state
        .world
        .set(back, "Size", Value::Vector3(Vec3::new(30.0, 30.0, 1.0)))
        .unwrap();
    ui.state.tool = Tool::Select;
    let mut h = harness(ui);
    let at = h
        .state()
        .viewport
        .screen_pos(DVec3::new(0.0, 0.0, 1.9))
        .unwrap();
    h.hover_at(at);
    h.run_steps(2);
    // Alt is held for the whole click, as on a real keyboard.
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::ALT,
        });
        h.run_steps(2);
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    assert_eq!(h.state().state.selection.ids(), &[back]);
    assert_ne!(back, front);
}

struct Shell {
    editor: EditorUi,
    exit: EditorExit,
}

fn dirty_shell(scene: std::path::PathBuf) -> (Harness<'static, Shell>, InstanceId) {
    let (mut ui, _model, part) = editor();
    ui.state.scene_path = scene;
    ui.state.can_save = true;
    ui.state
        .set_property(&[part], "Transparency", Value::Number(0.5));
    assert!(ui.state.is_dirty());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            |ui, shell: &mut Shell| {
                let exit = shell.editor.ui(ui, None);
                if exit != EditorExit::Stay {
                    shell.exit = exit;
                }
            },
            Shell {
                editor: ui,
                exit: EditorExit::Stay,
            },
        );
    h.run_steps(2);
    (h, part)
}

#[test]
fn closing_the_window_with_unsaved_changes_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, _) = dirty_shell(dir.path().join("main.mscene"));
    assert!(
        !h.state_mut().editor.request_quit(),
        "dirty: the close is held"
    );
    h.run_steps(2);
    assert!(h.state().editor.is_confirming());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!h.state().editor.is_confirming());
    assert_eq!(h.state().exit, EditorExit::Stay);
    // Ask again and choose Don't save: the shell is told to quit.
    assert!(!h.state_mut().editor.request_quit());
    h.run_steps(2);
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert_eq!(h.state().exit, EditorExit::Quit);
}

#[test]
fn save_from_the_prompt_closes_only_when_the_save_worked() {
    let dir = tempfile::tempdir().unwrap();
    // The folder does not exist, so Save fails.
    let (mut h, _) = dirty_shell(dir.path().join("missing").join("main.mscene"));
    assert!(!h.state_mut().editor.request_quit());
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert_eq!(
        h.state().exit,
        EditorExit::Stay,
        "a failed save keeps the game open"
    );
    assert!(h.state().editor.is_confirming());
    assert!(h.state().editor.save_error().is_some());
    assert!(h.state().editor.state.is_dirty());
    // Fix the path and save again: now it quits.
    h.state_mut().editor.state.scene_path = dir.path().join("main.mscene");
    // Move away and wait past the double-click window, as a person would.
    h.hover_at(egui::pos2(5.0, 400.0));
    h.step();
    std::thread::sleep(std::time::Duration::from_millis(400));
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert_eq!(
        h.state().exit,
        EditorExit::Quit,
        "{:?}",
        h.state().editor.save_error()
    );
    assert!(dir.path().join("main.mscene").exists());
}

#[test]
fn closing_with_unsaved_script_text_asks_first() {
    use egui::accesskit::Role;
    let (mut ui, _model, _part) = editor();
    let ws = ui.state.world.root("Workspace").unwrap();
    let script = ui.state.world.create("Script", Some(ws)).unwrap();
    ui.state.history.mark_saved();
    assert!(!ui.state.is_dirty());
    ui.state
        .request(metallic_studio::editor::UiRequest::OpenScript(script));
    let mut h = harness(ui);
    h.get_by_role(Role::MultilineTextInput).click();
    h.run_steps(2);
    h.event(Event::Text("print(1)".into()));
    h.run_steps(2);
    // The text is only in the tab so far; Alt+F4 (the window close) must still ask.
    assert!(!h.state().state.is_dirty());
    assert!(
        !h.state_mut().request_quit(),
        "unsaved script text holds the close"
    );
    h.run_steps(2);
    assert!(h.state().is_confirming());
    assert_eq!(
        h.state().state.world.get(script, "Source").unwrap(),
        Value::String("print(1)".into())
    );
}

#[test]
fn ctrl_comma_asks_for_the_settings_dialog() {
    let (ui, _, _) = editor();
    let mut h = harness(ui);
    assert!(!h.state_mut().take_settings_request());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(3);
    assert!(h.state_mut().take_settings_request());
    assert!(!h.state_mut().take_settings_request(), "asked once");
}
