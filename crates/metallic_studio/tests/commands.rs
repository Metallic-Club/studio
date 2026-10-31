//! Task 10 editing commands: undo/redo, delete with id restore, duplicate, group, dirty flag.
use metallic_core::{InstanceId, Value, Vec3, World, load_scene};
use metallic_studio::commands::{Command, HISTORY_LIMIT, History};
use metallic_studio::editor::EditorState;
use metallic_studio::output::OutputLog;
use std::path::Path;

fn state_in(dir: &Path) -> (EditorState, OutputLog) {
    let log = OutputLog::new(64);
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let folder = world.create("Folder", Some(ws)).unwrap();
    world
        .set(folder, "Name", Value::String("Course".into()))
        .unwrap();
    for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
        let p = world.create("Part", Some(folder)).unwrap();
        world.set(p, "Name", Value::String(name.into())).unwrap();
        world
            .set(
                p,
                "Position",
                Value::Vector3(Vec3::new(i as f64 * 5.0, 0.0, 0.0)),
            )
            .unwrap();
    }
    let scene = dir.join("main.mscene");
    let state = EditorState::new(world, dir, &scene, "Test", log.sink());
    (state, log)
}

fn named(state: &EditorState, name: &str) -> InstanceId {
    let ws = state.world.root("Workspace").unwrap();
    let course = state.world.find_first_child(ws, "Course").unwrap();
    state.world.find_first_child(course, name).unwrap()
}

fn course(state: &EditorState) -> InstanceId {
    let ws = state.world.root("Workspace").unwrap();
    state.world.find_first_child(ws, "Course").unwrap()
}

#[test]
fn set_property_undo_redo() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    s.set_property(&[a], "Transparency", Value::Number(0.5));
    assert_eq!(s.world.get(a, "Transparency").unwrap(), Value::Number(0.5));
    assert_eq!(s.history.len(), 1);
    s.undo();
    assert_eq!(s.world.get(a, "Transparency").unwrap(), Value::Number(0.0));
    s.redo();
    assert_eq!(s.world.get(a, "Transparency").unwrap(), Value::Number(0.5));
    // A new edit clears redo.
    s.undo();
    s.set_property(&[a], "Anchored", Value::Bool(true));
    assert!(s.history.redo_name(&s.world).is_none());
}

#[test]
fn multi_selection_property_is_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let ids = [named(&s, "A"), named(&s, "B"), named(&s, "C")];
    s.set_property(&ids, "Anchored", Value::Bool(true));
    assert_eq!(s.history.len(), 1);
    s.undo();
    for id in ids {
        assert_eq!(s.world.get(id, "Anchored").unwrap(), Value::Bool(false));
    }
}

#[test]
fn delete_undo_restores_subtree_and_ids_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let c = course(&s);
    let (a, b) = (named(&s, "A"), named(&s, "B"));
    let ws = s.world.root("Workspace").unwrap();
    let before: Vec<InstanceId> = s.world.children(c).to_vec();
    // Delete the folder (with its parts) and one part inside it: the folder wins.
    s.selection.set([b, c]);
    s.delete_selection();
    assert!(!s.world.children(ws).contains(&c));
    assert!(s.selection.is_empty(), "deleted ids leave the selection");
    assert!(!s.is_live(a));
    s.undo();
    // Same ids, same order, same parent, properties intact.
    assert_eq!(s.world.parent(c), Some(ws));
    assert_eq!(s.world.children(c), before.as_slice());
    assert!(s.is_live(a) && s.is_live(b));
    assert_eq!(s.display_name(a), "A");
    // Delete a middle child and undo: it returns to its index.
    s.selection.set([b]);
    s.delete_selection();
    assert_eq!(s.world.children(c).len(), 2);
    s.undo();
    assert_eq!(s.world.children(c), before.as_slice());
    // Redo deletes again; saving leaves deleted parts out of the file.
    s.redo();
    s.save().unwrap();
    let saved = load_scene(&s.scene_path).unwrap();
    let ws2 = saved.root("Workspace").unwrap();
    let c2 = saved.find_first_child(ws2, "Course").unwrap();
    assert_eq!(saved.children(c2).len(), 2);
    assert_eq!(saved.roots().len(), s.visible_roots().len());
}

#[test]
fn duplicate_offsets_and_selects_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    s.selection.select_only(a);
    s.duplicate_selection();
    let copy = s.selection.active().unwrap();
    assert_ne!(copy, a);
    assert_eq!(s.world.parent(copy), s.world.parent(a));
    assert_eq!(s.display_name(copy), "A");
    // Default part height is 1: the copy sits on top.
    assert_eq!(
        s.world.get(copy, "Position").unwrap(),
        Value::Vector3(Vec3::new(0.0, 1.0, 0.0))
    );
    s.undo();
    assert!(!s.is_live(copy));
    assert!(s.selection.is_empty());
    s.redo();
    assert!(s.is_live(copy), "redo restores the same copy id");
}

#[test]
fn group_creates_model_with_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let c = course(&s);
    let (a, b, cc) = (named(&s, "A"), named(&s, "B"), named(&s, "C"));
    s.selection.set([b, cc]);
    s.group_selection();
    let model = s.selection.active().unwrap();
    assert_eq!(s.world.class_name(model), Some("Model"));
    assert_eq!(s.world.children(model), &[b, cc]);
    assert_eq!(
        s.world.children(c),
        &[a, model],
        "the Model takes the first item's place"
    );
    s.undo();
    assert_eq!(s.world.children(c), &[a, b, cc]);
    assert!(!s.is_live(model));
    s.redo();
    assert_eq!(s.world.children(model), &[b, cc]);
}

#[test]
fn history_marks_dirty_and_save_clears() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    assert!(!s.is_dirty());
    let a = named(&s, "A");
    s.set_property(&[a], "Name", Value::String("Renamed".into()));
    assert!(s.is_dirty());
    s.save().unwrap();
    assert!(!s.is_dirty());
    let saved = load_scene(&s.scene_path).unwrap();
    let ws = saved.root("Workspace").unwrap();
    let c = saved.find_first_child(ws, "Course").unwrap();
    assert!(saved.find_first_child(c, "Renamed").is_some());
    s.undo();
    assert!(s.is_dirty(), "undo past the save is a change");
    s.redo();
    assert!(!s.is_dirty(), "redo back to the saved step is clean");
}

#[test]
fn reparent_undo_restores_index() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let c = course(&s);
    let (a, b, cc) = (named(&s, "A"), named(&s, "B"), named(&s, "C"));
    let ws = s.world.root("Workspace").unwrap();
    s.reparent(&[a], ws);
    assert_eq!(s.world.parent(a), Some(ws));
    s.undo();
    assert_eq!(s.world.children(c), &[a, b, cc]);
    // A cycle is refused and leaves no undo step.
    let steps = s.history.len();
    s.reparent(&[c], a);
    assert_eq!(s.history.len(), steps);
}

#[test]
fn insert_selects_and_undo_parks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let id = s
        .insert("Part", None, Some(Vec3::new(1.0, 2.0, 3.0)))
        .unwrap();
    assert_eq!(s.selection.ids(), &[id]);
    assert_eq!(
        s.world.get(id, "Position").unwrap(),
        Value::Vector3(Vec3::new(1.0, 2.0, 3.0))
    );
    s.undo();
    assert!(!s.is_live(id));
    assert!(s.history.is_parked(id));
    // A new edit drops the undone insert for good.
    let a = named(&s, "A");
    s.set_property(&[a], "Anchored", Value::Bool(true));
    assert!(!s.world.is_alive(id));
}

#[test]
fn history_limit_destroys_parked_deletes() {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let doomed = world.create("Part", Some(ws)).unwrap();
    let keep = world.create("Part", Some(ws)).unwrap();
    let mut h = History::new();
    h.apply(Command::delete(&[doomed]), &mut world).unwrap();
    for i in 0..HISTORY_LIMIT {
        let old = world.get(keep, "Transparency").unwrap();
        let new = Value::Number(((i % 10) as f64 + 1.0) / 20.0);
        h.apply(Command::set(keep, "Transparency", old, new), &mut world)
            .unwrap();
    }
    assert_eq!(h.len(), HISTORY_LIMIT);
    assert!(
        !world.is_alive(doomed),
        "the delete left the history, its part is gone"
    );
}

#[test]
fn batch_failure_leaves_world_unchanged() {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let p = world.create("Part", Some(ws)).unwrap();
    let mut h = History::new();
    let cmd = Command::Batch {
        name: "Bad".into(),
        commands: vec![
            Command::set(p, "Transparency", Value::Number(0.0), Value::Number(0.3)),
            Command::set(
                p,
                "Material",
                Value::String("Plastic".into()),
                Value::String("Lava".into()),
            ),
        ],
    };
    assert!(h.apply(cmd, &mut world).is_err());
    assert_eq!(world.get(p, "Transparency").unwrap(), Value::Number(0.0));
    assert!(h.is_empty());
}

#[test]
fn undoing_everything_after_the_cap_is_still_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    // Opened file: clean. Past the 200-step cap the first steps are gone, so undoing every
    // kept step does not bring back the opened file.
    for i in 0..(HISTORY_LIMIT + 5) {
        s.set_property(
            &[a],
            "Transparency",
            Value::Number(((i % 9) as f64 + 1.0) / 10.0),
        );
    }
    while s.history.undo(&mut s.world).is_some() {}
    assert!(s.is_dirty(), "the world is not the opened file");
    // Saving at the bottom, then redo and undo back, is clean again.
    s.save().unwrap();
    assert!(!s.is_dirty());
    s.redo();
    s.undo();
    assert!(!s.is_dirty());
}

#[test]
fn attribute_commands_undo_and_redo_with_names() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    let add = Command::SetAttribute {
        id: a,
        name: "Speed".into(),
        old: Value::Nil,
        new: Value::Number(3.0),
    };
    assert_eq!(add.name(&s.world), "Set attribute Speed of A");
    s.apply(add).unwrap();
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(3.0)));
    s.apply(Command::SetAttribute {
        id: a,
        name: "Speed".into(),
        old: Value::Number(3.0),
        new: Value::Number(4.0),
    })
    .unwrap();
    let remove = Command::SetAttribute {
        id: a,
        name: "Speed".into(),
        old: Value::Number(4.0),
        new: Value::Nil,
    };
    assert_eq!(remove.name(&s.world), "Remove attribute Speed of A");
    s.apply(remove).unwrap();
    assert_eq!(s.world.attribute(a, "Speed"), None);
    s.undo();
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(4.0)));
    s.undo();
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(3.0)));
    s.undo();
    assert_eq!(s.world.attribute(a, "Speed"), None);
    s.redo();
    s.redo();
    s.redo();
    assert_eq!(s.world.attribute(a, "Speed"), None);
    // A bad name is refused and leaves nothing on the undo stack.
    let before = s.history.len();
    assert!(
        s.apply(Command::SetAttribute {
            id: a,
            name: "bad name".into(),
            old: Value::Nil,
            new: Value::Bool(true),
        })
        .is_err()
    );
    assert_eq!(s.history.len(), before);
}

#[test]
fn attributes_follow_delete_undo_and_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    s.apply(Command::SetAttribute {
        id: a,
        name: "Team".into(),
        old: Value::Nil,
        new: Value::String("Red".into()),
    })
    .unwrap();
    s.selection.select_only(a);
    s.duplicate_selection();
    let copy = s.selection.ids()[0];
    assert_ne!(copy, a);
    assert_eq!(
        s.world.attribute(copy, "Team"),
        Some(&Value::String("Red".into()))
    );
    s.selection.select_only(a);
    s.delete_selection();
    s.undo();
    assert_eq!(
        s.world.attribute(a, "Team"),
        Some(&Value::String("Red".into()))
    );
}

#[test]
fn dragging_a_number_attribute_is_one_undo_step() {
    use metallic_studio::properties::Properties;
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _log) = state_in(dir.path());
    let a = named(&s, "A");
    s.apply(Command::SetAttribute {
        id: a,
        name: "Speed".into(),
        old: Value::Nil,
        new: Value::Number(1.0),
    })
    .unwrap();
    let before = s.history.len();
    let mut p = Properties::new();
    for i in 1..=100 {
        p.scrub_attribute(&mut s, a, "Speed", 1.0 + f64::from(i) * 0.1);
        assert_eq!(
            s.history.len(),
            before,
            "nothing is recorded during the drag"
        );
    }
    // The world shows the live value.
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(11.0)));
    p.end_attribute_scrub(&mut s);
    assert_eq!(s.history.len(), before + 1, "one step for the whole drag");
    s.undo();
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(1.0)));
    s.redo();
    assert_eq!(s.world.attribute(a, "Speed"), Some(&Value::Number(11.0)));
    // A drag that ends where it started records nothing; a second end does nothing.
    let before = s.history.len();
    p.scrub_attribute(&mut s, a, "Speed", 5.0);
    p.scrub_attribute(&mut s, a, "Speed", 11.0);
    p.end_attribute_scrub(&mut s);
    p.end_attribute_scrub(&mut s);
    assert_eq!(s.history.len(), before);
}
