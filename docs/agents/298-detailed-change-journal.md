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
