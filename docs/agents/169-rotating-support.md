# #169 Carry riders on rotating support

Goal: off-center riders follow a rotating platform and standing facing follows its
yaw, while movement input stays in world space and airborne momentum stays stable.

## Audit and design

- Fresh ready/unclaimed issue, branch `studio/169-rotating-support`, initial base
  `ebb94f3`; rebased onto `12a6a6b` before full workspace validation.
- #32 is closed and angular rates are available in radians/second. #34 / PR #53
  remains open; do not duplicate teleport cutoffs, and consume the reported rates.
- Anchored movers are reset to fixed bodies with zero Rapier velocities after each
  step; cached derived rates must be used for their support motion/momentum.
- Exact cached Rapier 0.36.1 docs/source confirms cast_shape witness1/normal1 are
  world-space collider contact data. RigidBody.velocity_at_point handles world COM.
  Versioned docs.rs URLs were attempted but inaccessible through the web tool;
  these API observations are verified against the actual installed package source.
- Implemented private body-local foot contact plus sampled body pose; standing uses
  the completed contact arc, avoiding tangent-integration radius drift. Jump momentum
  uses Rapier world-COM point velocity for dynamic bodies and cached origin velocity
  plus angular cross-product for anchored bodies. Radians/second becomes degrees for yaw.
- Project the capsule support point onto the ground hit plane to stabilize coplanar
  witnesses. Restore rotating-support skin clearance under the existing MAX_PUSH cap: the
  first prototype exposed occasional whole-step clipping when a floor's skin gap
  drifted about 0.005 stud. Witness stabilization alone did not fix that failure.
- Body-handle identity rejects rebuilt/removed supports; an externally changed root
  position rejects stale contact momentum. No new public API or dependencies.
- Keep the capsule upright, use world-up yaw, preserve explicit movement facing.

## Validation

- Actual controller-before-physics frame ordering, rider eight studs off the axis.
- Signed quarter turns, standing yaw, two revolutions with per-frame positional
  checks, stationary/translation/stop, jump momentum and landing recovery, explicit
  world-space input/facing, small signed tilts/contact lever arm, body replacement,
  script root repositioning during a turn, and a real dynamic platform dropping onto
  a floor while retaining its rider through impact.
- Against original main, six of seven initial tests failed (rotation, facing,
  tangential jump, input composition and tilt), while stationary/translation passed.
- First prototype passed six of seven; repeated turns exposed skin clipping. The
  added script-reposition regression also failed before rejecting stale contacts.
- Broad skin restoration initially regressed the existing walking-height tolerance
  (0.074 stud versus 0.05 allowed). The final correction only applies to a rotating
  support's probed contact plane and uses the remaining MAX_PUSH budget. Ordinary
  stationary/pure-translation walking retains its existing correction policy.
- Final `test -p metallic_physics -j 4 -- --test-threads=1`: all 47 tests pass,
  including all ten new cases and existing speed/height, slopes, steps and push-cap checks.
- Final `test --workspace -j 4 -- --test-threads=1`: **222 passed, 0 failed,
  0 ignored**, including the complete input-only Parkour route, player smoke tests,
  rendering checks and Luau suites. Workspace binaries were prebuilt before the
  coordinated quiet test window.
- `clippy --workspace --all-targets -j 4 -- -D warnings`: passed (19.16 s).
- `cargo deny check licenses`: passed (`licenses ok`; existing unmatched MPL-2.0
  allowance warning). `cargo fmt --all -- --check` and `git diff --check`: passed.
- The first workspace prebuild required a missing trait import for an intermediate
  broad AABB expansion; that expansion and import were subsequently removed.
- Commands use `python -I ../209-build-lane-guard/tools/cargo_lane.py --workspace .
  --target-dir <LOCALAPPDATA>/Metallic/cargo-target/lane121 -- ...`, which cleaned
  the previous worktree's local workspace artifacts before this worktree's first build.
  Focused command: `test -p metallic_physics --test rotating_support -j 4 -- --test-threads=1`.

## Limits

- Intended order is one controller step per completed physics tick (runtime fixed 60 Hz).
  This does not claim an arbitrary independently scheduled controller/physics integrator.
- Collision sweeps can shorten the carry; capsule/avatar remain upright, without
  support pitch/roll camera rotation or rigid parenting. Small signed tilts are tested;
  this is not an assertion that every rotating rigid-body configuration is stable.
- #34 / PR #53 teleport classification remains pending. Reported zero-rate motion is
  skipped, but this PR does not implement a second displacement/angle cutoff.
- Peer review requested an abrupt-stop dynamic support check because instantaneous
  zero rates can differ from the completed pose delta. The real falling/landing test
  passes without changing that gate; it does not prove every dynamic stop configuration.

## Review follow-up (2026-10-30)

- Merged `origin/main` (#343 Humanoid clamps / out-of-world kill / `in_world`, #331 quantised
  `LocalInput`, #348 test temp dirs, M2a `CharacterState`). Only ARCHITECTURE.md conflicted;
  `CharacterState` gained `support: Option<SupportContact>` so save/restore replays keep the carry.
  #53 (teleport rule) is still open and not merged.
- Finding 1: carry the bottom-sphere centre (straight below the root) instead of the foot contact;
  the contact is kept only as the jump lever arm. The carry is swept first, ignoring the support
  itself, then pushes apply: sweeping/pushing against the already rotated support caused a
  tangential lag of about 0.9 deg per 10 s at 20 deg tilt. Test `tilted_spinning_support_keeps_rider_radius`
  (8 and 20 deg, 30 deg/s, 10 s): radius error under 0.02 stud (measured 8e-6 and 2e-5); was 0.74 at 8 deg.
- Finding 2: facing turns by the twist around +Y of the support's pose change, not `w.y * dt`.
  Test `facing_and_position_agree_under_variable_dt` failed before (0.67 deg off on frame 1).
- #273 "mover lags one tick": not fixed. With a Heartbeat script moving a platform at 4 studs/s, the
  rider stays a constant 2 ticks (0.133 stud) behind the drawn platform, without drift. The cause is
  the tick order (characters, physics, scripts), which the M2 spec keeps.

## Review and next

- Core independently reviewed the support/probe geometry. Root reviewed the overall
  support motion before the final clearance narrowing. Dynamic stop caveat retained.
- Validation used main `12a6a6b`; final rebase onto `fd164c2` picked up only #180
  input-action design docs. `git diff 4ce5eb8..HEAD -- crates Cargo.toml Cargo.lock`
  was empty after rebase, so the tested Rust source and dependency graph are identical.
  Final public behavior docs are in the character module and ARCHITECTURE.md.
- Publish the branch, open/attach PR and inspect CI.
- Do not merge this session's PR.
