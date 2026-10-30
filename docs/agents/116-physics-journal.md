# Physics journal synchronization: issue 116

Status: in progress.

## Goal and scope

Consume the bounded instance change journal in `PhysicsWorld::sync_from` instead of collecting and reading every part each frame. Keep the existing transform polling and simulation behavior in `step`; teleport policy and ray-normal behavior in issue 34 are separate work.

## Acceptance

- An unchanged world examines no parts, traverses no scene tree, and does not rerun collision detection.
- Sparse edits examine only journaled parts and affected character-model subtrees. Repeated edits to one instance are deduplicated.
- Creation, destruction, generational slot reuse, subtree reparenting, and Workspace membership preserve body/query behavior.
- Creating, destroying, or reparenting a Humanoid refreshes sibling/descendant character collision membership, including nested models and multiple Humanoids.
- First synchronization, journal overflow, or a different world performs a full reconciliation. Same-world overflow keeps matching bodies and momentum; replacing the world resets physics state, contacts, and accumulated time.
- Independent readers have independent cursors. A small last-sync stats snapshot records deterministic operation counts for tests/profiling.

## Design notes

The journal identifies instances rather than properties. Part signatures still decide whether a body/collider must be rebuilt. Character membership additionally depends on a Model having a direct Humanoid child; the journal does not report the unchanged sibling parts when that child changes. Cache Humanoid parents and character-model membership to expand only affected model subtrees.

`Changes::Overflow` combines dropped history and a foreign-world cursor. A minimal `ChangeCursor::same_world` helper distinguishes those cases without exposing or persisting world identity.

## Work log

- Read repository agreement, issue 116, current physics/core/renderer source, issue 34 and PR 53, and the performance research.
- Claimed issue 116 and refined its acceptance in a claim comment.
- Added regression tests first, including a deterministic 5,000-part static/sparse workload.
- Implemented incremental synchronization, cached character membership, the identity helper, and last-sync stats.
- Tests-first run failed on the missing stats API as expected. After implementation, physics unit tests passed
  (12 tests, including 10 journal regressions); core journal tests passed (4 tests).
- The 5,000-part test records 5,000 examined parts on first sync, zero on an unchanged sync, and one examined/rebuilt
  part after two signature edits to one part. This is an operation-count check, not an elapsed-time benchmark.
- Full workspace validation pending.

## Commands

Use a command-scoped `CARGO_TARGET_DIR` pointing at the shared Metallic cargo target cache. Run Cargo with `-j 4` and use the installed cargo executable explicitly. Required final gates: workspace tests, workspace Clippy with warnings denied, and dependency license check.
