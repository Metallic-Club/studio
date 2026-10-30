use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_physics::PhysicsWorld;

fn part(world: &mut World, pos: Vec3, size: Vec3, anchored: bool) -> InstanceId {
    let ws = world.root("Workspace").unwrap();
    let id = world.create("Part", Some(ws)).unwrap();
    world.set(id, "Position", Value::Vector3(pos)).unwrap();
    world.set(id, "Size", Value::Vector3(size)).unwrap();
    world.set(id, "Anchored", Value::Bool(anchored)).unwrap();
    id
}

fn pos(world: &World, id: InstanceId) -> Vec3 {
    match world.get(id, "Position").unwrap() {
        Value::Vector3(v) => v,
        _ => panic!("not a vector"),
    }
}

fn run(phys: &mut PhysicsWorld, world: &mut World, seconds: f64) {
    for _ in 0..(seconds * 60.0).round() as usize {
        phys.step(world, 1.0 / 60.0);
    }
}

fn baseplate(world: &mut World) -> InstanceId {
    part(
        world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(512.0, 4.0, 512.0),
        true,
    )
}

#[test]
fn unanchored_part_falls_and_rests_on_baseplate() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let p = part(
        &mut world,
        Vec3::new(0.0, 10.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 2.0);
    // baseplate top is y = 2, the part is 1 stud tall, so it rests at 2.5
    assert!(
        (pos(&world, p).y - 2.5).abs() < 0.05,
        "y = {}",
        pos(&world, p).y
    );
}

#[test]
fn anchored_part_never_moves() {
    let mut world = World::new_with_services();
    let a = part(
        &mut world,
        Vec3::new(1.0, 20.0, 3.0),
        Vec3::new(4.0, 1.0, 2.0),
        true,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 1.0);
    assert_eq!(pos(&world, a), Vec3::new(1.0, 20.0, 3.0));
}

#[test]
fn touched_fires_once_per_contact() {
    let mut world = World::new_with_services();
    let base = baseplate(&mut world);
    let p = part(
        &mut world,
        Vec3::new(0.0, 6.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 2.0);
    let first = phys.drain_touched();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0] == (p, base) || first[0] == (base, p));
    run(&mut phys, &mut world, 1.0);
    assert!(phys.drain_touched().is_empty());
    // lift the part off the baseplate, let it separate, then drop it again
    world
        .set(p, "Position", Value::Vector3(Vec3::new(0.0, 8.0, 0.0)))
        .unwrap();
    phys.step(&mut world, 1.0 / 60.0);
    assert!(phys.drain_touched().is_empty());
    run(&mut phys, &mut world, 2.0);
    let second = phys.drain_touched();
    assert_eq!(second.len(), 1, "{second:?}");
}

#[test]
fn can_collide_false_is_touched_but_not_solid() {
    let mut world = World::new_with_services();
    let zone = part(
        &mut world,
        Vec3::new(0.0, 5.0, 0.0),
        Vec3::new(8.0, 4.0, 8.0),
        true,
    );
    world.set(zone, "CanCollide", Value::Bool(false)).unwrap();
    let p = part(
        &mut world,
        Vec3::new(0.0, 12.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 1.5);
    assert!(
        pos(&world, p).y < -10.0,
        "fell through: y = {}",
        pos(&world, p).y
    );
    let touched = phys.drain_touched();
    assert_eq!(touched.len(), 1, "{touched:?}");
    assert!(touched[0] == (p, zone) || touched[0] == (zone, p));
}

#[test]
fn raycast_hits_nearest_and_respects_ignore() {
    let mut world = World::new_with_services();
    let near = part(
        &mut world,
        Vec3::new(0.0, 0.0, -10.0),
        Vec3::new(4.0, 4.0, 4.0),
        true,
    );
    let far = part(
        &mut world,
        Vec3::new(0.0, 0.0, -20.0),
        Vec3::new(4.0, 4.0, 4.0),
        true,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    let origin = Vec3::new(0.0, 0.0, 0.0);
    let dir = Vec3::new(0.0, 0.0, -100.0);
    let hit = phys.raycast(origin, dir, &[]).unwrap();
    assert_eq!(hit.instance, near);
    assert!((hit.distance - 8.0).abs() < 1e-3);
    assert!((hit.position.z + 8.0).abs() < 1e-3);
    assert!((hit.normal.z - 1.0).abs() < 1e-3);
    let hit = phys.raycast(origin, dir, &[near]).unwrap();
    assert_eq!(hit.instance, far);
    assert!((hit.distance - 18.0).abs() < 1e-3);
    assert!(
        phys.raycast(origin, Vec3::new(0.0, 0.0, -5.0), &[])
            .is_none()
    );
}

#[test]
fn moved_anchored_part_has_velocity() {
    let mut world = World::new_with_services();
    let a = part(
        &mut world,
        Vec3::new(0.0, 10.0, 0.0),
        Vec3::new(4.0, 1.0, 4.0),
        true,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    phys.step(&mut world, 1.0 / 60.0);
    assert!(phys.velocity(a).unwrap().magnitude() < 1e-6);
    world
        .set(a, "Position", Value::Vector3(Vec3::new(1.0, 10.0, 0.0)))
        .unwrap();
    phys.step(&mut world, 1.0 / 60.0);
    let v = phys.velocity(a).unwrap();
    assert!((v.x - 60.0).abs() < 0.5, "v = {v:?}");
    assert_eq!(pos(&world, a), Vec3::new(1.0, 10.0, 0.0));
}

#[test]
fn moving_anchored_platform_carries_rider() {
    let mut world = World::new_with_services();
    let plat = part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(20.0, 2.0, 20.0),
        true,
    );
    let rider = part(
        &mut world,
        Vec3::new(0.0, 1.6, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.5);
    let x0 = pos(&world, rider).x;
    for i in 1..=60 {
        world
            .set(
                plat,
                "Position",
                Value::Vector3(Vec3::new(i as f64 * 0.1, 0.0, 0.0)),
            )
            .unwrap();
        phys.step(&mut world, 1.0 / 60.0);
    }
    assert!(
        pos(&world, rider).x - x0 > 3.0,
        "rider moved {}",
        pos(&world, rider).x - x0
    );
}

#[test]
fn parts_outside_workspace_are_ignored() {
    let mut world = World::new_with_services();
    let p = part(
        &mut world,
        Vec3::new(0.0, 10.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.5);
    assert!(pos(&world, p).y < 10.0);
    let storage = world.root("ReplicatedStorage").unwrap();
    world.set_parent(p, Some(storage)).unwrap();
    phys.sync_from(&world);
    let y = pos(&world, p).y;
    run(&mut phys, &mut world, 0.5);
    assert_eq!(pos(&world, p).y, y);
    assert!(phys.velocity(p).is_none());
}

#[test]
fn anchored_flip_without_sync_does_not_freeze_body() {
    let mut world = World::new_with_services();
    let p = part(
        &mut world,
        Vec3::new(0.0, 100.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.25);
    world.set(p, "Anchored", Value::Bool(true)).unwrap();
    let y = pos(&world, p).y;
    run(&mut phys, &mut world, 0.25);
    assert!(
        pos(&world, p).y < y - 1.0,
        "body froze at {}",
        pos(&world, p).y
    );
    phys.sync_from(&world);
    let y = pos(&world, p).y;
    run(&mut phys, &mut world, 0.25);
    assert_eq!(pos(&world, p).y, y);
}

#[test]
fn kinematic_body_touching_anchored_sensor_fires_touched() {
    let mut world = World::new_with_services();
    let zone = part(
        &mut world,
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(4.0, 4.0, 4.0),
        true,
    );
    world.set(zone, "CanCollide", Value::Bool(false)).unwrap();
    let mover = part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        true,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    for i in 1..=120 {
        world
            .set(
                mover,
                "Position",
                Value::Vector3(Vec3::new(i as f64 * 0.1, 0.0, 0.0)),
            )
            .unwrap();
        phys.step(&mut world, 1.0 / 60.0);
    }
    let touched = phys.drain_touched();
    assert_eq!(touched.len(), 1, "{touched:?}");
    assert!(touched[0] == (mover, zone) || touched[0] == (zone, mover));
}

#[test]
fn step_clamps_huge_dt() {
    let mut world = World::new_with_services();
    let p = part(
        &mut world,
        Vec3::new(0.0, 1000.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    phys.step(&mut world, 100.0);
    // 0.25 s of free fall is about 6.1 studs
    assert!(pos(&world, p).y > 990.0, "y = {}", pos(&world, p).y);
}
