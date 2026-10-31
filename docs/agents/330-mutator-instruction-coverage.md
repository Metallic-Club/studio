# #330 Creation/destruction instruction coverage

## Current status

Current integration is normal merge `7b67a95282e0755070cc05b04020bd699de8af29` of exact main `b4909b5`.
Its engine core source tree exactly equals that base (`f6eed2302ea5182b0e05680c344f521fb72f9f40`), and owned
workload/tool source is unchanged. The first real five-workload CI artifacts are retained, but their engine
snapshots differ and must not be called identical-engine calibration.

Last local source validation remains pinned to `c5f2f5a` on `d1eecce`: 1,224 workspace tests, strict Clippy,
licenses and formatting passed. Focused checks also passed: 13 Python tests, three Rust example tests, five
native release modes, matching local PDB symbols and native collector refusal of wall-time substitution.
Independent read-only review found no actionable issue. Earlier native artifacts remain pinned to `5211155`;
the core/benchmark source did not change in the initial unpublished refresh, before this later normal merge.

Cargo is held after integration pending the conductor's resource plan. New-head CI is next after append-only
push; no merged-source local gate or wall-time pass is claimed. Lane114 is idle. PR #338 and issue #330 remain
in review, with historical source/results preserved.

## Goal and scope

Close the existing instruction gate's create/destroy coverage gap without treating instruction counts as wall
time or changing the 5% threshold. Issue claimed on branch `studio/330-mutator-instruction-coverage`; exact base
`d1e473755907b02ddb54365c117ffa75875f4ec0`. The managed worktree tool rejected the nested Studio source root
because it is outside this chat's listed folder roots, so the repository's prescribed git-worktree fallback was
used. Worktree: `metallic-studio-worktrees/330-mutator-instruction-coverage`.

No production engine API, dependency, workflow, transport/physics code or PR #318 source is changed. Existing
benchmark source/toolchain copying still builds identical fixed workloads at the exact PR base/head. Existing
compiler/platform/machine/Valgrind/profile/source compatibility and failure handling remain unchanged.

## Initial implementation checkpoint (before Rust validation)

- Add `metallic_perf_create`: create exactly 1,000 additional Parts under the fixture's populated Workspace.
  It grows from 1,000 to 2,000 Parts and includes normal allocation/linking/journal costs.
- Add `metallic_perf_destroy`: individually destroy the fixture's 1,000 sibling Parts, including normal detach,
  property cleanup and journal costs. It is not a subtree drop or a loop over stale IDs.
- Keep the original fixture warmup, timing calls, postcondition checks and JSON output outside the named,
  non-inlined collection functions. Creation validates child count; destruction validates no live fixture IDs
  or remaining Workspace children. The fixed flat workload does not cover every tree shape/capacity boundary.
- Register both in the existing collector. All five workload names are now required, so missing mutator data
  and old three-workload reports fail closed; reports retain the same schema and source compatibility checks.
- Add example tests of actual mutation results and failed validation when mutations are skipped. These have
  not compiled or run yet because all Cargo is held during the conductor's actual workspace test window.
- Extend policy tests for every workload's improvement/5%/6% boundaries, omitted mutators/legacy reports,
  all named collection symbols with three retained profiles each, and failed completion without a partial
  report. Document expanded coverage and historical wall-time limits in `docs/performance-ci.md`.

## Validation so far

Tests first: before registering the new workloads, the 10 measurement-policy tests produced 12 expected
subcase failures exposing absent symbols, ignored mutator regressions and accepted incomplete reports.
After the registry change, `python -m unittest discover -s tools/perf -p 'test_*.py' -v` passed all 13 tests
(10 measurement-policy tests plus three existing storage-evidence tests). Mocked Callgrind profiles verify
collector policy; they are not real instruction measurements. `git diff --check` passed.

## Next and boundaries

Wait for a Cargo slot. Use owned lane114 through the current cargo_lane guard, with forced package-only cleanup
on this new worktree's owner transition and four jobs. First compile/run the exact core example's three tests,
build its release artifact, run all five modes and verify successful JSON/completion plus symbol provenance.
Real Callgrind measurement requires Linux; the existing PR CI must retain real base/head artifacts when
available. Then run required full workspace tests, strict all-target Clippy, licenses and formatting before PR.
Coordinate actual timing-sensitive suites; never run a third team compiler or weaken a budget to pass.

PR #318's wall-time target remains unresolved, including retained local creation +10.80% and destruction
+9.11% observations under documented host activity. New instruction coverage cannot erase those results or
establish a wall-time pass. No own merge, force push, threshold relaxation or new workflow.


## First focused Rust feedback

The conductor released a focused build slot after its actual workspace suite exited. Formatted this branch and
ran guarded `test --locked -p metallic_core --example perf_ci -j 4 -- --test-threads=1` in lane114 with forced
package-only owner cleanup (3.5 GiB). The exact example compiled in 8.12s; all three tests passed in 0.03s:
creation count/parent/class and existing sibling preservation, destruction with Workspace retained, and skipped
mutations rejected. Log: `integration-evidence/330-mutator-instructions/example-tests.log`.

A separately retained release example is now building in the owned `lane114/330-example-release` subdirectory,
with release debug level 1 and empty RUSTFLAGS, matching instruction CI's build settings. No compiler artifact
is copied or linked between lanes. Actual native modes/collector/symbol evidence remain pending. Broad required
gates are still held until the conductor grants a slot.


Focused checkpoint `521115517b3610e00f6dd4f5619f02873c3b8df5` passed the remaining native checks. The separate
release+debug1 build completed in 1m02s. All five modes returned successful completion JSON; an unknown mode
returned exit 1. A local-only, single-threaded Windows DbgHelp check resolved all five exact collection names
from this binary's matching PDB; no symbol server, external process or host address was used. It follows
Microsoft's [SymFromNameW](https://learn.microsoft.com/en-us/windows/win32/api/dbghelp/nf-dbghelp-symfromnamew)
and [SYMBOL_INFOW](https://learn.microsoft.com/en-us/windows/win32/api/dbghelp/ns-dbghelp-symbol_infow) APIs.

Source SHA256: `5139f102cb1359040af83d4e282361674e73215543896b0daedc9a3c2ed25d14`.
Release binary SHA256: `6b429a1936bc831ca43c0bdef5b5b7400d180d97957815ee43666e13baf86052`.
PDB SHA256: `b568e70b9b2476b81db307272749135cafa6d701fabe00909fb99f2d47107950`.
The local-only helper and `native-artifact-validation.json` retain names, module-relative addresses and artifact
provenance under `integration-evidence/330-mutator-instructions`. Public symbol sizes are zero in this debug
format; no function-size claim is inferred. Windows symbol resolution is not Linux Callgrind collection.

The real native collector retained three samples for each of the five workloads in `native-collector/report.json`.
These unpaired nanoseconds are harness-completion observations, not a performance baseline or threshold pass.
Passing that native wall-time report to the instruction comparator correctly returned exit 2 and no comparison
summary (`walltime-comparison-refusal.log`). Existing mocked policy tests still supply no real instructions;
real base/head Callgrind evidence is pending PR CI. Since this task changes no engine implementation, its own
base/head gate calibrates the expanded harness rather than measuring PR #318's unmerged implementation.

Both focused commands have exited and the slot was released to the conductor. Broad required gates remain
queued; no local Cargo is active. No whole-workspace or instruction acceptance claim at this checkpoint.


## Single pre-gate main refresh

Before broad gates, the shared main ref advanced to `d1eeccee2c3d46ca61921baffa562af4bd9472e8` (PR #323
Controls and PR #329 Studio data-folder ownership). There were no changes to metallic_core, perf tools or the
performance workflow. After committing focused evidence, one unpublished rebase completed cleanly, producing
`c5f2f5ad64e58b232b887358d5b1eecc5075e038`. A direct diff verifies the example, measurement source/tests and
performance docs are identical to focused checkpoint `5211155`; all incoming controls/data-folder code remains
preserved. Earlier focused artifacts remain pinned to their original source and are not relabelled. No published
branch was rewritten. Full guarded source validation awaits its next granted slot; lane114 is idle.


Independent C2 read-only review at `c5f2f5` found no actionable issue in fixed mutation counts, collection/setup
boundaries, completion checks, exact workload registration or fail-closed threshold/report handling. Reviewed
git blobs: example `b81a5f739612c37ffc713a123e2f374fc1a7f834`, collector
`53473a413e98e40cbc27e6199a850e8920bfbdfe`, policy tests
`91db250d795d04e0ac19d481a11190287be12c4c`. The reviewer performed no edits, builds or tests.


## Broad compile gates

After the conductor released the compile slot, guarded forced-clean workspace prebuild passed in 4m06s at
`c5f2f5a`/`d1eecce`. Command: `test --workspace --locked --no-run -j 4`. Package-only cleanup removed 204 local
files / 27.2 MiB and retained third-party caches and the separately named release example. Compile paths identify
this exact worktree. Strict `clippy --workspace --all-targets --locked -j 4 -- -D warnings` passed in 1m29s.
Formatting/diff checks and `cargo deny check licenses` (`licenses ok`) also passed. Logs are `workspace-prebuild.log`,
`clippy.log` and `licenses.log` in the retained integration evidence directory.

No actual workspace test has run at this checkpoint. Its binaries are ready, and the lane was released after
verified Clippy exit. `prebuilt-smoke-provenance.json` records the current workspace smoke and spawned player
hashes/timestamps before execution, avoiding an ambiguous feature-graph comparison if diagnosis is later needed.
The conductor owns the next serial timing-window grant. No source or budget changed during these checks.


## Final actual workspace gate

The conductor released the actual timing window after #305 exited. The guarded full workspace command on
unchanged `c5f2f5a`/`d1eecce` exited zero: **1,224 passed, zero failed, six existing ignored tests across 109
summaries**. Command: `test --workspace --locked -j 4 -- --test-threads=1`. Both Parkour routes and every default
Luau suite passed. No package-only graph, test exclusion, budget change or failure retry was used. The actual
window/lane was released immediately after verified exit and parsed totals. `workspace-tests.log` and
`workspace-results.json` retain the result and log checksum under the same integration evidence directory.

The earlier three example tests and 13 Python policy/storage tests are recorded separately, not added to this
workspace count. Required prebuild/Clippy/licenses/format checks are complete. Commit this documentation-only
validation record, push the owned branch, open/attach one PR and set issue #330 to review. Real five-workload
Callgrind artifacts remain pending CI; no new source commit or rerun is needed just to record later CI metadata.
Keep the named native artifact and all old #318 evidence intact. The wall-time target remains unresolved.


## Exact-main normal merge for attributable CI

PR #338 was published at `c2c9556ab860cc47c8ed36f5093574549b5b6ea5`, with actual base
`b4909b582241cd1f5c946aa69a7e91021a8271d4`. Main had acquired real core lighting/class/instance changes after
local pin `d1eecce`; the engine source trees differed. The first real Callgrind run completed all five workloads
and passed the unchanged gate, but it compares those different snapshots: reads -0.94%, writes -0.94%, clone
0.00%, create 310,696/310,696 Ir, destroy 4,447,067/4,447,067 Ir. It is not identical-engine calibration, isolated
measurement-change cost or a wall-time pass. Full reports/profiles/comparison are retained in
`integration-evidence/330-mutator-instructions/ci-first-c2c9556`; the PR body explicitly recorded this caveat.

At the conductor's request, normal merge `7b67a95` integrated that exact main without conflicts. Before committing,
the staged tree was `4890520edb1138f52bfcb7f9378b30a21b34b520`. Verification showed its core src tree exactly
matches base `f6eed2302ea5182b0e05680c344f521fb72f9f40`; `perf_ci.rs`, `measure.py`, `test_measure.py` and
`docs/performance-ci.md` match the prior published head byte-for-byte. The check is retained in
`main-integration-before-commit.json`. No production engine changes were handwritten and no old source result
was relabelled. No threshold, workflow or dependency policy was relaxed.

Record this integration, push append-only and inspect the next real CI comparison. It is attributable as a
harness calibration only if that CI event uses the same engine core baseline. Any future mismatch must remain
explicit rather than trigger a silent main chase. Cargo stays held after integration; no new local Rust build.
