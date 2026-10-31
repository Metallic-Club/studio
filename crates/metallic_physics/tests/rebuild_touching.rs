//! #405: rebuilding or re-adding a part that rests on another through `sync_from`, then
//! stepping, must keep Rapier's persistent islands consistent. In debug builds (which
//! `cargo test` uses) an inconsistency panics with "touching pair ... not linked in the
//! persistent islands". Each test settles a part on an anchored floor, applies one change that
//! rebuilds its body, syncs, steps 60 times, and checks the part still rests and touches work.

use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_physics::PhysicsWorld;

/// Floor top is y = 2; the 1-stud-tall part rests at y = 2.5.
const REST_Y: f64 = 2.5;

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

fn steps(phys: &mut PhysicsWorld, world: &mut World, n: usize) {
    for _ in 0..n {
        phys.step(world, 1.0 / 60.0);
    }
}

/// An anchored floor and a part settled on it for about 1 s (touch from landing drained).
fn resting() -> (World, PhysicsWorld, InstanceId, InstanceId) {
    let mut world = World::new_with_services();
    let floor = part(
        &mut world,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(64.0, 4.0, 64.0),
        true,
    );
    let p = part(
        &mut world,
        Vec3::new(0.0, 3.0, 0.0),
        Vec3::new(4.0, 1.0, 2.0),
        false,
    );
    let mut phys = PhysicsWorld::new();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    let landed = phys.drain_touched();
    assert_eq!(landed.len(), 1, "{landed:?}");
    (world, phys, floor, p)
}

fn assert_rests(world: &World, p: InstanceId, y: f64) {
    let at = pos(world, p).y;
    assert!((at - y).abs() < 0.05, "part at y = {at}, expected {y}");
}

/// Contacts still work: lift the part clear, drop it, and it lands on the floor (one touch).
fn assert_contacts_work(
    world: &mut World,
    phys: &mut PhysicsWorld,
    floor: InstanceId,
    p: InstanceId,
    y: f64,
) {
    phys.drain_touched();
    let at = pos(world, p);
    world
        .set(
            p,
            "Position",
            Value::Vector3(Vec3::new(at.x, y + 4.0, at.z)),
        )
        .unwrap();
    phys.step(world, 1.0 / 60.0);
    assert!(phys.drain_touched().is_empty());
    steps(phys, world, 120);
    assert_rests(world, p, y);
    let touched = phys.drain_touched();
    assert_eq!(touched.len(), 1, "{touched:?}");
    assert!(touched[0] == (p, floor) || touched[0] == (floor, p));
}

#[test]
fn resize_touching_part_does_not_panic() {
    let (mut world, mut phys, floor, p) = resting();
    world
        .set(p, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 3.0)))
        .unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    assert_contacts_work(&mut world, &mut phys, floor, p, REST_Y);
}

#[test]
fn material_change_on_touching_part_does_not_panic() {
    let (mut world, mut phys, floor, p) = resting();
    world
        .set(p, "Material", Value::String("Metal".into()))
        .unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    assert_contacts_work(&mut world, &mut phys, floor, p, REST_Y);
}

#[test]
fn can_collide_toggle_on_touching_part_does_not_panic() {
    let (mut world, mut phys, floor, p) = resting();
    // Off: the part becomes a sensor and sinks through the floor.
    world.set(p, "CanCollide", Value::Bool(false)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert!(pos(&world, p).y < REST_Y - 1.0, "sensor did not sink");
    // On again, placed back on the floor: it rests and touches like before.
    world
        .set(p, "Position", Value::Vector3(Vec3::new(0.0, REST_Y, 0.0)))
        .unwrap();
    world.set(p, "CanCollide", Value::Bool(true)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    assert_contacts_work(&mut world, &mut phys, floor, p, REST_Y);
}

#[test]
fn anchored_toggle_on_touching_part_does_not_panic() {
    let (mut world, mut phys, floor, p) = resting();
    world.set(p, "Anchored", Value::Bool(true)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    world.set(p, "Anchored", Value::Bool(false)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    assert_contacts_work(&mut world, &mut phys, floor, p, REST_Y);
}

#[test]
fn reparent_out_and_back_does_not_panic() {
    let (mut world, mut phys, floor, p) = resting();
    let storage = world.root("ReplicatedStorage").unwrap();
    world.set_parent(p, Some(storage)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert!(phys.velocity(p).is_none());
    let ws = world.root("Workspace").unwrap();
    world.set_parent(p, Some(ws)).unwrap();
    phys.sync_from(&world);
    steps(&mut phys, &mut world, 60);
    assert_rests(&world, p, REST_Y);
    assert_contacts_work(&mut world, &mut phys, floor, p, REST_Y);
}
