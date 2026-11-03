//! Pins and snapshots: what stays readable of a destroyed instance, and for how long.

use std::sync::Arc;

use super::classes::game;
use super::*;
use crate::dm::{Delivery, DmError, ListenerKey};
use crate::{CFrame, Vector3};

fn text(s: &str) -> Value {
    Value::String(Arc::from(s))
}

fn drain(dm: &mut DataModel) -> usize {
    let mut out: Vec<Delivery> = Vec::new();
    dm.drain_deliveries(&mut out);
    out.len()
}

#[test]
fn an_unpinned_destroy_leaves_nothing_behind() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "Door");
    dm.destroy(p).expect("destroy");
    assert_eq!(dm.snapshot_count(), 0);
    assert!(!dm.is_destroyed(p));
    assert_eq!(dm.class_of(p), Err(DmError::NoInstance(p)));
    assert_eq!(dm.name(p), Err(DmError::NoInstance(p)));
    assert_eq!(dm.get_ref(p, ids.transparency), Err(DmError::NoInstance(p)));
    assert_eq!(dm.set(p, ids.transparency, number(1.0)), Err(DmError::NoInstance(p)));
    assert_eq!(dm.destroy(p), Err(DmError::NoInstance(p)));
    assert!(!dm.pin(p), "nothing to pin");
    assert!(!dm.unpin(p));
}

#[test]
fn a_pinned_instance_reads_its_last_state_after_destroy() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p = part_in(&mut dm, &ids, folder, "Door");
    let hinge = part_in(&mut dm, &ids, p, "Hinge");
    let cframe = CFrame::new(Vector3::new(1.0, 2.0, 3.0));
    dm.set(p, ids.cframe, Value::CFrame(cframe)).expect("cframe");
    dm.set(p, ids.transparency, number(0.5)).expect("transparency");
    dm.set_attribute(p, "Team", Some(text("Red"))).expect("attribute");
    assert!(dm.pin(p));
    dm.destroy(p).expect("destroy");

    assert!(!dm.contains(p), "not live");
    assert!(dm.is_destroyed(p));
    assert_eq!(dm.snapshot_count(), 1, "only the pinned one; the hinge is gone");
    assert_eq!(dm.class_of(p), Ok(ids.part));
    assert_eq!(dm.class_name(p), Ok("Part"));
    assert_eq!(dm.is_a(p, ids.instance), Ok(true));
    assert_eq!(dm.name(p), Ok("Door"));
    assert_eq!(dm.get(p, ids.name), Ok(text("Door")));
    // Stored, default and computed values.
    assert_eq!(dm.get_ref(p, ids.transparency), Ok(&number(0.5)));
    assert_eq!(dm.number(p, ids.transparency), Ok(0.5));
    assert_eq!(dm.cframe(p, ids.cframe), Ok(cframe));
    assert_eq!(dm.bool(p, ids.anchored), Ok(false));
    assert_eq!(dm.get(p, ids.size), Ok(Value::Vector3(Vector3::new(4.0, 1.0, 2.0))));
    assert_eq!(dm.get(p, ids.position), Ok(Value::Vector3(Vector3::new(1.0, 2.0, 3.0))));
    assert_eq!(dm.get_by_name(p, "Transparency"), Ok(number(0.5)));
    assert_eq!(dm.attribute(p, "Team"), Ok(Some(&text("Red"))));
    assert_eq!(
        dm.grave(p),
        Some(("Part", "Door")),
        "pinned, so named even with nothing connected"
    );
    // No tree: no parent, no children, a full name of its own.
    assert_eq!(dm.parent(p), Ok(None));
    assert_eq!(dm.get(p, ids.parent), Ok(Value::Ref(None)));
    assert_eq!(dm.children(p).map(Iterator::count), Ok(0));
    assert_eq!(dm.child_count(p), Ok(0));
    assert_eq!(dm.descendants(p).map(Iterator::count), Ok(0));
    assert_eq!(dm.find_first_child(p, "Hinge", true), Ok(None));
    assert_eq!(dm.full_name(p), Ok("Door".to_owned()));
    assert_eq!(dm.is_descendant_of(p, folder), Ok(false));
    assert_eq!(dm.is_descendant_of(folder, p), Ok(false));
    assert_eq!(kids(&dm, folder), [], "the folder does not see it");
    // The unpinned child left nothing.
    assert_eq!(dm.name(hinge), Err(DmError::NoInstance(hinge)));
}

#[test]
fn writes_to_a_destroyed_instance_fail_clearly() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "Door");
    let other = part_in(&mut dm, &ids, ids.workspace, "Other");
    dm.pin(p);
    dm.destroy(p).expect("destroy");
    let destroyed = Err(DmError::Destroyed(p));
    assert_eq!(dm.set(p, ids.transparency, number(1.0)), destroyed);
    assert_eq!(dm.set(p, ids.position, Value::Vector3(Vector3::ONE)), destroyed);
    assert_eq!(dm.set(p, ids.name, text("Door")), destroyed, "even to the same name");
    assert_eq!(dm.set_name(p, "Door".into()), destroyed);
    assert_eq!(dm.set_parent(p, Some(ids.workspace)), destroyed);
    assert_eq!(dm.set(p, ids.parent, Value::Ref(Some(ids.workspace))), destroyed);
    assert_eq!(dm.set_parent(other, Some(p)), destroyed, "nor a new parent");
    assert_eq!(dm.set_attribute(p, "Team", Some(text("Red"))), destroyed);
    assert_eq!(dm.clone_tree(p), Err(DmError::Destroyed(p)));
    assert_eq!(dm.connect(p, ids.changed, ListenerKey(1)), Err(DmError::Destroyed(p)));
    assert_eq!(dm.fire(p, ids.changed, vec![]), destroyed);
    assert_eq!(DmError::Destroyed(p).to_string(), "the instance was destroyed");
    // Destroying again, or clearing its (no) children, does nothing, as in Roblox.
    assert_eq!(dm.destroy(p), Ok(()));
    assert_eq!(dm.destroy_children(p), Ok(()));
    assert_eq!(dm.get_ref(p, ids.transparency), Ok(&number(0.0)), "nothing was written");
}

#[test]
fn a_recycled_slot_never_reads_as_the_snapshot() {
    let (mut dm, ids) = game();
    let old = part_in(&mut dm, &ids, ids.workspace, "Old");
    dm.set(old, ids.transparency, number(0.25)).expect("set");
    dm.pin(old);
    dm.destroy(old).expect("destroy");
    let fresh = part_in(&mut dm, &ids, ids.workspace, "Fresh");
    assert_eq!(fresh.index(), old.index(), "the slot is reused");
    assert_ne!(fresh, old);

    assert_eq!(dm.name(old), Ok("Old"));
    assert_eq!(dm.number(old, ids.transparency), Ok(0.25));
    assert_eq!(dm.parent(old), Ok(None));
    assert_eq!(dm.name(fresh), Ok("Fresh"));
    assert_eq!(dm.number(fresh, ids.transparency), Ok(0.0));
    assert_eq!(dm.parent(fresh), Ok(Some(ids.workspace)));
    // Writing the new instance leaves the snapshot alone, and the other way round is refused.
    dm.set(fresh, ids.transparency, number(1.0)).expect("set");
    assert_eq!(dm.number(old, ids.transparency), Ok(0.25));
    assert_eq!(dm.set(old, ids.transparency, number(0.5)), Err(DmError::Destroyed(old)));
    assert_eq!(dm.number(fresh, ids.transparency), Ok(1.0));
    // Pins follow the generation too.
    assert!(dm.pin(fresh));
    assert!(dm.unpin(old));
    assert_eq!(dm.snapshot_count(), 0);
    assert_eq!(dm.name(old), Err(DmError::NoInstance(old)));
    assert_eq!(dm.name(fresh), Ok("Fresh"));
    assert!(dm.unpin(fresh));
}

#[test]
fn the_last_unpin_frees_the_snapshot() {
    let (mut dm, ids) = game();
    let parts: Vec<Id> = (0..10)
        .map(|i| part_in(&mut dm, &ids, ids.workspace, &format!("P{i}")))
        .collect();
    for &p in &parts {
        assert!(dm.pin(p));
        assert!(dm.pin(p));
    }
    // Pins taken and given back while alive leave nothing.
    let spare = part_in(&mut dm, &ids, ids.workspace, "Spare");
    assert!(dm.pin(spare));
    assert!(dm.unpin(spare));
    assert!(!dm.unpin(spare), "no pin left");
    dm.destroy(spare).expect("destroy");
    for &p in &parts {
        dm.destroy(p).expect("destroy");
    }
    assert_eq!(dm.snapshot_count(), 10);
    for (n, &p) in parts.iter().enumerate() {
        assert!(dm.unpin(p));
        assert_eq!(dm.snapshot_count(), 10 - n, "one pin is left");
        // A pin taken on the snapshot counts like any other.
        assert!(dm.pin(p));
        assert!(dm.unpin(p));
        assert!(dm.unpin(p));
        assert_eq!(dm.snapshot_count(), 9 - n);
        assert!(!dm.unpin(p), "nothing left to unpin");
        assert!(!dm.pin(p), "gone for good");
    }
}

#[test]
fn pins_past_a_byte_are_counted() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "Busy");
    for _ in 0..300 {
        assert!(dm.pin(p));
    }
    assert_eq!(dm.slots.get(p).map(|s| s.pins), Some(u8::MAX));
    // Give back some while it lives, crossing the byte's limit.
    for _ in 0..100 {
        assert!(dm.unpin(p));
    }
    assert_eq!(dm.slots.get(p).map(|s| s.pins), Some(200));
    for _ in 0..100 {
        assert!(dm.pin(p));
    }
    dm.destroy(p).expect("destroy");
    assert!(dm.pins.overflow.is_empty(), "the side count moved to the snapshot");
    for _ in 0..299 {
        assert!(dm.unpin(p));
    }
    assert_eq!(dm.name(p), Ok("Busy"));
    assert!(dm.unpin(p));
    assert_eq!(dm.snapshot_count(), 0);
}

#[test]
fn graves_keep_unpinned_instances_readable_until_their_events_are_delivered() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let kept = part_in(&mut dm, &ids, folder, "Kept");
    let dropped = part_in(&mut dm, &ids, folder, "Dropped");
    dm.set(dropped, ids.transparency, number(0.75)).expect("set");
    dm.connect(folder, ids.child_removed, ListenerKey(1)).expect("connect");
    dm.destroy(kept).expect("destroy");
    dm.destroy(dropped).expect("destroy");
    assert_eq!(dm.snapshot_count(), 2, "something is connected: both are graves");

    assert_eq!(drain(&mut dm), 2, "ChildRemoved twice");
    // The handlers run now: the children read as they were. One handler keeps its child.
    assert_eq!(dm.number(dropped, ids.transparency), Ok(0.75));
    assert_eq!(dm.name(kept), Ok("Kept"));
    assert!(dm.pin(kept));

    // The next drain ends the grave window: the unpinned child goes, the pinned one stays.
    drain(&mut dm);
    assert_eq!(dm.name(dropped), Err(DmError::NoInstance(dropped)));
    assert_eq!(dm.grave(dropped), None);
    assert_eq!(dm.name(kept), Ok("Kept"));
    assert_eq!(dm.snapshot_count(), 1);
    assert!(dm.unpin(kept));
    assert_eq!(dm.snapshot_count(), 0);
}

#[test]
fn unpinning_during_the_grave_window_waits_for_the_drain() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    dm.connect(p, ids.destroying, ListenerKey(1)).expect("connect");
    dm.pin(p);
    dm.destroy(p).expect("destroy");
    assert!(dm.unpin(p));
    assert_eq!(dm.name(p), Ok("P"), "its Destroying is not delivered yet");
    assert_eq!(drain(&mut dm), 1);
    assert_eq!(dm.name(p), Ok("P"), "the handler runs after the drain");
    drain(&mut dm);
    assert_eq!(dm.snapshot_count(), 0);
}

#[test]
fn every_data_model_has_its_own_uid() {
    let (a, _) = game();
    let (b, _) = game();
    assert_ne!(a.uid(), b.uid());
    assert_ne!(a.uid(), 0);
}
