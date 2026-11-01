use metallic_core::{CoreError, Value, Vec3, World};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn temporary_known_and_unknown_names_retain_exact_registration_identity() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut temporary = String::from("Position");
    world.property_changed(part, &temporary).connect(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
    });
    temporary.clear();
    assert_eq!(world.property_changed(part, "Position").len(), 1);
    let mut unknown = String::from("Unknown\u{1f514}");
    world
        .property_changed(part, &unknown)
        .connect(|_| panic!("unknown property fired"));
    unknown.clear();
    assert_eq!(world.property_changed(part, "Unknown\u{1f514}").len(), 1);
    world
        .property_changed(part, "position")
        .connect(|_| panic!("case-folded property fired"));
    let cursor = world.change_cursor();
    assert!(matches!(
        world.set(part, "Unknown\u{1f514}", Value::Nil),
        Err(CoreError::UnknownProperty { .. })
    ));
    assert_eq!(world.change_cursor(), cursor);
    world
        .set(part, "Position", Value::Vector3(Vec3::new(1.0, 2.0, 3.0)))
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(world.property_changed(part, "position").len(), 1);
}

#[test]
fn unresolved_registration_can_match_a_later_live_world_local_id() {
    let mut template = World::new_empty();
    let future = template.create("Part", None).unwrap();
    let mut world = World::new_empty();
    assert!(!world.is_alive(future));
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    world
        .property_changed(future, &String::from("Position"))
        .connect(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
        });
    let actual = world.create("Part", None).unwrap();
    assert_eq!(actual, future);
    world
        .set(actual, "Position", Value::Vector3(Vec3::new(1.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(world.property_changed(actual, "Position").len(), 1);

    let mut cloned = world.deep_clone();
    assert_eq!(cloned.property_changed(actual, "Position").len(), 0);
    cloned
        .set(actual, "Position", Value::Vector3(Vec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "deep_clone must not copy subscriptions"
    );
}

struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn destroy_cleans_dead_and_unresolved_signals_but_retains_live_unknown_names() {
    let mut template = World::new_empty();
    template.create("Part", None).unwrap();
    template.create("Part", None).unwrap();
    let future = template.create("Part", None).unwrap();
    let mut world = World::new_empty();
    let destroyed = world.create("Part", None).unwrap();
    let survivor = world.create("Part", None).unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    for (id, name) in [
        (destroyed, "Position"),
        (destroyed, "Unknown"),
        (future, "Position"),
        (survivor, "Unknown"),
    ] {
        let guard = Dropped(dropped.clone());
        world.property_changed(id, name).connect(move |_| {
            let _ = &guard;
        });
    }
    world.destroy(destroyed);
    assert_eq!(dropped.load(Ordering::Relaxed), 3);
    assert_eq!(world.property_changed(survivor, "Unknown").len(), 1);
    let reused = world.create("Part", None).unwrap();
    assert_ne!(reused, destroyed);
    let eventual = world.create("Part", None).unwrap();
    assert_eq!(eventual, future);
    assert_eq!(world.property_changed(eventual, "Position").len(), 0);
    world
        .property_changed(destroyed, "Position")
        .connect(|_| panic!("stale generation fired"));
    world
        .set(reused, "Position", Value::Vector3(Vec3::new(4.0, 0.0, 0.0)))
        .unwrap();
    world
        .set(
            eventual,
            "Position",
            Value::Vector3(Vec3::new(5.0, 0.0, 0.0)),
        )
        .unwrap();
    assert_eq!(world.property_changed(destroyed, "Position").len(), 1);
    world.destroy(eventual);
    assert_eq!(world.property_changed(destroyed, "Position").len(), 0);
}
