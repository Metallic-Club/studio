use metallic_core::{ConnectionId, Signal, Value, World};
use std::collections::HashSet;
use std::sync::{Arc, Barrier, Mutex};

#[test]
fn a_connection_from_another_same_type_signal_cannot_disconnect_a_handler() {
    let mut first = Signal::<()>::default();
    let mut second = Signal::<()>::default();
    let calls = Arc::new(Mutex::new(0));
    let first_id = first.connect(|_| {});
    let observed = Arc::clone(&calls);
    let second_id = second.connect(move |_| *observed.lock().unwrap() += 1);

    second.disconnect(first_id);
    second.fire(&());
    assert_eq!(*calls.lock().unwrap(), 1);
    second.disconnect(second_id);
    second.fire(&());
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(second.is_empty());
}

#[test]
fn property_connection_cannot_disconnect_the_worlds_child_added_handler() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace");
    let part = world.create("Part", workspace).unwrap();
    let added = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&added);
    world
        .child_added
        .connect(move |&(_, child)| observed.lock().unwrap().push(child));
    let changed = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&changed);
    let property_id = world
        .property_changed(part, "Anchored")
        .connect(move |value| observed.lock().unwrap().push(value.clone()));

    // The signal argument types differ: (InstanceId, InstanceId) versus Value.
    world.child_added.disconnect(property_id);
    let child = world.create("Part", workspace).unwrap();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    assert_eq!(*added.lock().unwrap(), vec![child]);
    assert_eq!(*changed.lock().unwrap(), vec![Value::Bool(true)]);
}

#[test]
fn a_dropped_signals_id_cannot_disconnect_a_recreated_signals_handler() {
    let stale = {
        let mut old = Signal::<usize>::default();
        old.connect(|_| {})
    };
    let mut replacement = Signal::<usize>::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&seen);
    replacement.connect(move |&value| observed.lock().unwrap().push(value));
    replacement.disconnect(stale);
    replacement.fire(&42);
    assert_eq!(*seen.lock().unwrap(), vec![42]);
}

fn concurrent_ids<Args>(start: &Barrier) -> Vec<ConnectionId> {
    let mut signal = Signal::<Args>::default();
    start.wait();
    (0..128).map(|_| signal.connect(|_| {})).collect()
}

#[test]
fn concurrent_connections_on_independent_signals_and_argument_types_have_unique_ids() {
    let start = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                if index % 2 == 0 {
                    concurrent_ids::<u32>(&start)
                } else {
                    concurrent_ids::<String>(&start)
                }
            })
        })
        .collect();
    let ids: Vec<_> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(ids.len(), 1024);
    assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
}

#[test]
fn interleaved_signals_preserve_local_handler_order_and_repeated_disconnect_is_harmless() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut local = Signal::<()>::default();
    let mut other = Signal::<bool>::default();
    let mut ids = Vec::new();
    for name in ["first", "middle", "last"] {
        let observed = Arc::clone(&calls);
        ids.push(local.connect(move |_| observed.lock().unwrap().push(name)));
        other.connect(|_| {});
    }
    local.disconnect(ids[1]);
    local.disconnect(ids[1]);
    local.fire(&());
    assert_eq!(*calls.lock().unwrap(), vec!["first", "last"]);
    assert_eq!(local.len(), 2);
    assert_eq!(other.len(), 3);
}

/// Ids are world-local (#263): the first Part of two independent worlds has the same id, so a
/// signal connected in world B with world A's handle fires for B's own Part.
#[test]
fn equal_ids_from_independent_worlds_address_the_local_instance() {
    let mut a = World::new_empty();
    let mut b = World::new_empty();
    let part_a = a.create("Part", None).unwrap();
    let part_b = b.create("Part", None).unwrap();
    assert_eq!(part_a, part_b);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&seen);
    b.property_changed(part_a, "Anchored")
        .connect(move |value| observed.lock().unwrap().push(value.clone()));
    b.set(part_b, "Anchored", Value::Bool(true)).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![Value::Bool(true)]);

    // An id that does not resolve in B is inert there: its signal never fires.
    let second_a = a.create("Part", None).unwrap();
    assert!(!b.is_alive(second_a));
    let quiet = Arc::new(Mutex::new(0));
    let observed = Arc::clone(&quiet);
    b.property_changed(second_a, "Anchored")
        .connect(move |_| *observed.lock().unwrap() += 1);
    b.set(part_b, "Anchored", Value::Bool(false)).unwrap();
    assert!(b.get(second_a, "Anchored").is_err());
    assert_eq!(*quiet.lock().unwrap(), 0);
}

/// #97: moving an instance to the parent it already has is a no-op, as in Roblox: no
/// `child_added`, the child keeps its place, and nothing is journaled.
#[test]
fn set_parent_to_the_current_parent_changes_nothing() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let a = world.create("Part", Some(workspace)).unwrap();
    let b = world.create("Part", Some(workspace)).unwrap();
    let added = Arc::new(Mutex::new(0));
    let observed = Arc::clone(&added);
    world
        .child_added
        .connect(move |_| *observed.lock().unwrap() += 1);
    let cursor = world.change_cursor();

    world.set_parent(a, Some(workspace)).unwrap();
    assert_eq!(world.children(workspace), [a, b]);
    assert_eq!(*added.lock().unwrap(), 0);

    // The same for a root that is made a root again.
    let loose = world.create("Part", None).unwrap();
    let roots = world.roots().to_vec();
    world.set_parent(loose, None).unwrap();
    assert_eq!(world.roots(), roots.as_slice());
    let first_root = roots[0];
    world.set_parent(first_root, None).unwrap();
    assert_eq!(world.roots(), roots.as_slice());

    let (changes, _) = world.changes_since(cursor);
    let metallic_core::Changes::Ids(ids) = changes else {
        panic!("the journal overflowed");
    };
    assert!(
        !ids.contains(&a) && !ids.contains(&first_root),
        "a no-op move is not a change"
    );

    // A real move still fires and goes last.
    world.set_parent(a, Some(b)).unwrap();
    world.set_parent(a, Some(workspace)).unwrap();
    assert_eq!(world.children(workspace), [b, a]);
    assert_eq!(*added.lock().unwrap(), 2);
}

/// #97: `set_parent_at` reorders within a parent (fires nothing, journals the move) and puts a
/// moved child at its index (fires `child_added` once).
#[test]
fn set_parent_at_reorders_and_places() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let a = world.create("Part", Some(workspace)).unwrap();
    let b = world.create("Part", Some(workspace)).unwrap();
    let c = world.create("Part", Some(workspace)).unwrap();
    let added = Arc::new(Mutex::new(0));
    let observed = Arc::clone(&added);
    world
        .child_added
        .connect(move |_| *observed.lock().unwrap() += 1);
    let cursor = world.change_cursor();

    world.set_parent_at(c, Some(workspace), 0).unwrap();
    assert_eq!(world.children(workspace), [c, a, b]);
    world.set_parent_at(c, Some(workspace), 99).unwrap();
    assert_eq!(world.children(workspace), [a, b, c]);
    assert_eq!(*added.lock().unwrap(), 0, "a reorder is not an add");
    let (changes, cursor) = world.changes_since(cursor);
    let metallic_core::Changes::Ids(ids) = changes else {
        panic!("the journal overflowed");
    };
    assert!(ids.contains(&c), "a reorder is journaled");

    // Already there: nothing at all.
    world.set_parent_at(a, Some(workspace), 0).unwrap();
    let (changes, _) = world.changes_since(cursor);
    assert!(matches!(changes, metallic_core::Changes::Ids(ids) if ids.is_empty()));

    // A move to another parent at an index.
    let x = world.create("Part", Some(b)).unwrap();
    let y = world.create("Part", Some(b)).unwrap();
    *added.lock().unwrap() = 0;
    world.set_parent_at(c, Some(b), 1).unwrap();
    assert_eq!(world.children(b), [x, c, y]);
    assert_eq!(world.children(workspace), [a, b]);
    assert_eq!(*added.lock().unwrap(), 1);
    // The same rules as set_parent: no cycles.
    assert!(world.set_parent_at(b, Some(x), 0).is_err());
}
