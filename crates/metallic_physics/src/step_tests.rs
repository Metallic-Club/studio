use super::*;

fn part(world: &mut World, x: f64, size: Vec3, anchored: bool, solid: bool) -> InstanceId {
    let workspace = world.root("Workspace").unwrap();
    let id = world.create("Part", Some(workspace)).unwrap();
    world
        .set(id, "Position", Value::Vector3(Vec3::new(x, 100.0, 0.0)))
        .unwrap();
    world.set(id, "Size", Value::Vector3(size)).unwrap();
    world.set(id, "Anchored", Value::Bool(anchored)).unwrap();
    world.set(id, "CanCollide", Value::Bool(solid)).unwrap();
    id
}

fn position(world: &World, id: InstanceId) -> Vec3 {
    match world.get(id, "Position").unwrap() {
        Value::Vector3(value) => value,
        _ => panic!("Position must be a Vector3"),
    }
}

fn has_pair(pairs: &[(InstanceId, InstanceId)], a: InstanceId, b: InstanceId) -> bool {
    pairs
        .iter()
        .any(|&(left, right)| (left == a && right == b) || (left == b && right == a))
}

#[test]
fn thin_sensor_records_a_fast_kinematic_crossing_inside_one_frame() {
    let mut world = World::new_with_services();
    let sensor = part(&mut world, 1.0, Vec3::new(0.1, 2.0, 2.0), true, false);
    let mover = part(&mut world, 0.0, Vec3::new(0.1, 2.0, 2.0), true, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    world
        .set(
            mover,
            "Position",
            Value::Vector3(Vec3::new(2.0, 100.0, 0.0)),
        )
        .unwrap();
    physics.step(&mut world, 1.0 / 60.0);
    assert!(has_pair(&physics.drain_touched(), mover, sensor));
    assert_eq!(physics.last_step_pipeline_steps(), 4);
    assert_eq!(physics.velocity(mover), Some(Vec3::new(120.0, 0.0, 0.0)));
}

#[test]
fn thin_sensor_records_a_fast_dynamic_crossing_inside_one_frame() {
    let mut world = World::new_with_services();
    let sensor = part(&mut world, 1.0, Vec3::new(0.1, 2.0, 2.0), true, false);
    let mover = part(&mut world, 0.0, Vec3::new(0.1, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.rapier.bodies[physics.entries[&mover].body]
        .set_linvel(Vector::new(120.0, 0.0, 0.0), true);
    physics.step(&mut world, 1.0 / 60.0);
    assert!(has_pair(&physics.drain_touched(), mover, sensor));
    assert_eq!(physics.last_step_pipeline_steps(), 4);
}

#[test]
fn a_long_frame_does_not_skip_a_thin_sensor() {
    let mut world = World::new_with_services();
    let sensor = part(&mut world, 2.0, Vec3::new(0.1, 2.0, 2.0), true, false);
    let mover = part(&mut world, 0.0, Vec3::new(0.1, 2.0, 2.0), true, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    world
        .set(
            mover,
            "Position",
            Value::Vector3(Vec3::new(8.0, 100.0, 0.0)),
        )
        .unwrap();
    physics.step(&mut world, MAX_STEP);
    assert!(has_pair(&physics.drain_touched(), mover, sensor));
    assert_eq!(physics.last_step_pipeline_steps(), 60);
}

#[test]
fn a_fast_dynamic_box_is_stopped_by_a_thin_wall() {
    let mut world = World::new_with_services();
    let wall = part(&mut world, 1.0, Vec3::new(0.1, 20.0, 20.0), true, true);
    let mover = part(&mut world, 0.0, Vec3::new(0.1, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.rapier.bodies[physics.entries[&mover].body]
        .set_linvel(Vector::new(120.0, 0.0, 0.0), true);
    physics.step(&mut world, 1.0 / 60.0);
    assert!(
        position(&world, mover).x < 1.0,
        "{:#?}",
        position(&world, mover)
    );
    assert!(has_pair(&physics.drain_touched(), mover, wall));
    assert_eq!(physics.last_step_pipeline_steps(), 4);
}

#[test]
fn high_refresh_frames_keep_the_original_elapsed_quantum_without_extra_latency() {
    let mut world = World::new_with_services();
    let id = part(&mut world, 0.0, Vec3::new(2.0, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    let mut elapsed = 0.0;
    for _ in 0..144 {
        let before = position(&world, id).y;
        physics.step(&mut world, 1.0 / 144.0);
        assert!((1..=2).contains(&physics.last_step_pipeline_steps()));
        if elapsed < 0.1 {
            assert_eq!(physics.last_step_pipeline_steps(), 1);
        }
        elapsed += 1.0 / 144.0;
        assert!(
            position(&world, id).y < before,
            "a 144 Hz call must not wait for a 60 Hz accumulator"
        );
        assert!(physics.accumulator < SUBSTEP + 1e-9);
        let simulated = elapsed - physics.accumulator;
        assert!(simulated <= elapsed + 1e-9);
    }
    assert!(physics.accumulator < 1e-9);
    assert!((physics.velocity(id).unwrap().y + GRAVITY).abs() < 0.01);
}

#[test]
fn tiny_deltas_accumulate_and_invalid_deltas_do_not_poison_later_frames() {
    let mut world = World::new_with_services();
    let id = part(&mut world, 0.0, Vec3::new(2.0, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    for _ in 0..4 {
        physics.step(&mut world, SUBSTEP / 5.0);
    }
    assert_eq!(position(&world, id).y, 100.0);
    physics.step(&mut world, f64::NAN);
    physics.step(&mut world, f64::INFINITY);
    physics.step(&mut world, -1.0);
    physics.step(&mut world, SUBSTEP / 5.0);
    assert!(position(&world, id).y < 100.0);
    assert!(physics.accumulator < 1e-9);
    assert!((physics.velocity(id).unwrap().y + GRAVITY * SUBSTEP).abs() < 1e-5);
}

#[test]
fn ordinary_and_catch_up_frames_batch_full_pipeline_runs() {
    let mut world = World::new_with_services();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    assert_eq!(physics.last_step_pipeline_steps(), 1);
    assert_eq!(
        physics.rapier.integration_parameters.num_solver_iterations,
        4
    );
    assert!((f64::from(physics.rapier.integration_parameters.dt) - MAX_PIPELINE_STEP).abs() < 1e-8);
    physics.step(&mut world, 1.0 / 30.0);
    assert_eq!(physics.last_step_pipeline_steps(), 2);
    physics.step(&mut world, MAX_STEP);
    assert_eq!(physics.last_step_pipeline_steps(), 15);
    physics.step(&mut world, SUBSTEP / 2.0);
    assert_eq!(physics.last_step_pipeline_steps(), 0);
}

#[test]
fn ordinary_dynamic_parts_use_one_pipeline_run_and_keep_gravity() {
    let mut world = World::new_with_services();
    let id = part(&mut world, 0.0, Vec3::new(2.0, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    assert_eq!(physics.last_step_pipeline_steps(), 1);
    assert!((physics.velocity(id).unwrap().y + GRAVITY / 60.0).abs() < 1e-5);
    assert!(position(&world, id).y < 100.0);
}

#[test]
fn clearing_sensor_configuration_restores_batching_on_the_next_frame() {
    let mut world = World::new_with_services();
    let sensor = part(&mut world, 0.0, Vec3::new(2.0, 2.0, 2.0), true, false);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    assert_eq!(physics.last_step_pipeline_steps(), 4);
    world.set(sensor, "CanCollide", Value::Bool(true)).unwrap();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    assert_eq!(physics.last_step_pipeline_steps(), 1);
}

#[test]
fn clamped_collider_dimensions_drive_the_conservative_motion_bound() {
    let mut world = World::new_with_services();
    // Existing collider construction clamps this X half extent to 0.001, rather than using
    // the absolute value of the negative metadata dimension.
    part(&mut world, 0.0, Vec3::new(-100.0, 2.0, 2.0), false, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    assert_eq!(physics.last_step_pipeline_steps(), 4);
}
