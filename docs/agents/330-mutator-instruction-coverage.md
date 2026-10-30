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
