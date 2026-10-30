//! Property validation must reject non-finite state without observable mutation.
use metallic_core::{Changes, Color3, CoreError, InstanceId, UDim2, Value, Vec3, World};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn observe(world: &mut World, id: InstanceId, property: &str) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let fired = Arc::clone(&count);
    world.property_changed(id, property).connect(move |_| {
        fired.fetch_add(1, Ordering::Relaxed);
    });
    count
}

fn reject_without_change(
    world: &mut World,
    id: InstanceId,
    property: &str,
    value: Value,
    fired: &AtomicUsize,
) {
    let old = world.get(id, property).unwrap();
    let before = world.change_cursor();
    let count = fired.load(Ordering::Relaxed);
    assert!(
        matches!(world.set(id, property, value), Err(CoreError::InvalidValue { property: name, .. }) if name == property)
    );
    assert_eq!(world.get(id, property).unwrap(), old);
    assert_eq!(world.change_cursor(), before);
    assert_eq!(world.changes_since(before).0, Changes::Ids(Vec::new()));
    assert_eq!(fired.load(Ordering::Relaxed), count);
}

fn world_and_part() -> (World, InstanceId) {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let part = world.create("Part", Some(workspace)).unwrap();
    (world, part)
}

#[test]
fn number_rejects_nan_and_both_infinities_without_side_effects() {
    let (mut world, part) = world_and_part();
    world
        .set(part, "Transparency", Value::Number(0.25))
        .unwrap();
    let fired = observe(&mut world, part, "Transparency");
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        reject_without_change(&mut world, part, "Transparency", Value::Number(bad), &fired);
    }
}

#[test]
fn every_vector_component_rejects_nan_and_both_infinities() {
    let (mut world, part) = world_and_part();
    world
        .set(part, "Position", Value::Vector3(Vec3::new(1.0, 2.0, 3.0)))
        .unwrap();
    let fired = observe(&mut world, part, "Position");
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for vector in [
            Vec3::new(bad, 2.0, 3.0),
            Vec3::new(1.0, bad, 3.0),
            Vec3::new(1.0, 2.0, bad),
        ] {
            reject_without_change(&mut world, part, "Position", Value::Vector3(vector), &fired);
        }
    }
}

#[test]
fn every_color_component_rejects_nan_and_both_infinities() {
    let (mut world, part) = world_and_part();
    world
        .set(part, "Color", Value::Color3(Color3::new(20.0, 40.0, 60.0)))
        .unwrap();
    let fired = observe(&mut world, part, "Color");
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for color in [
            Color3::new(bad, 40.0, 60.0),
            Color3::new(20.0, bad, 60.0),
            Color3::new(20.0, 40.0, bad),
        ] {
            reject_without_change(&mut world, part, "Color", Value::Color3(color), &fired);
        }
    }
}

#[test]
fn every_udim2_component_rejects_nan_and_both_infinities() {
    let (mut world, part) = world_and_part();
    let frame = world.create("Frame", Some(part)).unwrap();
    world
        .set(
            frame,
            "Position",
            Value::UDim2(UDim2::new(0.25, 4.0, 0.5, 8.0)),
        )
        .unwrap();
    let fired = observe(&mut world, frame, "Position");
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for value in [
            UDim2::new(bad, 4.0, 0.5, 8.0),
            UDim2::new(0.25, bad, 0.5, 8.0),
            UDim2::new(0.25, 4.0, bad, 8.0),
            UDim2::new(0.25, 4.0, 0.5, bad),
        ] {
            reject_without_change(&mut world, frame, "Position", Value::UDim2(value), &fired);
        }
    }
}

#[test]
fn repeated_nan_writes_do_not_fire_or_dirty_the_journal() {
    let (mut world, part) = world_and_part();
    let old = world.get(part, "Transparency").unwrap();
    let fired = observe(&mut world, part, "Transparency");
    let before = world.change_cursor();
    for _ in 0..3 {
        assert!(
            world
                .set(part, "Transparency", Value::Number(f64::NAN))
                .is_err()
        );
    }
    assert_eq!(world.get(part, "Transparency").unwrap(), old);
    assert_eq!(fired.load(Ordering::Relaxed), 0);
    assert_eq!(world.change_cursor(), before);
    assert_eq!(world.changes_since(before).0, Changes::Ids(Vec::new()));
}

#[test]
fn invalid_instance_unknown_property_and_type_mismatch_keep_precedence() {
    let (mut world, part) = world_and_part();
    let before = world.change_cursor();
    assert!(
        matches!(world.set(part, "NoSuchProperty", Value::Number(f64::NAN)),
        Err(CoreError::UnknownProperty { property, .. }) if property == "NoSuchProperty")
    );
    assert!(
        matches!(world.set(part, "Position", Value::Number(f64::INFINITY)),
        Err(CoreError::TypeMismatch { property, expected, found })
        if property == "Position" && expected == "Vector3" && found == "Number")
    );
    assert_eq!(world.change_cursor(), before);
    world.destroy(part);
    let after_destroy = world.change_cursor();
    assert_eq!(
        world.set(part, "NoSuchProperty", Value::Number(f64::NAN)),
        Err(CoreError::InvalidInstance)
    );
    assert_eq!(world.change_cursor(), after_destroy);
}

#[test]
fn finite_extremes_remain_accepted_without_new_range_clamps() {
    let (mut world, part) = world_and_part();
    let workspace = world.root("Workspace").unwrap();
    let frame = world.create("Frame", Some(workspace)).unwrap();
    for number in [
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        0.0,
        -0.0,
    ] {
        for (id, property, value) in [
            (workspace, "FallenPartsDestroyHeight", Value::Number(number)),
            (
                part,
                "Position",
                Value::Vector3(Vec3::new(number, -number, number)),
            ),
            (
                part,
                "Color",
                Value::Color3(Color3::new(number, -number, number)),
            ),
            (
                frame,
                "Position",
                Value::UDim2(UDim2::new(number, -number, number, -number)),
            ),
        ] {
            world.set(id, property, value.clone()).unwrap();
            assert_eq!(world.get(id, property).unwrap(), value);
        }
    }
}

#[test]
fn nonnumeric_values_and_existing_material_constraints_are_unchanged() {
    let (mut world, part) = world_and_part();
    world
        .set(
            part,
            "Name",
            Value::String("NaN and infinity are text here".into()),
        )
        .unwrap();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    world
        .set(part, "Material", Value::String("Metal".into()))
        .unwrap();
    let player = world.create("Player", world.root("Players")).unwrap();
    world
        .set(player, "RespawnLocation", Value::Ref(Some(part)))
        .unwrap();
    assert_eq!(
        world.get(player, "RespawnLocation").unwrap(),
        Value::Ref(Some(part))
    );
    let fired = observe(&mut world, part, "Material");
    reject_without_change(
        &mut world,
        part,
        "Material",
        Value::String("Unobtainium".into()),
        &fired,
    );
}
