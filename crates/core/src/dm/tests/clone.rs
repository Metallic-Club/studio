//! `clone_tree` and attributes.

use super::classes::game;
use super::*;
use crate::dm::DmError;
use crate::{Color3, Vector3};

#[test]
fn clone_copies_the_subtree_and_remaps_internal_references() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let outside = part_in(&mut dm, &ids, ws, "Outside");
    let model = folder_in(&mut dm, &ids, ws);
    dm.set_name(model, "Model".into()).expect("name");
    let a = part_in(&mut dm, &ids, model, "A");
    let b = part_in(&mut dm, &ids, model, "B");
    dm.set(a, ids.color, Value::Color3(Color3::WHITE)).expect("set");
    dm.set(a, ids.target, Value::Ref(Some(b))).expect("internal ref");
    dm.set(b, ids.target, Value::Ref(Some(outside))).expect("external ref");
    dm.set(model, ids.link, Value::Ref(Some(a))).expect("folder ref");
    dm.set_attribute(a, "Health", Some(Value::Number(100.0)))
        .expect("attribute");

    let copy = dm.clone_tree(model).expect("clone").expect("archivable");
    assert_ne!(copy, model);
    assert_eq!(dm.parent(copy), Ok(None));
    assert_eq!(dm.name(copy), Ok("Model"));
    let copies = kids(&dm, copy);
    assert_eq!(copies.len(), 2);
    let (a2, b2) = (copies[0], copies[1]);
    assert!(![a, b].contains(&a2) && ![a, b].contains(&b2));
    assert_eq!(dm.name(a2), Ok("A"));
    assert_eq!(dm.name(b2), Ok("B"));
    assert_eq!(dm.color3(a2, ids.color), Ok(Color3::WHITE));
    assert_eq!(
        dm.reference(a2, ids.target),
        Ok(Some(b2)),
        "internal reference remapped"
    );
    assert_eq!(
        dm.reference(b2, ids.target),
        Ok(Some(outside)),
        "external reference kept"
    );
    assert_eq!(dm.reference(copy, ids.link), Ok(Some(a2)));
    assert_eq!(dm.attribute(a2, "Health"), Ok(Some(&Value::Number(100.0))));
    // The original is untouched.
    assert_eq!(dm.reference(a, ids.target), Ok(Some(b)));
    assert_eq!(kids(&dm, model), [a, b]);
}

#[test]
fn clone_skips_non_archivable_instances() {
    let (mut dm, ids) = game();
    let model = folder_in(&mut dm, &ids, ids.workspace);
    let kept = part_in(&mut dm, &ids, model, "Kept");
    let skipped = folder_in(&mut dm, &ids, model);
    let _under_skipped = part_in(&mut dm, &ids, skipped, "Under");
    let after = part_in(&mut dm, &ids, model, "After");
    dm.set(skipped, ids.archivable, Value::Bool(false)).expect("set");
    dm.set(kept, ids.target, Value::Ref(Some(after))).expect("ref");

    let copy = dm.clone_tree(model).expect("clone").expect("archivable");
    let names: Vec<&str> = dm
        .descendants(copy)
        .expect("walk")
        .map(|x| dm.name(x).expect("name"))
        .collect();
    assert_eq!(names, vec!["Kept", "After"]);
    let [kept_copy, after_copy] = kids(&dm, copy)[..] else {
        panic!("two copies expected");
    };
    assert_eq!(dm.reference(kept_copy, ids.target), Ok(Some(after_copy)));

    dm.set(model, ids.archivable, Value::Bool(false)).expect("set");
    assert_eq!(dm.clone_tree(model), Ok(None));
}

#[test]
fn clone_drops_references_to_destroyed_instances_and_refuses_root_and_services() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let gone = part(&mut dm, &ids);
    dm.set(p, ids.target, Value::Ref(Some(gone))).expect("ref");
    dm.set(p, ids.size, Value::Vector3(Vector3::ONE)).expect("size");
    dm.destroy(gone).expect("destroy");
    let copy = dm.clone_tree(p).expect("clone").expect("archivable");
    assert_eq!(dm.reference(copy, ids.target), Ok(None));
    assert_eq!(stored_count(&dm, copy), 1, "only Size is stored");
    assert_eq!(dm.clone_tree(ids.game), Err(DmError::IsRoot { class: "Game" }));
    assert_eq!(
        dm.clone_tree(ids.workspace),
        Err(DmError::IsService { class: "Workspace" })
    );
    assert_eq!(dm.clone_tree(gone), Err(DmError::NoInstance(gone)));
}

#[test]
fn attributes_set_get_and_remove() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    assert_eq!(dm.attributes(p), Ok(&[][..]));
    assert_eq!(dm.attribute(p, "Speed"), Ok(None));
    dm.set_attribute(p, "Speed", Some(Value::Number(16.0))).expect("set");
    dm.set_attribute(p, "Team", Some(Value::String("Red".into())))
        .expect("set");
    assert_eq!(dm.attribute(p, "Speed"), Ok(Some(&Value::Number(16.0))));
    let names: Vec<&str> = dm.attributes(p).expect("list").iter().map(|(n, _)| &**n).collect();
    assert_eq!(names, vec!["Speed", "Team"]);

    take_notices(&mut dm);
    dm.set_attribute(p, "Speed", Some(Value::Number(16.0))).expect("same");
    dm.set_attribute(p, "Missing", None).expect("remove missing");
    assert!(take_notices(&mut dm).is_empty());
    dm.set_attribute(p, "Speed", Some(Value::Number(20.0))).expect("change");
    dm.set_attribute(p, "Speed", None).expect("remove");
    assert_eq!(
        take_notices(&mut dm),
        vec![
            Notice::AttributeChanged {
                id: p,
                name: "Speed".into()
            },
            Notice::AttributeChanged {
                id: p,
                name: "Speed".into()
            },
        ]
    );
    assert_eq!(dm.attribute(p, "Speed"), Ok(None));
    dm.set_attribute(p, "Team", None).expect("remove");
    assert!(
        dm.slots.body(p).is_some_and(|b| !b.has_extras()),
        "the empty extras box is freed"
    );

    let err = dm.set_attribute(p, "Friend", Some(Value::Ref(None))).expect_err("ref");
    assert_eq!(err.to_string(), "attribute Friend cannot hold an instance reference");
    let err = dm
        .set_attribute(p, "Bad", Some(Value::Number(f64::NAN)))
        .expect_err("nan");
    assert!(matches!(err, DmError::NotFinite { .. }));
    dm.destroy(p).expect("destroy");
    assert_eq!(dm.attribute(p, "Team"), Err(DmError::NoInstance(p)));
}
