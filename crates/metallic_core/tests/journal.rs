//! The change journal read through `&World`.
use metallic_core::{Changes, Value, Vec3, World};

fn ids(c: Changes) -> Vec<metallic_core::InstanceId> {
    match c {
        Changes::Ids(v) => v,
        Changes::Overflow => panic!("unexpected overflow"),
    }
}

#[test]
fn changes_since_reports_set_and_create_and_destroy() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let a = w.create("Part", Some(ws)).unwrap();
    let cur = w.change_cursor();

    let b = w.create("Part", Some(ws)).unwrap();
    w.set(a, "Position", Value::Vector3(Vec3::new(1.0, 2.0, 3.0)))
        .unwrap();
    w.set(a, "Position", Value::Vector3(Vec3::new(4.0, 2.0, 3.0)))
        .unwrap();
    let (changes, cur2) = w.changes_since(cur);
    assert_eq!(ids(changes), vec![b, a], "deduplicated, first-change order");

    w.destroy(b);
    let (changes, _) = w.changes_since(cur2);
    assert_eq!(ids(changes), vec![b]);

    // Reparenting reports the instance and its descendants.
    let m = w.create("Model", Some(ws)).unwrap();
    let kid = w.create("Part", Some(m)).unwrap();
    let cur3 = w.change_cursor();
    w.set_parent(m, None).unwrap();
    assert_eq!(ids(w.changes_since(cur3).0), vec![m, kid]);
}

#[test]
fn changes_since_empty_when_nothing_changed() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let a = w.create("Part", Some(ws)).unwrap();
    let cur = w.change_cursor();
    assert_eq!(w.changes_since(cur).0, Changes::Ids(vec![]));
    // Setting the same value is not a change.
    w.set(a, "Anchored", Value::Bool(false)).unwrap();
    assert_eq!(w.changes_since(cur).0, Changes::Ids(vec![]));
}

#[test]
fn journal_overflow_and_foreign_cursor_are_reported() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let a = w.create("Part", Some(ws)).unwrap();
    let old = w.change_cursor();
    for i in 0..40_000 {
        w.set(
            a,
            "Transparency",
            Value::Number(f64::from(i % 7) / 10.0 + 0.01),
        )
        .unwrap();
    }
    assert_eq!(w.changes_since(old).0, Changes::Overflow);
    let fresh = w.change_cursor();
    w.set(a, "Transparency", Value::Number(0.9)).unwrap();
    assert_eq!(ids(w.changes_since(fresh).0), vec![a]);
    let other = World::new_with_services();
    assert_eq!(w.changes_since(other.change_cursor()).0, Changes::Overflow);
}

#[test]
fn property_changes_since_lists_each_real_write_in_order() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let a = w.create("Part", Some(ws)).unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    let start = w.change_cursor();

    w.set(a, "Transparency", Value::Number(0.5)).unwrap();
    w.set(a, "Transparency", Value::Number(0.5)).unwrap(); // same value: not a change
    w.set(a, "Transparency", Value::Number(0.0)).unwrap(); // back again: a change
    w.set_parent(a, Some(m)).unwrap(); // hierarchy: not a property change
    w.set(m, "ScaleFactor", Value::Number(2.0)).unwrap(); // hidden, still reported
    let changes: Vec<_> = w
        .property_changes_since(start)
        .expect("in range")
        .map(|c| (c.id, c.prop.name, c.prop.hidden))
        .collect();
    assert_eq!(
        changes,
        vec![
            (a, "Transparency", false),
            (a, "Transparency", false),
            (m, "ScaleFactor", true),
        ]
    );
    let all: Vec<_> = w.property_changes_since(start).unwrap().collect();
    assert!(all[0].at > start && all[1].at > all[0].at);
    assert!(
        w.property_changes_since(w.change_cursor())
            .unwrap()
            .next()
            .is_none()
    );

    let other = World::new_with_services();
    assert!(w.property_changes_since(other.change_cursor()).is_none());
    assert_eq!(other.change_cursor().partial_cmp(&start), None);
    for i in 0..40_000 {
        w.set(
            a,
            "Transparency",
            Value::Number(f64::from(i % 7) / 10.0 + 0.01),
        )
        .unwrap();
    }
    assert!(w.property_changes_since(start).is_none(), "overflow");
}
