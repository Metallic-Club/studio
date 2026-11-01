use metallic_core::classes::{default_value, default_value_ref};
use metallic_core::{CoreError, Value, Vec3, World};

#[test]
fn inherited_and_class_specific_defaults_are_stable_across_worlds() {
    let mut first = World::new_empty();
    let mut second = World::new_empty();
    let part = first.create("Part", None).unwrap();
    let another = second.create("Part", None).unwrap();
    let checkpoint = first.create("Checkpoint", None).unwrap();
    let script = first.create("LocalScript", None).unwrap();

    for (id, property, expected) in [
        (part, "Material", Value::String("Plastic".into())),
        (part, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 2.0))),
        (checkpoint, "Name", Value::String("Checkpoint".into())),
        (script, "Name", Value::String("LocalScript".into())),
        (script, "Source", Value::String(String::new())),
    ] {
        let borrowed = first.get_ref(id, property).unwrap();
        assert_eq!(borrowed, &expected);
        assert!(std::ptr::eq(borrowed, first.get_ref(id, property).unwrap()));
        assert_eq!(first.get(id, property).unwrap(), *borrowed);
    }
    assert!(std::ptr::eq(
        first.get_ref(part, "Material").unwrap(),
        second.get_ref(another, "Material").unwrap()
    ));
}

#[test]
fn borrowed_reads_follow_changes_and_default_resets_without_affecting_owned_values() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let default = default_value_ref("Part", "Name").unwrap();
    assert!(std::ptr::eq(world.get_ref(part, "Name").unwrap(), default));
    world
        .set(part, "Name", Value::String("Longer stored name".into()))
        .unwrap();
    let owned = world.get(part, "Name").unwrap();
    let stored = world.get_ref(part, "Name").unwrap();
    assert_eq!(stored, &owned);
    assert!(std::ptr::eq(stored, world.get_ref(part, "Name").unwrap()));

    world
        .set(part, "Name", Value::String("Part".into()))
        .unwrap();
    assert!(std::ptr::eq(world.get_ref(part, "Name").unwrap(), default));
    assert_eq!(owned, Value::String("Longer stored name".into()));
}

#[test]
fn borrowed_reference_reads_filter_destroyed_targets_and_reused_slots() {
    let mut world = World::new_empty();
    let player = world.create("Player", None).unwrap();
    let spawn = world.create("SpawnLocation", None).unwrap();
    world
        .set(player, "RespawnLocation", Value::Ref(Some(spawn)))
        .unwrap();
    assert_eq!(
        world.get_ref(player, "RespawnLocation").unwrap(),
        &Value::Ref(Some(spawn))
    );
    world.destroy(spawn);
    let replacement = world.create("SpawnLocation", None).unwrap();
    assert_ne!(spawn, replacement);
    assert_eq!(
        world.get_ref(player, "RespawnLocation").unwrap(),
        &Value::Ref(None)
    );
    assert_eq!(
        world.get(player, "RespawnLocation").unwrap(),
        Value::Ref(None)
    );
}

#[test]
fn borrowed_reads_preserve_error_variants() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    assert_eq!(
        world.get_ref(part, "Missing").unwrap_err(),
        world.get(part, "Missing").unwrap_err()
    );
    assert!(
        matches!(world.get_ref(part, "Missing"), Err(CoreError::UnknownProperty { class, property })
        if class == "Part" && property == "Missing")
    );
    world.destroy(part);
    assert_eq!(world.get_ref(part, "Name"), Err(CoreError::InvalidInstance));
}

#[test]
fn registry_borrowed_defaults_match_owned_defaults_and_can_outlive_a_world() {
    let name: &'static Value = default_value_ref("Checkpoint", "Name").unwrap();
    assert_eq!(name, &Value::String("Checkpoint".into()));
    for (class, property) in [
        ("Part", "Shape"),
        ("TextLabel", "Size"),
        ("Humanoid", "Health"),
    ] {
        let borrowed = default_value_ref(class, property).unwrap();
        assert_eq!(Some(borrowed.clone()), default_value(class, property));
        assert!(std::ptr::eq(
            borrowed,
            default_value_ref(class, property).unwrap()
        ));
    }
    assert_eq!(default_value_ref("Unknown", "Name"), None);
    assert_eq!(default_value_ref("Part", "Missing"), None);
}
