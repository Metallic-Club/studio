//! Reads and the write path: every validation, no-op writes, sparse storage, computed properties.

use super::classes::game;
use super::*;
use crate::dm::DmError;
use crate::{CFrame, Color3, EnumItem, PropId, PropType, Vector3};

#[test]
fn new_instances_store_nothing_and_read_defaults() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    assert_eq!(stored_count(&dm, p), 0);
    assert_eq!(dm.get_ref(p, ids.transparency), Ok(&number(0.0)));
    assert_eq!(dm.vector3(p, ids.size), Ok(Vector3::new(4.0, 1.0, 2.0)));
    assert_eq!(dm.bool(p, ids.can_collide), Ok(true));
    assert_eq!(dm.cframe(p, ids.cframe), Ok(CFrame::IDENTITY));
    assert_eq!(dm.enum_item(p, ids.material).map(EnumItem::name), Ok("Plastic"));
    assert_eq!(dm.int(p, ids.layer), Ok(0));
    assert_eq!(dm.reference(p, ids.target), Ok(None));
    assert_eq!(dm.name(p), Ok("Part"));
    assert_eq!(dm.class_name(p), Ok("Part"));
}

#[test]
fn set_stores_and_writing_the_default_removes_the_entry() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    assert_eq!(stored_count(&dm, p), 2);
    assert_eq!(dm.number(p, ids.transparency), Ok(0.5));
    dm.set(p, ids.transparency, number(0.0)).expect("back to default");
    assert_eq!(stored_count(&dm, p), 1);
    assert_eq!(dm.number(p, ids.transparency), Ok(0.0));
}

#[test]
fn set_rejects_the_wrong_type() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let err = dm
        .set(p, ids.transparency, Value::String("x".into()))
        .expect_err("type");
    assert_eq!(err.to_string(), "Transparency expects number, got string");
    let err = dm.set(p, ids.layer, number(1.0)).expect_err("int is strict");
    assert!(matches!(
        err,
        DmError::TypeMismatch {
            expected: PropType::Int,
            ..
        }
    ));
    let wrong_enum = Value::Enum(EnumItem::new("Shape", "Ball"));
    let err = dm.set(p, ids.material, wrong_enum).expect_err("enum type");
    assert_eq!(err.to_string(), "Material expects Enum.Material, got Enum.Shape");
    assert_eq!(stored_count(&dm, p), 0);
}

#[test]
fn set_rejects_nan_and_infinity() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let cases = [
        (ids.transparency, number(f64::NAN)),
        (ids.transparency, number(f64::INFINITY)),
        (ids.size, Value::Vector3(Vector3::new(1.0, f32::NAN, 1.0))),
        (
            ids.cframe,
            Value::CFrame(CFrame::new(Vector3::new(0.0, f32::INFINITY, 0.0))),
        ),
        (ids.color, Value::Color3(Color3::new(f32::NAN, 0.0, 0.0))),
    ];
    for (prop, value) in cases {
        let err = dm.set(p, prop, value.clone()).expect_err("non-finite");
        assert!(matches!(err, DmError::NotFinite { .. }), "{value:?}: {err}");
    }
    assert_eq!(stored_count(&dm, p), 0);
}

#[test]
fn set_clamps_to_the_declared_range() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    dm.set(p, ids.transparency, number(2.0)).expect("set");
    assert_eq!(dm.number(p, ids.transparency), Ok(1.0));
    dm.set(p, ids.transparency, number(-3.0)).expect("set");
    assert_eq!(dm.number(p, ids.transparency), Ok(0.0));
    assert_eq!(stored_count(&dm, p), 0, "clamped to the default, so not stored");
    dm.set(p, ids.layer, Value::Int(99)).expect("set");
    assert_eq!(dm.int(p, ids.layer), Ok(31));
    dm.set(p, ids.size, Value::Vector3(Vector3::new(0.0, 5000.0, 3.0)))
        .expect("set");
    assert_eq!(dm.vector3(p, ids.size), Ok(Vector3::new(0.05, 2048.0, 3.0)));
}

#[test]
fn set_checks_enum_items_exist() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let err = dm
        .set(p, ids.material, Value::Enum(EnumItem::new("Material", "Cheese")))
        .expect_err("unknown item");
    assert_eq!(
        err.to_string(),
        "Material expects an item of Enum.Material, and Enum.Material.Cheese does not exist"
    );
    dm.set(p, ids.material, Value::Enum(EnumItem::new("Material", "Neon")))
        .expect("set");
    assert_eq!(dm.enum_item(p, ids.material).map(EnumItem::name), Ok("Neon"));
}

#[test]
fn set_checks_reference_targets() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let other = part(&mut dm, &ids);
    let folder = dm.create(ids.folder).expect("folder");
    let err = dm
        .set(p, ids.target, Value::Ref(Some(folder)))
        .expect_err("wrong class");
    assert_eq!(err.to_string(), "Target expects Part, got Folder");
    dm.set(p, ids.target, Value::Ref(Some(other))).expect("a part is fine");
    assert_eq!(dm.reference(p, ids.target), Ok(Some(other)));
    let gone = part(&mut dm, &ids);
    dm.destroy(gone).expect("destroy");
    let err = dm
        .set(p, ids.target, Value::Ref(Some(gone)))
        .expect_err("destroyed target");
    assert_eq!(err, DmError::DestroyedRef { prop: "Target" });
    dm.set(p, ids.target, Value::Ref(None)).expect("nil");
    assert_eq!(stored_count(&dm, p), 0);
}

#[test]
fn references_to_destroyed_instances_read_nil() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let target = part(&mut dm, &ids);
    dm.set(p, ids.target, Value::Ref(Some(target))).expect("set");
    dm.destroy(target).expect("destroy");
    assert_eq!(dm.get_ref(p, ids.target), Ok(&Value::Ref(None)));
    assert_eq!(dm.get(p, ids.target), Ok(Value::Ref(None)));
    assert_eq!(dm.reference(p, ids.target), Ok(None));
    // Writing nil over it is a no-op: it already reads nil.
    take_notices(&mut dm);
    dm.set(p, ids.target, Value::Ref(None)).expect("set nil");
    assert!(take_notices(&mut dm).is_empty());
}

#[test]
fn equal_writes_do_nothing() {
    let (mut dm, ids) = game();
    let consumer = dm.register_consumer(crate::DirtyBits::ALL).expect("consumer");
    let p = part(&mut dm, &ids);
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    let mut out = Vec::new();
    dm.drain_changes(consumer, &mut out);
    take_notices(&mut dm);
    out.clear();

    dm.set(p, ids.anchored, Value::Bool(true)).expect("same value");
    dm.set(p, ids.transparency, number(0.0)).expect("same as default");
    dm.set(p, ids.transparency, number(-1.0))
        .expect("clamps to the current value");
    dm.drain_changes(consumer, &mut out);
    assert!(out.is_empty(), "{out:?}");
    assert!(take_notices(&mut dm).is_empty());
}

#[test]
fn writes_report_the_change() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    take_notices(&mut dm);
    dm.set(p, ids.color, Value::Color3(Color3::WHITE)).expect("set");
    assert_eq!(
        take_notices(&mut dm),
        vec![Notice::PropertyChanged { id: p, prop: ids.color }]
    );
}

#[test]
fn computed_properties_get_and_set() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    assert_eq!(dm.get(p, ids.name), Ok(Value::String("Part".into())));
    dm.set(p, ids.name, Value::String("Door".into())).expect("rename");
    assert_eq!(dm.name(p), Ok("Door"));
    assert_eq!(dm.get_by_name(p, "Name"), Ok(Value::String("Door".into())));

    dm.set(p, ids.position, Value::Vector3(Vector3::new(1.0, 2.0, 3.0)))
        .expect("position");
    assert_eq!(
        dm.cframe(p, ids.cframe).map(CFrame::position),
        Ok(Vector3::new(1.0, 2.0, 3.0))
    );
    assert_eq!(dm.vector3(p, ids.position), Ok(Vector3::new(1.0, 2.0, 3.0)));

    dm.set(p, ids.parent, Value::Ref(Some(ids.workspace))).expect("parent");
    assert_eq!(dm.parent(p), Ok(Some(ids.workspace)));
    assert_eq!(dm.get(p, ids.parent), Ok(Value::Ref(Some(ids.workspace))));

    assert_eq!(dm.number(p, ids.mass), Ok(8.0));
    let err = dm.set(p, ids.mass, number(1.0)).expect_err("no setter");
    assert_eq!(err.to_string(), "Mass of Part cannot be written");
    let err = dm.get_ref(p, ids.position).expect_err("computed");
    assert!(matches!(err, DmError::ComputedRead { prop: "Position", .. }));
    let err = dm.set(p, ids.position, number(1.0)).expect_err("type checked first");
    assert!(matches!(err, DmError::TypeMismatch { .. }));
    let err = dm
        .set(p, ids.parent, Value::Ref(Some(p)))
        .expect_err("cycle via setter");
    assert!(err.to_string().contains("descendants"), "{err}");
}

#[test]
fn unknown_properties_and_stale_ids_are_refused() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let bogus = PropId::new(500);
    let err = dm.get_ref(p, bogus).expect_err("unknown id");
    assert_eq!(err.to_string(), "#500 is not a valid property of Part");
    let err = dm.set_by_name(p, "Colour", number(1.0)).expect_err("unknown name");
    assert_eq!(err.to_string(), "Colour is not a valid property of Part");
    let folder = dm.create(ids.folder).expect("folder");
    assert!(
        dm.get_ref(folder, ids.transparency).is_err(),
        "Folder has no Transparency"
    );

    dm.destroy(p).expect("destroy");
    for result in [
        dm.get_ref(p, ids.transparency).map(|_| ()),
        dm.number(p, ids.transparency).map(|_| ()),
        dm.set(p, ids.transparency, number(0.5)),
        dm.get(p, ids.name).map(|_| ()),
    ] {
        assert_eq!(result, Err(DmError::NoInstance(p)));
    }
}

#[test]
fn typed_getters_check_the_declared_type() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let err = dm.number(p, ids.anchored).expect_err("bool, not number");
    assert_eq!(err.to_string(), "Anchored expects number, got boolean");
    assert!(dm.string(p, ids.anchored).is_err());
    assert!(dm.enum_item(p, ids.size).is_err());
    let folder = dm.create(ids.folder).expect("folder");
    dm.set(folder, ids.note, Value::String("hello".into())).expect("set");
    assert_eq!(dm.string(folder, ids.note), Ok("hello"));
    assert_eq!(dm.color3(p, ids.color), Ok(Color3::new(0.64, 0.64, 0.64)));
    assert!(dm.vector2(p, ids.size).is_err());
}

/// A root with a wrapping number (`Hour`, 0 to 24), a wrapping int (`Step`, 1 to 5) and a clamped number.
fn dial() -> (DataModel, Id) {
    use crate::{ClassDesc, PropMeta, Registry, RegistryInput};
    use std::sync::Arc;

    static DIAL: ClassDesc = ClassDesc::new("Dial", None).props(&[
        PropMeta::number("Hour", 14.0).wrap(0.0, 24.0),
        PropMeta::int("Step", 2).wrap(1.0, 5.0),
        PropMeta::number("Level", 1.0).range(0.0, 2.0),
    ]);
    let registry = Registry::build(RegistryInput {
        classes: &[&DIAL],
        ..RegistryInput::default()
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let class = registry.class_id("Dial").expect("class");
    let mut dm = DataModel::new(Arc::new(registry));
    let dial = dm.create_root(class).expect("root");
    (dm, dial)
}

#[test]
fn set_wraps_a_wrapping_number_into_its_range() {
    let (mut dm, dial) = dial();
    let hour = |dm: &DataModel| match dm.get_by_name(dial, "Hour") {
        Ok(Value::Number(n)) => n,
        other => panic!("{other:?}"),
    };
    for (written, read) in [
        (6.5, 6.5),
        (24.0, 0.0),
        (25.0, 1.0),
        (-1.0, 23.0),
        (-24.0, 0.0),
        (49.5, 1.5),
        (-0.5, 23.5),
    ] {
        dm.set_by_name(dial, "Hour", number(written)).expect("set");
        assert_eq!(hour(&dm), read, "{written}");
    }
    // A negative value so small that the sum rounds to the span itself must not escape the range.
    dm.set_by_name(dial, "Hour", number(-1.0e-20)).expect("set");
    assert!((0.0..24.0).contains(&hour(&dm)), "{}", hour(&dm));
    // Plain ranges still clamp.
    dm.set_by_name(dial, "Level", number(9.0)).expect("set");
    assert_eq!(dm.get_by_name(dial, "Level"), Ok(number(2.0)));
}

#[test]
fn set_wraps_a_wrapping_int_and_stores_nothing_for_the_default() {
    let (mut dm, dial) = dial();
    for (written, read) in [(5, 1), (6, 2), (0, 4), (-3, 1)] {
        dm.set_by_name(dial, "Step", Value::Int(written)).expect("set");
        assert_eq!(dm.get_by_name(dial, "Step"), Ok(Value::Int(read)), "{written}");
    }
    // The extremes wrap without overflowing and stay in range.
    for written in [i64::MAX, i64::MIN] {
        dm.set_by_name(dial, "Step", Value::Int(written)).expect("set");
        assert!(matches!(dm.get_by_name(dial, "Step"), Ok(Value::Int(1..=4))));
    }
    // Wrapping to the default removes the stored entry, like any write of the default.
    dm.set_by_name(dial, "Step", Value::Int(2)).expect("set");
    dm.set_by_name(dial, "Hour", number(14.0 + 24.0)).expect("set");
    assert_eq!(stored_count(&dm, dial), 0);
}
