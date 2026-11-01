//! Rebuilding the collider of a resting unanchored part (a `Size` or `Shape` change) keeps the
//! physics world consistent: no debug assertion in Rapier's island bookkeeping, and the part
//! keeps resting (#423). Rapier 0.36.1's collision pipeline left bodies inserted after a step
//! out of the islands; `sync_from` no longer runs it once the simulation has stepped.

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

fn run(phys: &mut PhysicsWorld, world: &mut World, seconds: f64) {
    for _ in 0..(seconds * 60.0).round() as usize {
        phys.step(world, 1.0 / 60.0);
    }
}

fn y(world: &World, id: InstanceId) -> f64 {
    match world.get(id, "Position").unwrap() {
        Value::Vector3(v) => v.y,
        _ => panic!("not a vector"),
    }
}

/// A part resting (asleep) on an anchored baseplate.
fn resting(seconds: f64) -> (World, PhysicsWorld, InstanceId) {
    let mut world = World::new_with_services();
    part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(512.0, 4.0, 512.0),
        true,
    );
    let p = part(
        &mut world,
        Vec3::new(0.0, 6.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    run(&mut phys, &mut world, seconds);
    (world, phys, p)
}

#[test]
fn resizing_a_resting_part_keeps_it_resting() {
    for seconds in [0.5, 2.0, 5.0] {
        let (mut world, mut phys, p) = resting(seconds);
        world
            .set(p, "Size", Value::Vector3(Vec3::new(4.0, 2.0, 2.0)))
            .unwrap();
        phys.sync_from(&world);
        run(&mut phys, &mut world, 1.0);
        assert!((y(&world, p) - 3.0).abs() < 0.05, "y = {}", y(&world, p));
    }
}

#[test]
fn reshaping_a_resting_part_keeps_it_resting() {
    for seconds in [0.5, 2.0, 5.0] {
        let (mut world, mut phys, p) = resting(seconds);
        world
            .set(p, "Shape", Value::String("Ball".to_owned()))
            .unwrap();
        phys.sync_from(&world);
        run(&mut phys, &mut world, 1.0);
        assert!(y(&world, p) > 2.0, "y = {}", y(&world, p));
    }
}

#[test]
fn a_part_added_after_the_first_step_falls_and_rests() {
    let (mut world, mut phys, _) = resting(1.0);
    let q = part(
        &mut world,
        Vec3::new(10.0, 8.0, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    phys.sync_from(&world);
    run(&mut phys, &mut world, 3.0);
    assert!((y(&world, q) - 3.0).abs() < 0.05, "y = {}", y(&world, q));
}

#[test]
fn a_rebuilt_part_is_hit_by_raycasts_before_the_next_step() {
    let (mut world, mut phys, p) = resting(2.0);
    world
        .set(p, "Size", Value::Vector3(Vec3::new(4.0, 5.0, 2.0)))
        .unwrap();
    phys.sync_from(&world);
    // The new collider's top is 2 studs higher than the old one's.
    let hit = phys
        .raycast(Vec3::new(0.0, 20.0, 0.0), Vec3::new(0.0, -30.0, 0.0), &[])
        .expect("hits the rebuilt part");
    assert_eq!(hit.instance, p);
    assert!(hit.position.y > 4.5, "hit at y = {}", hit.position.y);
}
