# 372 Parkour grid

Goal: after the spec 2.2 grid rule (#337, min corner on the grid), every 1-stud Parkour slab has its faces on the
1-stud grid.

Done:
- `examples/build_parkour/level.rs`: every walkable top that was a half stud moves up 0.5 (58 one-stud parts:
  platforms, lava-walk tiles and strips, the lava river and pool, both movers, the tower pads). Rims, supports, flags,
  the spinning bar and its hub move with their platforms. Horizontal positions and gaps are unchanged.
- Scenes regenerated with `cargo run -p metallic_runtime --example build_parkour`.
- `tests/parkour_route.rs` waypoints follow; new `tests/parkour.rs` check: every unrotated 1-stud part has a
  whole-stud bottom.

Height differences: the course climbed in 1.5-stud steps, so whole-stud tops cannot keep every rise. Rises between a
moved and an unmoved platform change by 0.5 (1 or 2 instead of 1.5; the tower alternates 3 and 2 instead of 2.5).
The headless bot still finishes on keyboard, gamepad and touch.
