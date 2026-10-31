//! Play and Stop through the editor (#11): the registered Play mode runs the real Game on a copy
//! of the edit world; Stop drops it and the edit world, its history and the selection are as
//! they were.
use eframe::egui;
use egui_kittest::Harness;
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_runtime::TICK;
use metallic_studio::commands::Command;
use metallic_studio::editor::EditorState;
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::output::{Level, OutputLog, OutputPanel, Tag};
use metallic_studio::play::{PLAY_TAG, PlayStatus};
use std::sync::Arc;

/// A Probe part (anchored, at the origin) and a server Script with `source`.
fn world(source: &str) -> (World, InstanceId) {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let part = world.create("Part", Some(ws)).unwrap();
    world
        .set(part, "Name", Value::String("Probe".into()))
        .unwrap();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    let scripts = world.root("ServerScriptService").unwrap();
    let script = world.create("Script", Some(scripts)).unwrap();
    world
        .set(script, "Name", Value::String("Probe".into()))
        .unwrap();
    world
        .set(script, "Source", Value::String(source.into()))
        .unwrap();
    (world, part)
}

fn editor(source: &str, log: &OutputLog) -> (EditorState, InstanceId) {
    let (world, part) = world(source);
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Test",
        log.sink(),
    );
    state.can_save = false;
    (state, part)
}

/// Advances the running session by `n` frames of one tick each.
fn frames(state: &mut EditorState, n: usize) {
    for _ in 0..n {
        state.play.update(TICK);
    }
}

/// How many times `text` was printed (repeats collapse into one row with a count).
fn count(log: &mut OutputLog, text: &str) -> u32 {
    log.drain();
    log.lines()
        .filter(|l| l.text == text)
        .map(|l| l.repeats)
        .sum()
}

fn instances(world: &World) -> usize {
    let mut n = 0;
    let mut stack: Vec<InstanceId> = world.roots().to_vec();
    while let Some(id) = stack.pop() {
        n += 1;
        stack.extend_from_slice(world.children(id));
    }
    n
}

/// Reads the play world through the session's read-only view.
fn in_play<R>(state: &EditorState, read: impl FnOnce(&World) -> R) -> R {
    let mut read = Some(read);
    let mut out = None;
    state
        .play
        .view()
        .expect("a local session is drawn in the viewport")
        .inspect_world(&mut |w| out = read.take().map(|f| f(w)));
    out.expect("the play world can be read")
}

fn position(world: &World, id: InstanceId) -> Value {
    world.get(id, "Position").unwrap()
}

#[test]
fn play_does_not_change_edit_world() {
    let mut log = OutputLog::new(256);
    let (mut state, part) = editor(
        "game:GetService('RunService').Heartbeat:Connect(function() \
           workspace.Probe.Position = workspace.Probe.Position + Vector3.new(1, 0, 0) end)",
        &log,
    );
    state
        .apply(Command::set(
            part,
            "Transparency",
            Value::Number(0.0),
            Value::Number(0.5),
        ))
        .unwrap();
    state.selection.select_only(part);
    let edit_before = position(&state.world, part);
    let history = (
        state.history.len(),
        state.history.revision(),
        state.is_dirty(),
    );
    let cursor = state.world.change_cursor();

    assert!(state.start_play("play.start"));
    assert!(state.play.is_playing());
    frames(&mut state, 10);
    let moved = in_play(&state, |w| position(w, part));
    assert_ne!(moved, edit_before, "the script moves the play copy");
    assert_eq!(
        position(&state.world, part),
        edit_before,
        "not the edit world"
    );
    state.stop_play();

    assert!(!state.play.is_playing());
    assert!(state.play.view().is_none());
    assert_eq!(position(&state.world, part), edit_before);
    assert_eq!(
        state.world.change_cursor(),
        cursor,
        "no write reached the edit world"
    );
    assert_eq!(
        (
            state.history.len(),
            state.history.revision(),
            state.is_dirty()
        ),
        history
    );
    assert_eq!(state.selection.ids(), &[part]);
    state.undo();
    assert_eq!(
        state.world.get(part, "Transparency").unwrap(),
        Value::Number(0.0),
        "the undo history from before Play still works"
    );
    log.drain();
    assert!(!log.lines().any(|l| l.level == Level::Error));
}

#[test]
fn stop_with_pending_delay_fires_nothing_after_stop() {
    let mut log = OutputLog::new(512);
    let (mut state, _) = editor(
        "task.delay(0.5, function() print('late') end)\n\
         task.spawn(function() task.wait(0.5); print('waited') end)\n\
         game:GetService('RunService').Heartbeat:Connect(function() print('beat') end)",
        &log,
    );
    assert!(state.start_play("play.start"));
    frames(&mut state, 6);
    assert_eq!(count(&mut log, "beat"), 6);
    state.stop_play();
    // Long after the delay would have fired: no session, nothing runs.
    for _ in 0..120 {
        state.play.update(TICK);
    }
    assert_eq!(count(&mut log, "late"), 0);
    assert_eq!(count(&mut log, "waited"), 0);
    assert_eq!(count(&mut log, "beat"), 6);

    // A new Play schedules its own delay; the old one never comes back.
    assert!(state.start_play("play.start"));
    frames(&mut state, 24);
    assert_eq!(count(&mut log, "late"), 0, "0.4 s into the new session");
    frames(&mut state, 12);
    assert_eq!(count(&mut log, "late"), 1, "only the new session's delay");
    assert_eq!(count(&mut log, "waited"), 1);
    state.stop_play();
}

#[test]
fn play_twice_connects_handlers_once() {
    let mut log = OutputLog::new(256);
    let (mut state, _) = editor(
        "game:GetService('RunService').Heartbeat:Connect(function() print('beat') end)",
        &log,
    );
    for round in 1..=2u32 {
        assert!(state.start_play("play.start"));
        frames(&mut state, 5);
        assert_eq!(
            count(&mut log, "beat"),
            5 * round,
            "one handler in round {round}"
        );
        let players = in_play(&state, |w| w.children(w.root("Players").unwrap()).len());
        assert_eq!(players, 1, "one local player");
        state.stop_play();
    }
}

#[test]
fn edit_while_playing_is_refused() {
    let log = OutputLog::new(64);
    let (mut state, part) = editor("", &log);
    state.selection.select_only(part);
    assert!(state.start_play("play.start"));
    let before = position(&state.world, part);
    let cursor = state.world.change_cursor();

    let moved = Value::Vector3(Vec3::new(5.0, 0.0, 0.0));
    assert!(
        state
            .apply(Command::set(
                part,
                "Position",
                before.clone(),
                moved.clone()
            ))
            .is_err()
    );
    state.set_property(&[part], "Position", moved);
    state.delete_selection();
    state.duplicate_selection();
    assert!(state.insert("Part", None, None).is_none());
    state.undo();
    state.redo();

    assert_eq!(position(&state.world, part), before);
    assert_eq!(state.world.change_cursor(), cursor);
    assert!(state.history.is_empty());
    assert_eq!(state.selection.ids(), &[part], "selection unaffected");
    assert_eq!(state.current_note(), Some("Stop the game to edit"));
    state.stop_play();
    state
        .apply(Command::set(
            part,
            "Position",
            before,
            Value::Vector3(Vec3::new(5.0, 0.0, 0.0)),
        ))
        .unwrap();
}

#[test]
fn error_at_start_keeps_edit_mode() {
    let mut log = OutputLog::new(64);
    let (mut world, part) = world("print('ran before the failure')");
    // No Workspace: the character cannot spawn, so the game fails to start.
    world.destroy(world.root("Workspace").unwrap());
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Broken",
        log.sink(),
    );
    assert!(!state.start_play("play.start"));
    assert!(!state.play.is_playing());
    assert!(state.play.status().is_none());
    log.drain();
    assert!(
        log.lines()
            .any(|l| l.level == Level::Error && l.text.starts_with("Could not start")),
        "Output says why"
    );
    assert_eq!(
        state.current_note(),
        Some("Play could not start; see Output")
    );
    // Still editing.
    let ws = state.world.create("Folder", None).unwrap();
    assert!(state.insert("Part", Some(ws), None).is_some());
    let _ = part;
}

#[test]
fn script_errors_go_to_output_with_the_play_tag() {
    let mut log = OutputLog::new(64);
    let (mut state, _) = editor("print('hello'); error('boom')", &log);
    assert!(state.start_play("play.start"));
    frames(&mut state, 1);
    log.drain();
    let play = Tag::Other(Arc::from(PLAY_TAG));
    assert!(
        log.lines()
            .any(|l| l.tag == play && l.level == Level::Info && l.text == "hello")
    );
    assert!(log.lines().any(|l| l.tag == play
        && l.level == Level::Error
        && l.text.contains("ServerScriptService.Probe:1: boom")));
    assert!(
        state.play.is_playing(),
        "one failing script does not stop the game"
    );
    assert!(matches!(state.play.status(), Some(PlayStatus::Running(_))));
    state.stop_play();
}

#[test]
fn ten_play_stop_cycles_leak_nothing() {
    let mut log = OutputLog::new(4096);
    let (mut state, part) = editor(
        "task.delay(30, function() print('never') end)\n\
         game:GetService('RunService').Heartbeat:Connect(function() print('beat') end)\n\
         workspace.Probe.Touched:Connect(function() end)",
        &log,
    );
    let edit_instances = instances(&state.world);
    let edit_listeners = state.world.property_changed(part, "Position").len();
    let mut play_instances = None;
    for cycle in 1..=10u32 {
        assert!(state.start_play("play.start"), "cycle {cycle}");
        frames(&mut state, 3);
        assert_eq!(
            count(&mut log, "beat"),
            3 * cycle,
            "one handler in cycle {cycle}"
        );
        let (n, players) = in_play(&state, |w| {
            (instances(w), w.children(w.root("Players").unwrap()).len())
        });
        assert_eq!(players, 1);
        assert_eq!(
            *play_instances.get_or_insert(n),
            n,
            "play world size in cycle {cycle}"
        );
        state.stop_play();
        assert!(state.play.view().is_none());
        assert_eq!(instances(&state.world), edit_instances);
        assert_eq!(
            state.world.property_changed(part, "Position").len(),
            edit_listeners
        );
    }
    assert_eq!(count(&mut log, "never"), 0);
    assert!(state.history.is_empty());
}

fn harness(state: EditorState, output: OutputPanel) -> Harness<'static, EditorUi> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            |ui, editor: &mut EditorUi| {
                editor.ui(ui, None);
            },
            EditorUi::new(state, output),
        );
    h.run_steps(3);
    h
}

#[test]
fn f5_plays_keys_reach_the_game_and_escape_stops() {
    let output = OutputPanel::new();
    let (world, part) = world("");
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Test",
        output.log.sink(),
    );
    state.can_save = false;
    state.tool = metallic_studio::editor::Tool::Select;
    let mut h = harness(state, output);
    // Selected after the first frames (the builder's accessibility check runs on those).
    h.state_mut().state.selection.select_only(part);
    h.run_steps(2);
    h.key_press(egui::Key::F5);
    h.run_steps(2);
    assert!(h.state().state.play.is_playing());

    let root = |w: &World| {
        let player = w
            .find_first_child(w.root("Players").unwrap(), "Player1")
            .unwrap();
        let Value::Ref(Some(model)) = w.get(player, "Character").unwrap() else {
            panic!("a character");
        };
        position(w, w.find_first_child(model, "HumanoidRootPart").unwrap())
    };
    let start = in_play(&h.state().state, root);
    // W held for a few frames walks the character; the Move tool is not picked by W.
    h.hover_at(h.state().viewport.rect().center());
    h.event(egui::Event::Key {
        key: egui::Key::W,
        physical_key: Some(egui::Key::W),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..20 {
        h.step();
    }
    h.event(egui::Event::Key {
        key: egui::Key::W,
        physical_key: Some(egui::Key::W),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    let walked = in_play(&h.state().state, root);
    let (Value::Vector3(a), Value::Vector3(b)) = (start, walked) else {
        panic!("positions");
    };
    // Falling changes only the height; walking moves it across the ground.
    assert!(
        (b.x - a.x).abs() + (b.z - a.z).abs() > 0.1,
        "W moves the character"
    );
    assert_eq!(h.state().state.tool, metallic_studio::editor::Tool::Select);

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let s = &h.state().state;
    assert!(!s.play.is_playing());
    assert!(s.history.is_empty());
    assert_eq!(s.selection.ids(), &[part]);
    assert_eq!(
        position(&s.world, part),
        Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
    );
}
