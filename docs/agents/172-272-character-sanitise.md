# #172 / #272: sanitise Humanoid tuning and the character controller

Branch `studio/172-272-character-sanitise`.

## Done
- `metallic_core::classes`: Humanoid `Health`/`MaxHealth` (0..1e9), `WalkSpeed`/`JumpPower` (0..1000) and
  `MaxSlopeAngle` (0..89) use the existing per-property `NumRange`, so `World::set`, scene loading and Luau all clamp.
  The limits are public consts (`MAX_HUMANOID_HEALTH`, `MAX_WALK_SPEED`, `MAX_JUMP_POWER`, `MAX_SLOPE_ANGLE`). This is
  registry data only, and the generic `World::set` path is unchanged (it already matched on `def.range`), so the
  `instructions` gate (Part reads and writes) should not move. Non-finite writes were already refused (#98).
- `metallic_physics::character`: tuning is read defensively (missing or non-finite becomes the default, then clamped).
  Death is checked with `health.is_nan() || health <= 0`. `Motion` is sanitised every step and on `restore_state`.
  A non-finite move direction is ignored, and a non-finite pose is never written. A root that is non-finite or beyond
  `WORLD_LIMIT` (1e6 studs on any axis) dies like a fall below `FallenPartsDestroyHeight`, is not swept through
  Rapier, and respawns.
- Tests: `crates/metallic_core/tests/humanoid_limits.rs` and six new tests at the end of
  `crates/metallic_physics/tests/character.rs`.

## Notes
- `metallic_core/tests/scene_file.rs` uses a fixed temp dir shared by every session on this PC, and it failed once
  when runs overlapped. It passes on rerun and on main. Not changed here.

## Commands
```
cargo test -j 2 -p metallic_core -p metallic_physics
cargo clippy -j 2 -p metallic_core -p metallic_physics --all-targets -- -D warnings
```
