//! New/rebuilt touching bodies must join Rapier's persistent islands before real ticks.
use metallic_physics2d::*;

const DT: f64 = 1.0 / 60.0;
const FLOOR: BodyId = BodyId(1);
const BOX: BodyId = BodyId(2);

fn floor() -> BodyDesc {
    BodyDesc::new(FLOOR, BodyKind::Static, Vec2::new(0.0, -1.0))
        .with_collider(ColliderDesc::rect(40.0, 2.0))
}

fn resting_box() -> BodyDesc {
    BodyDesc::new(BOX, BodyKind::Dynamic, Vec2::new(0.0, 1.0))
        .with_collider(ColliderDesc::rect(2.0, 2.0))
}

fn bootstrapped() -> (PhysicsWorld2d, Scene2d) {
    let mut world = PhysicsWorld2d::new(WorldConfig::default()).unwrap();
    let mut scene = Scene2d {
        bodies: vec![floor()],
        joints: vec![],
    };
    world.sync_from(&scene).unwrap();
    world.step(&mut scene, DT).unwrap();
    (world, scene)
}

fn assert_resting(world: &mut PhysicsWorld2d, scene: &mut Scene2d) {
    let ticks = world.tick_count();
    for _ in 0..60 {
        assert_eq!(world.step(scene, DT).unwrap(), 1);
    }
    assert_eq!(world.tick_count(), ticks + 60);
    let state = world.body_state(BOX).unwrap();
    assert!((state.pose.position.y - 1.0).abs() < 0.05, "{state:?}");
    assert!(state.pose.position.x.abs() < 0.05, "{state:?}");
    assert!(
        state.sleeping || state.linear_velocity.length() < 0.05,
        "{state:?}"
    );
    assert!(world.is_touching(FLOOR, BOX));
}

#[test]
fn inserting_a_touching_dynamic_body_after_island_bootstrap_keeps_it_resting() {
    let (mut world, mut scene) = bootstrapped();
    scene.bodies.push(resting_box());
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
}

#[test]
fn rebuilding_a_touching_dynamic_body_keeps_it_resting() {
    let (mut world, mut scene) = bootstrapped();
    scene.bodies.push(resting_box());
    // First avoid the late-insertion path so this separately exercises the rebuild.
    scene.bodies[1].position.y = 5.0;
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
    scene.bodies[1].colliders[0].material.friction = 0.8;
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
}

fn settled() -> (PhysicsWorld2d, Scene2d) {
    let mut world = PhysicsWorld2d::new(WorldConfig::default()).unwrap();
    let mut scene = Scene2d {
        bodies: vec![floor(), resting_box()],
        joints: vec![],
    };
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
    world.drain_touched();
    (world, scene)
}

#[test]
fn resizing_a_touching_collider_preserves_rest_and_fresh_contacts() {
    let (mut world, mut scene) = settled();
    scene.bodies[1].colliders[0] = ColliderDesc::rect(3.0, 2.0);
    world.sync_from(&scene).unwrap();
    assert_eq!(world.drain_touched(), [(FLOOR, BOX)]);
    assert_resting(&mut world, &mut scene);
    assert!(
        world.drain_touched().is_empty(),
        "continuing contact is not a new touch"
    );
    // A real separation and landing must still produce exactly one fresh touch.
    scene.bodies[1].position.y = 5.0;
    world.step(&mut scene, DT).unwrap();
    assert!(!world.is_touching(FLOOR, BOX));
    assert_resting(&mut world, &mut scene);
    assert_eq!(world.drain_touched(), [(FLOOR, BOX)]);
}

#[test]
fn changing_a_touching_body_to_static_and_back_preserves_rest() {
    let (mut world, mut scene) = settled();
    scene.bodies[1].kind = BodyKind::Static;
    world.sync_from(&scene).unwrap();
    world.step(&mut scene, DT).unwrap();
    scene.bodies[1].kind = BodyKind::Dynamic;
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
}

#[test]
fn switching_a_touching_sensor_back_to_solid_preserves_events_and_rest() {
    let (mut world, mut scene) = settled();
    scene.bodies[1].colliders[0].sensor = true;
    world.sync_from(&scene).unwrap();
    assert_eq!(
        world.drain_sensor_events(),
        [SensorEvent {
            sensor: BOX,
            other: FLOOR,
            kind: SensorEventKind::Enter,
        }]
    );
    world.step(&mut scene, DT).unwrap();
    assert!(
        scene.bodies[1].position.y < 1.0,
        "sensor must not support the box"
    );
    scene.bodies[1].colliders[0].sensor = false;
    scene.bodies[1].position.y = 1.0;
    world.sync_from(&scene).unwrap();
    assert_eq!(
        world.drain_sensor_events(),
        [SensorEvent {
            sensor: BOX,
            other: FLOOR,
            kind: SensorEventKind::Exit,
        }]
    );
    assert_resting(&mut world, &mut scene);
}

#[test]
fn removing_and_readding_a_touching_body_preserves_rest() {
    let (mut world, mut scene) = settled();
    let body = scene.bodies.pop().unwrap();
    world.sync_from(&scene).unwrap();
    assert!(!world.is_touching(FLOOR, BOX));
    assert!(world.body_state(BOX).is_none());
    scene.bodies.push(body);
    world.sync_from(&scene).unwrap();
    assert_resting(&mut world, &mut scene);
}

#[test]
fn sync_refresh_preserves_pending_time_pose_velocity_settings_and_queries() {
    let config = WorldConfig {
        gravity: Vec2::new(0.0, -12.0),
        tick: 1.0 / 90.0,
        solver_substeps: 3,
        ..WorldConfig::default()
    };
    let mut scene = Scene2d {
        bodies: vec![floor(), resting_box()],
        joints: vec![],
    };
    scene.bodies[1].position.y = 30.0;
    let mut control_scene = scene.clone();
    let mut world = PhysicsWorld2d::new(config).unwrap();
    let mut control = PhysicsWorld2d::new(config).unwrap();
    for sim in [&mut world, &mut control] {
        sim.sync_from(&scene).unwrap();
        sim.set_velocity(BOX, Vec2::new(3.25, -1.5), 0.5).unwrap();
    }
    assert_eq!(world.step(&mut scene, config.tick * 0.5).unwrap(), 0);
    assert_eq!(
        control.step(&mut control_scene, config.tick * 0.5).unwrap(),
        0
    );
    let before = world.body_state(BOX).unwrap();
    let alpha = world.interpolation_alpha();
    for _ in 0..5 {
        world.sync_from(&scene).unwrap();
    }
    assert_eq!(world.config(), &config);
    assert_eq!(world.tick_count(), 0);
    assert_eq!(world.interpolation_alpha(), alpha);
    assert_eq!(world.body_state(BOX).unwrap(), before);
    assert!(world.drain_touched().is_empty());
    let hit = world
        .raycast(
            Vec2::new(10.0, 5.0),
            Vec2::new(0.0, -10.0),
            &RayFilter::default(),
        )
        .unwrap();
    assert_eq!(hit.body, FLOOR);
    assert!(hit.point.y.abs() < 1e-6);
    assert_eq!(world.step(&mut scene, config.tick * 0.5).unwrap(), 1);
    assert_eq!(
        control.step(&mut control_scene, config.tick * 0.5).unwrap(),
        1
    );
    assert_eq!(world.tick_count(), 1);
    assert_eq!(world.body_state(BOX), control.body_state(BOX));
    assert!(scene.bodies[1].position.y < before.pose.position.y);
}
