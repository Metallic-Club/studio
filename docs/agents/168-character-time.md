# #168 Ignore non-finite character frame times

Goal: NaN and both infinities are no-ops before world/controller mutation; finite
times share the existing MAX_STEP cap so extreme values cannot overflow motion or
fast-forward respawn time.

## Scope

- Preserve normal finite behavior and finite negatives treated as zero. Apply the
  issue-requested shared MAX_STEP (0.25 s) cap to long finite values, including f64::MAX.
- No unrelated input, JumpPower/Health property sanitization or physics changes.
- Fresh branch/worktree `studio/168-character-time` from main `ebb94f3`.

## Done

- Claimed #168 after confirming ready/unclaimed.
- Read character movement/death/state handling: dt.max(0) occurs after controller
  maintenance and lets +infinity through; NaN-as-zero can still apply jump/input.
- Added API-level tests with real World/PhysicsWorld/CharacterController: cold
  invalid input, grounded jump suppression, airborne gravity recovery, dead timer
  and respawn, plus preserved finite zero/negative/normal behavior.
- Tests check world journal, pose, controller state/velocity, events and subsequent
  normal evolution against an identical reference scene.
- Added the first-operation non-finite guard and immediate shared finite clamp before
  maintenance/state/input/timer/movement mutations. Reused the already-public MAX_STEP;
  updated its docs, controller docs and architecture.
- Four invalid-time tests failed against the original controller; the finite baseline
  case passed. Three long-finite tests then failed before the cap: 0.5 s versus 0.25 s,
  extreme finite motion, and death-timer evolution. All eight focused tests now pass.
- The extreme-finite probe runs in its own hidden child test process with a 10 s
  watchdog, allowing safe baseline failure without hanging the main test runner.

## Next

- CI and maintainer integration after PR delivery. Do not merge this session's PR.

## Final validation

- Eight focused API tests passed after both the non-finite guard and finite cap.
- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: passed again on the final
  implementation, including all eight new tests, existing physics/character cases,
  full input-only Parkour route, player smoke, Luau and auth suites.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed (existing unused MPL-2.0 allowance warning).
- `cargo fmt --check`, `git diff --check`: passed.
- Independent read-only review passed for the final guard/shared cap and all eight
  test paths. No additional actionable findings. No dependencies or visibility changes.

## Coordination

No Cargo started while #205/#201 timed suites were active. Use the reviewed #209
lane helper with workspace selection when switching the existing lane121 here;
it must clean only local workspace packages and preserve third-party cache data.
