# #99 Reuse stored property keys

## Current status

Final tested source `2d08a846253f87d1954d235e6661bedcd05e842e`, main `238c6fb`: all mandatory gates passed.
The locked serial workspace suite exited zero: 1,585 passed, zero failed, eight ignored across 123 summaries.
Ignores comprise seven upstream tests plus this issue's new manual release-timing probe, whose planned 18
release invocations already passed separately. Strict full Clippy, licenses, formatting and whitespace passed.
The actual window/lane123 are released. Record-only final prose and PR publication/CI are next; no own merge.

All paired timing observations remain pinned to the original matching baseline/fixed probe. Wide read controls
prevent a reliable speedup claim. The deterministic result is zero warmed no-listener scalar-key allocations;
signal-key/owned-Value/first-insertion costs remain. Named release artifacts are frozen, with no copying.

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

## Executed fixed allocation and timing observations

Fixed source `b64bd23` passed all three allocation/semantic/unwind tests in debug (31.33s fresh build) and
release. Changed and no-op Position rows are zero allocations/bytes; default resets remain zero; unrelated
and matching listeners are each 1,000 / 8,000, preserving the explicitly remaining signal lookup cost.
The fixed release prebuild passed in 1m01s. Both release targets are frozen; gates use separate `99-dev`.

After explicit release from the other worker and a no-Cargo/rustc entry snapshot, the system-allocator probes
ran exactly one warm-up each plus eight alternating pairs. Every before/after executable hash matched,
including the final trial. Timing exit was zero and the window was immediately released. Median paired
fixed/baseline ratios: owned get 1.0710 (0.9574–1.5726), borrowed get 0.9800 (0.6686–1.3268), setter 0.5190
(0.3909–0.8613). Although all setter pairs were lower, the read controls moved widely. No reliable speedup,
clean-host or universal wall-time pass is claimed. CPU samples were 38.8%/34.5% busy, with no causal claim.

See `docs/research/2026-10-30-property-write-allocations.md` and its linked complete JSON evidence. Root's
independent read-only review of `b64bd23` found no actionable concern; it ran no tests. Mandatory full workspace,
strict Clippy/licenses/fmt remain next, after an explicit build/timing-window grant. No own merge or force push.

## Mandatory-gate checkpoint

The conductor explicitly released warm lane123 from completed PR #305; its prior logs/source evidence are
saved and there is no binary-retention requirement. The guard cleaned only its local workspace packages,
retaining third-party dependencies; no artifact copying. This supersedes the earlier `99-dev` full-gate plan,
while all named baseline/fixed release targets remain frozen. Workspace no-run passed in 2m36s at `c2b2009`.

Strict Clippy then returned Cargo 101 for `assert!(!cfg!(debug_assertions))` in the ignored manual probe. The
pre-loop guard now explicitly panics in debug mode; no lint suppression or measured-loop change. Original
timing results stay pinned to `56d4585`/`b64bd23` and their original identical probe fingerprint. No release
artifact is rebuilt or result relabeled. Next: refresh no-run, rerun strict Clippy, then the sole actual full
workspace window granted after #356's release. Licenses/fmt/diff already passed before this guard-only change.

Refreshed no-run and strict workspace/all-target Clippy passed at `af80ea9` (Clippy 5.78s). Before the actual
suite, the remaining listener-key allocation assertions were relaxed only downward: they enforce the current
upper bound, allowing a future signal-key optimization to reach zero. Exact observed counts remain printed
and retained; all callback/value/cursor checks and exact-zero changed/no-op/default-reset assertions stay.
This avoids freezing an unrelated known allocation as a behavioral requirement. No production/timing change.

## Final mandatory gates

At `2d08a84` on pinned main `238c6fb`, refreshed workspace no-run and strict workspace/all-target Clippy passed
(Clippy 1.56s after the final listener upper-bound change). The full guarded command
`test --workspace --locked -j 4 -- --test-threads=1` then exited zero: 1,585 passed, zero failed, eight ignored
across 123 summaries, including all Lua, Parkour and new allocation cases. No failing test was retried, removed
or newly ignored; the only new ignore is the explicitly required manual release timing probe, executed separately.

`workspace-tests.log` and `workspace-result.json` retain the exact source and log checksum under
`integration-evidence/99-reuse-property-keys`. Earlier Clippy failure/guard correction and initial fixture error
remain separate observations. The actual window was released immediately after verified exit/totals. Final
format/license/diff checks run before publication; the final commit records prose only. Open/attach the PR,
move #99 to review, and preserve exact-head CI separately from these pinned local results.
