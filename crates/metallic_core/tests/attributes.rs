//! Attributes: names, values, change log, cloning, scene files.

use metallic_core::*;

mod common;
use common::TestDir;

fn world_with_part() -> (World, InstanceId) {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let p = w.create("Part", Some(ws)).unwrap();
    (w, p)
}

#[test]
fn set_get_remove_and_list_sorted() {
    let (mut w, p) = world_with_part();
    assert_eq!(w.attribute(p, "Speed"), None);
    w.set_attribute(p, "Speed", Value::Number(12.5)).unwrap();
    w.set_attribute(p, "Label", Value::String("hi".into()))
        .unwrap();
    w.set_attribute(p, "Alive", Value::Bool(true)).unwrap();
    assert_eq!(w.attribute(p, "Speed"), Some(&Value::Number(12.5)));
    let names: Vec<&str> = w.attributes(p).map(|(n, _)| n).collect();
    assert_eq!(names, ["Alive", "Label", "Speed"]);
    w.set_attribute(p, "Speed", Value::Nil).unwrap();
    assert_eq!(w.attribute(p, "Speed"), None);
    // Removing something that is not there is fine.
    w.set_attribute(p, "Speed", Value::Nil).unwrap();
    w.set_attribute(p, "Label", Value::Nil).unwrap();
    w.set_attribute(p, "Alive", Value::Nil).unwrap();
    assert_eq!(w.attributes(p).count(), 0);
}

#[test]
fn every_supported_value_type_is_stored() {
    let (mut w, p) = world_with_part();
    let values = [
        Value::Bool(false),
        Value::Number(-3.0),
        Value::String("text".into()),
        Value::Vector3(Vec3::new(1.0, 2.0, 3.0)),
        Value::Color3(Color3::new(0.1, 0.2, 0.3)),
        Value::UDim2(UDim2::new(0.5, 4.0, 1.0, -2.0)),
        Value::CFrame(Box::new(CFrame::IDENTITY)),
    ];
    for (i, v) in values.iter().enumerate() {
        let name = format!("A{i}");
        w.set_attribute(p, &name, v.clone()).unwrap();
        assert_eq!(w.attribute(p, &name), Some(v));
    }
}

#[test]
fn invalid_names_are_refused() {
    let (mut w, p) = world_with_part();
    let long = "a".repeat(101);
    for bad in [
        "",
        "with space",
        "dash-ed",
        "dot.ted",
        "ünï",
        "RBXTest",
        "rbxlower",
        "RbXMixed",
        &long,
    ] {
        let err = w.set_attribute(p, bad, Value::Number(1.0)).unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidValue { .. }),
            "{bad}: {err}"
        );
    }
    assert_eq!(w.attributes(p).count(), 0);
    // The boundary: 100 characters, underscores, digits, and "RB" or "XRB" are fine.
    for ok in ["_x_1", "9lives", "RB", "XRBX", "aRBX"] {
        w.set_attribute(p, ok, Value::Bool(true)).unwrap();
    }
    w.set_attribute(p, &"a".repeat(100), Value::Bool(true))
        .unwrap();
    assert!(w.set_attribute(p, "RBX", Value::Bool(true)).is_err());
}

#[test]
fn invalid_values_are_refused() {
    let (mut w, p) = world_with_part();
    assert!(w.set_attribute(p, "A", Value::Ref(None)).is_err());
    assert!(w.set_attribute(p, "A", Value::Ref(Some(p))).is_err());
    assert!(w.set_attribute(p, "A", Value::Number(f64::NAN)).is_err());
    assert!(
        w.set_attribute(p, "A", Value::Vector3(Vec3::new(f64::INFINITY, 0.0, 0.0)))
            .is_err()
    );
    assert!(
        w.set_attribute(p, "A", Value::String("x".repeat(MAX_ATTRIBUTE_STRING + 1)))
            .is_err()
    );
    w.set_attribute(p, "A", Value::String("x".repeat(MAX_ATTRIBUTE_STRING)))
        .unwrap();
    assert_eq!(w.attributes(p).count(), 1);
}

#[test]
fn an_instance_holds_a_bounded_number_of_attributes() {
    let (mut w, p) = world_with_part();
    for i in 0..MAX_ATTRIBUTES {
        w.set_attribute(p, &format!("A{i}"), Value::Number(i as f64))
            .unwrap();
    }
    assert!(w.set_attribute(p, "OneMore", Value::Bool(true)).is_err());
    // Changing or removing existing ones still works.
    w.set_attribute(p, "A0", Value::Number(-1.0)).unwrap();
    w.set_attribute(p, "A1", Value::Nil).unwrap();
    w.set_attribute(p, "OneMore", Value::Bool(true)).unwrap();
}

#[test]
fn dead_instances_have_no_attributes_and_reject_writes() {
    let (mut w, p) = world_with_part();
    w.set_attribute(p, "A", Value::Bool(true)).unwrap();
    w.destroy(p);
    assert_eq!(w.attribute(p, "A"), None);
    assert_eq!(w.attributes(p).count(), 0);
    assert_eq!(
        w.set_attribute(p, "A", Value::Bool(true)),
        Err(CoreError::InvalidInstance)
    );
}

#[test]
fn changes_are_logged_once_each_and_equal_writes_are_not() {
    let (mut w, p) = world_with_part();
    let q = w
        .create("Part", Some(w.root("Workspace").unwrap()))
        .unwrap();
    let cursor = w.attribute_cursor();
    let journal = w.change_cursor();
    w.set_attribute(p, "A", Value::Number(1.0)).unwrap();
    w.set_attribute(p, "A", Value::Number(1.0)).unwrap(); // same value: nothing
    w.set_attribute(q, "B", Value::Bool(true)).unwrap();
    w.set_attribute(p, "A", Value::Number(2.0)).unwrap();
    w.set_attribute(p, "A", Value::Nil).unwrap();
    w.set_attribute(p, "A", Value::Nil).unwrap(); // already gone: nothing
    let (changes, next) = w.attribute_changes_since(cursor);
    let AttributeChanges::Changes(list) = changes else {
        panic!("overflow")
    };
    let seen: Vec<(InstanceId, &str)> = list.iter().map(|c| (c.id, c.name.as_str())).collect();
    assert_eq!(seen, [(p, "A"), (q, "B"), (p, "A"), (p, "A")]);
    // An attribute change marks the instance changed in the journal.
    let (ids, _) = w.changes_since(journal);
    assert_eq!(
        ids,
        Changes::Ids(vec![p, q]),
        "both instances show up as changed"
    );
    // Nothing new since `next`.
    let (again, _) = w.attribute_changes_since(next);
    assert_eq!(again, AttributeChanges::Changes(Vec::new()));
    // A cursor of another world overflows.
    let other = World::new_with_services();
    assert_eq!(
        w.attribute_changes_since(other.attribute_cursor()).0,
        AttributeChanges::Overflow
    );
}

#[test]
fn the_log_is_bounded_and_overflows_old_cursors() {
    let (mut w, p) = world_with_part();
    let old = w.attribute_cursor();
    for i in 0..10_000 {
        w.set_attribute(p, "N", Value::Number(f64::from(i)))
            .unwrap();
    }
    assert_eq!(w.attribute_changes_since(old).0, AttributeChanges::Overflow);
    let fresh = w.attribute_cursor();
    w.set_attribute(p, "N", Value::Number(-1.0)).unwrap();
    assert!(matches!(
        w.attribute_changes_since(fresh).0,
        AttributeChanges::Changes(c) if c.len() == 1
    ));
}

#[test]
fn clones_copy_attributes_and_deep_clone_keeps_them() {
    let (mut w, p) = world_with_part();
    w.set_attribute(p, "Team", Value::String("Red".into()))
        .unwrap();
    let copy = w.clone_subtree(p).unwrap();
    assert_eq!(
        w.attribute(copy, "Team"),
        Some(&Value::String("Red".into()))
    );
    // Independent afterwards.
    w.set_attribute(copy, "Team", Value::String("Blue".into()))
        .unwrap();
    assert_eq!(w.attribute(p, "Team"), Some(&Value::String("Red".into())));
    let snapshot = w.deep_clone();
    assert_eq!(
        snapshot.attribute(p, "Team"),
        Some(&Value::String("Red".into()))
    );
}

#[test]
fn scene_round_trip_keeps_every_attribute_type() {
    let dir = TestDir::new("attributes_round_trip");
    let (mut w, p) = world_with_part();
    let model = w
        .create("Model", Some(w.root("Workspace").unwrap()))
        .unwrap();
    let values = [
        ("B", Value::Bool(true)),
        ("N", Value::Number(2.5)),
        ("S", Value::String("quote \" and\nnewline".into())),
        ("V", Value::Vector3(Vec3::new(1.0, -2.0, 3.5))),
        ("C", Value::Color3(Color3::new(0.25, 0.5, 0.75))),
        ("U", Value::UDim2(UDim2::new(0.5, 10.0, 0.25, -4.0))),
        ("F", Value::CFrame(Box::new(CFrame::IDENTITY))),
    ];
    for (name, v) in &values {
        w.set_attribute(p, name, v.clone()).unwrap();
    }
    w.set_attribute(model, "OnModel", Value::Number(1.0))
        .unwrap();
    let a = dir.file("attr_a.ron");
    let b = dir.file("attr_b.ron");
    save_scene(&w, &a).unwrap();
    let w2 = load_scene(&a).unwrap();
    save_scene(&w2, &b).unwrap();
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        std::fs::read_to_string(&b).unwrap()
    );
    let ws = w2.root("Workspace").unwrap();
    let kids = w2.children(ws);
    let p2 = kids
        .iter()
        .copied()
        .find(|&k| w2.class_name(k) == Some("Part"))
        .unwrap();
    for (name, v) in &values {
        assert_eq!(w2.attribute(p2, name), Some(v), "{name}");
    }
    let m2 = kids
        .iter()
        .copied()
        .find(|&k| w2.class_name(k) == Some("Model"))
        .unwrap();
    assert_eq!(w2.attribute(m2, "OnModel"), Some(&Value::Number(1.0)));
}

#[test]
fn scenes_without_attributes_look_as_before_and_old_files_load() {
    let dir = TestDir::new("attributes_backward");
    let (w, _) = world_with_part();
    let path = dir.file("plain.ron");
    save_scene(&w, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("attributes"),
        "no attribute block unless used"
    );
    // A file written before attributes existed has no such field and loads.
    let loaded = load_scene(&path).unwrap();
    let ws = loaded.root("Workspace").unwrap();
    assert_eq!(loaded.children(ws).len(), 1);
}

#[test]
fn a_scene_with_a_reference_attribute_is_refused() {
    let dir = TestDir::new("attributes_ref");
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
                (
                    class: "Part",
                    attributes: [
                        ("Target", Ref(Some(0))),
                    ],
                ),
            ],
        ),
    ],
)"#,
    )
    .unwrap();
    assert!(matches!(load_scene(&path), Err(CoreError::Scene(_))));
}
