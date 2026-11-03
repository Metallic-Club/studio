//! Change tracking: dirty bits per flag, drains per consumer, removals, `mark_all`.

use super::classes::game;
use super::*;
use crate::dm::{ConsumerId, DirtyBits, DmError, MAX_CONSUMERS};
use crate::{CFrame, Color3};

fn drain(dm: &mut DataModel, consumer: ConsumerId) -> Vec<(Id, DirtyBits)> {
    let mut out = Vec::new();
    dm.drain_changes(consumer, &mut out);
    out
}

fn removed(dm: &mut DataModel, consumer: ConsumerId) -> Vec<Id> {
    let mut out = Vec::new();
    dm.drain_removed(consumer, &mut out);
    out
}

#[test]
fn at_most_four_consumers() {
    let (mut dm, _) = game();
    for i in 0..MAX_CONSUMERS {
        assert_eq!(dm.register_consumer(DirtyBits::ALL).map(ConsumerId::index), Ok(i));
    }
    assert_eq!(
        dm.register_consumer(DirtyBits::ALL),
        Err(DmError::TooManyConsumers { max: MAX_CONSUMERS })
    );
}

#[test]
fn an_unregistered_consumer_records_nothing_and_its_place_is_reused() {
    let (mut dm, ids) = game();
    let render = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let host = dm.register_consumer(DirtyBits::STRUCTURE).expect("consumer");
    let kept = part_in(&mut dm, &ids, ids.workspace, "Kept");
    let gone = part_in(&mut dm, &ids, ids.workspace, "Gone");
    drain(&mut dm, host);
    dm.set_parent(kept, None).expect("move");

    dm.unregister_consumer(host);
    dm.unregister_consumer(host);
    assert!(drain(&mut dm, host).is_empty(), "what was not drained is dropped");
    dm.destroy(gone).expect("destroy");
    part(&mut dm, &ids);
    assert!(drain(&mut dm, host).is_empty());
    assert!(removed(&mut dm, host).is_empty(), "no removals either");
    assert!(
        removed(&mut dm, render).is_empty(),
        "render never drained `gone`'s creation"
    );

    // The place is reused, with no bits left over from before.
    let again = dm.register_consumer(DirtyBits::STRUCTURE).expect("consumer");
    assert_eq!(again, host);
    dm.set_parent(kept, Some(ids.workspace)).expect("move");
    assert_eq!(drain(&mut dm, again), vec![(kept, DirtyBits::STRUCTURE)]);
    for _ in 2..MAX_CONSUMERS {
        dm.register_consumer(DirtyBits::ALL).expect("consumer");
    }
    assert_eq!(
        dm.register_consumer(DirtyBits::ALL),
        Err(DmError::TooManyConsumers { max: MAX_CONSUMERS })
    );
}

#[test]
fn creating_and_parenting_marks_created_and_structure_once() {
    let (mut dm, ids) = game();
    let c = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    assert_eq!(
        drain(&mut dm, c),
        vec![(p, DirtyBits::CREATED | DirtyBits::STRUCTURE | DirtyBits::PROPERTY)]
    );
    assert!(drain(&mut dm, c).is_empty(), "drained");
}

#[test]
fn property_flags_decide_the_bits() {
    let (mut dm, ids) = game();
    let c = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let p = part(&mut dm, &ids);
    drain(&mut dm, c);
    let cases = [
        (
            ids.cframe,
            Value::CFrame(CFrame::new(crate::Vector3::X)),
            DirtyBits::TRANSFORM | DirtyBits::PHYSICS,
        ),
        (ids.color, Value::Color3(Color3::BLACK), DirtyBits::VISUAL),
        (ids.anchored, Value::Bool(true), DirtyBits::PHYSICS),
        (ids.layer, Value::Int(3), DirtyBits::NONE),
    ];
    for (prop, value, bits) in cases {
        dm.set(p, prop, value).expect("set");
        assert_eq!(drain(&mut dm, c), vec![(p, bits | DirtyBits::PROPERTY)], "{prop:?}");
    }
}

#[test]
fn two_consumers_drain_independently() {
    let (mut dm, ids) = game();
    let render = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let physics = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let p = part(&mut dm, &ids);
    let q = part(&mut dm, &ids);
    drain(&mut dm, render);
    drain(&mut dm, physics);

    dm.set(p, ids.color, Value::Color3(Color3::BLACK)).expect("set");
    assert_eq!(
        drain(&mut dm, render),
        vec![(p, DirtyBits::VISUAL | DirtyBits::PROPERTY)]
    );
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    dm.set(q, ids.anchored, Value::Bool(true)).expect("set");
    assert_eq!(
        drain(&mut dm, render),
        vec![
            (p, DirtyBits::PHYSICS | DirtyBits::PROPERTY),
            (q, DirtyBits::PHYSICS | DirtyBits::PROPERTY)
        ]
    );
    // Physics has not drained since before the first write, so it sees both, merged into one entry for `p`.
    assert_eq!(
        drain(&mut dm, physics),
        vec![
            (p, DirtyBits::VISUAL | DirtyBits::PHYSICS | DirtyBits::PROPERTY),
            (q, DirtyBits::PHYSICS | DirtyBits::PROPERTY)
        ]
    );
}

#[test]
fn interest_filters_what_a_consumer_receives() {
    let (mut dm, ids) = game();
    let render = dm
        .register_consumer(DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE)
        .expect("consumer");
    let p = part(&mut dm, &ids);
    assert_eq!(
        drain(&mut dm, render),
        vec![(p, DirtyBits::CREATED)],
        "CREATED is always kept"
    );
    dm.set(p, ids.layer, Value::Int(4)).expect("set");
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    assert!(drain(&mut dm, render).is_empty());
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    assert_eq!(drain(&mut dm, render), vec![(p, DirtyBits::VISUAL)]);
}

#[test]
fn moving_marks_the_whole_subtree() {
    let (mut dm, ids) = game();
    let c = dm.register_consumer(DirtyBits::STRUCTURE).expect("consumer");
    let model = folder_in(&mut dm, &ids, ids.workspace);
    let a = part_in(&mut dm, &ids, model, "A");
    let b = part_in(&mut dm, &ids, a, "B");
    let other = folder_in(&mut dm, &ids, ids.workspace);
    drain(&mut dm, c);
    let revision = dm.structure_revision();
    dm.set_parent(model, Some(other)).expect("move");
    assert!(dm.structure_revision() > revision);
    assert_eq!(
        drain(&mut dm, c),
        vec![
            (model, DirtyBits::STRUCTURE),
            (a, DirtyBits::STRUCTURE),
            (b, DirtyBits::STRUCTURE)
        ]
    );
}

#[test]
fn destroyed_instances_go_to_the_removed_list() {
    let (mut dm, ids) = game();
    let early = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let late = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let model = folder_in(&mut dm, &ids, ids.workspace);
    let a = part_in(&mut dm, &ids, model, "A");
    drain(&mut dm, early);

    // Changed, then destroyed before either consumer drained the change.
    dm.set(a, ids.anchored, Value::Bool(true)).expect("set");
    dm.destroy(model).expect("destroy");
    assert!(drain(&mut dm, early).is_empty(), "destroyed ids are skipped");
    assert_eq!(removed(&mut dm, early), vec![model, a]);
    // `late` never saw them created, so for it they never existed.
    assert!(drain(&mut dm, late).is_empty());
    assert!(removed(&mut dm, late).is_empty());
    assert!(removed(&mut dm, early).is_empty(), "drained");

    // A slot reused after a destroy does not inherit the old dirty state.
    let fresh = part(&mut dm, &ids);
    assert_eq!(drain(&mut dm, early), vec![(fresh, DirtyBits::CREATED)]);
}

#[test]
fn mark_all_queues_every_instance_once() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    let c = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    assert!(drain(&mut dm, c).is_empty(), "registered late: nothing yet");
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    dm.mark_all(c);
    let out = drain(&mut dm, c);
    assert_eq!(out.len(), dm.len());
    let all = DirtyBits::CREATED | DirtyBits::STRUCTURE;
    assert!(out.iter().all(|(_, bits)| bits.contains(all)));
    assert!(out.contains(&(p, all | DirtyBits::PHYSICS | DirtyBits::PROPERTY)));
}

#[test]
fn dirty_bits_debug_by_name() {
    assert_eq!(
        format!("{:?}", DirtyBits::TRANSFORM | DirtyBits::CREATED),
        "DirtyBits(TRANSFORM | CREATED)"
    );
    assert_eq!(format!("{:?}", DirtyBits::NONE), "DirtyBits(NONE)");
    assert!(DirtyBits::ALL.contains(DirtyBits::PROPERTY));
    assert!(DirtyBits::NONE.is_empty());
    assert!(DirtyBits::VISUAL.intersects(DirtyBits::ALL));
    assert_eq!(DirtyBits::STRUCTURE.bits(), 1 << 3);
}
