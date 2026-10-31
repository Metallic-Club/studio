//! Detailed changes preserve operation order without replacing the coarse invalidation reader.
use metallic_core::{Change, ChangeCursor, ChangeKind, Changes, DetailedChanges, Value, World};

fn entries(world: &World, cursor: ChangeCursor) -> Vec<Change> {
    match world.changes_since_detailed(cursor).0 {
        DetailedChanges::Changes(entries) => entries,
        DetailedChanges::Overflow => panic!("unexpected detailed overflow"),
    }
}

fn coarse(world: &World, cursor: ChangeCursor) -> Vec<metallic_core::InstanceId> {
    match world.changes_since(cursor).0 {
        Changes::Ids(ids) => ids,
        Changes::Overflow => panic!("unexpected coarse overflow"),
    }
}

fn change(id: metallic_core::InstanceId, kind: ChangeKind) -> Change {
    Change { id, kind }
}

#[test]
fn detailed_changes_keep_repeated_mutations_in_real_order() {
    let mut world = World::new_empty();
    let before = world.change_cursor();
    let first = world.create("Part", None).unwrap();
    let second = world.create("Folder", None).unwrap();
    world.set(first, "Anchored", Value::Bool(true)).unwrap();
    world.set(first, "Anchored", Value::Bool(false)).unwrap();
    world.set_parent(first, Some(second)).unwrap();
    world.destroy(first);
    assert_eq!(
        entries(&world, before),
        [
            change(first, ChangeKind::Created),
            change(second, ChangeKind::Created),
            change(first, ChangeKind::Property("Anchored")),
            change(first, ChangeKind::Property("Anchored")),
            change(first, ChangeKind::Reparented),
            change(first, ChangeKind::Destroyed),
        ]
    );
    assert_eq!(coarse(&world, before), [first, second]);
    let (_, detailed_now) = world.changes_since_detailed(before);
    let (_, coarse_now) = world.changes_since(before);
    assert_eq!(detailed_now, coarse_now);
    assert!(entries(&world, detailed_now).is_empty());
}

#[test]
fn property_tokens_are_canonical_static_registry_names() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let before = world.change_cursor();
    let caller_owned_name = String::from("Name");
    world
        .set(part, &caller_owned_name, Value::String("renamed".into()))
        .unwrap();
    drop(caller_owned_name);
    let changes = entries(&world, before);
    let ChangeKind::Property(name) = changes[0].kind else {
        panic!("expected property change");
    };
    let canonical = metallic_core::classes::find_property("Part", "Name")
        .unwrap()
        .name;
    assert_eq!(name, "Name");
    assert_eq!(name.as_ptr(), canonical.as_ptr());
    // Retained records own no property string and outlive the source world.
    drop(world);
    assert_eq!(name, "Name");
    fn assert_copy<T: Copy>() {}
    assert_copy::<Change>();
    assert_copy::<ChangeKind>();
}

#[test]
fn reparent_detail_reports_only_the_direct_operation_and_keeps_coarse_descendants() {
    let mut world = World::new_empty();
    let root = world.create("Folder", None).unwrap();
    let first = world.create("Part", Some(root)).unwrap();
    let branch = world.create("Folder", Some(root)).unwrap();
    let leaf = world.create("Part", Some(branch)).unwrap();
    let destination = world.create("Folder", None).unwrap();
    let before = world.change_cursor();
    world.set_parent(root, Some(destination)).unwrap();
    assert_eq!(
        entries(&world, before),
        [change(root, ChangeKind::Reparented)]
    );
    assert_eq!(coarse(&world, before), [root, branch, leaf, first]);
    assert_eq!(world.parent(first), Some(root));
    assert_eq!(world.parent(branch), Some(root));
    assert_eq!(world.parent(leaf), Some(branch));
}

#[test]
fn same_parent_assignment_preserves_order_and_both_cursors() {
    let mut world = World::new_empty();
    let parent = world.create("Folder", None).unwrap();
    let first = world.create("Part", Some(parent)).unwrap();
    let second = world.create("Part", Some(parent)).unwrap();
    let before = world.change_cursor();
    world.set_parent(first, Some(parent)).unwrap();
    assert_eq!(world.children(parent), [first, second]);
    assert_eq!(world.change_cursor(), before);
    assert!(entries(&world, before).is_empty());
    assert!(coarse(&world, before).is_empty());
}

#[test]
fn explicit_reorders_report_only_the_moved_sibling_in_operation_order() {
    let mut world = World::new_empty();
    let parent = world.create("Folder", None).unwrap();
    let first = world.create("Folder", Some(parent)).unwrap();
    let leaf = world.create("Part", Some(first)).unwrap();
    let second = world.create("Part", Some(parent)).unwrap();
    let signals = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = signals.clone();
    world
        .child_added
        .connect(move |event| observed.lock().unwrap().push(*event));
    let before = world.change_cursor();
    world
        .set_parent_at(first, Some(parent), usize::MAX)
        .unwrap();
    assert_eq!(world.children(parent), [second, first]);
    world.set_parent_at(first, Some(parent), 0).unwrap();
    assert_eq!(world.children(parent), [first, second]);
    assert_eq!(
        entries(&world, before),
        [
            change(first, ChangeKind::Reordered),
            change(first, ChangeKind::Reordered),
        ]
    );
    assert_eq!(coarse(&world, before), [first, parent]);
    assert_eq!(world.parent(leaf), Some(first));
    assert!(signals.lock().unwrap().is_empty());
    let settled = world.change_cursor();
    world.set_parent_at(first, Some(parent), 0).unwrap();
    world
        .set_parent_at(second, Some(parent), usize::MAX)
        .unwrap();
    assert_eq!(world.change_cursor(), settled);
}

#[test]
fn root_reorder_and_cross_parent_placement_have_distinct_notices() {
    let mut world = World::new_empty();
    let first = world.create("Folder", None).unwrap();
    let second = world.create("Folder", None).unwrap();
    let existing = world.create("Part", Some(second)).unwrap();
    let leaf = world.create("Part", Some(first)).unwrap();
    let before = world.change_cursor();
    world.set_parent_at(first, None, usize::MAX).unwrap();
    assert_eq!(world.roots(), [second, first]);
    assert_eq!(
        entries(&world, before),
        [change(first, ChangeKind::Reordered)]
    );
    assert_eq!(coarse(&world, before), [first]);
    let before_move = world.change_cursor();
    world.set_parent_at(first, Some(second), 0).unwrap();
    assert_eq!(world.children(second), [first, existing]);
    assert_eq!(
        entries(&world, before_move),
        [change(first, ChangeKind::Reparented)]
    );
    assert_eq!(coarse(&world, before_move), [first, leaf]);
}

#[test]
fn destroying_subtree_records_every_removed_id_in_actual_traversal_order() {
    let mut world = World::new_empty();
    let root = world.create("Folder", None).unwrap();
    let first = world.create("Part", Some(root)).unwrap();
    let branch = world.create("Folder", Some(root)).unwrap();
    let leaf = world.create("Part", Some(branch)).unwrap();
    let before = world.change_cursor();
    world.destroy(root);
    let removed = [root, branch, leaf, first];
    assert_eq!(
        entries(&world, before),
        removed.map(|id| change(id, ChangeKind::Destroyed))
    );
    assert_eq!(coarse(&world, before), removed);
    for id in removed {
        assert!(!world.is_alive(id));
    }
    let empty = world.change_cursor();
    world.destroy(root);
    assert_eq!(world.change_cursor(), empty);
    assert!(entries(&world, empty).is_empty());
}

#[test]
fn clone_creates_parent_before_children_and_retains_remapped_reference_values() {
    let mut world = World::new_empty();
    let root = world.create("Folder", None).unwrap();
    let player = world.create("Player", Some(root)).unwrap();
    let branch = world.create("Model", Some(root)).unwrap();
    let part = world.create("Part", Some(branch)).unwrap();
    world
        .set(player, "Character", Value::Ref(Some(part)))
        .unwrap();
    let before = world.change_cursor();
    let copy = world.clone_subtree(root).unwrap();
    let copy_player = world.children(copy)[0];
    let copy_branch = world.children(copy)[1];
    let copy_part = world.children(copy_branch)[0];
    let created = [copy, copy_player, copy_branch, copy_part];
    assert_eq!(
        entries(&world, before),
        created.map(|id| change(id, ChangeKind::Created))
    );
    assert_eq!(coarse(&world, before), created);
    assert_eq!(
        world.get(copy_player, "Character").unwrap(),
        Value::Ref(Some(copy_part))
    );
    assert_eq!(
        world.get(player, "Character").unwrap(),
        Value::Ref(Some(part))
    );
    assert_eq!(world.parent(copy), None);
    assert_eq!(world.parent(copy_player), Some(copy));
    assert!(!world.is_builtin(copy));
}

#[test]
fn errors_and_equal_value_writes_leave_both_views_and_cursor_unchanged() {
    let mut world = World::new_empty();
    let parent = world.create("Folder", None).unwrap();
    let part = world.create("Part", Some(parent)).unwrap();
    let stale = world.create("Part", None).unwrap();
    world.destroy(stale);
    let before = world.change_cursor();
    world.set(part, "Anchored", Value::Bool(false)).unwrap();
    world
        .set(part, "Name", Value::String("Part".into()))
        .unwrap();
    assert!(world.create("UnknownClass", None).is_err());
    assert!(world.create("Part", Some(stale)).is_err());
    assert!(world.set(part, "Missing", Value::Bool(true)).is_err());
    assert!(world.set(part, "Anchored", Value::Number(1.0)).is_err());
    assert!(
        world
            .set(part, "Transparency", Value::Number(f64::NAN))
            .is_err()
    );
    assert!(
        world
            .set(part, "Material", Value::String("unknown".into()))
            .is_err()
    );
    assert!(world.set(stale, "Anchored", Value::Bool(true)).is_err());
    assert!(world.set_parent(parent, Some(part)).is_err());
    assert!(world.set_parent(part, Some(stale)).is_err());
    assert!(world.set_parent_at(parent, Some(part), 0).is_err());
    assert!(world.set_parent_at(part, Some(stale), 0).is_err());
    assert!(world.set_parent_at(stale, None, 0).is_err());
    assert!(world.clone_subtree(stale).is_err());
    world.destroy(stale);
    assert_eq!(world.change_cursor(), before);
    assert_eq!(world.parent(part), Some(parent));
    assert!(entries(&world, before).is_empty());
    assert!(coarse(&world, before).is_empty());
}

#[test]
fn both_readers_are_independent_and_resume_from_the_same_cursor() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let before = world.change_cursor();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    let (first, cursor) = world.changes_since_detailed(before);
    assert_eq!(coarse(&world, before), [part]);
    assert_eq!(world.changes_since_detailed(before).0, first);
    world.set(part, "Anchored", Value::Bool(false)).unwrap();
    assert_eq!(
        entries(&world, cursor),
        [change(part, ChangeKind::Property("Anchored"))]
    );
    assert_eq!(entries(&world, before).len(), 2);
    assert_eq!(coarse(&world, before), [part]);
}

#[test]
fn destroyed_then_reused_slot_has_distinct_create_and_destroy_records() {
    let mut world = World::new_empty();
    let original = world.create("Part", None).unwrap();
    let before = world.change_cursor();
    world.destroy(original);
    let replacement = world.create("Part", None).unwrap();
    assert_ne!(original, replacement);
    assert_eq!(
        entries(&world, before),
        [
            change(original, ChangeKind::Destroyed),
            change(replacement, ChangeKind::Created)
        ]
    );
    assert_eq!(coarse(&world, before), [original, replacement]);
}

#[test]
fn overflow_and_foreign_cursors_match_the_existing_reader() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let before = world.change_cursor();
    for index in 0..40_000 {
        world
            .set(
                part,
                "Transparency",
                Value::Number((index % 7) as f64 / 10.0 + 0.01),
            )
            .unwrap();
    }
    let (detail, detail_now) = world.changes_since_detailed(before);
    let (coarse, coarse_now) = world.changes_since(before);
    assert_eq!(detail, DetailedChanges::Overflow);
    assert_eq!(coarse, Changes::Overflow);
    assert_eq!(detail_now, coarse_now);
    let other = World::new_empty();
    assert_eq!(
        world.changes_since_detailed(other.change_cursor()).0,
        DetailedChanges::Overflow
    );
    world
        .set(part, "Transparency", Value::Number(0.99))
        .unwrap();
    assert_eq!(
        entries(&world, detail_now),
        [change(part, ChangeKind::Property("Transparency"))]
    );
}

#[test]
fn construction_and_deep_clone_start_with_empty_independent_history() {
    let mut world = World::new_with_services();
    let start = world.change_cursor();
    assert!(entries(&world, start).is_empty());
    let part = world.create("Part", world.root("Workspace")).unwrap();
    world.set(part, "Anchored", Value::Bool(true)).unwrap();
    let copy = world.deep_clone();
    assert!(copy.is_alive(part));
    assert!(entries(&copy, copy.change_cursor()).is_empty());
    assert_eq!(
        copy.changes_since_detailed(start).0,
        DetailedChanges::Overflow
    );
    assert_eq!(
        world.changes_since_detailed(copy.change_cursor()).0,
        DetailedChanges::Overflow
    );
}

#[test]
fn attributes_and_tags_are_coarse_only_and_clones_journal_once_in_order() {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(ws)).unwrap();
    let kid = world.create("Part", Some(model)).unwrap();
    world.add_tag(kid, "Lava").unwrap();
    let cursor = world.change_cursor();
    world
        .set_attribute(model, "Score", Value::Number(1.0))
        .unwrap();
    world.add_tag(model, "Boss").unwrap();
    world.remove_tag(model, "Boss").unwrap();
    assert_eq!(coarse(&world, cursor), vec![model]);
    assert!(entries(&world, cursor).is_empty());

    let cursor = world.change_cursor();
    let copy = world.clone_subtree(model).unwrap();
    let copy_kid = world.children(copy)[0];
    assert_eq!(
        entries(&world, cursor),
        vec![
            change(copy, ChangeKind::Created),
            change(copy_kid, ChangeKind::Created)
        ]
    );
    assert_eq!(coarse(&world, cursor), vec![copy, copy_kid]);
    world.set_parent(copy, Some(ws)).unwrap();
    assert!(world.tagged("Lava").contains(&copy_kid));
}
