//! Property tests: random operation sequences keep the mesh valid; the closed-manifold-preserving subset keeps a
//! cube a closed, consistently wound sphere-like surface; undoing everything gives back the start, redoing everything
//! gives back the end.

mod common;

use std::collections::BTreeSet;

use common::*;
use glam::{Quat, Vec3};
use metallic_modeling::{
    DeleteKind, EditMesh, History, Mesh, MeshCommand, Op, SelectMode, Selection, Symmetry,
    Transform,
};
use proptest::prelude::*;

/// One random step: an operation kind plus seeds for picking elements and parameters.
#[derive(Debug, Clone)]
struct Step {
    kind: u8,
    pick: Vec<u16>,
    amount: f32,
}

fn step() -> impl Strategy<Value = Step> {
    (
        0u8..16,
        prop::collection::vec(any::<u16>(), 1..4),
        0.05f32..0.6,
    )
        .prop_map(|(kind, pick, amount)| Step { kind, pick, amount })
}

fn pick<T: Copy + Ord>(items: &[T], seeds: &[u16]) -> BTreeSet<T> {
    if items.is_empty() {
        return BTreeSet::new();
    }
    seeds
        .iter()
        .map(|&s| items[s as usize % items.len()])
        .collect()
}

/// Builds the op for a step against the current mesh. `closed_only` restricts to operations that keep a closed
/// manifold closed.
fn op_for(mesh: &Mesh, s: &Step, closed_only: bool) -> (Selection, Op) {
    let faces = mesh.face_ids();
    let edges = mesh.edge_ids();
    let verts = mesh.vert_ids();
    let fsel = if closed_only {
        // A face and one edge-neighbour: a region whose faces share an edge (vertex-only contact would make the
        // region's extrusion legitimately non-manifold, as in other modellers).
        let f = faces[s.pick[0] as usize % faces.len().max(1)];
        let mut set = BTreeSet::from([f]);
        if let Some(&e) = mesh.face_edges(f).get(s.pick[0] as usize % 3) {
            set.extend(mesh.edge_faces(e).iter().copied());
        }
        Selection::from_faces(mesh, set)
    } else {
        Selection::from_faces(mesh, pick(&faces, &s.pick))
    };
    let esel = Selection::from_edges(mesh, pick(&edges, &s.pick));
    let vsel = Selection::from_verts(mesh, pick(&verts, &s.pick));
    let kind = if closed_only { s.kind % 9 } else { s.kind };
    match kind {
        0 => (
            fsel,
            Op::ExtrudeFaces {
                distance: s.amount,
                individual: false,
            },
        ),
        1 => (
            fsel,
            Op::ExtrudeFaces {
                distance: s.amount,
                individual: true,
            },
        ),
        2 => (
            fsel,
            Op::Inset {
                thickness: s.amount * 0.2,
                depth: -s.amount * 0.1,
                individual: s.pick[0].is_multiple_of(2),
            },
        ),
        3 => {
            let e = edges[s.pick[0] as usize % edges.len().max(1)];
            (
                Selection::new(SelectMode::Edge),
                Op::LoopCut {
                    edge: e,
                    cuts: 1 + u32::from(s.pick[0] % 3),
                },
            )
        }
        4 => (
            vsel,
            Op::Transform(Transform::translate(Vec3::splat(s.amount))),
        ),
        5 => (
            vsel,
            Op::Transform(
                Transform::rotate(Quat::from_rotation_y(s.amount), Vec3::ZERO)
                    .with_symmetry(Symmetry::default()),
            ),
        ),
        6 => (fsel, Op::Subdivide { levels: 1 }),
        7 => (
            Selection::from_edges(mesh, pick(&edges, &s.pick[..1])),
            Op::Bevel {
                width: s.amount * 0.05,
                segments: 1 + u32::from(s.pick[0] % 3),
            },
        ),
        8 => (fsel, Op::FlipNormals),
        9 => (fsel, Op::Delete(DeleteKind::Faces)),
        10 => (vsel, Op::MergeAtCenter),
        11 => (esel, Op::DissolveEdges),
        12 => (vsel, Op::DissolveVerts),
        13 => (
            esel,
            Op::ExtrudeEdges {
                offset: Vec3::Y * s.amount,
            },
        ),
        14 => (fsel, Op::Delete(DeleteKind::OnlyFaces)),
        _ => (vsel, Op::MergeByDistance { distance: s.amount }),
    }
}

fn run_sequence(steps: &[Step], closed_only: bool) -> Result<(), TestCaseError> {
    run_sequence_counting(steps, closed_only, &mut [0; 16]).map(|_| ())
}

/// As `run_sequence`, also counting successful steps per operation kind in `ok`.
fn run_sequence_counting(
    steps: &[Step],
    closed_only: bool,
    ok: &mut [usize; 16],
) -> Result<(), TestCaseError> {
    let mut edit = EditMesh::new(cube());
    let start = edit.mesh.content_hash();
    let mut history = History::new();
    for s in steps {
        if edit.mesh.face_count() == 0 || edit.mesh.edge_count() == 0 {
            break;
        }
        // Keep the meshes small enough for a fast test.
        if edit.mesh.face_count() > 400 {
            break;
        }
        let (sel, op) = op_for(&edit.mesh, s, closed_only);
        edit.selection = sel;
        let before = edit.mesh.content_hash();
        match MeshCommand::run(&mut edit, &op) {
            Ok(cmd) => {
                let kind = if closed_only { s.kind % 9 } else { s.kind };
                ok[kind as usize % 16] += 1;
                history.push(cmd);
            }
            Err(_) => {
                // Failed operations must leave the mesh untouched.
                prop_assert_eq!(
                    edit.mesh.content_hash(),
                    before,
                    "failed {} changed the mesh",
                    op.label()
                );
            }
        }
        if let Err(e) = edit.mesh.validate() {
            return Err(TestCaseError::fail(format!("after {}: {e}", op.label())));
        }
        if closed_only {
            if matches!(op, Op::FlipNormals) {
                // Flipping some faces breaks orientation; recalculate to restore it.
                let all = Selection::from_faces(&edit.mesh, edit.mesh.face_ids());
                edit.selection = all;
                history
                    .apply(&mut edit, &Op::RecalculateOutside)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
            }
            prop_assert!(
                edit.mesh.is_closed_manifold(),
                "after {}: not closed manifold",
                op.label()
            );
            prop_assert!(
                edit.mesh.is_consistently_oriented(),
                "after {}: winding",
                op.label()
            );
            prop_assert_eq!(edit.mesh.euler_characteristic(), 2, "after {}", op.label());
        }
    }
    let end = edit.mesh.content_hash();
    while history.undo(&mut edit).is_some() {}
    prop_assert_eq!(edit.mesh.content_hash(), start);
    while history.redo(&mut edit).is_some() {}
    prop_assert_eq!(edit.mesh.content_hash(), end);
    edit.mesh
        .validate()
        .map_err(|e| TestCaseError::fail(e.to_string()))?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn random_ops_keep_invariants(steps in prop::collection::vec(step(), 1..10)) {
        run_sequence(&steps, false)?;
    }

    #[test]
    fn closed_preserving_ops_keep_a_closed_manifold(steps in prop::collection::vec(step(), 1..8)) {
        run_sequence(&steps, true)?;
    }
}

/// Deterministic companion to the property tests: over many pseudo-random sequences, every operation kind must
/// succeed at least a few times, so the properties above are not passing just because operations fail.
#[test]
fn every_random_op_kind_succeeds_sometimes() {
    let mut seed: u64 = 0x5eed_1234_abcd_0001;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as u32
    };
    for closed_only in [false, true] {
        let mut ok = [0usize; 16];
        for _ in 0..300 {
            let steps: Vec<Step> = (0..1 + next() % 6)
                .map(|_| Step {
                    kind: (next() % 16) as u8,
                    pick: (0..1 + next() % 3)
                        .map(|_| (next() % 65_536) as u16)
                        .collect(),
                    amount: 0.05 + (next() % 1000) as f32 / 1000.0 * 0.55,
                })
                .collect();
            if let Err(e) = run_sequence_counting(&steps, closed_only, &mut ok) {
                panic!("sequence failed: {e}");
            }
        }
        let kinds = if closed_only { 9 } else { 16 };
        for (k, n) in ok.iter().enumerate().take(kinds) {
            assert!(
                *n >= 3,
                "op kind {k} succeeded only {n} times (closed_only = {closed_only}): {ok:?}"
            );
        }
    }
}
