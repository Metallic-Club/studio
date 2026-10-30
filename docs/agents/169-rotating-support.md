# #169 Carry riders on rotating support

Goal: off-center riders follow a rotating platform and standing facing follows its
yaw, while movement input stays in world space and airborne momentum stays stable.

## Audit and design

- Fresh ready/unclaimed issue, branch `studio/169-rotating-support`, base `ebb94f3`.
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
  witnesses. Restore grounded skin clearance under the existing MAX_PUSH cap: the
  first prototype exposed occasional whole-step clipping when a floor's skin gap
  drifted about 0.005 stud. Witness stabilization alone did not fix that failure.
- Body-handle identity rejects rebuilt/removed supports; an externally changed root
  position rejects stale contact momentum. No new public API or dependencies.
- Keep the capsule upright, use world-up yaw, preserve explicit movement facing.

## Validation so far

- Actual controller-before-physics frame ordering, rider eight studs off the axis.
- Signed quarter turns, standing yaw, two revolutions with per-frame positional
  checks, stationary/translation/stop, jump momentum and landing recovery, explicit
  world-space input/facing, small signed tilts/contact lever arm, body replacement,
  and script root repositioning during a turn.
- Against original main, six of seven initial tests failed (rotation, facing,
  tangential jump, input composition and tilt), while stationary/translation passed.
- First prototype passed six of seven; repeated turns exposed skin clipping. The
  added script-reposition regression also failed before rejecting stale contacts.
- All nine final focused tests pass after both fixes (1.73 s test execution).
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

## Next

- Coordinate at most two heavy builds. Run complete workspace gates (including
  actual Parkour), inspect peer review, fetch/rebase latest main, then PR and attach.
- Do not merge this session's PR.
