use super::*;
use metallic_core::{Color3, Vec3};

fn part(world: &mut World, parent: InstanceId, x: f64) -> InstanceId {
    let id = world.create("Part", Some(parent)).unwrap();
    world.set(id, "Anchored", Value::Bool(true)).unwrap();
    world
        .set(id, "Position", Value::Vector3(Vec3::new(x, 100.0, 0.0)))
        .unwrap();
    id
}

fn fixture() -> (World, InstanceId, InstanceId, InstanceId) {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(workspace)).unwrap();
    let body = part(&mut world, model, 0.0);
    (world, workspace, model, body)
}

#[test]
fn ordinary_humanoid_properties_and_pose_ticks_do_not_resync_a_large_scene() {
    let (mut world, workspace, model, body) = fixture();
    let humanoid = world.create("Humanoid", Some(model)).unwrap();
    for index in 0..1_000 {
        part(&mut world, workspace, 20.0 + index as f64 * 8.0);
    }
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work.part_reads, 1_001);
    assert_eq!(cache.work.full_scans, 1);

    for (name, value) in [
        ("Health", Value::Number(80.0)),
        ("WalkSpeed", Value::Number(24.0)),
        ("JumpPower", Value::Number(55.0)),
        ("Name", Value::String("renamed Humanoid".to_owned())),
    ] {
        world.set(humanoid, name, value).unwrap();
    }
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());

    world
        .set(body, "Position", Value::Vector3(Vec3::new(1.0, 100.0, 0.0)))
        .unwrap();
    world
        .set(
            body,
            "Orientation",
            Value::Vector3(Vec3::new(0.0, 20.0, 0.0)),
        )
        .unwrap();
    world
        .set(body, "Color", Value::Color3(Color3::new(20.0, 30.0, 40.0)))
        .unwrap();
    let ui = world.create("Frame", world.root("StarterGui")).unwrap();
    world
        .set(ui, "Name", Value::String("unrelated UI".to_owned()))
        .unwrap();
    let script = world
        .create("Script", world.root("ServerScriptService"))
        .unwrap();
    world
        .set(
            script,
            "Source",
            Value::String("print('not running in this cache test')".to_owned()),
        )
        .unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(
        cache.work,
        SyncWork {
            part_reads: 1,
            ..SyncWork::default()
        }
    );
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());
}

#[test]
fn membership_transitions_only_inspect_the_affected_model_and_sync_once() {
    let (mut world, workspace, model, first) = fixture();
    let second = part(&mut world, model, 8.0);
    for index in 0..500 {
        part(&mut world, workspace, 30.0 + index as f64 * 8.0);
    }
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);
    let humanoid = world.create("Humanoid", Some(model)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&first].character && cache.parts[&second].character);
    assert_eq!(cache.work.full_scans, 0);
    assert_eq!(cache.work.full_instances, 0);
    assert_eq!(cache.work.part_reads, 2);
    assert_eq!(cache.work.physics_syncs, 1);
    assert_eq!(cache.work.candidate_unions, 1);

    world.destroy(humanoid);
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts[&first].character && !cache.parts[&second].character);
    assert!(!cache.humanoids.contains_key(&humanoid));
    assert!(!cache.model_humanoids.contains_key(&model));
    assert_eq!(cache.work.part_reads, 2);
    assert_eq!(cache.work.physics_syncs, 1);
    assert_eq!(cache.work.full_scans, 0);

    for (id, name, value) in [
        (first, "Size", Value::Vector3(Vec3::new(3.0, 4.0, 5.0))),
        (first, "Shape", Value::String("Ball".to_owned())),
        (first, "Material", Value::String("Metal".to_owned())),
        (second, "Anchored", Value::Bool(false)),
        (second, "CanCollide", Value::Bool(false)),
    ] {
        world.set(id, name, value).unwrap();
    }
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work.part_reads, 2);
    assert_eq!(cache.work.physics_syncs, 1);
    assert_eq!(cache.work.full_scans, 0);
}

#[test]
fn non_last_humanoids_and_net_replacement_do_not_walk_descendants() {
    let (mut world, _, model, body) = fixture();
    let first = world.create("Humanoid", Some(model)).unwrap();
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);

    let second = world.create("Humanoid", Some(model)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());
    world.destroy(first);
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());
    world.destroy(second);
    let replacement = world.create("Humanoid", Some(model)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());
    assert_eq!(cache.model_humanoids[&model], 1);
    assert_eq!(cache.humanoids.get(&replacement), Some(&model));
    assert!(!cache.humanoids.contains_key(&first) && !cache.humanoids.contains_key(&second));
    assert!(cache.parts[&body].character);
}

#[test]
fn humanoid_reparenting_updates_both_models_and_workspace_exit() {
    let (mut world, workspace, first, first_body) = fixture();
    let second = world.create("Model", Some(workspace)).unwrap();
    let second_body = part(&mut world, second, 20.0);
    let humanoid = world.create("Humanoid", Some(first)).unwrap();
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);

    world.set_parent(humanoid, Some(second)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts[&first_body].character && cache.parts[&second_body].character);
    assert_eq!(cache.work.part_reads, 2);
    assert_eq!(cache.work.physics_syncs, 1);
    world
        .set_parent(humanoid, world.root("ReplicatedStorage"))
        .unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts[&second_body].character);
    assert!(!cache.humanoids.contains_key(&humanoid));
    world.set_parent(humanoid, Some(first)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&first_body].character && !cache.parts[&second_body].character);
    assert_eq!(cache.work.full_scans, 0);
}

#[test]
fn nested_model_folder_and_part_moves_track_inherited_character_membership() {
    let (mut world, workspace, outer, outer_body) = fixture();
    world.create("Humanoid", Some(outer)).unwrap();
    let inner = world.create("Model", Some(outer)).unwrap();
    let inner_body = part(&mut world, inner, 20.0);
    let inner_humanoid = world.create("Humanoid", Some(inner)).unwrap();
    let ordinary = world.create("Model", Some(workspace)).unwrap();
    let folder = world.create("Folder", Some(ordinary)).unwrap();
    let folder_body = part(&mut world, folder, 40.0);
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);

    world.destroy(inner_humanoid);
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&inner_body].character);
    assert_eq!(cache.work.physics_syncs, 0);
    world.set_parent(inner, Some(ordinary)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts[&inner_body].character && cache.parts[&outer_body].character);
    assert_eq!(cache.work.physics_syncs, 1);
    world.set_parent(folder, Some(outer)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&folder_body].character);
    world.set_parent(inner_body, Some(outer)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&inner_body].character);
    assert_eq!(cache.work.full_scans, 0);
}

#[test]
fn deleted_generation_ids_transients_and_workspace_moves_leave_no_stale_membership() {
    let (mut world, workspace, model, body) = fixture();
    let humanoid = world.create("Humanoid", Some(model)).unwrap();
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, false);
    world.destroy(humanoid);
    let replacement = world.create("Folder", None).unwrap();
    assert_ne!(replacement, humanoid);
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts[&body].character);
    assert!(!cache.humanoids.contains_key(&humanoid));

    let transient = world.create("Humanoid", Some(model)).unwrap();
    world.destroy(transient);
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work, SyncWork::default());
    let live_humanoid = world.create("Humanoid", Some(model)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&body].character);
    world
        .set_parent(model, world.root("ReplicatedStorage"))
        .unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(!cache.parts.contains_key(&body));
    assert!(!cache.humanoids.contains_key(&live_humanoid));
    world.set_parent(model, Some(workspace)).unwrap();
    cache.synchronize(&mut physics, &world, false);
    assert!(cache.parts[&body].character);
    assert_eq!(cache.humanoids.get(&live_humanoid), Some(&model));
    assert_eq!(cache.work.full_scans, 0);
    world.destroy(model);
    cache.synchronize(&mut physics, &world, false);
    assert!(
        cache.parts.is_empty() && cache.humanoids.is_empty() && cache.model_humanoids.is_empty()
    );
}

#[test]
fn forced_sync_overflow_and_foreign_world_refresh_only_reconcile_when_needed() {
    let (mut world, _, model, body) = fixture();
    let mut cache = PhysicsSyncCache::default();
    let mut physics = PhysicsWorld::new();
    cache.synchronize(&mut physics, &world, true);
    assert_eq!(cache.work.full_scans, 1);
    cache.synchronize(&mut physics, &world, true);
    assert_eq!(
        cache.work,
        SyncWork {
            physics_syncs: 1,
            ..SyncWork::default()
        }
    );
    let new_body = part(&mut world, model, 30.0);
    cache.synchronize(&mut physics, &world, true);
    assert_eq!(
        cache.work,
        SyncWork {
            part_reads: 1,
            physics_syncs: 1,
            ..SyncWork::default()
        }
    );
    assert_eq!(
        physics
            .raycast(Vec3::new(30.0, 110.0, 0.0), Vec3::new(0.0, -20.0, 0.0), &[])
            .unwrap()
            .instance,
        new_body
    );

    let cursor = cache.cursor.unwrap();
    for index in 0..40_000 {
        world
            .set(body, "Name", Value::String(index.to_string()))
            .unwrap();
    }
    assert_eq!(world.changes_since(cursor).0, Changes::Overflow);
    cache.synchronize(&mut physics, &world, false);
    assert_eq!(cache.work.full_scans, 1);
    assert_eq!(cache.work.part_reads, 2);
    let mut cloned = world.deep_clone();
    cloned.destroy(model);
    cache.synchronize(&mut physics, &cloned, false);
    assert_eq!(cache.work.full_scans, 1);
    assert!(cache.parts.is_empty() && cache.humanoids.is_empty());
    // Physics-world replacement reset is outside this runtime cache's contract (#174).
    cache.synchronize(&mut physics, &cloned, false);
    assert_eq!(cache.work, SyncWork::default());
}
