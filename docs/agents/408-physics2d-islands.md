# Issue 408: 2D touching-body persistent islands

## Scope and source

Branch `studio/408-physics2d-islands`, pinned main `995f880ced789284354ccbb1e64b6bf841382abc`.
Claimed after confirming no active claim. The app rejected the nested repository source root, so the existing
repository worktree fallback created this checkout. Only the standalone 2D world, its tests and documentation
are changed. The separately owned #405 3D branch was read for context; none of its code was imported or edited.

## Reproduction and cause

Both new debug regressions failed on unchanged production: inserting a dynamic box already resting on a fixed
floor after island bootstrap, and rebuilding a settled box after a material change. Each panicked in rapier2d
0.36.1 `island_manager/manager.rs:468`: `touching pair (edge 0) not linked in the persistent islands`.
The command exited 101 with zero passed / two failed. Full red log is retained outside Git in
`integration-evidence/408-physics2d-islands/red-rebuild.log`; no test budget or tolerance was changed.

Rapier's [collision-only pipeline](https://github.com/dimforge/rapier/blob/v0.36.1/src/pipeline/collision_pipeline.rs)
handles body changes without the island manager and clears their change flags. Re-marking the new bodies after
that pass does not replay the already-touching contact transition. The existing 2D workaround was insufficient.
The [physics pipeline](https://github.com/dimforge/rapier/blob/v0.36.1/src/pipeline/physics_pipeline/substep.rs)
handles user changes with the island manager before contact creation; its zero-time branch returns before
solver, integration and CCD. This source was also inspected in the actual locked rapier2d 0.36.1 registry code.
A bounded primary-source issue search found no matching upstream issue; none was filed by this worker.

## Fix and focused results

`sync_from` now calls a private refresh helper that saves Rapier's tick duration, sets dt to zero, calls the
full `step_with_events` with the existing hooks/collector, then restores dt. It does not change scene poses,
configuration, accumulator, tick count, character processing or event collection. No panic-recovery claim is
made. It removes the collision-only/manual-mark workaround and keeps normal fixed stepping unchanged.

All seven focused debug regressions pass: the original insertion/material cases; collider resize with fresh
separation/recontact and no repeated steady touch; static/dynamic transition; sensor/solid enter/exit behavior;
removal/reinsertion; and repeated refreshes preserving pending half-tick, pose, linear/angular velocity and
custom settings, with immediate raycasts and the next real tick matching an unrefreshed control.

## Validation status and next

Focused command (guarded owned lane122):
`cargo test --locked -p metallic_physics2d --test rebuild_touching -j 4 -- --test-threads=1`.
Red: 0 passed / 2 failed. Green: 7 passed / 0 failed in 0.14s. Original cold/warm dependency compile evidence is
retained; package dev-dependencies include GUI libraries even though these tests perform no UI/GPU execution.
At that checkpoint, the full 2D package, workspace, strict Clippy and license gates were pending. Use four jobs, respect the
coordinator's actual Lua/GPU timing windows and pause for disk below 5 GB. Preserve historical evidence and
benchmark domains. After all gates and independent review, publish a review PR with exact-head CI; no own merge,
performance result, upstream fix or milestone acceptance is claimed.


## Final local validation

All checks passed at reviewed source `e15963cca9521fe551b631e0f91fcfc765b939ec` on exact pinned main above:

- Full 2D package: **50 passed, zero failed, zero ignored**, including all seven new regressions.
- Full workspace prebuild: passed in 5m10s.
- Strict full workspace/all-target Clippy: passed in 1m36s with `--locked -j 4 -- -D warnings`.
- `cargo deny check licenses`, `cargo fmt --all -- --check`, task whitespace: passed.
- `cargo test --locked --workspace -j 4 -- --test-threads=1` in guarded lane122:
  **1,912 passed, zero failed, eight existing upstream ignores across 141 summaries**.
  No failure was retried, excluded or weakened. Actual Lua/GPU execution used the reserved quiet window.

The coordinator independently reviewed the production helper and seven tests with no actionable concern.
After verified exit zero, lane122 and the sole actual window were explicitly released for the next task.
The final validation note changes documentation only; exact executable-source equality is recorded with the
full logs in external `integration-evidence/408-physics2d-islands`. Publish the review PR and watch its exact
head CI; do not call local checks CI or owner acceptance. No own merge, performance claim or 3D source change.
