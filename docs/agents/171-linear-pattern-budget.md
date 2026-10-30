# Issue 171: linear pattern work and per-attempt complexity

Goal: ordinary linear scans/substitutions must not exhaust a complexity allowance shared by every starting
position and match. Branch `studio/171-linear-pattern-budget`, worktree `171-linear-pattern-budget`, base
`fd164c2`. No code from pending pattern or error-formatting branches is imported.

## Design and safety

Reset only the complexity allowance for each outer match attempt; retain a separate continuous work counter
for deadline polling across short attempts. Keep the 10 million per-attempt cap, recursion/capture/output
limits, 100 ms script budget, memory cap and sticky timeout behavior. A per-attempt work cap remains a coarse
bound, not a proof that a pattern is nonlinear.

Issue #166 is actively claimed by metallic-1e-work (claim at 11:31 UTC, branch
`studio/166-pattern-class-steps`). It owns bracket-class and max_expand work charging. This issue's State/tick
accounting may overlap during integration; do not import or modify that worker's class/scan changes. Existing
uninterruptible bracket-class/scan risks remain tracked by #166.

## Tests first

Added deterministic native-call regressions for the two 12 MB issue repros and gmatch's many-start search.
These deliberately run without a scheduler deadline to isolate complexity accounting. Empty substitutions,
zero-length captures and gmatch exhaustion compare with native Luau. The actual budgeted acceptance must be
measured separately; a timeout under the unchanged budget will not be reported as success.

Baseline on unchanged production code: the match, gsub and gmatch 12 MB regressions all failed with the expected
`pattern too complex`; the native Luau empty-match/boundary comparison passed. The focused binary took 2.59 s
after a 20.52 s compile, with no scheduler deadlines. Guarded lane ownership cleanup removed 969.5 MiB of local
package output and verified both core/script source paths from this worktree.

Read the pinned Luau 0.740 `VM/src/lstrlib.cpp` implementation and the
[official library reference](https://luau.org/library/#string-library) for match/capture, substitution count,
empty-match progress and iterator behavior. Test small boundary cases against the pinned native VM rather
than relying on pattern intuition. The per-attempt step cap and deadline polling are engine policy, not a
Luau language rule.

## Measured refinement

Counter separation passes all three private accounting tests: a fresh per-attempt allowance, uninterrupted
polling over 4,096 one-step failures, and preservation of partially accumulated poll work. The four native
correctness/boundary integration tests passed after the change. Debug no-deadline observations: 12 MB match
555.2731 ms, gsub 1.7216902 s; these exceed the real script budget and are not acceptance successes.

Counter separation alone left both actual repros timing out in debug and release around 100–102 ms. Added a
narrow single-literal-byte path for match, and for same-length single-byte gsub string/number replacements.
Both process 4,096-byte chunks, check the same scheduler deadline and mark timeout sticky through stop_error;
gsub checks before allocating its output and both paths check after result creation. Conservative magic-byte
exclusions keep malformed patterns on the general matcher. Function/table replacements and percent expansion
remain on the general matcher. No native uninterruptible pattern function or pending-branch code is used.

Revised focused checks: four private accounting/expired-deadline tests and eight native/differential cases
passed. These cover all 256 single-byte patterns, binary/non-UTF8 subjects and replacements, negative/init
bounds, finite max_n truncation, replacement counts, empty matches, iterator boundaries, percent expansion,
function/table replacements, and preservation of the current exceptional-number maximum policy. The separate
many-short-attempt timeout test uses a general optional pattern, not the accelerated literal case, and passed
with sticky timeout/recovery in 0.12 s before the fast-path refinement; it remains in the final full gate.

Actual probes under the unchanged 100 ms budget (coordinated builds held; OS/background activity unmeasured):

| Build | Precreated match | Precreated gsub | Original end-to-end match | Original end-to-end gsub |
|---|---|---|---|---|
| Debug, earlier focused | succeeded, 1.8918 ms | timeout, 100.8655 ms | succeeded, 6.7841 ms | timeout, 101.5283 ms |
| Debug, final workspace | succeeded, 2.2638 ms | succeeded, 85.2862 ms | succeeded, 6.9565 ms | succeeded, 80.8623 ms |
| Release, final | succeeded, 7.8784 ms | succeeded, 44.9659 ms | succeeded, 8.6664 ms | succeeded, 45.4529 ms |

The release and final workspace debug rows ran after the pre-allocation deadline review fix. Earlier focused
debug results preceded only that additional check. These are individual observations with different Cargo
feature graphs, not a cross-profile speed comparison or a universal latency claim. The original release repro
acceptance is met, and the final workspace debug probe also passed. Earlier debug gsub timeouts remain part of
the evidence: the deadline depends on build configuration and machine load.

The manual probe is ignored by default because it measures actual wall-clock acceptance; its release build
asserts 4/4 successes after validating returned bytes/counts. An initial optimized probe rejected an Integer
count because the fixture assumed Number only. Corrected that extractor to accept either numeric variant;
this was a fixture error, not a product failure. The corrected complete release probe passed 4/4.

## Final local validation

- `cargo test --workspace -j 4 -- --test-threads=1` through the reviewed lane guard: 226 passed, zero failures,
  one explicitly named manual probe ignored. Includes the new >64 MB gsub-output rejection with max_n=0,
  unchanged pathological tests and all Lua timeout/recovery suites.
- The manual probe was separately exercised against the final workspace debug build and optimized build:
  4/4 actual budgeted successes in each, with observations and earlier limitations above.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings` through the lane guard: passed, 21.46 s.
- `cargo deny check licenses`: passed (`licenses ok`; existing unused MPL-2.0 allowance warning).
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Optimized existing tests, unchanged: `cargo test -p metallic_script --test vm --test vm_round2 --release
  pathological -j 4 -- --nocapture --test-threads=1` through the lane guard: both passed (0.17 s and 0.50 s).
- Independent read-only accounting/helper review: no blocking findings. The requested deadline check before
  gsub buffer allocation was implemented before the final full gate and optimized probe.

All builds used the owned `cargo-target-114` lane and `-j 4`. Forced local-package cleanup before the workspace
prebuild removed 77.0 MiB, retained third-party cache and compiled all seven packages from this worktree in
55.52 s. A subsequent test-only cap regression was compiled with the exact workspace feature graph before the
actual suite. Release pathological binaries were separately prebuilt before their quiet timing window.

All required local gates are complete. Next: publish the PR, inspect final CI and review feedback; do not merge.
