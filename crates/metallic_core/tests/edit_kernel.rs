//! In-memory edit-kernel acceptance; no network, window, clock lease or durable session.
use metallic_core::edit::{
    CommittedBatch, EditAccess, EditAuthority, EditBatch, EditError, EditId, EditLimits, EditOp,
    EditReplica, EditSnapshot, EditValue, EditorId, Submission,
};
use metallic_core::{CFrame, Changes, CoreError, Value, Vec3, World};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

const A: EditorId = EditorId(1);
const B: EditorId = EditorId(2);

fn batch(sequence: u64, operations: Vec<EditOp>) -> EditBatch {
    EditBatch {
        client_sequence: sequence,
        operations,
    }
}

fn create(id: EditId, class: &str, parent: Option<EditId>) -> EditOp {
    EditOp::Create {
        id,
        class: class.into(),
        parent,
        after: None,
        properties: Vec::new(),
    }
}

fn set(id: EditId, property: &str, value: EditValue) -> EditOp {
    EditOp::Set {
        id,
        properties: vec![(property.into(), value)],
    }
}

fn authority(world: World) -> EditAuthority {
    let mut authority =
        EditAuthority::new(world, EditLimits::default()).expect("valid initial document");
    authority.set_access(A, EditAccess::Edit).expect("editor A");
    authority.set_access(B, EditAccess::Edit).expect("editor B");
    authority
}

fn commit(authority: &mut EditAuthority, editor: EditorId, value: &EditBatch) -> CommittedBatch {
    match authority.submit(editor, value).expect("accepted input") {
        Submission::Committed(record) => record,
        Submission::Duplicate(_) => panic!("expected new record"),
    }
}

#[test]
fn late_rejection_keeps_live_identity_values_journal_and_original_subscriptions() {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").expect("workspace");
    let part = world.create("Part", Some(workspace)).expect("part");
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    world
        .property_changed(part, "Transparency")
        .connect(move |value| observed.lock().expect("events").push(value.clone()));
    let mut authority = authority(world);
    let id = authority.id_of(part).expect("stable part");
    let cursor = authority.world().change_cursor();
    let before = authority.snapshot().expect("snapshot");
    let rejected = batch(
        1,
        vec![
            set(id, "Transparency", EditValue::Number(0.5)),
            set(id, "not_a_property", EditValue::Number(f64::NAN)),
        ],
    );
    assert!(matches!(
        authority.submit(A, &rejected),
        Err(EditError::Core {
            operation: 1,
            source: CoreError::UnknownProperty { .. }
        })
    ));
    assert_eq!(authority.sequence(), 0);
    assert_eq!(authority.snapshot().expect("snapshot"), before);
    assert_eq!(authority.world().change_cursor(), cursor);
    assert!(events.lock().expect("events").is_empty());
    commit(
        &mut authority,
        A,
        &batch(1, vec![set(id, "Transparency", EditValue::Number(0.5))]),
    );
    assert_eq!(*events.lock().expect("events"), vec![Value::Number(0.5)]);
    assert!(
        matches!(authority.world().changes_since(cursor).0, Changes::Ids(ids) if ids==vec![part])
    );
    let cursor = authority.world().change_cursor();
    commit(
        &mut authority,
        A,
        &batch(2, vec![set(id, "Transparency", EditValue::Number(0.5))]),
    );
    assert_eq!(
        authority.world().change_cursor(),
        cursor,
        "accepted no-op keeps World semantics"
    );
    assert_eq!(events.lock().expect("events").len(), 1);
}

#[test]
fn creation_is_atomic_and_duplicate_or_old_identifiers_never_reapply() {
    let mut authority = authority(World::new_empty());
    let id = EditId::new(1, 1);
    let invalid = batch(
        1,
        vec![
            create(id, "Part", None),
            set(id, "Transparency", EditValue::Number(f64::INFINITY)),
        ],
    );
    assert!(matches!(
        authority.submit(A, &invalid),
        Err(EditError::Core { .. })
    ));
    assert!(authority.instance_of(id).is_none());
    assert!(authority.world().roots().is_empty());
    let request = batch(1, vec![create(id, "Part", None)]);
    let first = commit(&mut authority, A, &request);
    let cursor = authority.world().change_cursor();
    assert_eq!(
        authority.submit(A, &request).expect("repeat"),
        Submission::Duplicate(first.receipt)
    );
    assert_eq!(authority.world().change_cursor(), cursor);
    assert_eq!(
        authority.submit(A, &batch(1, vec![create(id, "Folder", None)])),
        Err(EditError::RequestReuse)
    );
    let old_local = authority.instance_of(id).expect("local id");
    commit(&mut authority, A, &batch(2, vec![EditOp::Destroy { id }]));
    assert_eq!(
        authority.submit(A, &batch(3, vec![create(id, "Part", None)])),
        Err(EditError::CreationReuse)
    );
    let next = EditId::new(1, 2);
    commit(
        &mut authority,
        A,
        &batch(3, vec![create(next, "Part", None)]),
    );
    assert_ne!(
        authority.instance_of(next).expect("new local id"),
        old_local
    );
    assert_eq!(
        authority.submit(A, &batch(4, vec![create(EditId::new(2, 3), "Part", None)])),
        Err(EditError::CreationReuse)
    );
}

#[test]
fn strict_anchors_cycles_deleted_targets_and_builtin_roots_are_rejected() {
    let mut authority = authority(World::new_with_services());
    let workspace_local = authority.world().root("Workspace").expect("workspace");
    let workspace = authority.id_of(workspace_local).expect("stable workspace");
    let folder = EditId::new(1, 1);
    let child = EditId::new(1, 2);
    let other = EditId::new(1, 3);
    commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                create(folder, "Folder", Some(workspace)),
                create(child, "Part", Some(folder)),
                create(other, "Part", Some(workspace)),
            ],
        ),
    );
    let before = authority.snapshot().expect("snapshot");
    assert!(matches!(
        authority.submit(
            A,
            &batch(
                2,
                vec![EditOp::Move {
                    id: child,
                    parent: Some(folder),
                    after: Some(other)
                }]
            )
        ),
        Err(EditError::InvalidAnchor(_))
    ));
    assert!(matches!(
        authority.submit(
            A,
            &batch(
                2,
                vec![EditOp::Move {
                    id: child,
                    parent: Some(folder),
                    after: Some(EditId::new(8, 8))
                }]
            )
        ),
        Err(EditError::InvalidAnchor(_))
    ));
    assert!(matches!(
        authority.submit(
            A,
            &batch(
                2,
                vec![EditOp::Move {
                    id: folder,
                    parent: Some(child),
                    after: None
                }]
            )
        ),
        Err(EditError::Core {
            source: CoreError::Cycle,
            ..
        })
    ));
    assert_eq!(authority.snapshot().expect("snapshot"), before);
    for operation in [
        EditOp::Destroy { id: workspace },
        EditOp::Move {
            id: workspace,
            parent: Some(folder),
            after: None,
        },
        create(EditId::new(1, 4), "Workspace", None),
    ] {
        assert_eq!(
            authority.submit(A, &batch(2, vec![operation])),
            Err(EditError::ProtectedBuiltin)
        );
    }
    commit(
        &mut authority,
        A,
        &batch(2, vec![EditOp::Destroy { id: folder }]),
    );
    for operation in [
        set(child, "Name", EditValue::String("gone".into())),
        EditOp::Move {
            id: child,
            parent: Some(workspace),
            after: None,
        },
        create(EditId::new(1, 4), "Part", Some(folder)),
    ] {
        assert!(matches!(
            authority.submit(A, &batch(3, vec![operation])),
            Err(EditError::InvalidObject(_))
        ));
    }
}

#[test]
fn builtin_service_names_are_protected_while_registered_settings_remain_editable() {
    let mut authority = authority(World::new_with_services());
    let workspace = authority
        .id_of(authority.world().root("Workspace").expect("workspace"))
        .expect("id");
    let lighting = authority
        .id_of(authority.world().root("Lighting").expect("lighting"))
        .expect("id");
    let before = authority.snapshot().expect("snapshot");
    let cursor = authority.world().change_cursor();
    for id in [workspace, lighting] {
        assert_eq!(
            authority.submit(
                A,
                &batch(
                    1,
                    vec![
                        set(
                            workspace,
                            "FallenPartsDestroyHeight",
                            EditValue::Number(-1000.0)
                        ),
                        set(id, "Name", EditValue::String("renamed".into()))
                    ]
                )
            ),
            Err(EditError::ProtectedBuiltin)
        );
        assert_eq!(authority.snapshot().expect("snapshot"), before);
        assert_eq!(authority.world().change_cursor(), cursor);
    }
    commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![set(
                workspace,
                "FallenPartsDestroyHeight",
                EditValue::Number(-1000.0),
            )],
        ),
    );
    assert_eq!(
        authority.world().get(
            authority.instance_of(workspace).expect("workspace"),
            "FallenPartsDestroyHeight"
        ),
        Ok(Value::Number(-1000.0))
    );
    assert!(authority.world().root("Workspace").is_some());
    assert!(authority.world().root("Lighting").is_some());
}

#[test]
fn ordering_and_server_order_preserve_independent_properties() {
    let mut authority = authority(World::new_empty());
    let one = EditId::new(1, 1);
    let two = EditId::new(1, 2);
    let three = EditId::new(1, 3);
    commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                create(one, "Part", None),
                create(two, "Part", None),
                EditOp::Create {
                    id: three,
                    class: "Part".into(),
                    parent: None,
                    after: Some(one),
                    properties: Vec::new(),
                },
            ],
        ),
    );
    assert_eq!(
        authority.snapshot().expect("snapshot").roots,
        vec![one, three, two]
    );
    commit(
        &mut authority,
        A,
        &batch(2, vec![set(one, "Name", EditValue::String("first".into()))]),
    );
    commit(
        &mut authority,
        B,
        &batch(1, vec![set(one, "Name", EditValue::String("last".into()))]),
    );
    commit(
        &mut authority,
        A,
        &batch(3, vec![set(one, "Transparency", EditValue::Number(0.75))]),
    );
    let local = authority.instance_of(one).expect("local");
    assert_eq!(
        authority.world().get(local, "Name").expect("name"),
        Value::String("last".into())
    );
    assert_eq!(
        authority.world().get(local, "Transparency").expect("alpha"),
        Value::Number(0.75)
    );
    assert_eq!(authority.sequence(), 4);
}

#[test]
fn all_source_paths_share_ownership_and_references_use_stable_ids() {
    let mut authority = authority(World::new_empty());
    let script = EditId::new(1, 1);
    let player = EditId::new(1, 2);
    let model = EditId::new(1, 3);
    commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                EditOp::Create {
                    id: script,
                    class: "Script".into(),
                    parent: None,
                    after: None,
                    properties: vec![("Source".into(), EditValue::String("original".into()))],
                },
                create(player, "Player", None),
                create(model, "Model", None),
                set(player, "Character", EditValue::Ref(Some(model))),
            ],
        ),
    );
    assert_eq!(
        authority
            .snapshot()
            .expect("snapshot")
            .nodes
            .iter()
            .find(|n| n.id == player)
            .expect("player")
            .properties["Character"],
        EditValue::Ref(Some(model))
    );
    for operation in [
        set(script, "Source", EditValue::String("not owned".into())),
        EditOp::ScriptText {
            id: script,
            text: "not owned".into(),
        },
    ] {
        assert_eq!(
            authority.submit(B, &batch(1, vec![operation])),
            Err(EditError::ScriptNotOwned)
        );
    }
    authority
        .set_script_owner(script, Some(B))
        .expect("trusted host transfer");
    commit(
        &mut authority,
        B,
        &batch(
            1,
            vec![EditOp::ScriptText {
                id: script,
                text: "owned".into(),
            }],
        ),
    );
    assert_eq!(
        authority.submit(
            A,
            &batch(
                2,
                vec![set(
                    player,
                    "Character",
                    EditValue::Ref(Some(EditId::new(9, 9)))
                )]
            )
        ),
        Err(EditError::InvalidObject(EditId::new(9, 9)))
    );
    let cursor = authority.world().change_cursor();
    authority
        .set_access(A, EditAccess::Play)
        .expect("revoke authoring");
    assert_eq!(
        authority.submit(A, &batch(2, vec![EditOp::Destroy { id: model }])),
        Err(EditError::PermissionDenied)
    );
    assert_eq!(authority.world().change_cursor(), cursor);
}

#[test]
fn limits_and_receipt_eviction_remain_bounded_without_reapplying_old_input() {
    let limits = EditLimits {
        max_clients: 1,
        max_receipts: 1,
        max_operations: 2,
        max_script_bytes: 8,
        ..EditLimits::default()
    };
    let mut authority = EditAuthority::new(World::new_empty(), limits).expect("limits");
    authority.set_access(A, EditAccess::Edit).expect("A");
    assert_eq!(
        authority.set_access(B, EditAccess::Edit),
        Err(EditError::ClientLimit)
    );
    let id = EditId::new(1, 1);
    let original = batch(1, vec![create(id, "Script", None)]);
    commit(&mut authority, A, &original);
    commit(
        &mut authority,
        A,
        &batch(2, vec![set(id, "Name", EditValue::String("new".into()))]),
    );
    assert_eq!(authority.submit(A, &original), Err(EditError::StaleRequest));
    let before = authority.snapshot().expect("snapshot");
    assert!(matches!(
        authority.submit(
            A,
            &batch(
                3,
                vec![EditOp::ScriptText {
                    id,
                    text: "too much source".into()
                }]
            )
        ),
        Err(EditError::Limit(_))
    ));
    assert!(matches!(
        authority.submit(A, &batch(3, vec![EditOp::Destroy { id }; 3])),
        Err(EditError::Limit(_))
    ));
    assert_eq!(authority.snapshot().expect("snapshot"), before);
    assert_eq!(authority.sequence(), 2);
}

fn initial_world(noise: usize) -> World {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").expect("workspace");
    for _ in 0..noise {
        let temporary = world.create("Folder", Some(workspace)).expect("temporary");
        world.destroy(temporary);
    }
    world
        .create("Part", Some(workspace))
        .expect("baseline part");
    world
}

fn tree_hash(snapshot: &EditSnapshot) -> u64 {
    // A test-build hash plus full equality; this is not a published cross-version protocol hash.
    let canonical = ron::to_string(snapshot).expect("canonical test snapshot");
    let mut hash = DefaultHasher::new();
    canonical.hash(&mut hash);
    hash.finish()
}

#[test]
fn cframe_math_uses_registered_pose_components_without_a_new_value_escape_hatch() {
    let mut authority = authority(initial_world(0));
    let mut replica = EditReplica::new(initial_world(2), EditLimits::default()).expect("replica");
    let workspace = authority.world().root("Workspace").expect("workspace");
    let part = authority.world().children(workspace)[0];
    let id = authority.id_of(part).expect("stable part");
    let transform =
        CFrame::from_position(Vec3::new(1.0, 2.0, 3.0)) * CFrame::from_euler_yxz(0.2, -0.4, 0.7);
    let position = transform.position;
    let orientation = transform.orientation_degrees();
    let record = commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                set(id, "Position", EditValue::Vector3(position)),
                set(id, "Orientation", EditValue::Vector3(orientation)),
            ],
        ),
    );
    replica.apply(&record).expect("pose replay");
    assert_eq!(
        authority.snapshot().expect("authority"),
        replica.snapshot().expect("replica")
    );
    let rebuilt = CFrame::from_position_orientation(position, orientation);
    for (actual, expected) in rebuilt.components().into_iter().zip(transform.components()) {
        assert!((actual - expected).abs() < 1e-12);
    }
    let before = authority
        .snapshot()
        .expect("before rejected virtual property");
    let cursor = authority.world().change_cursor();
    assert!(matches!(
        authority.submit(
            A,
            &batch(
                2,
                vec![
                    set(id, "Position", EditValue::Vector3(Vec3::new(9.0, 8.0, 7.0))),
                    set(id, "CFrame", EditValue::Vector3(position)),
                ]
            )
        ),
        Err(EditError::Core {
            operation: 1,
            source: CoreError::UnknownProperty { .. }
        })
    ));
    assert_eq!(authority.snapshot().expect("unchanged"), before);
    assert_eq!(authority.world().change_cursor(), cursor);
    assert_eq!(authority.sequence(), 1);
}

fn frame_value(frame: CFrame) -> EditValue {
    EditValue::CFrame(Box::new(frame))
}

fn frame_components(values: [f64; 12]) -> CFrame {
    CFrame::from_parts(
        Vec3::new(values[0], values[1], values[2]),
        [
            [values[3], values[4], values[5]],
            [values[6], values[7], values[8]],
            [values[9], values[10], values[11]],
        ],
    )
}

#[test]
fn stored_cframes_create_serialize_and_replay_with_stable_references() {
    let mut authority = authority(initial_world(0));
    let mut replica_world = initial_world(2);
    let spare_a = replica_world.create("Folder", None).expect("spare A");
    let spare_b = replica_world.create("Folder", None).expect("spare B");
    replica_world.destroy(spare_a);
    replica_world.destroy(spare_b);
    let mut replica = EditReplica::new(replica_world, EditLimits::default()).expect("replica");
    let model = EditId::new(1, 1);
    let part = EditId::new(1, 2);
    let frame =
        CFrame::from_position(Vec3::new(1.0, 2.0, 3.0)) * CFrame::from_euler_yxz(0.2, -0.4, 0.7);
    let record = commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                EditOp::Create {
                    id: model,
                    class: "Model".into(),
                    parent: None,
                    after: None,
                    properties: vec![("WorldPivot".into(), frame_value(frame))],
                },
                EditOp::Create {
                    id: part,
                    class: "Part".into(),
                    parent: Some(model),
                    after: None,
                    properties: vec![("PivotOffset".into(), frame_value(frame.inverse()))],
                },
                set(model, "PrimaryPart", EditValue::Ref(Some(part))),
            ],
        ),
    );
    let encoded = ron::to_string(&record).expect("serialize boxed CFrame record");
    let decoded: CommittedBatch = ron::from_str(&encoded).expect("deserialize CFrame record");
    assert_eq!(decoded, record);
    replica.apply(&decoded).expect("replica");
    assert_ne!(authority.instance_of(part), replica.instance_of(part));
    assert_eq!(
        authority.snapshot().expect("authority"),
        replica.snapshot().expect("replica")
    );
    assert_eq!(
        replica
            .world()
            .get(replica.instance_of(model).expect("model"), "PrimaryPart"),
        Ok(Value::Ref(replica.instance_of(part)))
    );
    assert_eq!(
        authority
            .world()
            .get(authority.instance_of(model).expect("model"), "WorldPivot"),
        Ok(Value::CFrame(Box::new(frame)))
    );
}

#[test]
fn every_cframe_component_rejects_nonfinite_batches_and_has_exact_request_identity() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).expect("part");
    let calls = Arc::new(Mutex::new(0));
    let observed = Arc::clone(&calls);
    world
        .property_changed(part, "PivotOffset")
        .connect(move |_| *observed.lock().expect("calls") += 1);
    let mut authority = authority(world);
    let id = authority.id_of(part).expect("stable");
    let before = authority.snapshot().expect("before");
    let cursor = authority.world().change_cursor();
    for component in 0..12 {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut values = CFrame::IDENTITY.components();
            values[component] = invalid;
            assert!(matches!(
                authority.submit(
                    A,
                    &batch(
                        1,
                        vec![
                            set(id, "Name", EditValue::String("must roll back".into())),
                            set(id, "PivotOffset", frame_value(frame_components(values))),
                        ]
                    )
                ),
                Err(EditError::Core {
                    operation: 1,
                    source: CoreError::InvalidValue { .. }
                })
            ));
            assert_eq!(authority.snapshot().expect("unchanged"), before);
            assert_eq!(authority.world().change_cursor(), cursor);
        }
    }
    assert_eq!(*calls.lock().expect("calls"), 0);
    let original = batch(
        1,
        vec![set(id, "PivotOffset", frame_value(CFrame::IDENTITY))],
    );
    commit(&mut authority, A, &original);
    for component in 0..12 {
        let mut values = CFrame::IDENTITY.components();
        values[component] = if values[component] == 0.0 {
            -0.0
        } else {
            f64::from_bits(values[component].to_bits() ^ 1)
        };
        assert_eq!(
            authority.submit(
                A,
                &batch(
                    1,
                    vec![set(
                        id,
                        "PivotOffset",
                        frame_value(frame_components(values))
                    ),]
                )
            ),
            Err(EditError::RequestReuse)
        );
    }
    assert_eq!(
        authority.world().change_cursor(),
        cursor,
        "Core's equal-default write stays a no-op"
    );
    assert_eq!(*calls.lock().expect("calls"), 0);
    assert_eq!(
        authority
            .submit(A, &original)
            .expect("exact duplicate")
            .receipt()
            .server_sequence,
        1
    );
}

#[test]
fn cframe_owned_heap_bytes_bound_requests_and_retained_receipts() {
    let mut world = World::new_empty();
    let local = world.create("Part", None).expect("part");
    let mut authority = EditAuthority::new(
        world,
        EditLimits {
            max_batch_bytes: 400,
            max_receipt_bytes: 700,
            ..EditLimits::default()
        },
    )
    .expect("limits");
    authority.set_access(A, EditAccess::Edit).expect("access");
    let id = authority.id_of(local).expect("id");
    let before = authority.snapshot().expect("before");
    let two_frames = batch(
        1,
        vec![EditOp::Set {
            id,
            properties: vec![
                ("PivotOffset".into(), frame_value(CFrame::IDENTITY)),
                ("PivotOffset".into(), frame_value(CFrame::IDENTITY)),
            ],
        }],
    );
    assert_eq!(
        authority.submit(A, &two_frames),
        Err(EditError::Limit("batch bytes"))
    );
    let mut metadata_control = two_frames;
    if let EditOp::Set { properties, .. } = &mut metadata_control.operations[0] {
        for (_, value) in properties {
            *value = EditValue::Number(1.0);
        }
    }
    assert!(
        matches!(
            authority.submit(A, &metadata_control),
            Err(EditError::Core {
                source: CoreError::TypeMismatch { .. },
                ..
            })
        ),
        "the same native metadata fits; boxed payload bytes exceed the input cap"
    );
    assert_eq!(authority.snapshot().expect("unchanged"), before);
    let first = batch(
        1,
        vec![set(
            id,
            "PivotOffset",
            frame_value(CFrame::from_position(Vec3::new(1.0, 0.0, 0.0))),
        )],
    );
    commit(&mut authority, A, &first);
    for sequence in 2..=3 {
        commit(
            &mut authority,
            A,
            &batch(
                sequence,
                vec![set(
                    id,
                    "PivotOffset",
                    frame_value(CFrame::from_position(Vec3::new(sequence as f64, 0.0, 0.0))),
                )],
            ),
        );
    }
    assert_eq!(
        authority.submit(A, &first),
        Err(EditError::StaleRequest),
        "owned CFrame payload bytes must also evict retained receipt history"
    );
    assert_eq!(authority.sequence(), 3);
}

#[test]
fn replica_sequence_guards_and_stable_references_survive_different_local_allocations() {
    let mut authority = authority(initial_world(0));
    let mut replica_world = initial_world(3);
    let spare_a = replica_world.create("Folder", None).expect("spare A");
    let spare_b = replica_world.create("Folder", None).expect("spare B");
    replica_world.destroy(spare_a);
    replica_world.destroy(spare_b);
    let mut replica = EditReplica::new(replica_world, EditLimits::default()).expect("replica");
    let player = EditId::new(1, 1);
    let character = EditId::new(1, 2);
    let first = commit(
        &mut authority,
        A,
        &batch(
            1,
            vec![
                create(player, "Player", None),
                create(character, "Model", None),
                set(player, "Character", EditValue::Ref(Some(character))),
            ],
        ),
    );
    let before = replica.snapshot().expect("before");
    let mut gap = first.clone();
    gap.receipt.server_sequence = 2;
    assert_eq!(
        replica.apply(&gap),
        Err(EditError::ServerSequence { expected: 1 })
    );
    let mut inconsistent = first.clone();
    inconsistent.receipt.client_sequence = 2;
    assert_eq!(
        replica.apply(&inconsistent),
        Err(EditError::ClientSequence { expected: 1 })
    );
    assert_eq!(replica.snapshot().expect("unchanged"), before);
    replica.apply(&first).expect("ordered first");
    assert_ne!(
        authority.instance_of(character),
        replica.instance_of(character)
    );
    assert_eq!(
        replica
            .world()
            .get(replica.instance_of(player).expect("player"), "Character"),
        Ok(Value::Ref(replica.instance_of(character)))
    );
    let cursor = replica.world().change_cursor();
    replica.apply(&first).expect("exact last duplicate");
    assert_eq!(replica.world().change_cursor(), cursor);
    let mut changed_repeat = first.clone();
    changed_repeat
        .batch
        .operations
        .push(set(player, "Name", EditValue::String("changed".into())));
    assert_eq!(
        replica.apply(&changed_repeat),
        Err(EditError::ServerSequence { expected: 2 })
    );
    let destroyed = commit(
        &mut authority,
        B,
        &batch(1, vec![EditOp::Destroy { id: character }]),
    );
    replica.apply(&destroyed).expect("destroy referenced model");
    assert_eq!(
        replica
            .world()
            .get(replica.instance_of(player).expect("player"), "Character"),
        Ok(Value::Ref(None))
    );
    assert_eq!(
        authority.snapshot().expect("authority"),
        replica.snapshot().expect("replica")
    );
}

#[test]
fn request_numeric_bits_and_every_source_path_obey_the_input_boundary() {
    let limits = EditLimits {
        max_script_bytes: 4,
        ..EditLimits::default()
    };
    let mut authority = EditAuthority::new(World::new_empty(), limits).expect("authority");
    authority.set_access(A, EditAccess::Edit).expect("access");
    let part = EditId::new(1, 1);
    let script = EditId::new(1, 2);
    commit(
        &mut authority,
        A,
        &batch(1, vec![create(part, "Part", None)]),
    );
    let signed = batch(2, vec![set(part, "Transparency", EditValue::Number(-0.0))]);
    commit(&mut authority, A, &signed);
    assert_eq!(
        authority.submit(
            A,
            &batch(2, vec![set(part, "Transparency", EditValue::Number(0.0))])
        ),
        Err(EditError::RequestReuse)
    );
    let before = authority.snapshot().expect("before");
    let mut oversized_create = create(script, "Script", None);
    if let EditOp::Create { properties, .. } = &mut oversized_create {
        properties.push(("Source".into(), EditValue::String("ééé".into())));
    }
    assert_eq!(
        authority.submit(A, &batch(3, vec![oversized_create])),
        Err(EditError::Limit("script bytes"))
    );
    assert_eq!(authority.snapshot().expect("unchanged"), before);
    commit(
        &mut authority,
        A,
        &batch(
            3,
            vec![
                create(script, "Script", None),
                set(script, "Source", EditValue::String("éé".into())),
            ],
        ),
    );
    let cursor = authority.world().change_cursor();
    assert_eq!(
        authority.submit(
            A,
            &batch(
                4,
                vec![set(script, "Source", EditValue::String("ééé".into()))]
            )
        ),
        Err(EditError::Limit("script bytes"))
    );
    assert_eq!(authority.world().change_cursor(), cursor);
    authority.set_access(A, EditAccess::None).expect("revoke");
    assert_eq!(
        authority.submit(A, &signed),
        Err(EditError::PermissionDenied)
    );
    authority.set_access(A, EditAccess::Edit).expect("restore");
    assert!(matches!(
        authority.submit(A, &signed),
        Ok(Submission::Duplicate(_))
    ));
    assert_eq!(authority.sequence(), 3);
}

#[test]
fn five_thousand_seeded_two_editor_requests_converge_with_different_local_slots() {
    let limits = EditLimits {
        max_instances: 128,
        ..EditLimits::default()
    };
    let mut authority = EditAuthority::new(initial_world(0), limits).expect("authority");
    authority.set_access(A, EditAccess::Edit).expect("A");
    authority.set_access(B, EditAccess::Edit).expect("B");
    let mut left = EditReplica::new(initial_world(1), limits).expect("left");
    let mut right = EditReplica::new(initial_world(2), limits).expect("right");
    let workspace = authority
        .id_of(authority.world().root("Workspace").expect("workspace"))
        .expect("stable workspace");
    let mut random = 0x8a5c_19d2_73b4_001fu64;
    let mut next_request = [1u64; 2];
    let mut next_object = [1u64; 2];
    let mut live = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;
    for step in 0..5000 {
        random = random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let author = (random as usize >> 16) & 1;
        let editor = if author == 0 { A } else { B };
        live.retain(|id| authority.instance_of(*id).is_some());
        let target = if live.is_empty() {
            None
        } else {
            Some(live[(random as usize >> 20) % live.len()])
        };
        let mut created = None;
        let operation = match ((random >> 32) % 6, target) {
            (0, _) | (_, None) => {
                let id = EditId::new(editor.0, next_object[author]);
                next_object[author] += 1;
                created = Some(id);
                create(id, "Folder", Some(workspace))
            }
            (1, Some(id)) => set(id, "Name", EditValue::String(format!("item-{step}"))),
            (2, Some(id)) => EditOp::Destroy { id },
            (3, Some(id)) => EditOp::Move {
                id,
                parent: Some(workspace),
                after: Some(live[(random as usize >> 24) % live.len()]),
            },
            (4, Some(id)) => EditOp::Move {
                id,
                parent: Some(live[(random as usize >> 24) % live.len()]),
                after: None,
            },
            (_, Some(id)) => set(id, "unknown_property", EditValue::Number(f64::NAN)),
        };
        let input = batch(next_request[author], vec![operation]);
        match authority.submit(editor, &input) {
            Ok(Submission::Committed(record)) => {
                next_request[author] += 1;
                accepted += 1;
                if let Some(id) = created {
                    live.push(id);
                }
                left.apply(&record).expect("left ordered replay");
                right.apply(&record).expect("right ordered replay");
                if step % 17 == 0 {
                    assert_eq!(
                        authority.submit(editor, &input).expect("duplicate"),
                        Submission::Duplicate(record.receipt)
                    );
                    let cursor = left.world().change_cursor();
                    left.apply(&record).expect("replica repeat");
                    assert_eq!(left.world().change_cursor(), cursor);
                }
            }
            Err(_) => rejected += 1,
            Ok(Submission::Duplicate(_)) => panic!("new client sequence repeated"),
        }
    }
    assert!(
        accepted > 1000 && rejected > 100,
        "exercise acceptance and conflicts"
    );
    let expected = authority.snapshot().expect("authority snapshot");
    let left = left.snapshot().expect("left snapshot");
    let right = right.snapshot().expect("right snapshot");
    assert_eq!(expected, left);
    assert_eq!(expected, right);
    assert_eq!(tree_hash(&expected), tree_hash(&left));
    assert_eq!(tree_hash(&expected), tree_hash(&right));
}
