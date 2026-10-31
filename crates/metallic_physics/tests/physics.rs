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
fn touch_ended_fires_once_per_separation() {
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
    assert_eq!(phys.drain_touched().len(), 1);
    assert!(phys.drain_touch_ended().is_empty(), "still touching");
    run(&mut phys, &mut world, 1.0);
    assert!(phys.drain_touch_ended().is_empty());
    // lift the part off the baseplate: one stop, for the pair that was reported
    world
        .set(p, "Position", Value::Vector3(Vec3::new(0.0, 12.0, 0.0)))
        .unwrap();
    phys.step(&mut world, 1.0 / 60.0);
    let ended = phys.drain_touch_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert!(ended[0] == (p, base) || ended[0] == (base, p));
    assert!(phys.drain_touch_ended().is_empty(), "reported once");
    // touching again reports Touched and, after the next separation, TouchEnded again
    run(&mut phys, &mut world, 2.0);
    assert_eq!(phys.drain_touched().len(), 1);
    world
        .set(p, "Position", Value::Vector3(Vec3::new(0.0, 12.0, 0.0)))
        .unwrap();
    phys.step(&mut world, 1.0 / 60.0);
    assert_eq!(phys.drain_touch_ended().len(), 1);
}

#[test]
fn a_destroyed_part_ends_its_touches() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let p = part(
        &mut world,
        Vec3::new(0.0, 6.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 2.0);
    assert_eq!(phys.drain_touched().len(), 1);
    world.destroy(p);
    phys.sync_from(&world);
    phys.step(&mut world, 1.0 / 60.0);
    // The baseplate still exists and hears that `p` left.
    let ended = phys.drain_touch_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert!(ended[0].0 == p || ended[0].1 == p);
    run(&mut phys, &mut world, 0.5);
    assert!(phys.drain_touch_ended().is_empty(), "once");
    assert!(phys.drain_touched().is_empty());
}

/// A part resting on the baseplate, settled, with its first touch drained.
fn resting() -> (World, PhysicsWorld, InstanceId, InstanceId) {
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
    assert_eq!(phys.drain_touched().len(), 1);
    (world, phys, base, p)
}

/// A part resting on the baseplate and an anchored zone (CanCollide off) around it, settled,
/// with the first touches drained.
fn zone_around_resting_part() -> (World, PhysicsWorld, InstanceId) {
    let (mut world, mut phys, _, _) = resting();
    let zone = part(
        &mut world,
        Vec3::new(0.0, 2.5, 0.0),
        Vec3::new(6.0, 3.0, 6.0),
        true,
    );
    world.set(zone, "CanCollide", Value::Bool(false)).unwrap();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    let first = phys.drain_touched();
    assert_eq!(
        first.len(),
        1,
        "the zone touches the resting part: {first:?}"
    );
    (world, phys, zone)
}

/// Changing `prop` rebuilds the collider of the unanchored part resting on the baseplate; it
/// keeps touching the baseplate, so neither a second `Touched` nor a `TouchEnded` fires (and
/// Rapier's islands stay consistent, #423).
fn rebuild_resting_part(prop: &str, value: Value) {
    let (mut world, mut phys, _, p) = resting();
    world.set(p, prop, value).unwrap();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.5);
    assert!(phys.drain_touched().is_empty(), "{prop}: a second Touched");
    assert!(phys.drain_touch_ended().is_empty(), "{prop}: a TouchEnded");
    assert!(pos(&world, p).y > 2.0, "{prop}: fell into the baseplate");
}

/// Changing `prop` rebuilds the zone's collider; it keeps touching the part, so neither a
/// second `Touched` nor a `TouchEnded` fires.
fn rebuild_while_touching(prop: &str, value: Value) {
    let (mut world, mut phys, zone) = zone_around_resting_part();
    world.set(zone, prop, value).unwrap();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    assert!(phys.drain_touched().is_empty(), "{prop}: a second Touched");
    assert!(phys.drain_touch_ended().is_empty(), "{prop}: a TouchEnded");
}

#[test]
fn resizing_a_touching_part_fires_no_second_touched() {
    rebuild_resting_part("Size", Value::Vector3(Vec3::new(4.0, 1.2, 2.0)));
}

#[test]
fn reshaping_a_touching_part_fires_no_second_touched() {
    rebuild_resting_part("Shape", Value::String("Ball".into()));
}

#[test]
fn changing_the_material_of_a_touching_part_fires_no_second_touched() {
    rebuild_resting_part("Material", Value::String("Ice".into()));
}

/// An unanchored part with `CanCollide` off falls through the floor, so this one rebuilds an
/// anchored zone instead.
#[test]
fn turning_on_can_collide_on_a_touching_part_fires_no_second_touched() {
    rebuild_while_touching("CanCollide", Value::Bool(true));
}

#[test]
fn a_rebuild_that_separates_the_parts_ends_the_touch_once() {
    let (mut world, mut phys, zone) = zone_around_resting_part();
    // Shrunk to a sliver in a corner, away from the part.
    world
        .set(zone, "Size", Value::Vector3(Vec3::new(0.2, 0.2, 0.2)))
        .unwrap();
    world
        .set(zone, "Position", Value::Vector3(Vec3::new(2.8, 3.9, 2.8)))
        .unwrap();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    let ended = phys.drain_touch_ended();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert!(ended[0].0 == zone || ended[0].1 == zone);
    assert!(phys.drain_touched().is_empty());
}

#[test]
fn no_touch_ended_without_a_touched() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // Never touching anything: moved around, rebuilt, destroyed.
    let p = part(
        &mut world,
        Vec3::new(0.0, 20.0, 0.0),
        Vec3::new(1.0, 1.0, 1.0),
        true,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    world
        .set(p, "Position", Value::Vector3(Vec3::new(5.0, 30.0, 0.0)))
        .unwrap();
    world
        .set(p, "Size", Value::Vector3(Vec3::new(2.0, 2.0, 2.0)))
        .unwrap();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    world.destroy(p);
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.2);
    assert!(phys.drain_touched().is_empty());
    assert!(phys.drain_touch_ended().is_empty());
}

#[test]
fn a_touch_that_starts_and_stops_between_drains_reports_both() {
    let mut world = World::new_with_services();
    let base = baseplate(&mut world);
    // Placed overlapping the baseplate, then lifted away before anyone drains.
    let p = part(
        &mut world,
        Vec3::new(0.0, 2.2, 0.0),
        Vec3::new(1.0, 1.0, 1.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    phys.step(&mut world, 1.0 / 60.0);
    world
        .set(p, "Position", Value::Vector3(Vec3::new(0.0, 30.0, 0.0)))
        .unwrap();
    run(&mut phys, &mut world, 0.1);
    let touched = phys.drain_touched();
    let ended = phys.drain_touch_ended();
    assert_eq!(touched.len(), 1, "{touched:?}");
    assert_eq!(ended.len(), 1, "{ended:?}");
    let pair = |(a, b): (InstanceId, InstanceId)| if a <= b { (a, b) } else { (b, a) };
    assert_eq!(pair(touched[0]), pair(ended[0]));
    assert!(pair(touched[0]) == pair((p, base)));
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

#[test]
fn nan_dt_does_not_freeze() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let p = part(
        &mut world,
        Vec3::new(0.0, 100.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    phys.step(&mut world, f64::NAN);
    run(&mut phys, &mut world, 1.0);
    // 1 s of free fall is about 98 studs; without the guard the part stays at y = 100
    assert!(pos(&world, p).y < 50.0, "y = {}", pos(&world, p).y);
}

#[test]
fn infinite_dt_is_ignored() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let p = part(
        &mut world,
        Vec3::new(0.0, 100.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    phys.step(&mut world, f64::INFINITY);
    phys.step(&mut world, f64::NEG_INFINITY);
    assert_eq!(pos(&world, p), Vec3::new(0.0, 100.0, 0.0));
    run(&mut phys, &mut world, 1.0);
    assert!(pos(&world, p).y < 50.0, "y = {}", pos(&world, p).y);
}

fn spin(phys: &mut PhysicsWorld, world: &mut World, id: InstanceId, deg_per_step: f64, steps: u32) {
    for i in 1..=steps {
        world
            .set(
                id,
                "Orientation",
                Value::Vector3(Vec3::new(0.0, f64::from(i) * deg_per_step, 0.0)),
            )
            .unwrap();
        phys.step(world, 1.0 / 60.0);
    }
}

#[test]
fn rotating_anchored_part_has_angular_velocity() {
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
    assert!(phys.angular_velocity(a).unwrap().magnitude() < 1e-6);
    // 6 degrees per 1/60 s = 360 deg/s = 2*pi rad/s about +Y, also across the 180 deg wrap
    spin(&mut phys, &mut world, a, 6.0, 40);
    let w = phys.angular_velocity(a).unwrap();
    assert!(
        (w.y - std::f64::consts::TAU).abs() < 0.05 && w.x.abs() < 1e-3 && w.z.abs() < 1e-3,
        "w = {w:?}"
    );
    assert!(phys.velocity(a).unwrap().magnitude() < 1e-6);
    // standing still again clears it
    phys.step(&mut world, 1.0 / 60.0);
    assert!(phys.angular_velocity(a).unwrap().magnitude() < 1e-6);
}

#[test]
fn rotating_anchored_bar_pushes_loose_part() {
    let mut world = World::new_with_services();
    part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(512.0, 2.0, 512.0),
        true,
    );
    let bar = part(
        &mut world,
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::new(20.0, 2.0, 1.0),
        true,
    );
    let boxy = part(
        &mut world,
        Vec3::new(0.0, 2.0, 7.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.25);
    let start = pos(&world, boxy);
    spin(&mut phys, &mut world, bar, 6.0, 60);
    let end = pos(&world, boxy);
    let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
    assert!(
        moved > 2.0,
        "box moved only {moved} studs: {start:?} -> {end:?}"
    );
}

#[test]
fn rotating_anchored_platform_carries_rider() {
    let mut world = World::new_with_services();
    let plat = part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(20.0, 2.0, 20.0),
        true,
    );
    let rider = part(
        &mut world,
        Vec3::new(6.0, 1.6, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 0.5);
    // a quarter turn about +Y in one second takes (6, y, 0) toward (0, y, -6)
    spin(&mut phys, &mut world, plat, 1.5, 60);
    let p = pos(&world, rider);
    let turned = (-p.z).atan2(p.x).to_degrees();
    assert!(turned > 45.0, "rider turned {turned} deg, at {p:?}");
}

#[test]
fn replica_physics_never_moves_loose_parts() {
    let mut world = World::new_with_services();
    let p = part(
        &mut world,
        Vec3::new(0.0, 20.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new_replica();
    phys.sync_from(&world);
    run(&mut phys, &mut world, 1.0);
    assert_eq!(pos(&world, p), Vec3::new(0.0, 20.0, 0.0));
    let hit = phys
        .raycast(Vec3::new(0.0, 30.0, 0.0), Vec3::new(0.0, -30.0, 0.0), &[])
        .expect("ray hits the loose part");
    assert_eq!(hit.instance, p);
    // It follows the World when the replication layer moves it.
    world
        .set(p, "Position", Value::Vector3(Vec3::new(50.0, 20.0, 0.0)))
        .unwrap();
    run(&mut phys, &mut world, 0.1);
    assert!(
        phys.raycast(Vec3::new(50.0, 30.0, 0.0), Vec3::new(0.0, -30.0, 0.0), &[])
            .is_some()
    );
}
