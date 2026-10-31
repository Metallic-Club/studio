//! Destroyed-target restoration must not recreate motion state or affect a live replacement.
use metallic_core::{Value, Vec3, World};
use metallic_physics::{
    CharacterController, MoveInput, PhysicsWorld, place_character, spawn_character,
};

#[test]
fn restoring_a_cleaned_destroyed_character_should_not_recreate_motion_state() {
    let mut world = World::new_with_services();
    let players = world.root("Players").unwrap();
    let player = world.create("Player", Some(players)).unwrap();
    let character = spawn_character(&mut world, player).unwrap();
    let physics = PhysicsWorld::new();
    let mut controller = CharacterController::new();
    controller.step(&mut world, &physics, character, MoveInput::default(), 0.0);
    let saved = controller.save_state(&world, character).unwrap();
    assert!(controller.state(character).is_some());
    controller.drain_events();
    world.destroy(character);
    controller.step(&mut world, &physics, character, MoveInput::default(), 0.0);
    assert_eq!(
        controller.state(character),
        None,
        "existing cleanup control"
    );
    controller.drain_events();
    let cursor = world.change_cursor();
    controller.restore_state(&mut world, character, &saved);
    assert!(!world.is_alive(character));
    assert_eq!(world.get(player, "Character").unwrap(), Value::Ref(None));
    assert_eq!(world.change_cursor(), cursor);
    assert!(controller.drain_events().is_empty());
    assert_eq!(controller.save_state(&world, character), None);
    assert_eq!(
        controller.state(character),
        None,
        "stale restore must not recreate pruned state"
    );
}

#[test]
fn live_character_restore_retains_movement_and_pose() {
    let mut world = World::new_with_services();
    let player = world.create("Player", world.root("Players")).unwrap();
    let character = spawn_character(&mut world, player).unwrap();
    let physics = PhysicsWorld::new();
    let mut controller = CharacterController::new();
    controller.step(
        &mut world,
        &physics,
        character,
        MoveInput {
            dir: Vec3::new(0.6, 0.0, -0.8),
            run: true,
            ..MoveInput::default()
        },
        1.0 / 60.0,
    );
    let saved = controller.save_state(&world, character).unwrap();
    place_character(&mut world, character, Vec3::new(20.0, 30.0, 40.0), 80.0);
    assert_ne!(
        controller.save_state(&world, character).unwrap().position,
        saved.position
    );
    controller.restore_state(&mut world, character, &saved);
    assert_eq!(controller.save_state(&world, character), Some(saved));
    assert_eq!(controller.state(character), Some(saved.state));
    assert_eq!(
        world.get(player, "Character").unwrap(),
        Value::Ref(Some(character))
    );
}

#[test]
fn stale_restore_does_not_touch_a_new_character_generation() {
    let mut world = World::new_with_services();
    let player = world.create("Player", world.root("Players")).unwrap();
    let old = spawn_character(&mut world, player).unwrap();
    let physics = PhysicsWorld::new();
    let mut controller = CharacterController::new();
    controller.step(&mut world, &physics, old, MoveInput::default(), 0.0);
    let old_state = controller.save_state(&world, old).unwrap();
    world.destroy(old);
    controller.step(&mut world, &physics, old, MoveInput::default(), 0.0);
    let replacement = spawn_character(&mut world, player).unwrap();
    assert_ne!(
        old, replacement,
        "replacement must have a fresh generational handle"
    );
    place_character(&mut world, replacement, Vec3::new(10.0, 20.0, 30.0), 50.0);
    controller.step(&mut world, &physics, replacement, MoveInput::default(), 0.0);
    let replacement_state = controller.save_state(&world, replacement).unwrap();
    controller.drain_events();
    let cursor = world.change_cursor();
    for _ in 0..3 {
        controller.restore_state(&mut world, old, &old_state);
    }
    assert_eq!(controller.state(old), None);
    assert_eq!(controller.yaw(old), None);
    assert_eq!(controller.velocity(old), None);
    assert_eq!(
        controller.save_state(&world, replacement),
        Some(replacement_state)
    );
    assert_eq!(
        world.get(player, "Character").unwrap(),
        Value::Ref(Some(replacement))
    );
    assert_eq!(world.change_cursor(), cursor);
    assert!(controller.drain_events().is_empty());
}

#[test]
fn valid_target_keeps_existing_nonfinite_state_sanitization() {
    let mut world = World::new_with_services();
    let player = world.create("Player", world.root("Players")).unwrap();
    let character = spawn_character(&mut world, player).unwrap();
    let mut controller = CharacterController::new();
    let mut state = controller.save_state(&world, character).unwrap();
    state.position = Vec3::new(f64::NAN, 1.0, 2.0);
    state.vertical = f64::INFINITY;
    state.inherited = Vec3::new(0.0, f64::NEG_INFINITY, 0.0);
    state.yaw = f64::NAN;
    state.velocity = Vec3::new(f64::INFINITY, 0.0, 0.0);
    let cursor = world.change_cursor();
    controller.restore_state(&mut world, character, &state);
    let restored = controller.save_state(&world, character).unwrap();
    assert_eq!(restored.position, Vec3::new(0.0, 3.0, 0.0));
    assert_eq!(restored.vertical, 0.0);
    assert_eq!(restored.inherited, Vec3::default());
    assert_eq!(restored.yaw, 0.0);
    assert_eq!(restored.velocity, Vec3::default());
    assert_eq!(
        world.change_cursor(),
        cursor,
        "invalid position still leaves the World alone"
    );
}
