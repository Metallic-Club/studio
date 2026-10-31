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

## Refreshed publication gates

The evidence commit was followed by one clean unpublished rebase onto main
`e20f0828454d21ae2325b671add29f9bf5c3a7c3`, producing source checkpoint
`ccc42c1862fc12f1f12f5c1f208290e6905c73e3`. The intervening changes are the M2a plan and release/template
packaging, including workspace package versions 0.1.1; no Rust source or third-party dependency versions changed.
A direct diff verifies core source/tests and the existing world benchmark remain identical to reviewed compact
checkpoint `dffc435`. Earlier benchmark observations remain tied to their original commits and are not retimed
or relabelled for this refreshed base.

At `ccc42c`: guarded forced-clean workspace prebuild passed in 7m52s (`--locked --no-run -j 4`), removing
121.2 MiB of local package outputs; compile paths identify this worktree and 0.1.1 packages. Strict workspace
all-target Clippy passed in 3m09s with `--locked -j 4 -- -D warnings`. Licenses (`licenses ok`), format and diff
checks passed. Logs are `workspace-prebuild-final.log`, `clippy-final.log`, `licenses-final.log`, `fmt-final.log`
in the local integration evidence. All three benchmark executables remain separately retained.

After the conductor's C2 timing window released, the actual guarded serial workspace suite passed on `ccc42c`:
1,006 tests, zero failures, seven manual/UI probes ignored. Both Parkour routes and every default Luau suite
passed. The seven ignores are six existing window/manual probes plus this branch's layout observation,
explicitly executed before and after compaction. Actual command: `test --workspace --locked -j 4 --
--test-threads=1`; log `workspace-tests-final.log` in the retained integration evidence. No failure was retried
or excluded. The timing window and lane114 were released immediately after verified exit and parsed totals.

All required local gates are green. Commit this final documentation record, publish for review and verify the
existing CI against the final pushed head; no further Rust build is needed for the documentation-only record.
The 5% wall-time target remains unresolved even if instruction CI passes. No own merge or acceptance claim.


## Published branch CFrame integration

PR #318 was published at `57af160657edbe27ee4f8206ffa11d33d7ac3a8e`. Main then advanced to
`abef7fc130a2c5916b30b0371897a84291abf07d` (PR #304, computed CFrame and BasePart.CFrame support).
The published branch received a normal merge, `1c48712c022c54f271ba8dd4f3709aed1e0227a1`; only
`ARCHITECTURE.md` and the core crate exports required additive conflict resolution. Both CFrame and the detailed
journal exports/documentation are retained. There is no journal implementation change, no new stored Value
variant, and no absorption of PR #186 or the unpublished A2/C2 branches.

The 1,006-test result above remains pinned to the earlier source/base. Fresh merged-source gates follow in the
owned guarded lane after package-only cleanup. Original benchmark binaries and every timing observation remain
unchanged; the wall-time target remains unresolved. No own merge or force push.


The forced-clean merged-source workspace prebuild passed in 4m39s at `1c48712`, removing 3.4 GiB of local
workspace outputs while retaining third-party caches. Strict workspace/all-target Clippy passed in 1m43s with
`--locked -j 4 -- -D warnings`; licenses, formatting and diff checks passed. Logs are
`workspace-prebuild-cframe-merge.log`, `clippy-cframe-merge.log` and `licenses-cframe-merge.log` in the same
retained integration evidence directory. All default workspace test binaries are prebuilt; actual tests await
the next managed quiet window. No previous result is relabelled as validation of this merge.


The first merged-source actual workspace run exited 101 before reaching the script crate suites: 598 passed,
one failed and four ignored across 56 summaries. `metallic_player/tests/smoke.rs::parkour_smoke_scene_passes_headless`
ran 1,200 frames without its success marker; stderr reported
`error: __mlua_index:19: Script timeout: exhausted allowed execution time`. The complete failure is retained in
`workspace-tests-cframe-merge.log`; no test was removed or budget weakened. The managed timing window was
released immediately after verified exit. A short observation taken after the exit recorded 76.16% host CPU and
active compiler/linker processes; it does not measure the failed interval or establish causality. The pending
#311 remove-player guard does not change the existing startup path and is not imported or claimed as a fix.
Merged-source full validation is not green at this checkpoint; an isolated observation and complete gate remain.


The package-selected isolated Cargo command unexpectedly began a separate feature-unification dependency build.
It was cancelled by terminating only the verified Cargo process and descendants of its lane114 guard, before
any actual test ran; the guard returned -1. No cleanup or metallic_player compile occurred. The already-built
workspace test and spawned player executables retained last-write times 23:04:15Z/23:04:22Z, before that command
started at 23:18:55Z. Their hashes/timestamps are recorded in `smoke-isolated-provenance.json`. Running that exact
smoke binary from `crates/metallic_player` passed the single smoke test in 6.03s, with no source or budget change.
This is an isolated observation, not a replacement for the failed full gate or evidence of its cause. The full
workspace command will be repeated in the next managed quiet window; no mixed artifact result is relabelled.


The complete guarded workspace repeat on unchanged `1c48712` exited zero: **1,016 passed, zero failed, seven
manual/UI probes ignored across 104 summaries**. The command selected the full workspace graph, rebuilt only
metallic_player in 17.90s before any tests, and used `--locked -j 4 -- --test-threads=1`. Both Parkour routes and
all default Luau suites passed. `workspace-tests-cframe-merge-repeat.log` and `merged-gate-results.json` retain
the complete run and both full-run totals. No source, budget or test selection was changed between the failed
full gate, the isolated observation and this complete repeat. The initial timeout remains an actual failed
observation with an undetermined cause; the passing repeat does not erase it. The actual window and lane114
were released immediately after verified exit and parsed totals.

Merged-source required gates are now complete. Publish the normal main merge plus this documentation-only
record by append-only push, then inspect existing PR #318 CI on the resulting head. The 5% local wall-time target
remains unresolved regardless of the separate instruction-count CI outcome. No own merge or new workflow.

## Explicit ordering compatibility refresh

Integrating exact main `a0bb1bfc0e84ffd637fe8d1759f4822322601cd8` preserves its #97 same-parent
`set_parent` no-op and explicit `set_parent_at` operation. The clean textual merge left two calls incompatible
with the detailed journal's private `record` signature. Those now retain upstream coarse invalidations and add
one typed `Reordered` notice for an actual same-parent/root reorder. Parent invalidation is coarse only;
unchanged descendants receive no entry. A cross-parent placement remains one direct `Reparented` notice,
whose final position is read from current state. No ordering snapshots or extra heap strings are stored.

The obsolete reappend test now checks upstream no-op semantics. New coverage exercises sibling moves in both
directions, append, root ordering, cross-parent placement, unchanged-position cursors, errors and silent
same-parent child-added signals. Formatting and whitespace checks passed; Rust execution is pending the shared
resource schedule. Earlier test and benchmark evidence remains pinned to its historical source. In particular,
the original 5% wall-time target is still unresolved; this compatibility work makes no new timing claim.

The task coordinator transferred validation/publication ownership to the Codex platform-backend worker at exact
`f23d161cc38577412919401e52a4c9a17665ed84`. Read-only review found no production semantic concern in the typed
Reordered notice, coarse parent/descendant behavior, shared cursor boundary or no-op/error paths. It found one
stale public API sentence that still described same-parent reappend as Reparented; that comment now matches the
new no-op/explicit-reorder contract. No executable behavior changed during this correction.

Cargo remains held for resource coordination after a disk-space interruption in another lane. Owned lane116
firstparty dev artifacts were cleaned using native package-scoped Cargo after path/owner/cache-tag validation;
thirdparty cache and all external historical performance/source evidence were preserved. No fresh #318 Rust
gate or timing result is claimed yet. The eight current upstream manual/UI ignores and any new layout probe
remain separate from execution success; actual counts will be recorded only after a verified run.
