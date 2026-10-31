# #99 Reuse stored property keys

## Scope and base

Claimed by Codex studio_core on branch `studio/99-reuse-property-keys`, exact base
`238c6fb8af7a05f031cb29930e59c8afe57cf0fa`. PRs #112 and #185 are merged; borrowed reads and cached defaults
already exist. This task changes only warmed property-map updates, preserving validation, clamping, default
removal, no-op journal behavior and signals. No schema/API/layout, signal-map, backend or UI changes.

The managed worktree tool rejected the nested Studio source path, so the repository's documented git-worktree
fallback created `metallic-studio-worktrees/99-reuse-property-keys`. The old journal/performance artifacts stay
untouched. A previously owned, idle #349 target may be reused through the lane guard after an explicit grant.

## Read-only findings

Current `World::set` inserts `prop.to_string()` into a `BTreeMap<String, Value>` even for existing keys and
unchanged values. PR #185 did not change that setter. The old A1 measurement of 1,000 allocations / 12,000
requested bytes was **Transparency**, not Position. No current Position count or speed improvement is assumed.
Owned Value clones, owned defaults, first insertion and the temporary signal-lookup key remain separate costs.

## Test-first checkpoint

Added a thread-local allocator with a Drop guard that disables counting during unwind. Fixture construction,
shared-registry warmup, journal growth past its first eviction, listener construction and postcondition checks
stay outside counts. The matrix covers 1,000 changed Position writes, no-ops, default resets on distinct Parts,
an unrelated listener and a matching listener. Values/cursors and callback count/payload checks happen separately.
Default-reset pointer identity checks confirm overrides were removed. A semantic case covers rejected types,
non-finite values, invalid enums/unknown properties, clamping, clamped no-ops and default callbacks.

The original #99 acceptance is retained as an ignored release-only get/set observation test, with owned and
borrowed reads as unchanged controls. Repeated balanced baseline/fixed executions must wait for a coordinated
timing window. Counts measure successful allocator requests, not RSS or peak/retained heap. No result has run yet.

Next: compile and execute this test-only baseline with the granted pure-core slot, retain exact counts/red,
then implement the narrow get_mut branch and rerun. All mandatory workspace, strict Clippy and license gates
remain required before a PR. Full Lua and paired timings are held until the conductor releases those windows.

## Initial baseline observation

The first test-only run returned Cargo 101. The allocation matrix reached its real postcondition checks and
printed changed/no-op Position at 1,000 allocations / 8,000 requested bytes each; default resets at zero;
unrelated/matching listener rows at 2,000 / 16,000 each. Its desired zero-key-allocation assertion failed.
A separate semantic fixture also failed because it incorrectly assumed Part.Transparency has a registry range.
That is a test-fixture error, not a product red. It is corrected to the actually ranged Lighting.ExposureCompensation
property before rerunning the baseline. Both logs will be retained. Production remains unchanged.

## Corrected baseline and implementation

At test-only `97d2b70`, the corrected baseline returned Cargo 101 with exactly one allocation regression:
two cases passed, one failed, one manual timing probe ignored. Semantic/value/cursor/callback checks passed.
The five allocation rows repeated exactly. Baseline release compilation passed (1m11s); the executable remains
in the owned #349 release output, with its path/source/hash in `baseline-provenance.json`. It has not run timings.

The production change is an existing-key `get_mut` branch between default removal and first insertion. It
replaces only the value, preserving every validation/range/old-value comparison and event path. It does not
early-return on no-ops or change signal storage. API docs explicitly retain allocation caveats. Next: focused
green and candidate release prebuild in a separate output directory, then coordinated timing/full gates.

## Timing provenance correction

The first ignored timing definition lived in the allocator-instrumented test target. It never ran. Review
identified the allocator-hook confound, so test-only `56d4585` moves it into `property_write_timing.rs`, which
uses the default system allocator and contains no counting hook. That baseline is a child of `97d2b70`, with
production identical to main. It was normally merged into the fixed branch so both checkpoints remain in
history. The uninstrumented baseline release prebuild passed in 54.65s; its separate `99-baseline-timing`
target and the original instrumented baseline remain retained. No target artifacts were copied.

`baseline-system-provenance.json` records executable/source/probe fingerprints. Candidate and baseline probe
bytes match SHA256 `e643cc85bb2e21d5ad6f7891880f4a4894ace546b1cdb1379be95f02f674c47c`. Candidate timing will
use another release target, and workspace gates will use `99-dev`, preventing later guard owner/profile
transitions from cleaning retained release artifacts. No actual timing has executed yet.
