# #34: teleported anchored parts fling riders; RayHit.normal can be zero

## Goal
1. Teleporting an anchored part must not launch what touches it (derived velocity = distance / sim_time was ~3000 studs/s for a 50-stud reset).
2. `RayHit.normal` must always be unit length and finite, also when the ray starts inside a collider.

## Done
- `metallic_physics::TELEPORT_SPEED` (500 studs/s). In `PhysicsWorld::step`, an anchored move whose derived speed exceeds it is a plain `set_position` on the fixed body: no kinematic sweep, `velocity()` reports zero.
- `PhysicsWorld::raycast` normalizes the Rapier normal; a degenerate one (ray starts inside a collider, hit at distance 0) becomes the reversed ray direction. Still casts with `solid = true`, so the containing part is reported at distance 0.
- Tests in `crates/metallic_physics/tests/physics.rs` (all fail without the fix): `teleported_anchored_part_has_zero_velocity`, `teleporting_anchored_platform_does_not_launch_rider`, `raycast_normal_is_unit_even_from_inside_a_part`.

## Next
- PR (lead/owner). Rebase on #32 (kinematic angular velocity) and #33 (NaN dt), which touch the same anchored-move block in `step`.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
```

## Open questions
- Teleport rule (design call left to the lead): speed threshold, not an explicit "teleport" flag, because it needs no new API and is frame-rate independent. 500 studs/s is well above walking (16), falling after 1 s (~196) and normal moving platforms. A long frame (sim_time up to `MAX_STEP`) can still sweep a big move: 50 studs over 0.25 s is 200 studs/s. Only position counts; rotation is still set directly (see #32 for angular velocity).
- Ray starting inside a part: we report that part at distance 0 with normal = -direction. Roblox instead ignores the part the ray starts in; switch to that (or `solid = false`) if the character ground check (#5) wants it.
