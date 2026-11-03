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
fn at_most_max_consumers() {
    assert_eq!(MAX_CONSUMERS, 8);
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

/// A registered consumer and what it should drain next: every bit recorded since its last drain, within its
/// interest.
struct Expected {
    id: ConsumerId,
    interest: DirtyBits,
    pending: Vec<(Id, DirtyBits)>,
}

impl Expected {
    fn register(dm: &mut DataModel, interest: DirtyBits) -> Self {
        let id = dm.register_consumer(interest).expect("consumer");
        Self {
            id,
            interest: interest | DirtyBits::CREATED,
            pending: Vec::new(),
        }
    }

    /// Records that `bits` happened to `id`, merging into its pending entry like the data model does.
    fn record(&mut self, id: Id, bits: DirtyBits) {
        let bits = bits & self.interest;
        if bits.is_empty() {
            return;
        }
        match self.pending.iter_mut().find(|(p, _)| *p == id) {
            Some((_, b)) => *b |= bits,
            None => self.pending.push((id, bits)),
        }
    }

    /// Drains the consumer and checks it got exactly what was pending.
    fn check(&mut self, dm: &mut DataModel, label: &str) {
        assert_eq!(
            drain(dm, self.id),
            std::mem::take(&mut self.pending),
            "{label}: {:?}",
            self.id
        );
    }
}

#[test]
fn a_full_game_and_three_more_drain_independently() {
    let (mut dm, ids) = game();
    let interests = [
        // Render, physics, the script host, Studio, replication.
        DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE,
        DirtyBits::TRANSFORM | DirtyBits::PHYSICS | DirtyBits::STRUCTURE,
        DirtyBits::STRUCTURE,
        DirtyBits::ALL,
        DirtyBits::ALL,
        // Three more, each caring about one kind of change.
        DirtyBits::VISUAL,
        DirtyBits::PHYSICS,
        DirtyBits::PROPERTY,
    ];
    assert_eq!(interests.len(), MAX_CONSUMERS);
    let mut consumers: Vec<Expected> = interests.iter().map(|&i| Expected::register(&mut dm, i)).collect();
    let indices: Vec<usize> = consumers.iter().map(|c| c.id.index()).collect();
    assert_eq!(indices, (0..MAX_CONSUMERS).collect::<Vec<_>>());
    assert_eq!(
        dm.register_consumer(DirtyBits::ALL),
        Err(DmError::TooManyConsumers { max: MAX_CONSUMERS })
    );
    let record = |consumers: &mut [Expected], id: Id, bits: DirtyBits| {
        for c in consumers {
            c.record(id, bits);
        }
    };

    let created = DirtyBits::CREATED | DirtyBits::STRUCTURE | DirtyBits::PROPERTY;
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    let q = part_in(&mut dm, &ids, ids.workspace, "Q");
    record(&mut consumers, p, created);
    record(&mut consumers, q, created);
    for c in &mut consumers {
        c.check(&mut dm, "created");
    }

    // Writes between drains, with only some consumers draining in between: each keeps its own bits.
    dm.set(p, ids.color, Value::Color3(Color3::BLACK)).expect("set");
    record(&mut consumers, p, DirtyBits::VISUAL | DirtyBits::PROPERTY);
    for c in consumers.iter_mut().step_by(2) {
        c.check(&mut dm, "after a colour");
    }
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    dm.set(q, ids.cframe, Value::CFrame(CFrame::new(crate::Vector3::X)))
        .expect("set");
    record(&mut consumers, p, DirtyBits::PHYSICS | DirtyBits::PROPERTY);
    record(
        &mut consumers,
        q,
        DirtyBits::TRANSFORM | DirtyBits::PHYSICS | DirtyBits::PROPERTY,
    );
    for c in consumers.iter_mut().rev() {
        c.check(&mut dm, "after more writes");
    }
    for c in &mut consumers {
        c.check(&mut dm, "drained");
    }

    // Every consumer saw `q` created, so every one is told it is gone.
    dm.destroy(q).expect("destroy");
    for c in &mut consumers {
        assert_eq!(removed(&mut dm, c.id), vec![q], "{:?}", c.id);
        c.check(&mut dm, "after a destroy");
    }

    // Free the script host and one of the extras with changes still queued, then register two again: they get
    // the freed places back, with nothing left over.
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    record(&mut consumers, p, DirtyBits::VISUAL | DirtyBits::PROPERTY);
    for index in [2, 6] {
        dm.unregister_consumer(consumers[index].id);
    }
    let script = Expected::register(&mut dm, DirtyBits::STRUCTURE);
    let extra = Expected::register(&mut dm, DirtyBits::PHYSICS);
    assert_eq!((script.id.index(), extra.id.index()), (2, 6));
    consumers[2] = script;
    consumers[6] = extra;
    assert_eq!(
        dm.register_consumer(DirtyBits::ALL),
        Err(DmError::TooManyConsumers { max: MAX_CONSUMERS })
    );
    for c in &mut consumers {
        c.check(&mut dm, "re-registered");
    }

    dm.set_parent(p, None).expect("move");
    dm.set(p, ids.anchored, Value::Bool(false)).expect("set");
    record(&mut consumers, p, DirtyBits::STRUCTURE | DirtyBits::PROPERTY);
    record(&mut consumers, p, DirtyBits::PHYSICS | DirtyBits::PROPERTY);
    for c in &mut consumers {
        c.check(&mut dm, "after re-registering");
    }
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
