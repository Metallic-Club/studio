use super::*;
use metallic_core::Color3;

fn part(world: &mut World, parent: InstanceId, x: f64, anchored: bool) -> InstanceId {
    let id = world.create("Part", Some(parent)).unwrap();
    world
        .set(id, "Position", Value::Vector3(Vec3::new(x, 100.0, 0.0)))
        .unwrap();
    world.set(id, "Anchored", Value::Bool(anchored)).unwrap();
    id
}

#[test]
fn static_and_sparse_sync_examine_only_journaled_parts() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let ids: Vec<_> = (0..5_000)
        .map(|n| part(&mut world, workspace, f64::from(n) * 10.0, true))
        .collect();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    assert!(physics.last_sync_stats().full_scan);
    assert_eq!(physics.last_sync_stats().examined_parts, 5_000);
    let untouched = physics.entries[&ids[0]].body;
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats(), PhysicsSyncStats::default());
    world
        .set(ids[100], "Size", Value::Vector3(Vec3::new(8.0, 2.0, 2.0)))
        .unwrap();
    world
        .set(ids[100], "Material", Value::String("Metal".into()))
        .unwrap();
    physics.sync_from(&world);
    assert_eq!(
        physics.last_sync_stats(),
        PhysicsSyncStats {
            full_scan: false,
            examined_parts: 1,
            rebuilt_parts: 1,
            removed_parts: 0,
        }
    );
    assert_eq!(physics.entries[&ids[0]].body, untouched);
    assert_eq!(physics.entries.len(), 5_000);
}

#[test]
fn unrelated_properties_keep_dynamic_momentum_and_body_handles() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let id = part(&mut world, workspace, 0.0, false);
    let folder = world.create("Folder", Some(workspace)).unwrap();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    let body = physics.entries[&id].body;
    let velocity = physics.velocity(id);
    world
        .set(id, "Name", Value::String("Renamed".into()))
        .unwrap();
    world
        .set(id, "Color", Value::Color3(Color3::new(10.0, 20.0, 30.0)))
        .unwrap();
    world
        .set(folder, "Name", Value::String("Organization".into()))
        .unwrap();
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().examined_parts, 1);
    assert_eq!(physics.last_sync_stats().rebuilt_parts, 0);
    assert_eq!(physics.entries[&id].body, body);
    assert_eq!(physics.velocity(id), velocity);
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats(), PhysicsSyncStats::default());
}

#[test]
fn incremental_creation_destruction_and_slot_reuse_update_queries() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    let first = part(&mut world, workspace, 0.0, true);
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().rebuilt_parts, 1);
    assert_eq!(
        physics
            .raycast(Vec3::new(0.0, 110.0, 0.0), Vec3::new(0.0, -20.0, 0.0), &[])
            .unwrap()
            .instance,
        first
    );
    world.destroy(first);
    let second = part(&mut world, workspace, 10.0, true);
    assert_ne!(first, second);
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().removed_parts, 1);
    assert_eq!(physics.last_sync_stats().rebuilt_parts, 1);
    assert!(physics.velocity(first).is_none());
    assert_eq!(physics.entries.len(), 1);
    assert!(
        physics
            .raycast(Vec3::new(0.0, 110.0, 0.0), Vec3::new(0.0, -20.0, 0.0), &[])
            .is_none()
    );
    assert_eq!(
        physics
            .raycast(Vec3::new(10.0, 110.0, 0.0), Vec3::new(0.0, -20.0, 0.0), &[])
            .unwrap()
            .instance,
        second
    );
}

#[test]
fn reparenting_and_destroying_subtrees_reconcile_all_descendants() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let storage = world.root("ReplicatedStorage").unwrap();
    let folder = world.create("Folder", Some(workspace)).unwrap();
    let nested = world.create("Model", Some(folder)).unwrap();
    let first = part(&mut world, folder, 0.0, true);
    let second = part(&mut world, nested, 10.0, true);
    let unrelated = part(&mut world, workspace, 20.0, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    let stable = physics.entries[&unrelated].body;
    world.set_parent(folder, Some(storage)).unwrap();
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().removed_parts, 2);
    assert!(!physics.entries.contains_key(&first));
    assert!(!physics.entries.contains_key(&second));
    world.set_parent(folder, Some(workspace)).unwrap();
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().rebuilt_parts, 2);
    world.destroy(folder);
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats().removed_parts, 2);
    assert_eq!(physics.entries.len(), 1);
    assert_eq!(physics.entries[&unrelated].body, stable);
}

#[test]
fn humanoid_creation_and_destruction_reclassify_unchanged_sibling_parts() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(workspace)).unwrap();
    let nested = world.create("Folder", Some(model)).unwrap();
    let limb = part(&mut world, nested, 0.0, true);
    let unrelated = part(&mut world, workspace, 10.0, true);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    let stable = physics.entries[&unrelated].body;
    let humanoid = world.create("Humanoid", Some(model)).unwrap();
    physics.sync_from(&world);
    assert!(physics.characters.contains(&limb));
    assert_eq!(physics.last_sync_stats().examined_parts, 1);
    let character_body = physics.entries[&limb].body;
    let another = world.create("Humanoid", Some(model)).unwrap();
    world.destroy(humanoid);
    physics.sync_from(&world);
    assert!(physics.characters.contains(&limb));
    assert_eq!(physics.entries[&limb].body, character_body);
    world.destroy(another);
    physics.sync_from(&world);
    assert!(!physics.characters.contains(&limb));
    assert_eq!(physics.last_sync_stats().rebuilt_parts, 1);
    assert_eq!(physics.entries[&unrelated].body, stable);
}

#[test]
fn moving_a_humanoid_updates_both_models_and_preserves_outer_character_membership() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let first_model = world.create("Model", Some(workspace)).unwrap();
    let second_model = world.create("Model", Some(workspace)).unwrap();
    let first = part(&mut world, first_model, 0.0, true);
    let second = part(&mut world, second_model, 10.0, true);
    let humanoid = world.create("Humanoid", Some(first_model)).unwrap();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    assert!(physics.characters.contains(&first));
    world.set_parent(humanoid, Some(second_model)).unwrap();
    physics.sync_from(&world);
    assert!(!physics.characters.contains(&first));
    assert!(physics.characters.contains(&second));
    world.set_parent(first_model, Some(second_model)).unwrap();
    physics.sync_from(&world);
    assert!(physics.characters.contains(&first));
    let inner_humanoid = world.create("Humanoid", Some(first_model)).unwrap();
    physics.sync_from(&world);
    let stable = physics.entries[&first].body;
    world.destroy(inner_humanoid);
    physics.sync_from(&world);
    assert!(physics.characters.contains(&first));
    assert_eq!(physics.entries[&first].body, stable);
}

#[test]
fn overflow_reconciles_missed_lifecycle_changes_without_resetting_momentum() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let falling = part(&mut world, workspace, 0.0, false);
    let removed = part(&mut world, workspace, 10.0, true);
    let noise = world.create("Folder", Some(workspace)).unwrap();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    let body = physics.entries[&falling].body;
    let velocity = physics.velocity(falling);
    world.destroy(removed);
    let created = part(&mut world, workspace, 20.0, true);
    for index in 0..20_000 {
        world
            .set(noise, "Name", Value::String(index.to_string()))
            .unwrap();
    }
    physics.sync_from(&world);
    assert!(physics.last_sync_stats().full_scan);
    assert!(physics.velocity(removed).is_none());
    assert!(physics.velocity(created).is_some());
    assert_eq!(physics.entries[&falling].body, body);
    assert_eq!(physics.velocity(falling), velocity);
    physics.sync_from(&world);
    assert_eq!(physics.last_sync_stats(), PhysicsSyncStats::default());
}

#[test]
fn deep_clone_world_switch_discards_old_motion_contacts_and_accumulated_time() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let falling = part(&mut world, workspace, 0.0, false);
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    physics.step(&mut world, 1.0 / 60.0);
    physics.step(&mut world, SUBSTEP / 2.0);
    assert!(physics.velocity(falling).unwrap().y < 0.0);
    assert!(physics.accumulator > 0.0);
    let mut cloned = world.deep_clone();
    cloned
        .set(
            falling,
            "Position",
            Value::Vector3(Vec3::new(50.0, 100.0, 0.0)),
        )
        .unwrap();
    physics.sync_from(&cloned);
    assert!(physics.last_sync_stats().full_scan);
    assert_eq!(physics.velocity(falling), Some(Vec3::default()));
    assert_eq!(physics.accumulator, 0.0);
    assert!(physics.drain_touched().is_empty());
    assert_eq!(
        physics
            .raycast(Vec3::new(50.0, 110.0, 0.0), Vec3::new(0.0, -20.0, 0.0), &[])
            .unwrap()
            .instance,
        falling
    );
}

#[test]
fn replacing_a_world_discards_queued_contacts_from_old_instance_ids() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    part(&mut world, workspace, 0.0, false);
    let sensor = part(&mut world, workspace, 0.0, true);
    world.set(sensor, "CanCollide", Value::Bool(false)).unwrap();
    let mut physics = PhysicsWorld::new();
    physics.sync_from(&world);
    let mut cloned = world.deep_clone();
    cloned.destroy(sensor);
    physics.sync_from(&cloned);
    assert!(physics.drain_touched().is_empty());
    assert!(physics.velocity(sensor).is_none());
}

#[test]
fn separate_physics_readers_do_not_consume_each_others_changes() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let id = part(&mut world, workspace, 0.0, true);
    let mut first = PhysicsWorld::new();
    let mut second = PhysicsWorld::new();
    first.sync_from(&world);
    second.sync_from(&world);
    world.set(id, "CanCollide", Value::Bool(false)).unwrap();
    first.sync_from(&world);
    second.sync_from(&world);
    assert_eq!(first.last_sync_stats().examined_parts, 1);
    assert_eq!(second.last_sync_stats().examined_parts, 1);
    assert!(!first.entries[&id].signature.can_collide);
    assert!(!second.entries[&id].signature.can_collide);
}
