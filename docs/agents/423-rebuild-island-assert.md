# #423: Rapier island assertion after rebuilding a resting part

## Cause

Rapier 0.36.1 bug, not our remove/insert order. `PhysicsWorld::sync_from` ended with
`rapier.detect_collisions()` (Rapier's `CollisionPipeline::step`) to refresh the query tree. That pipeline calls
`handle_user_changes_to_rigid_bodies` without the island manager and then clears the bodies' change flags, so a body
inserted after the first `step` is never added to an island. A pair it touches is never linked in the persistent
islands, which trips `touching pair (edge N) not linked in the persistent islands` (`island_manager/manager.rs:468`)
in debug builds and leaves the body islandless in release. Before the first step it is harmless: the first
`update_islands` bootstraps the islands from scratch.

A pure-Rapier repro (fixed ground, dynamic box at rest; remove and re-insert the box, `detect_collisions()`, `step()`)
panics with `body ... has no island` (`persistent.rs:853`); the same with a box spawned in mid-air after a step. With
no `detect_collisions()` in between it is fine. Rapier 0.36.1 is the newest release and no upstream issue matches
(searched dimforge/rapier for "detect_collisions", "persistent islands", "has no island", "CollisionPipeline
island"), so there is nothing to upgrade to or pin.

## Fix

`sync_from` runs the collision pipeline only before the first step. After it, new and rebuilt colliders go into the
query tree with `BroadPhaseBvh::set_aabb` (public, meant for this: the next broad-phase update picks the change up),
so raycasts still see them at once; their contacts start in the next step, whose contact pass also settles rebuilt
pairs (#417).

## Tests

- `metallic_physics/tests/rebuild.rs`: resizing and reshaping a resting part (after 0.5, 2 and 5 s, so awake and
  asleep), a part added after the first step falls and rests, a rebuilt part is hit by raycasts before the next step.
  The first two panicked before the fix.
- `metallic_physics/tests/physics.rs`: #417's rebuild tests use the unanchored resting part again (Size, Shape, and
  Material); only the `CanCollide` one keeps the anchored zone.
