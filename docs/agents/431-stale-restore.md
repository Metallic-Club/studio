# Issue431: destroyed-target character restore

## Scope and ownership

Branch `studio/431-stale-restore` from frozen main `8699192791825af0c8c44edb9d833339440cc508`.
Claimed after confirming ready/unclaimed. Existing app nested-repository worktree rejection permits the standard
repository fallback; this issue has its own checkout. The only executable production change is a lifetime guard
in CharacterController::restore_state. PR422 owns adjacent step/touch cleanup; its method scope remains untouched.
No runtime roster cleanup, client authority, transport or first-spawn policy is added. Completed408 is preserved.

## Executed regression

The external public diagnostic was copied into the dedicated `stale_restore` integration target. On unchanged
production, spawn/step/save/destroy/cleanup established state=None; then restoring the saved state recreated
Some(Freefall). Earlier assertions showed the World remained destroyed, its cursor unchanged and no new events.
This is now an actual confirmed failure, not the earlier source-only candidate. Baseline command exited101:
zero passed / one failed. Raw evidence is outside Git in `integration-evidence/431-stale-restore`.

## Fix and focused controls

Return immediately when the World says the target is not alive, before inserting either motion or body cache.
Do not change live-target semantics, finite-value sanitization, support or touch history, or the method signature.
The new public doc sentence and architecture note state that lifetime behavior.

All four dedicated tests pass: cleaned destroyed target stays absent with no World/event changes; live movement
and pose restore remains exact; repeated stale restore cannot affect a newly allocated character's state/Player
reference/cursor; and existing nonfinite sanitization remains active for a live target. The test does not assume
an allocator slot or a foreign-World handle mapping. No stale Instance is resurrected by either old or new code;
the bug was reintroducing controller cache entries after cleanup.

## Commands and remaining gates

Guarded owned lane122, four jobs:
`cargo test --locked -p metallic_physics --test stale_restore -j4 -- --test-threads=1`.
Baseline0/1, corrected4/0/0. At that checkpoint the full physics package and mandatory workspace/strictClippy/license/format gates were
next, with actual Lua/GPU serialized by the coordinator and a pause if disk drops below5GB. Preserve original
logs, external audit and benchmark domains. Get independent source review before publication; final source/head
and exact-head CI must be recorded. No own merge or performance/client prediction acceptance claim.


## Final local validation and disk interruption

Reviewed source `6ab2720f9dc6ba70be792b444a25020ae3457c7b` on the pinned main above passed:

- Dedicated stale/live/replacement/sanitization controls: **4 passed / zero failed**.
- Full physics package: **88 passed / zero failed / zero ignored**, preserving all existing save/restore controls.
- Full workspace no-run: first attempt intentionally stopped below the coordinator's5GB disk threshold, before
  completion (guard exit-1; no compiler error reported). Its log and interruption receipt remain preserved.
  After another released lane was cleaned, the same frozen source resumed in a separate log and passed.
- Full serial workspace: `cargo test --locked --workspace -j4 -- --test-threads=1` through lane122 guard:
  **2,019 passed / zero failed / eight upstream ignores**, across148 result summaries. No failed test was
  retried, excluded or weakened. Actual Lua/GPU execution used the coordinator's sole managed window.
- Strict full workspace/all-target Clippy: passed in34.47s with `--locked -j4 -- -D warnings`.
- License, formatting and task whitespace checks: passed.
- Independent coordinator source review: no blocker in the early lifetime guard and four tests; scope retained.

After all checks, the coordinator-authorized native Cargo clean acquired the verified lane122 lock and removed
only21 explicitly named first-party dev-profile packages. It reclaimed7.1GiB and left17,388,093,440 bytes free.
Third-party dependencies, source, all external evidence and named benchmark domains were preserved. No test or
benchmark binary retention is required for this correctness change. Actual/compile windows are released.

Logs, source/result records and cleanup receipts live outside Git in `integration-evidence/431-stale-restore`.
The final progress child is documentation only; crate-tree equality with6ab2720 will be recorded before push.
Publish a review PR and inspect exact-head CI without further source commits for status-only updates. No own
merge, new runtime/client policy or performance claim; PR422's adjacent touch cleanup remains separately owned.
