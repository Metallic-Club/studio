# #32 Kinematic angular velocity (branch `studio/32-kinematic-angvel`)

## Goal
Rotating an anchored part must push and carry loose parts (Parkour spinning bar #8, rotating platforms #5).
Before: `PhysicsWorld::step` jumped the rotation with `set_rotation` and gave the kinematic body linear velocity only.

## Done
- `crates/metallic_physics/src/lib.rs`: `angular_velocity_between` derives ω from the orientation delta
  (`q_new * conj(q_old)`, shortest arc, f64) over the simulated time; the mover gets `set_angvel` with its linvel
  and rotates through the substeps instead of jumping; ω is zeroed when it lands and becomes fixed again.
- New public `PhysicsWorld::angular_velocity(id)` (rad/s, world axis). Point velocity = `v + ω × (p - Position)`.
- Tests in `tests/physics.rs`: `rotating_anchored_part_has_angular_velocity`,
  `rotating_anchored_bar_pushes_loose_part` (the issue repro), `rotating_anchored_platform_carries_rider`.
  All three fail with the old code (box moved 0.07 studs, rider turned -0.4 deg, ω = 0).
- ARCHITECTURE.md physics line mentions `velocity`/`angular_velocity`.

## Next
- PR with `Closes #32` (not opened by this session).
- #5 character controller can use `velocity` + `angular_velocity` for "carried by rotating parts".

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

## Open questions
- A turn of more than 180 deg in one step is taken the short way round (an orientation alone cannot tell).
  Fine at 60 Hz for any realistic spin speed (< 10800 deg/s).
- No `velocity_at(id, point)` helper yet; add it if #5 wants one.
