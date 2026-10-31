# #345: unique temp folders for core tests; spawn pad outside the world

Branch `studio/345-test-temp-dirs`.

## Done
- `crates/metallic_core/tests/common/mod.rs`: `TestDir`, a `tempfile::TempDir` named `metallic_core-<test>-<random>`
  that is deleted on drop. `scene_file.rs`, `lighting.rs` and `humanoid_limits.rs` use it, one folder per test, and
  no shared fixed paths are left. `tempfile` is a dev-dependency of `metallic_core` only (MIT OR Apache-2.0, already
  in the lockfile), so the `instructions` gate does not see it.
- `crates/metallic_core/benches/world.rs`: a per-process bench folder, removed at the end.
- Other `temp_dir()` uses in the workspace already add the process id and a per-test name (audio `decode.rs`, player
  `smoke.rs`, launcher `data_dirs.rs`), so they were left alone.
- `metallic_physics::character::spawn_point` skips a RespawnLocation or SpawnLocation whose top is outside
  `WORLD_LIMIT` and uses the default (0, 3, 0), so there is no die/respawn loop. Test:
  `spawn_location_outside_the_world_is_skipped`.
- The NaN-Health guard now has a comment saying `World::set` refuses NaN, so the check is a guard only.

## Commands
```
cargo test -j 2 -p metallic_core -p metallic_physics
```
