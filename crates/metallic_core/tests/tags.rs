//! Tags: the index, membership in the game tree, the tag log, clones, scene files.

use metallic_core::*;

mod common;
use common::TestDir;

fn part(w: &mut World, parent: Option<InstanceId>) -> InstanceId {
    let parent = parent.or_else(|| w.root("Workspace"));
    w.create("Part", parent).unwrap()
}

fn changes(w: &World, cursor: TagCursor) -> Vec<(InstanceId, String, bool)> {
    match w.tag_changes_since(cursor).0 {
        TagChanges::Changes(list) => list.into_iter().map(|c| (c.id, c.tag, c.added)).collect(),
        TagChanges::Overflow => panic!("overflow"),
    }
}

#[test]
fn add_has_remove_and_sorted_listing() {
    let mut w = World::new_with_services();
    let p = part(&mut w, None);
    assert!(!w.has_tag(p, "Lava"));
    w.add_tag(p, "Lava").unwrap();
    w.add_tag(p, "Hazard").unwrap();
    w.add_tag(p, "Lava").unwrap(); // twice is fine
    assert!(w.has_tag(p, "Lava"));
    assert_eq!(w.tags(p).collect::<Vec<_>>(), ["Hazard", "Lava"]);
    assert_eq!(w.all_tags(), ["Hazard", "Lava"]);
    w.remove_tag(p, "Lava").unwrap();
    w.remove_tag(p, "Lava").unwrap(); // not there: fine
    w.remove_tag(p, "Never").unwrap();
    assert!(!w.has_tag(p, "Lava"));
    assert_eq!(w.all_tags(), ["Hazard"]);
    w.remove_tag(p, "Hazard").unwrap();
    assert!(w.all_tags().is_empty());
    assert_eq!(w.tags(p).count(), 0);
}

#[test]
fn bad_tags_and_limits() {
    let mut w = World::new_with_services();
    let p = part(&mut w, None);
    for bad in ["", "tab\there", "nul\0byte"] {
        assert!(
            matches!(w.add_tag(p, bad), Err(CoreError::InvalidValue { .. })),
            "{bad:?}"
        );
    }
    assert!(w.add_tag(p, &"x".repeat(MAX_TAG + 1)).is_err());
    w.add_tag(p, &"x".repeat(MAX_TAG)).unwrap();
    // Spaces, punctuation and non-ASCII are fine.
    for ok in ["two words", "a.b-c", "Ünï", "🔥"] {
        w.add_tag(p, ok).unwrap();
    }
    for i in 0..(MAX_TAGS - 5) {
        w.add_tag(p, &format!("t{i}")).unwrap();
    }
    assert_eq!(w.tags(p).count(), MAX_TAGS);
    assert!(w.add_tag(p, "one-too-many").is_err());
    w.remove_tag(p, "t0").unwrap();
    w.add_tag(p, "one-too-many").unwrap();
    // Dead instances.
    w.destroy(p);
    assert_eq!(w.add_tag(p, "A"), Err(CoreError::InvalidInstance));
    assert_eq!(w.remove_tag(p, "A"), Err(CoreError::InvalidInstance));
    assert!(!w.has_tag(p, "A"));
}

#[test]
fn tagged_lists_only_instances_in_the_game_tree_in_id_order() {
    let mut w = World::new_with_services();
    let a = part(&mut w, None);
    let b = part(&mut w, None);
    let model = w.create("Model", w.root("Workspace")).unwrap();
    let c = part(&mut w, Some(model));
    for id in [c, a, b] {
        w.add_tag(id, "T").unwrap();
    }
    assert_eq!(w.tagged("T"), [a, b, c]);
    assert!(w.tagged("Other").is_empty());
    // Detached: out of GetTagged, tag kept, back when reparented.
    w.set_parent(model, None).unwrap();
    assert_eq!(w.tagged("T"), [a, b]);
    assert!(w.has_tag(c, "T"));
    w.set_parent(model, w.root("ReplicatedStorage")).unwrap();
    assert_eq!(w.tagged("T"), [a, b, c]);
    // A part in a service other than Workspace counts too.
    assert!(w.is_in_tree(c));
    let copy = w.clone_subtree(a).unwrap();
    assert!(!w.is_in_tree(copy));
}

#[test]
fn destroyed_instances_and_subtrees_leave_the_sets() {
    let mut w = World::new_with_services();
    let model = w.create("Model", w.root("Workspace")).unwrap();
    let inner = part(&mut w, Some(model));
    let other = part(&mut w, None);
    for id in [model, inner, other] {
        w.add_tag(id, "T").unwrap();
    }
    w.add_tag(inner, "OnlyInner").unwrap();
    w.destroy(model);
    assert_eq!(w.tagged("T"), [other]);
    assert!(w.tagged("OnlyInner").is_empty());
    assert_eq!(
        w.all_tags(),
        ["T"],
        "the empty set is gone, not left behind"
    );
    w.destroy(other);
    assert!(w.all_tags().is_empty());
}

#[test]
fn the_log_has_one_entry_per_change_in_the_tree() {
    let mut w = World::new_with_services();
    let p = part(&mut w, None);
    let cursor = w.tag_cursor();
    w.add_tag(p, "A").unwrap();
    w.add_tag(p, "A").unwrap(); // no change
    w.add_tag(p, "B").unwrap();
    w.remove_tag(p, "A").unwrap();
    w.remove_tag(p, "A").unwrap(); // no change
    assert_eq!(
        changes(&w, cursor),
        [
            (p, "A".into(), true),
            (p, "B".into(), true),
            (p, "A".into(), false)
        ]
    );
    // A detached instance logs nothing until it enters the tree.
    let loose = w.clone_subtree(p).unwrap();
    let cursor = w.tag_cursor();
    w.add_tag(loose, "C").unwrap();
    assert!(changes(&w, cursor).is_empty());
    w.set_parent(loose, w.root("Workspace")).unwrap();
    let mut entered = changes(&w, cursor);
    entered.sort();
    assert_eq!(
        entered,
        [(loose, "B".into(), true), (loose, "C".into(), true)]
    );
}

#[test]
fn reparenting_and_destroying_log_the_subtree_once() {
    let mut w = World::new_with_services();
    let model = w.create("Model", w.root("Workspace")).unwrap();
    let a = part(&mut w, Some(model));
    let b = part(&mut w, Some(model));
    let c = part(&mut w, Some(model)); // untagged
    w.add_tag(a, "T").unwrap();
    w.add_tag(b, "T").unwrap();
    w.add_tag(b, "U").unwrap();
    let _ = c;
    let cursor = w.tag_cursor();
    // Moving inside the tree changes nothing.
    w.set_parent(model, w.root("ReplicatedStorage")).unwrap();
    assert!(changes(&w, cursor).is_empty());
    // Leaving the tree removes, entering adds.
    w.set_parent(model, None).unwrap();
    let mut left = changes(&w, cursor);
    left.sort();
    assert_eq!(
        left,
        [
            (a, "T".into(), false),
            (b, "T".into(), false),
            (b, "U".into(), false)
        ]
    );
    let cursor = w.tag_cursor();
    w.set_parent(model, w.root("Workspace")).unwrap();
    assert_eq!(changes(&w, cursor).len(), 3);
    assert!(changes(&w, cursor).iter().all(|c| c.2));
    // Destroying an in-tree subtree removes.
    let cursor = w.tag_cursor();
    w.destroy(model);
    let mut gone = changes(&w, cursor);
    gone.sort();
    assert_eq!(gone.len(), 3);
    assert!(gone.iter().all(|c| !c.2));
    // Destroying a detached subtree logs nothing.
    let loose = w.create("Model", None).unwrap();
    let inner = w.create("Part", Some(loose)).unwrap();
    w.add_tag(inner, "T").unwrap();
    let cursor = w.tag_cursor();
    w.destroy(loose);
    assert!(changes(&w, cursor).is_empty());
}

#[test]
fn tag_changes_mark_the_instance_changed_and_the_log_is_bounded() {
    let mut w = World::new_with_services();
    let p = part(&mut w, None);
    let journal = w.change_cursor();
    w.add_tag(p, "A").unwrap();
    assert_eq!(w.changes_since(journal).0, Changes::Ids(vec![p]));
    let old = w.tag_cursor();
    for _ in 0..6000 {
        w.remove_tag(p, "A").unwrap();
        w.add_tag(p, "A").unwrap();
    }
    assert_eq!(w.tag_changes_since(old).0, TagChanges::Overflow);
    let other = World::new_with_services();
    assert_eq!(
        w.tag_changes_since(other.tag_cursor()).0,
        TagChanges::Overflow
    );
}

#[test]
fn clones_keep_their_tags_in_the_index() {
    let mut w = World::new_with_services();
    let model = w.create("Model", w.root("Workspace")).unwrap();
    let inner = part(&mut w, Some(model));
    w.add_tag(model, "M").unwrap();
    w.add_tag(inner, "T").unwrap();
    let copy = w.clone_subtree(model).unwrap();
    assert!(w.has_tag(copy, "M"));
    assert_eq!(
        w.tagged("M"),
        [model],
        "the detached copy is not in the game"
    );
    w.set_parent(copy, w.root("Workspace")).unwrap();
    assert_eq!(w.tagged("M").len(), 2);
    assert_eq!(w.tagged("T").len(), 2);
    // Independent afterwards.
    w.remove_tag(copy, "M").unwrap();
    assert!(w.has_tag(model, "M"));
    w.destroy(copy);
    assert_eq!(w.tagged("T"), [inner]);
    let snapshot = w.deep_clone();
    assert_eq!(snapshot.tagged("T"), [inner]);
    assert_eq!(snapshot.tagged("M"), [model]);
}

#[test]
fn scene_round_trip_keeps_tags_and_old_files_still_load() {
    let dir = TestDir::new("tags_round_trip");
    let mut w = World::new_with_services();
    let p = part(&mut w, None);
    let model = w.create("Model", w.root("Workspace")).unwrap();
    w.add_tag(p, "Lava").unwrap();
    w.add_tag(p, "tag with \"quotes\" and spaces").unwrap();
    w.add_tag(model, "Group").unwrap();
    w.set_attribute(p, "Damage", Value::Number(5.0)).unwrap();
    let a = dir.file("tags_a.ron");
    let b = dir.file("tags_b.ron");
    save_scene(&w, &a).unwrap();
    let w2 = load_scene(&a).unwrap();
    save_scene(&w2, &b).unwrap();
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        std::fs::read_to_string(&b).unwrap()
    );
    let tagged = w2.tagged("Lava");
    assert_eq!(tagged.len(), 1);
    assert!(w2.has_tag(tagged[0], "tag with \"quotes\" and spaces"));
    assert_eq!(w2.attribute(tagged[0], "Damage"), Some(&Value::Number(5.0)));
    assert_eq!(w2.tagged("Group").len(), 1);
    // A scene without tags has no tag block, and a file from before tags loads.
    let plain = World::new_with_services();
    let path = dir.file("plain.ron");
    save_scene(&plain, &path).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("tags"));
    assert!(load_scene(&path).is_ok());
}

#[test]
fn a_scene_with_a_bad_tag_is_refused() {
    let dir = TestDir::new("tags_bad");
    let path = dir.file("bad.ron");
    std::fs::write(
        &path,
        r#"(
    version: 1,
    roots: [
        (
            class: "Workspace",
            builtin: true,
            children: [
                (class: "Part", tags: [""]),
            ],
        ),
    ],
)"#,
    )
    .unwrap();
    assert!(load_scene(&path).is_err());
}

#[test]
fn many_instances_with_few_tagged_are_found_through_the_index() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let mut chosen = Vec::new();
    for i in 0..5000 {
        let p = w.create("Part", Some(ws)).unwrap();
        if i % 1000 == 7 {
            w.add_tag(p, "Rare").unwrap();
            chosen.push(p);
        }
    }
    assert_eq!(w.tagged("Rare"), chosen);
    assert!(w.tagged("Absent").is_empty());
}
