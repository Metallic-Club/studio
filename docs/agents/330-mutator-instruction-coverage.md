# #330 Creation/destruction instruction coverage

## Goal and scope

Close the existing instruction gate's create/destroy coverage gap without treating instruction counts as wall
time or changing the 5% threshold. Issue claimed on branch `studio/330-mutator-instruction-coverage`; exact base
`d1e473755907b02ddb54365c117ffa75875f4ec0`. The managed worktree tool rejected the nested Studio source root
because it is outside this chat's listed folder roots, so the repository's prescribed git-worktree fallback was
used. Worktree: `metallic-studio-worktrees/330-mutator-instruction-coverage`.

No production engine API, dependency, workflow, transport/physics code or PR #318 source is changed. Existing
benchmark source/toolchain copying still builds identical fixed workloads at the exact PR base/head. Existing
compiler/platform/machine/Valgrind/profile/source compatibility and failure handling remain unchanged.

## Implemented, not yet Rust-validated

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
