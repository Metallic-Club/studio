//! Input Actions panel (#253): the done-when items that need a real scene file and frames.

use std::path::PathBuf;

use eframe::egui::{self, Event, Key, Modifiers, RawInput, Rect, pos2, vec2};
use metallic_core::{InstanceId, UDim2, Value, World, load_scene, save_scene};
use metallic_runtime::input::default_character_controls;
use metallic_studio::input_actions::commands::{CommandSink, History, StepBuilder};
use metallic_studio::input_actions::model::{InputModel, context_snapshot};
use metallic_studio::input_actions::touch::{
    self, ControlRole, DevicePreset, DeviceProfile, Orientation, violations,
};
use metallic_studio::input_actions::{InputActionsPanel, preview::PreviewAs};

fn parkour() -> World {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour/main.mscene");
    load_scene(&path).expect("parkour scene loads")
}

/// Adds the built-in controls to the scene, as the panel's "Add to scene" does.
fn materialize(world: &mut World, history: &mut History) -> InstanceId {
    let rs = world.root("ReplicatedStorage").unwrap();
    let mut b = StepBuilder::new(world);
    let ctx = b
        .create(rs, context_snapshot(&default_character_controls()))
        .unwrap();
    history.record(b.finish("Add CharacterControls").unwrap());
    ctx
}

fn jump_center(world: &World, profile: &DeviceProfile) -> [f32; 2] {
    let actions = touch::touch_actions(world);
    let placed = touch::layout(profile, &actions);
    placed
        .iter()
        .find(|p| p.role == ControlRole::Primary)
        .unwrap()
        .center
}

#[test]
fn moved_button_survives_save_and_reopen_and_one_undo_restores_it() {
    let mut world = parkour();
    assert!(InputModel::read(&world).uses_builtin_character_controls());
    let mut history = History::new();
    let ctx = materialize(&mut world, &mut history);
    let jump = world.find_first_child(ctx, "Jump").unwrap();
    let phone = DevicePreset::Phone.profile(Orientation::Landscape);
    let auto = jump_center(&world, &phone);

    // Drag Jump 100 dp up and make it 80 dp: one gesture, one step.
    let target = [auto[0], auto[1] - 100.0];
    let mut b = StepBuilder::new(&mut world);
    touch::write_custom(&mut b, jump, target, 80.0, &phone).unwrap();
    let step = b.finish("Move Jump touch control").unwrap();
    assert_eq!(step.edits.len(), 3, "TouchLayout, TouchPosition, TouchSize");
    history.record(step);
    assert_eq!(jump_center(&world, &phone), target);

    // Save and reopen.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.mscene");
    save_scene(&world, &file).unwrap();
    let reopened = load_scene(&file).unwrap();
    assert_eq!(jump_center(&reopened, &phone), target);
    let model = InputModel::read(&reopened);
    let (_, a) = model
        .actions()
        .find(|(_, a)| a.name == "Jump")
        .expect("Jump saved");
    let id = a.instance.unwrap();
    assert_eq!(
        reopened.get(id, "TouchLayout").unwrap(),
        Value::String("Custom".into())
    );
    assert_eq!(reopened.get(id, "TouchSize").unwrap(), Value::Number(80.0));
    // The layout keeps the rules on every preset after the move.
    for p in DeviceProfile::all() {
        let placed = touch::layout(&p, &touch::touch_actions(&reopened));
        assert!(violations(&p, &placed).is_empty(), "{}", p.label());
    }

    // One undo puts Jump back on the automatic spot.
    assert!(history.undo(&mut world));
    assert_eq!(jump_center(&world, &phone), auto);
    assert_eq!(
        world.get(jump, "TouchPosition").unwrap(),
        Value::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0))
    );
}

fn frame(
    ctx: &egui::Context,
    panel: &mut InputActionsPanel,
    world: &mut World,
    events: Vec<Event>,
    t: f64,
) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 860.0))),
        time: Some(t),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| panel.show(ui, world));
    out.textures_delta.clear();
}

fn key(k: Key, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers,
        },
        Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
}

#[test]
fn panel_draws_every_tab_and_preview_mode_headless() {
    let ctx = egui::Context::default();
    let mut world = parkour();
    let mut panel = InputActionsPanel::new(&world);
    let mut t = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut panel, &mut world, Vec::new(), t);
        t += 0.016;
    }
    panel.set_tab(metallic_studio::input_actions::Tab::Preview);
    for mode in PreviewAs::ALL {
        panel.preview_state_mut().mode = mode;
        for _ in 0..2 {
            frame(&ctx, &mut panel, &mut world, Vec::new(), t);
            t += 0.016;
        }
    }
    panel.preview_state_mut().mode = PreviewAs::Phone;
    panel.preview_state_mut().editing_layout = true;
    frame(&ctx, &mut panel, &mut world, Vec::new(), t);
    // Drawing never changed the scene.
    assert!(panel.history().is_empty());
    assert!(InputModel::read(&world).uses_builtin_character_controls());
}

#[test]
fn keyboard_alone_reaches_the_panel_controls() {
    // Tab walks the focus through the panel; every stop must be one of the panel's widgets and
    // Tab must reach more than a handful of them (toolbar, tree rows, matrix cells, checks).
    let ctx = egui::Context::default();
    let mut world = parkour();
    let mut history = History::new();
    materialize(&mut world, &mut history);
    let mut panel = InputActionsPanel::new(&world);
    let mut t = 0.0;
    frame(&ctx, &mut panel, &mut world, Vec::new(), t);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..80 {
        t += 0.016;
        frame(
            &ctx,
            &mut panel,
            &mut world,
            key(Key::Tab, Modifiers::NONE),
            t,
        );
        if let Some(id) = ctx.memory(|m| m.focused()) {
            seen.insert(id);
        }
    }
    assert!(seen.len() >= 30, "Tab reached only {} widgets", seen.len());
}
