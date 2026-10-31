# #374 Platform follow-ups (idle on tilted spinners, Heartbeat mover lag)

Goal: from the #256 re-review. (1) A rider standing still on a tilted spinner reads as still
(idle animation). (2) A rider on a platform a Heartbeat script moves trails by one tick or less,
keeping the M2 tick order (characters, then physics). Part of #273.

## Done

- `PhysicsWorld::scripted_pose(world, id)`: the pose the next step gives a part (world pose for
  anchored/replica parts, using the same "moved" test as `step`; body pose for simulated parts).
  `scripted_body_pose` (crate-private) returns the exact body pose when nothing moves.
- Controller: collisions still use body poses. After the ground probe, the rider gets a lead,
  the rigid step of the bottom-sphere centre from the body pose to the scripted pose, swept
  against everything but the support, plus that move's world-up twist for facing. The contact
  is stored aligned with the scripted pose, so after physics applies it the next carry is zero.
  The zero-rate gate now applies only to simulated bodies; anchored supports use the exact pose
  difference (so a body that did not reach the pose pulls the rider back with it).
- `relative_velocity` = the character's own displacement: velocity minus carry and lead actually
  applied (the support's motion at the bottom-sphere centre). Jump momentum still uses the foot
  contact's point velocity.

## Tests (each fails on main 585d1d35)

- `still_rider_on_tilted_spinner_has_no_relative_speed` (8 and 20 deg): main 0.09 / 0.18 stud/s,
  now about 3e-14.
- `script_moved_support_keeps_rider_within_one_tick` (slide 4 studs/s, lift and drop 6 studs/s,
  spin 90 deg/s, script after physics): main 0.133 stud (2 ticks) on the slide, now exactly one
  tick in every case.
- Runtime `moving_platform_lag.rs`: a Heartbeat script slides a platform at 4 studs/s; the rider
  is within one tick (0.0667 stud) of where the script put it; main 0.133.
- `small_tilts_carry_the_bottom_sphere...` updated to the new `relative_velocity` meaning, with
  the jump lever-arm check moved to an actual jump.

## Notes

- Shared `CARGO_TARGET_DIR` across worktrees can reuse another worktree's test binaries (cargo's
  metadata hash ignores the checkout path). Touch the sources before trusting a cross-tree run.
- #34 / PR #53: a teleport rule must apply in `scripted_pose` too, or riders will be led
  through a teleport.
- Gates: fmt, clippy -D warnings, `cargo test -p metallic_physics -p metallic_runtime`,
  `cargo check --workspace`.
