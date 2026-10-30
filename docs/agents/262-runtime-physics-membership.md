# #262 Runtime physics membership synchronization

Status: **implemented, independently reviewed and locally tested; PR publication/CI next**.

Branch: `studio/262-runtime-physics-membership`; base main
`69a10d386bd1a662cc7e0f7b55bd6edb824b57e4`. Relevant runtime/core source is unchanged
from `fd164c2`. Issue freshly checked, unclaimed, then claimed by this worker.
Before wider gates, rebased onto new main `e3e5d7802e5238362f194651629696303c2c6843`,
which includes the wave crates, default skinned avatar, quota isolation and pattern
budget fixes. Earlier focused/baseline results below remain tied to their original base.
Rebased again onto main `749316acfd11e81d7948afbd31e22be5ff20b0a3` before
publication. This advance includes production core finite-value validation, script
property-read changes, runtime project parsing, Studio templates/settings and tagged
updater code with a Velopack feature change, plus infrastructure/research merges.
It is not a documentation-only update. Both rebases were unpublished normal rebases;
the second had no conflicts. Required gates passed again on this final base. A final
fetch before publication confirmed main was still `749316a`.

## Problem and evidence boundary

The runtime gates synchronization using only cached BasePart shape fields. A changed
Humanoid ID, especially a destroyed one, does not compare against an existing part
cache entry. This can skip character-subtree membership changes that physics needs.
The issue has source permalinks. The corrected private-composition fixture now reproduces
both transitions after establishing its ordinary/character observation controls.

The external fixture is `integration-evidence/260/runtime_humanoid_gate.rs`, outside
the repositories. It checks ordinary/character controls and Humanoid-only creation
and removal using public Game/World APIs, a real server Script and captured output.
The root worker gated the frozen reviewed composition before installing this
separately identified scratch fixture for red reproduction. This fix does not
change the issue #260 tree or its reviewed production inputs.

The initial one-jump moving-sensor fixture was inconclusive because its ordinary
control also observed zero touches. The corrected fixture keeps the NPC sensor fixed
and moves the solid target in 0.1-stud increments, matching existing public physics
coverage. On test tree `6c4aeb0d3bfc744f5a291a0f6853bab8bd9a875e`, ordinary control
passed with one touch and initial-character control passed with zero; creation failed
with one instead of zero, and removal failed with zero instead of one (0.39 s).
That tree differs from reviewed `389090d57d9356144103ddd2290834e72c9aa1b8` only by
the new runtime fixture; production blobs were unchanged. The corrected fixture was
copied into this branch as `tests/physics_membership.rs` before any production edits.
On this branch's main-based source, both controls also passed and both transitions
failed with the same counts (0.30 s test execution after a 37.89 s compile). The
actual redirected output is the local `metallic-262-baseline.log` temporary artifact.

## Implementation and review

The private `physics_sync` module now owns the dependency/signature cache and actual
physics synchronization call. Game uses it for normal and forced paths, replacing
the unconditional workspace snapshot. Seven private tests measure real traversal,
part-read and sync-call sites through cfg(test)-only counters. Public runtime tests
retain the proven observation controls. Production APIs/dependencies are unchanged;
all seven private structural/lifecycle tests passed (0.13 s), and all six public
runtime cases passed (0.55 s), including creation, removal and reparent-in/out while
both ordinary/character controls remain valid.

Rebase resolutions preserve main's Avatar/default_model and locomotion imports, avatar
field/initializer, update_avatar/root_pose/root_position, skinned rendering, frame
update and avatar tests. Both runtime module sets and architecture descriptions are
retained; only the intended cache extraction/wiring differs from new main.

Root-worker review requested removal of redundant ordinary-tick candidate hashing.
The direct path now consumes World's already deduplicated journal IDs. Only a net
Model-membership transition builds a subtree union set; cfg(test)-only counters verify
zero unions on ordinary writes/pose ticks. After rebase and this change, all seven
private cases passed again (0.20 s) and all six public cases passed again (0.67 s).
The root worker coordinator reviewed the post-rebase source and conflict decisions
without a blocking finding. Review here is worker feedback, not repository acceptance.

The large-scene private test records no physics sync/full scan/part reads for ordinary
Humanoid property updates. Part pose/color changes read only that part and do not
sync. Membership changes in a small Model inspect its two affected parts and perform
one actual physics sync, leaving 500 unrelated parts outside the runtime scan.
These are runtime cache/call counts, not elapsed-time or physics-wrapper cost claims.

The full serial workspace suite on the earlier `e3e5d78` base passed 618 tests,
with no failures and one ignored manual pattern-budget probe. That result does not
cover the later `749316a` changes. On the latest base, the complete serial workspace
suite passed **647 tests, zero failures, three ignored**. Ignored cases were the two
manual property-getter diagnostic probes and the real-budget twelve-MB pattern
acceptance probe; none were run or counted as passing. The thirteen new tests,
avatar/runtime tests, updater checks and full input-driven Parkour route passed.
Strict all-target Clippy, license, formatting and whitespace checks also passed.
The ten Python performance/storage evidence-policy tests passed. Build-lane guard
tests passed 22 cases, with one Windows symlink-privilege skip (23 total).

Final commands (task-scoped lane121, four build jobs):

```text
python -I tools/cargo_lane.py --workspace . --target-dir <owned-lane121> -- test --workspace --no-run -j 4
python -I tools/cargo_lane.py --workspace . --target-dir <owned-lane121> -- test --workspace -j 4 -- --test-threads=1
python -I tools/cargo_lane.py --workspace . --target-dir <owned-lane121> -- clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --check
git diff --check
python -I tools/test_cargo_lane.py
python -I -m unittest discover -s tools/perf -p 'test_*.py' -v
```

Saved local output: `metallic-262-workspace-749.log` and
`metallic-262-clippy-749.log` temporary artifacts. No Linux Callgrind/GPU execution
is claimed from Windows checks; CI runs its own portability and instruction jobs.

## Proposed private cache

- Own the runtime journal cursor, eligible BasePart shapes, a Humanoid ID to direct
  Model parent index, and per-Model direct Humanoid counts.
- Add effective inherited character membership to the cached PartShape comparison.
  Position/Orientation and unrelated rendering fields remain excluded.
- Track only eligible Model parents under Workspace. An outside Humanoid entering
  Workspace can be discovered from its current class/parent; an exiting or destroyed
  Humanoid is identified through its old cached ID/parent.
- Preserve existing PhysicsWorld public APIs and dependencies. The cache is private
  runtime implementation, with no new public test or diagnostic hooks.

## Journal update phases

1. Process Humanoid index changes first. Compare cached old eligible parent against
   current eligible parent, including a missing current instance. Apply all count
   deltas before comparing each affected Model's final has-Humanoid boolean with
   its value before this batch. Ordinary property updates, non-last removals and a
   same-tick replacement do not cause a membership transition.
2. Consume World's deduplicated journal IDs directly on ordinary ticks. If a live
   Model's final character-membership boolean changed, union current/cached BasePart
   IDs with its descendants before reading shapes. Destroyed parts are found through
   the old part cache.
3. Compute current Workspace eligibility, shape and effective character membership
   using the updated Model-count index and ancestor chain. Compare/update only the
   candidate entries. Call physics sync only for a real physical signature or
   eligibility change, or an explicit forced sync.
4. World::set_parent journals the entire moved subtree. Its descendant BasePart IDs
   therefore catch Model/Folder/Part hierarchy moves between character ancestors or
   into/out of Workspace without retaining a second full hierarchy graph. Class
   lookup alone is never used to detect a destroyed cached Humanoid/part.
5. Initial use and journal overflow/world replacement do one full cache reconciliation.
   Start/respawn keep their explicit query-ready physics sync, but reuse an incremental
   cache refresh when history is valid. Remove the current unconditional full
   workspace_parts snapshot after every sync.

## Scope of sparse work

PR #174 is still open. Main's PhysicsWorld::sync_from itself remains a full scan;
this independent runtime fix can avoid unnecessary calls and full outer cache
snapshots using the existing API. Complete physics-layer sparsity is verified with
#174 in composition, not claimed as already merged or copied into this branch.
Likewise, complete physics-world replacement reset depends on #174. This branch
only guarantees its own runtime cache fallback on a foreign-world/overflow cursor
and preserves the existing PhysicsWorld API behavior available on main.

Use private test-only operation counts at actual traversal/sync-call sites to verify
work on a large static scene. Avoid timing thresholds and public instrumentation.
Cached entries remain bounded by live eligible parts/Humanoids/Models; deleted and
reused generation IDs must not leave stale membership dependencies.

## Test matrix

| Case | Required result |
| --- | --- |
| External public-runtime controls | Ordinary sensor emits one touch; initially classified character suppresses it; startup marker present, script errors absent |
| Humanoid-only create/remove | Correct later touch filtering after a normal end-of-tick sync opportunity |
| Humanoid reparent | Both old and new Model subtrees update, including outside/back into Workspace |
| Multiple Humanoids/net replacement | Non-last removal/additional child/same-tick replacement preserve classification without redundant subtree work |
| Nested character Models | Removing an inner marker keeps inherited outer character membership |
| Model/Folder/Part reparent | Current ancestor membership and Workspace entry/exit update without shape-field edits |
| Destroy/recreate and slot reuse | Old generation cache entries retire; transient create/destroy does not leave classification |
| Health/WalkSpeed/JumpPower/Name writes | No physics sync, full snapshot, or dependent subtree traversal when parent is unchanged |
| Part pose/color and unrelated UI/script IDs | No unnecessary physics sync or full cache reconciliation |
| Size/Shape/Material/Anchored/CanCollide | Only affected runtime cache entries; one sync decision per journal batch |
| Initial/overflow/world replacement | Runtime cache fully reconciles once, then sparse updates resume; full physics-world replacement reset is verified only with #174 |
| Respawn | Incremental cache refresh and query-ready physics before CharacterAdded |
| Full integration | Existing movement, touches, Parkour route and required repository gates remain intact |

## Next

- Root-worker review found the outline/matrix sound; world-replacement scope is
  explicitly limited above. The frozen #260 production composition passed its own
  277-test workspace gate and strict Clippy before installing the separate fixture.
- Keep progress/source docs current, open and attach a separate PR, inspect CI.
  No merges of this worker's PR or changes to published reviewed branches.
