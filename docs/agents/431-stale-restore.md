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
Baseline0/1, corrected4/0/0. Full physics package and mandatory workspace/strictClippy/license/format gates are
next, with actual Lua/GPU serialized by the coordinator and a pause if disk drops below5GB. Preserve original
logs, external audit and benchmark domains. Get independent source review before publication; final source/head
and exact-head CI must be recorded. No own merge or performance/client prediction acceptance claim.
