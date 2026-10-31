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
