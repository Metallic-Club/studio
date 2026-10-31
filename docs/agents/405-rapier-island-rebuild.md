# #405 Rapier island panic when rebuilding a touching part

Goal: changing `Size`, `Material`, `CanCollide` or `Anchored` on a part that rests on another, or
moving it out of Workspace and back, then `sync_from` + `step`, must not panic in debug builds
("touching pair ... not linked in the persistent islands") and must keep islands consistent.

Branch `studio/405-rapier-island-rebuild` from `origin/main` `a0bb1bf`.

## Root cause

Not the removal side. `remove_entry` calls `rapier.remove_body`, which already passes the
`IslandManager`, removes the attached collider with the body and wakes what it touched.

The problem is the end of `sync_from`: it called `PhysicsWorld::detect_collisions`, Rapier's
collision-only `CollisionPipeline`. In rapier3d 0.36.1 that pipeline runs
`handle_user_changes_to_rigid_bodies(None, ...)` (no island manager) and then clears the new bodies'
change flags (`clear_modified_bodies`). So the new body is not registered in the persistent islands
yet when the narrow phase finds its contact with the floor already touching; `link_contact` sees no
island on either side (the floor is fixed) and records nothing. Rapier only links a contact on its
touching *transition*, so the following real `step` never links it either. The debug check in
`IslandManager::update_islands` then panics. Release builds skip the check, but the island graph is
wrong (sleep and wake decisions can be off).

The first sync of a world is not affected, because the first real step bootstraps the persistent
islands from every touching pair. A part added later that is *not* touching yet is also fine.

## Upstream

This is an upstream rapier3d 0.36.1 issue: `PhysicsWorld::detect_collisions` is documented as
"useful after editing transforms when contacts and scene queries must be refreshed immediately",
but mixing it with `step` breaks the island invariants. The `CollisionPipeline` docs do say it
never registers bodies in the `IslandManager`. Minimal repro (debug build, plain rapier3d 0.36.1):

```rust
use rapier3d::prelude::*;
let mut world = PhysicsWorld::new();
world.insert(RigidBodyBuilder::fixed(), ColliderBuilder::cuboid(10.0, 1.0, 10.0));
world.step(); // bootstraps the persistent islands
world.insert(
    RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 1.5, 0.0)),
    ColliderBuilder::cuboid(0.5, 0.5, 0.5),
); // already resting on the floor
world.detect_collisions(&(), &());
world.step(); // panics in persistent.rs:853 "body ... has no island"
```

With our colliders (collision events on) the same sequence panics one check later, at
`manager.rs:468` "touching pair (edge 0) not linked in the persistent islands". Without the
`detect_collisions` call both variants pass. No upstream issue was filed from this session (no
GitHub actions allowed here); the lead or owner can file it with the repro above and link it on #405.

## Fix (workaround in our code)

`sync_from` now ends with `refresh_collisions`: a full `step_with_events` with
`integration_parameters.dt = 0` (restored afterwards) instead of `detect_collisions`. The physics
pipeline handles the user changes with the island manager first, then runs the broad and narrow
phase, so new touching contacts are linked. With `dt = 0` nothing integrates, gravity adds no
velocity and sleep timers do not advance (`IntegrationParameters::inv_dt` returns 0 for `dt = 0`).
Raycasts and touches still work right after `sync_from`. `step` is untouched.

## Tests

`crates/metallic_physics/tests/rebuild_touching.rs`: a part settled 1 s on an anchored floor, the
change, `sync_from`, 60 steps, then it must still rest at y = 2.5 and a lift-and-drop must land it
and report exactly one touch with the floor.

- `resize_touching_part_does_not_panic`
- `material_change_on_touching_part_does_not_panic`
- `can_collide_toggle_on_touching_part_does_not_panic` (off: sinks as a sensor; on again: rests)
- `anchored_toggle_on_touching_part_does_not_panic` (anchored, then unanchored again)
- `reparent_out_and_back_does_not_panic` (ReplicatedStorage and back)

Before the fix all five panicked in debug at `manager.rs:468`; in release all five passed. After it
all five pass in debug.

## Commands

```
cargo test -j 4 -p metallic_physics --test rebuild_touching
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all --check
```

## Next / open

- `metallic_physics2d` (`crates/metallic_physics2d/src/world.rs`, `sync_from`) also calls
  `detect_collisions` after inserting bodies with rapier2d 0.36.1, so it likely has the same bug.
  Not changed here (out of scope); worth its own issue.
- PR #53 (`studio/34-teleport-ray-normal`, at `d2689cc`) changes `step` in the same file. Dry-run
  merge (scratch detached worktree, not committed or pushed): the first version conflicted only in
  ARCHITECTURE.md (both edited the "kinematic movers" line), so the #405 sentence moved to the end
  of the metallic_physics bullet. After that the merge is clean (`lib.rs` auto-merges; this fix
  stays out of `step`) and `cargo test -j 4 -p metallic_physics` passes on the merged tree
  (physics.rs 21, rebuild_touching 5, character 37, rotating_support 14, unit 8).

## Status

Implemented and tested locally; not pushed, no PR yet. Full workspace test, clippy, deny and fmt
pass (see the report on the branch commits).
