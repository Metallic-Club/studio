//! Signals: connections, each built-in event, firing from class code, and deferred delivery.

use std::sync::Arc;

use super::classes::{game, game_with, registry_with};
use super::*;
use crate::dm::{Delivery, DmError, ListenerKey};
use crate::{CFrame, ClassDesc, PropId, PropMeta, ScriptError, ScriptValue, SignalId, Vector3};

/// What a listener is handed: `(listener, source, args)`.
type Got = (u64, Id, Vec<ScriptValue>);

fn drain(dm: &mut DataModel) -> Vec<Got> {
    let mut out: Vec<Delivery> = Vec::new();
    dm.drain_deliveries(&mut out);
    out.into_iter().map(|d| (d.listener.0, d.source, d.args)).collect()
}

fn key(n: u64) -> ListenerKey {
    ListenerKey(n)
}

fn inst(id: Id) -> ScriptValue {
    ScriptValue::Instance(id)
}

fn text(s: &str) -> ScriptValue {
    ScriptValue::String(Arc::from(s))
}

fn connect(dm: &mut DataModel, id: Id, signal: SignalId, listener: u64) -> crate::dm::ConnectionId {
    dm.connect(id, signal, key(listener)).expect("connect")
}

#[test]
fn deliveries_come_in_fire_order() {
    let (mut dm, ids) = game();
    let bindable = dm.create(ids.bindable).expect("create");
    let p = part(&mut dm, &ids);
    connect(&mut dm, bindable, ids.event, 1);
    connect(&mut dm, p, ids.changed, 2);

    dm.fire(bindable, ids.event, vec![number_arg(1.0)]).expect("fire");
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    dm.fire_by_name(bindable, "Event", vec![number_arg(2.0)]).expect("fire");
    assert_eq!(dm.pending_events(), 3);
    assert_eq!(
        drain(&mut dm),
        vec![
            (1, bindable, vec![number_arg(1.0)]),
            (2, p, vec![text("Transparency")]),
            (1, bindable, vec![number_arg(2.0)]),
        ]
    );
    assert_eq!(dm.pending_events(), 0, "the drain empties the queue");
    assert!(drain(&mut dm).is_empty());
}

fn number_arg(n: f64) -> ScriptValue {
    ScriptValue::Number(n)
}

/// Every kind of change, on `p` (a part under a folder under the workspace) and around it.
fn change_everything(dm: &mut DataModel, ids: &classes::Ids, p: Id, bindable: Id) {
    let ws = ids.workspace;
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    dm.set(p, ids.name, text_value("Renamed"))
        .expect("rename through the property");
    dm.set_name(p, "Again".into()).expect("rename");
    dm.set_attribute(p, "Health", Some(number(10.0))).expect("attribute");
    dm.set_parent(p, Some(ws)).expect("move");
    let child = part_in(dm, ids, p, "Child");
    dm.fire(bindable, ids.event, vec![]).expect("fire");
    dm.destroy(child).expect("destroy child");
    dm.destroy(p).expect("destroy");
}

fn text_value(s: &str) -> Value {
    Value::String(Arc::from(s))
}

#[test]
fn nothing_is_queued_when_nothing_listens() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p = part_in(&mut dm, &ids, folder, "P");
    let bindable = dm.create(ids.bindable).expect("create");
    change_everything(&mut dm, &ids, p, bindable);
    assert_eq!(dm.pending_events(), 0);
    assert_eq!(dm.grave(p), None, "no grave when nothing is connected anywhere");

    // Something listens elsewhere, or to another signal of the same instance: still nothing queued.
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p = part_in(&mut dm, &ids, folder, "P");
    let bindable = dm.create(ids.bindable).expect("create");
    let elsewhere = dm.create(ids.part).expect("create");
    connect(&mut dm, elsewhere, ids.changed, 1);
    connect(&mut dm, bindable, ids.child_added, 2);
    assert!(dm.is_watched(elsewhere) && dm.is_watched(bindable));
    assert!(!dm.is_watched(p));
    change_everything(&mut dm, &ids, p, bindable);
    assert_eq!(dm.pending_events(), 0);
}

#[test]
fn reparenting_fires_each_tree_event_once() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let old = folder_in(&mut dm, &ids, ws);
    let new = folder_in(&mut dm, &ids, ws);
    let p = part_in(&mut dm, &ids, old, "P");
    let c = part_in(&mut dm, &ids, p, "C");
    connect(&mut dm, old, ids.child_removed, 1);
    connect(&mut dm, new, ids.child_added, 2);
    connect(&mut dm, old, ids.descendant_removing, 3);
    connect(&mut dm, ws, ids.descendant_removing, 4);
    connect(&mut dm, ws, ids.descendant_added, 5);
    connect(&mut dm, ids.game, ids.descendant_added, 6);
    connect(&mut dm, p, ids.ancestry_changed, 7);
    connect(&mut dm, c, ids.ancestry_changed, 8);
    connect(&mut dm, p, ids.changed, 9);
    dm.connect_property_changed(p, ids.parent, key(10)).expect("connect");
    // Signals of the right instances that must stay quiet.
    connect(&mut dm, old, ids.child_added, 90);
    connect(&mut dm, new, ids.child_removed, 91);
    connect(&mut dm, c, ids.changed, 92);

    dm.set_parent(p, Some(new)).expect("move");
    assert_eq!(
        drain(&mut dm),
        vec![
            (3, old, vec![inst(p)]),
            (3, old, vec![inst(c)]),
            (4, ws, vec![inst(p)]),
            (4, ws, vec![inst(c)]),
            (1, old, vec![inst(p)]),
            (2, new, vec![inst(p)]),
            (5, ws, vec![inst(p)]),
            (5, ws, vec![inst(c)]),
            (6, ids.game, vec![inst(p)]),
            (6, ids.game, vec![inst(c)]),
            (7, p, vec![inst(p), inst(new)]),
            (8, c, vec![inst(p), inst(new)]),
            (9, p, vec![text("Parent")]),
            (10, p, vec![]),
        ]
    );

    // Leaving the tree: the new parent is nil.
    dm.set_parent(p, None).expect("unparent");
    let got = drain(&mut dm);
    assert!(got.contains(&(7, p, vec![inst(p), ScriptValue::Nil])), "{got:?}");
    assert!(
        !got.iter().any(|(listener, ..)| *listener == 5),
        "no DescendantAdded: nothing was added"
    );
}

#[test]
fn destroy_fires_destroying_and_child_removed() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p = part_in(&mut dm, &ids, folder, "P");
    let c = part_in(&mut dm, &ids, p, "C");
    connect(&mut dm, folder, ids.child_removed, 1);
    connect(&mut dm, p, ids.destroying, 2);
    connect(&mut dm, c, ids.destroying, 3);
    connect(&mut dm, c, ids.ancestry_changed, 4);
    connect(&mut dm, folder, ids.descendant_removing, 5);
    let before = dm.connection_count();

    dm.destroy(p).expect("destroy");
    assert_eq!(
        drain(&mut dm),
        vec![
            (2, p, vec![]),
            (3, c, vec![]),
            (5, folder, vec![inst(p)]),
            (5, folder, vec![inst(c)]),
            (1, folder, vec![inst(p)]),
            (4, c, vec![inst(p), ScriptValue::Nil]),
        ]
    );
    assert_eq!(
        dm.connection_count(),
        before - 3,
        "the destroyed instances' connections are dropped by the drain that delivered their events"
    );
    assert!(drain(&mut dm).is_empty());
}

#[test]
fn property_writes_and_renames_fire_changed() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    connect(&mut dm, p, ids.changed, 1);
    dm.connect_property_changed(p, ids.transparency, key(2))
        .expect("connect");
    dm.connect_property_changed(p, ids.name, key(3)).expect("connect");
    connect(&mut dm, p, ids.attribute_changed, 4);

    dm.set(p, ids.transparency, number(0.5)).expect("set");
    assert_eq!(drain(&mut dm), vec![(1, p, vec![text("Transparency")]), (2, p, vec![])]);

    dm.set(p, ids.name, text_value("Door")).expect("write Name");
    assert_eq!(drain(&mut dm), vec![(1, p, vec![text("Name")]), (3, p, vec![])]);
    dm.set_name(p, "Gate".into()).expect("rename");
    assert_eq!(drain(&mut dm), vec![(1, p, vec![text("Name")]), (3, p, vec![])]);

    // Another property's signal stays quiet; attributes have their own event and do not fire `Changed`.
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    assert_eq!(drain(&mut dm), vec![(1, p, vec![text("Anchored")])]);
    dm.set_attribute(p, "Health", Some(number(5.0))).expect("attribute");
    dm.set_attribute(p, "Health", None).expect("remove attribute");
    assert_eq!(
        drain(&mut dm),
        vec![(4, p, vec![text("Health")]), (4, p, vec![text("Health")])]
    );
}

#[test]
fn no_op_writes_fire_nothing() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    for signal in [ids.changed, ids.ancestry_changed, ids.attribute_changed] {
        connect(&mut dm, p, signal, 1);
    }
    connect(&mut dm, ids.workspace, ids.child_added, 2);
    dm.connect_property_changed(p, ids.transparency, key(3))
        .expect("connect");
    dm.set_attribute(p, "Health", Some(number(1.0))).expect("attribute");
    drain(&mut dm);

    dm.set(p, ids.transparency, number(0.0)).expect("default again");
    dm.set(p, ids.transparency, number(-1.0))
        .expect("clamps to the current value");
    dm.set_name(p, "P".into()).expect("same name");
    dm.set_parent(p, Some(ids.workspace)).expect("same parent");
    dm.set_attribute(p, "Health", Some(number(1.0)))
        .expect("same attribute");
    dm.set_attribute(p, "Missing", None).expect("remove nothing");
    assert_eq!(dm.pending_events(), 0);
}

#[test]
fn disconnect_stops_delivery_even_before_the_drain() {
    let (mut dm, ids) = game();
    let b = dm.create(ids.bindable).expect("create");
    let first = connect(&mut dm, b, ids.event, 1);
    let second = connect(&mut dm, b, ids.event, 2);
    dm.fire(b, ids.event, vec![]).expect("fire");
    assert!(dm.disconnect(first));
    assert!(!dm.disconnect(first), "already disconnected");
    assert_eq!(drain(&mut dm), vec![(2, b, vec![])]);

    // A connection made after an event fired does not receive it.
    dm.fire(b, ids.event, vec![]).expect("fire");
    let late = connect(&mut dm, b, ids.event, 3);
    assert_eq!(drain(&mut dm), vec![(2, b, vec![])]);

    // Once nothing listens, the instance is unwatched and events are not queued at all.
    assert!(dm.disconnect(second));
    assert!(dm.is_watched(b));
    assert!(dm.disconnect(late));
    assert!(!dm.is_watched(b));
    assert_eq!(dm.connection_count(), 0);
    dm.fire(b, ids.event, vec![]).expect("fire");
    assert_eq!(dm.pending_events(), 0);

    // A stale connection id never names the connection that reuses its table entry.
    let reused = connect(&mut dm, b, ids.event, 4);
    assert_ne!(reused, first);
    assert!(!dm.disconnect(first));
    assert!(dm.disconnect(reused));
}

#[test]
fn two_connections_on_one_signal_are_both_delivered_in_order() {
    let (mut dm, ids) = game();
    let b = dm.create(ids.bindable).expect("create");
    connect(&mut dm, b, ids.event, 7);
    connect(&mut dm, b, ids.event, 3);
    connect(&mut dm, b, ids.event, 7);
    dm.fire(b, ids.event, vec![text("hi")]).expect("fire");
    assert_eq!(
        drain(&mut dm),
        vec![
            (7, b, vec![text("hi")]),
            (3, b, vec![text("hi")]),
            (7, b, vec![text("hi")])
        ]
    );
}

#[test]
fn graves_keep_the_name_readable_while_events_are_delivered() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p = part_in(&mut dm, &ids, folder, "Door");
    let c = part_in(&mut dm, &ids, p, "Hinge");
    connect(&mut dm, p, ids.destroying, 1);
    connect(&mut dm, folder, ids.child_removed, 2);
    dm.destroy(p).expect("destroy");
    assert!(!dm.contains(p));
    assert_eq!(dm.grave(p), Some(("Part", "Door")));
    assert_eq!(
        dm.grave(c),
        Some(("Part", "Hinge")),
        "unwatched, but a handler may still be handed it"
    );
    assert_eq!(dm.grave(folder), None, "alive");

    let got = drain(&mut dm);
    assert_eq!(got, vec![(1, p, vec![]), (2, folder, vec![inst(p)])]);
    // The listeners run now, after the drain: the grave is still there for them.
    assert_eq!(dm.grave(p), Some(("Part", "Door")));
    assert_eq!(dm.connect(p, ids.destroying, key(3)), Err(DmError::NoInstance(p)));

    // The next drain begins after they ran, and clears it.
    drain(&mut dm);
    assert_eq!(dm.grave(p), None);
    assert_eq!(dm.grave(c), None);
    assert_eq!(dm.connection_count(), 1, "only the folder's connection is left");
}

#[test]
fn a_destroyed_instances_connection_can_still_be_disconnected_before_the_drain() {
    let (mut dm, ids) = game();
    let p = part_in(&mut dm, &ids, ids.workspace, "P");
    let conn = connect(&mut dm, p, ids.destroying, 1);
    dm.destroy(p).expect("destroy");
    assert!(dm.disconnect(conn));
    assert!(drain(&mut dm).is_empty());
    assert_eq!(dm.connection_count(), 0);
}

#[test]
fn listeners_changes_queue_for_the_next_drain() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    connect(&mut dm, p, ids.changed, 1);
    dm.set(p, ids.transparency, number(0.5)).expect("set");
    let mut out = Vec::new();
    dm.drain_deliveries(&mut out);
    assert_eq!(out.len(), 1);
    // A "listener" reacting to the delivery: its change is not in `out`, it waits for the next drain.
    for delivery in &out {
        dm.set(delivery.source, ids.reflectance, number(0.5)).expect("set");
    }
    assert_eq!(out.len(), 1);
    assert_eq!(dm.pending_events(), 1);
    assert_eq!(drain(&mut dm), vec![(1, p, vec![text("Reflectance")])]);
}

#[test]
fn stale_ids_and_unknown_members_are_refused() {
    let (mut dm, ids) = game();
    let p = part(&mut dm, &ids);
    let gone = part(&mut dm, &ids);
    dm.destroy(gone).expect("destroy");
    assert_eq!(dm.connect(gone, ids.changed, key(1)), Err(DmError::NoInstance(gone)));
    assert_eq!(
        dm.connect_property_changed(gone, ids.name, key(1)),
        Err(DmError::NoInstance(gone))
    );
    assert_eq!(dm.fire(gone, ids.changed, vec![]), Err(DmError::NoInstance(gone)));
    assert!(!dm.is_watched(gone));

    assert_eq!(
        dm.connect(p, SignalId::new(999), key(1)),
        Err(DmError::UnknownSignal {
            class: "Part",
            signal: "#999".into()
        })
    );
    assert_eq!(
        dm.connect(p, ids.event, key(1)),
        Err(DmError::UnknownSignal {
            class: "Part",
            signal: format!("#{}", ids.event.raw())
        }),
        "Bindable's own signal is not a Part signal"
    );
    assert!(dm.fire(p, SignalId::new(999), vec![]).is_err());
    assert_eq!(
        dm.fire_by_name(p, "Nope", vec![]),
        Err(DmError::UnknownSignal {
            class: "Part",
            signal: "Nope".into()
        })
    );
    assert!(matches!(
        dm.connect_property_changed(p, PropId::new(999), key(1)),
        Err(DmError::UnknownProperty { .. })
    ));
    assert_eq!(dm.connection_count(), 0);
}

#[test]
fn hidden_signals_cannot_be_connected() {
    static SEALED: ClassDesc = ClassDesc::new("Sealed", Some("Part")).creatable().hide(&["Destroying"]);
    let registry = registry_with(&[&SEALED]);
    let sealed = registry.class_id("Sealed").expect("class");
    let (mut dm, ids) = game_with(registry);
    let s = dm.create(sealed).expect("create");
    assert!(matches!(
        dm.connect(s, ids.destroying, key(1)),
        Err(DmError::UnknownSignal { class: "Sealed", .. })
    ));
    assert!(dm.signal_id(s, "Destroying").is_err());
    assert_eq!(dm.signal_id(s, "Changed"), Ok(ids.changed));
}

#[test]
fn signal_ids_are_the_same_in_every_class_that_inherits_them() {
    let (dm, ids) = game();
    let r = dm.registry();
    for class in [ids.part, ids.folder, ids.bindable, ids.game_class, ids.workspace_class] {
        assert_eq!(r.signal_id(class, "Changed"), Some(ids.changed));
        assert_eq!(r.signal_id(class, "Destroying"), Some(ids.destroying));
        assert_eq!(r.signal_id(class, "AncestryChanged"), Some(ids.ancestry_changed));
    }
    assert_eq!(
        r.signal_id(ids.bindable, "Event").map(|s| s.index()),
        Some(r.class(ids.instance).expect("class").signals().len()),
        "a class's own signals come after the inherited ones"
    );
}

#[test]
fn services_report_joining_the_root() {
    let registry = registry_with(&[]);
    let r = &*registry;
    let (game_class, workspace) = (
        r.class_id("Game").expect("class"),
        r.class_id("Workspace").expect("class"),
    );
    let child_added = r.signal_id(game_class, "ChildAdded").expect("signal");
    let mut dm = DataModel::new(Arc::clone(&registry));
    let root = dm.create_root(game_class).expect("root");
    dm.connect(root, child_added, key(1)).expect("connect");
    let ws = dm.create_service(workspace).expect("service");
    assert_eq!(drain(&mut dm), vec![(1, root, vec![inst(ws)])]);
}

// Derived properties: `Body` is a `Part` whose `Position` and `Spin` are derived from `CFrame`, and `Mass` from
// `Size`. The shared `Part` declares no derivations: the perf gate builds the shared classes on the base branch too.

fn body_cframe(dm: &DataModel, id: Id) -> CFrame {
    dm.prop_id(id, "CFrame")
        .and_then(|p| dm.cframe(id, p))
        .unwrap_or_default()
}

fn set_body_cframe(dm: &mut DataModel, id: Id, cframe: CFrame) -> Result<(), ScriptError> {
    let prop = dm.prop_id(id, "CFrame")?;
    Ok(dm.set(id, prop, Value::CFrame(cframe))?)
}

fn vector_arg(value: Value) -> Result<Vector3, ScriptError> {
    match value {
        Value::Vector3(v) => Ok(v),
        _ => Err(ScriptError::wrong_type("Vector3", "other")),
    }
}

fn body_position_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(body_cframe(dm, id).position())
}

fn body_position_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let rotation = body_cframe(dm, id).rotation();
    set_body_cframe(dm, id, CFrame::from_parts(vector_arg(value)?, rotation))
}

fn body_spin_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(body_cframe(dm, id).to_orientation_deg())
}

fn body_spin_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let angles = vector_arg(value)?;
    let rotation = CFrame::from_orientation_deg(angles.x, angles.y, angles.z).rotation();
    let position = body_cframe(dm, id).position();
    set_body_cframe(dm, id, CFrame::from_parts(position, rotation))
}

fn body_mass_get(_: &DataModel, _: Id) -> Value {
    Value::Number(1.0)
}

static BODY: ClassDesc = ClassDesc::new("Body", Some("Part")).creatable().props(&[
    PropMeta::vector3("Position", 0.0, 0.0, 0.0)
        .computed(body_position_get, Some(body_position_set))
        .derived_from(&["CFrame"])
        .override_inherited(),
    PropMeta::vector3("Spin", 0.0, 0.0, 0.0)
        .computed(body_spin_get, Some(body_spin_set))
        .derived_from(&["CFrame"]),
    PropMeta::number("Mass", 0.0)
        .computed(body_mass_get, None)
        .derived_from(&["Size"])
        .override_inherited()
        .read_only(),
]);

/// A data model with `Body`, one body, and the id of its `Spin`.
fn body() -> (DataModel, classes::Ids, Id, PropId) {
    let registry = registry_with(&[&BODY]);
    let class = registry.class_id("Body").expect("class");
    let (mut dm, ids) = game_with(registry);
    let b = dm.create(class).expect("create");
    let spin = dm.prop_id(b, "Spin").expect("prop");
    (dm, ids, b, spin)
}

#[test]
fn derived_properties_fire_after_their_source_in_declaration_order() {
    let (mut dm, ids, b, spin) = body();
    connect(&mut dm, b, ids.changed, 1);
    for (prop, listener) in [(ids.cframe, 2), (ids.position, 3), (spin, 4)] {
        dm.connect_property_changed(b, prop, key(listener)).expect("connect");
    }
    let moved = vec![
        (1, b, vec![text("CFrame")]),
        (2, b, vec![]),
        (1, b, vec![text("Position")]),
        (3, b, vec![]),
        (1, b, vec![text("Spin")]),
        (4, b, vec![]),
    ];

    dm.set(b, ids.cframe, Value::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0))))
        .expect("set CFrame");
    assert_eq!(drain(&mut dm), moved, "writing the source");

    // A computed write fires nothing itself: its setter's one `CFrame` write fires each property once.
    dm.set(b, ids.position, Value::Vector3(Vector3::new(4.0, 5.0, 6.0)))
        .expect("set Position");
    assert_eq!(drain(&mut dm), moved, "writing Position");
    dm.set(b, spin, Value::Vector3(Vector3::new(0.0, 90.0, 0.0)))
        .expect("set Spin");
    assert_eq!(drain(&mut dm), moved, "writing Spin");

    dm.set(b, ids.position, Value::Vector3(Vector3::new(4.0, 5.0, 6.0)))
        .expect("set Position");
    assert!(
        drain(&mut dm).is_empty(),
        "a write that leaves CFrame unchanged fires nothing"
    );

    dm.set(b, ids.size, Value::Vector3(Vector3::new(2.0, 2.0, 2.0)))
        .expect("set Size");
    assert_eq!(
        drain(&mut dm),
        vec![(1, b, vec![text("Size")]), (1, b, vec![text("Mass")])],
        "each source fires only its own dependents"
    );
    dm.set(b, ids.transparency, number(0.5)).expect("set");
    assert_eq!(drain(&mut dm), vec![(1, b, vec![text("Transparency")])]);
}

#[test]
fn derived_properties_fire_only_where_something_listens() {
    let (mut dm, ids, b, spin) = body();
    let cframe = Value::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0)));

    // Unwatched: nothing is queued.
    dm.set(b, spin, Value::Vector3(Vector3::new(0.0, 45.0, 0.0)))
        .expect("set Spin");
    assert_eq!(dm.pending_events(), 0);

    // Only Position's own signal: one event per write, whichever property is written.
    dm.connect_property_changed(b, ids.position, key(1)).expect("connect");
    dm.set(b, ids.cframe, cframe.clone()).expect("set CFrame");
    dm.set(b, spin, Value::Vector3(Vector3::new(0.0, 90.0, 0.0)))
        .expect("set Spin");
    assert_eq!(drain(&mut dm), vec![(1, b, vec![]), (1, b, vec![])]);

    // A plain `Part` declares no derivation: its `CFrame` fires alone.
    let p = part(&mut dm, &ids);
    connect(&mut dm, p, ids.changed, 2);
    dm.connect_property_changed(p, ids.position, key(3)).expect("connect");
    dm.set(p, ids.cframe, cframe).expect("set CFrame");
    assert_eq!(drain(&mut dm), vec![(2, p, vec![text("CFrame")])]);
}
