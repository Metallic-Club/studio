# Issue 298: ordered detailed core journal (M2 A1)

## Scope and checkpoint

User-authorized foundation for multiplayer and Team Create, not networking, hosting, M1 acceptance or completion
of Epic #15. Base `fbaa17d03d4321b300e6f3ce6d17dc38d5c36223`; branch `studio/298-detailed-change-journal`.
Read the current working agreement, status, M2 design A1/sections 3.2/4.3 and networking research section 4.2.
No recent overlapping A1 claim was found before creating/claiming #298.

Root approved this API before implementation:

- Copy `ChangeKind::{Created, Destroyed, Reparented, Property(&'static str)}` and `Change { id, kind }`.
- `DetailedChanges::{Changes(Vec<Change>), Overflow}` returned with the shared cursor from
  `World::changes_since_detailed`.
- Canonical property name from the existing static registry; no journal-owned String/Value or dependency on
  unpublished A2 wire IDs. Root owns `classes.rs` and will add ServerStorage to SERVICES separately; leave both
  that registry and service list untouched.
- One bounded history/cursor shared with the old reader. Descendant invalidations from reparent remain coarse
  entries without falsely reporting direct child Parent changes. Detailed reparent reports the operation root.
- Clone creation order becomes deterministic parent-before-child allocation order, replacing old randomized
  HashMap iteration. Coarse dedup, cursor identity and subtree invalidation remain otherwise compatible.
- Preserve existing same-parent reappend/event semantics (#97), not an unrelated behavior fix.

PR #186 / #104 is open and green but **not imported**. Main still uses the existing half-drop Vec algorithm;
this change will touch the journal entry shape/reader and its mutation call sites. A future integration must
retain #186's full-capacity ring/floor semantics while carrying typed metadata. Do not edit its branch.

## Tests and measurement plan

New regression file covers ordered mixed mutations/repeated writes; static inherited property tokens; direct
reparent vs coarse descendants; current same-parent behavior; subtree destruction; parent-before-child clones
with remapped refs; errors/equal-value writes; independent readers; reused slots; overflow/foreign cursors;
constructor/deep-clone history. Tests are written first; the new API does not exist on the initial base.

Measure the existing `world` Criterion workload against the exact base and candidate, with explicit isolated
paired provenance. The design's 5% target is a measurement target, not an assumed result; report regressions and
host limitations. No source edits during a build; no copied compiler cache objects between lanes. Required
focused/workspace tests, strict Clippy, licenses and fmt/diff follow; Cargo -j4 via the owned lane114 guard.

The conductor granted the first build slot after approving the API. The exact-base unchanged world benchmark
compiled in an owned `lane114/benchmark-base` output directory: Cargo reported 1m27s; complete wrapper wall time
88.92 s. Named executable/provenance and compiler JSON/stderr are retained under
`integration-evidence/298-detailed-journal` outside the checkout. No compiler artifacts were copied, and no
production source changed during that build. Timing runs are held until team compilation is quiet.

The approved implementation is now written: one typed optional-metadata entry shape and shared cursor lookup;
coarse-only reparent descendant invalidations; canonical property tokens; deterministic clone creation order.
The half-drop algorithm, SERVICES and classes.rs remain untouched. A warmed scalar-write allocation regression
allows the existing property-map key allocation but rejects an additional per-record name allocation. Focused
tests are prebuilding; no behavior, allocation or benchmark result is claimed yet. New API tests were written
before implementation but not executed against the missing API; do not describe that as an observed behavior red.

First focused run passed all 16 tests: eleven detailed-journal regressions, three existing coarse-reader cases,
and two allocation cases. The current-source incremental compile reported 5.89 s; binary test times were
0.16/0.04/0.15 s. Warmed 1,000 scalar writes requested exactly 1,000 allocations / 12,000 bytes, all attributable
to the existing property-map key cost; metadata adds no second name allocation. The existing requested-heap
fixture reports 1,172.456 bytes per Part at 5,000 Parts with this typed history; this is not RSS, a component-size
claim, zero storage overhead or a measured baseline delta. Wider bounded records do use more history storage.

The candidate benchmark will use the unchanged workload in a second owned output subdirectory,
`lane114/benchmark-candidate`, so both immutable named executables remain available across later dev-profile
guarded checks. No compiler artifacts/cache objects are copied or linked between output lanes. Isolated paired
timing is still held while other team compilation is active. The C2 worker will add its `pub mod edit;` directly
after the instance module doc and a separate `pub use instance::edit;` export; A1 leaves those locations untouched.

## Initial paired timing and compact metadata checkpoint

The fixed eight-run ABBAABBA world comparison completed successfully for all 19 matching workloads. It compared
exact base `fbaa17d` and initial source `1176dda`; no timing sample/run was removed. Settings: 20 samples,
0.3-second warmup, 1-second measurement, 10,000 resamples, no plots. The observed median paired ratios did NOT
meet the 5% target: set_position +18.70%, create_1000_parts +27.75%, deep_clone_5000 +31.28%, borrowed 5,000-Part
reads +14.21%, get_name +7.88%, get_ref_name +12.35%, save_5000 +14.94%. Ranges were wide (set_position
0.813-1.479; destroy 0.375-2.506). Short activity snapshots recorded 74.11% and 63.44% host CPU across 12 logical
processors despite the managed team hold. This does not establish causality or justify dismissing regressions
as noise; no clean-host or universal target pass is claimed. The full table/limits are in
`docs/research/2026-10-30-a1-detailed-journal.md`; raw per-run estimates/CIs, binary hashes and activity data are
retained outside the checkout under `integration-evidence/298-detailed-journal`.

Following root's review, a manual probe measured the actual entry/public type layouts before changing storage.
The private metadata now retains `&'static PropDef` and forms `Property(def.name)` on read; public API unchanged.
Entry size falls from 40 to 32 bytes, optional metadata 24 to 16 bytes; public kind/change stay 24/32 bytes.
Warm capacity/retention remain 32,768/15,425, so requested history-buffer bytes fall from 1,310,720 to 1,048,576
(20%). The 5,000-Part requested-heap fixture now reports 1,120.027 bytes/Part; warmed 1,000 writes still request
1,000 allocations / 12,000 bytes. This establishes bounded storage/allocation improvement, not a timing cause.

All 56 default core tests pass after that change. One new ignored manual layout probe was explicitly executed
before and after storage changes; it prints observations without imposing a compiler-specific size threshold.
Initial baseline/candidate artifacts are retained unchanged. Next: a separately named compact candidate, a narrow
mutation workload with unchanged controls, and mandatory workspace/Clippy/license/fmt gates. The 5% acceptance
target is still unproven and must not be silently waived. Root granted the diagnostic/candidate build slot;
coordinate actual timing windows with the Simulation worker.

The separate compact benchmark artifact built successfully at `dffc435`, tree
`1bb5b5183a3c5d6073ad1720d789003a69268c65`: Cargo 1m56s, wrapper 122.99 s. Post-build checkout was clean,
and SHA256 checks confirmed all three named executables remain separate/intact. Independent C2 peer review of
this source found no additional actionable finding in static token lifetime, shared retention, clone order or
coarse invalidation compatibility.

After the root and Simulation suites released the managed timing window, the fixed narrow plan completed all
40 selected runs: five workloads, one workload per process, four balanced adjacent pairs each, 30 samples,
0.5-second warmup, 1.5-second measurement, 10,000 resamples. Median paired changes: stored read control -31.10%,
is_a control -10.58%, set_position -6.63%, create +10.80%, destroy +9.11%. **The 5% wall-time target remains
unresolved.** All runs/CIs are retained, including wide ranges; no speedup or noise-causality assertion. Host
CPU snapshots were 95.59%/100%, with active unrelated compilation despite the managed team hold.
See the research note and committed sanitized per-run JSON. Existing PR instruction CI supplies an independent
metric after publication, not a substitute wall-time pass; no new workflow is needed.

Next: commit this evidence, refresh the unpublished branch only across a verified plan/release-only main
advance, then obtain the next released slot for full mandatory gates. No full-workspace build or further timing
is active in lane114. No server action, M1/Epic15 acceptance change, own merge or force push.
