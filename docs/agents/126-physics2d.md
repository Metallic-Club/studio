# #126 metallic_physics2d: 2D physics and platformer character

Branch `studio/126-physics2d`. Status: implemented and tested (not yet wired into runtime; that comes after #8, under epic #16).

## Goal
A standalone `crates/metallic_physics2d` that matches the shape of `metallic_physics` (3D), keeps the backend behind our own types, and gives M3 a 2D character controller that feels good in a platformer.

## Backend decision: rapier2d now, Box2D v3 later if its bindings mature (assistant's call)
The spec says "Box2D (v3) through Rust bindings, when 2D arrives" and lists "which Rust bindings for Box2D" as an open question. Research done 2026-10-30:

| Option | Licence | State (Oct 2026) | Build | Verdict |
|---|---|---|---|---|
| Box2D v3 (C library itself) | MIT | Excellent: soft-step solver with substeps, continuous collision, cross-platform determinism (since 3.1), sensor events, character mover (3.1, experimental), 3.2 released | C compiler | Best-in-class engine |
| `boxdd` / `boxdd-sys` 0.6 (Latias94) | MIT OR Apache-2.0 | Single maintainer, 13 stars, about 1.6k downloads, first release Sept 2025, 0.6 was "breaking, soundness-focused". Pins a Box2D 3.2 development snapshot. `World` is `!Send`/`!Sync`. Misuse panics by default (`try_*` variants exist). | `cc` builds vendored C: MSVC on Windows, clang on macOS. Wasm needs Emscripten (not `wasm32-unknown-unknown`), and callback APIs are compiled out there | Promising, too young to build on |
| `box2d_sys` 0.2 (Carroted) | MIT OR Apache-2.0 | Raw FFI for 3.0; last release Sept 2024 | C build + bindgen | Unmaintained, raw `unsafe` only |
| `box2d-rs` 0.0.4 | MIT | Port of Box2D **2.4**, last release May 2023 | Pure Rust | Old engine, unmaintained |
| `rapier2d` 0.36.1 (Dimforge) | Apache-2.0 | About 1M downloads, released monthly (0.36.1 on 2026-10-10). Same version and family as our 3D `rapier3d`. TGS-soft solver with substeps, CCD, joints, sensors, contact hooks, `enhanced-determinism` feature for cross-platform bit-identical results | Pure Rust, no C toolchain, plain `wasm32` works | **Chosen** |

To put it plainly: the Box2D v3 bindings are not solid enough yet, so this uses rapier2d. The spec's wording allows revisiting it.
Against the criteria:
- **Determinism:** same inputs give bit-identical results in one build (tested). The `cross-platform-determinism` crate feature turns on rapier's `enhanced-determinism` (software maths, slower) for lockstep or rollback later. Box2D 3.1+ gets this without a feature flag, which is the main thing we give up.
- **Stacking:** a 10-box tower stays still (tested). Rapier's soft solver uses substeps, the same approach as Box2D v3.
- **Continuous collision:** `BodyDesc::bullet` turns on CCD. Tested at 1,500 studs/s against a 0.2-stud wall.
- **Character controller:** our own (below). Box2D's mover is experimental. Rapier's kinematic controller has a 2D bug: it measures slope angles with a signed `angle_to`, so steep slopes that face right count as walkable. It also adds platform motion on its own, which doubled our carry. We use rapier's shape casts and do the logic ourselves.
- **Build friction:** none on Windows, macOS or Linux (pure Rust, no `cc`, no cmake). Box2D bindings add a C toolchain to every machine and to CI.
- **Wasm later:** works with rapier. With boxdd it would need Emscripten.
- **Performance:** see the benchmark below. Box2D v3 is probably faster at very large body counts (SIMD wide solver, graph colouring). For our scenes, 2,000 bodies fit easily in one frame.
- **Swappable:** no rapier type crosses the public API. A Box2D backend means rewriting `world.rs`, `shapes.rs`, `events.rs` and the shape casts in `character.rs` against the same structs. Revisit when `boxdd` reaches 1.0, or when Box2D publishes official Rust bindings.

## API (mirrors `metallic_physics`)
- `PhysicsWorld2d::new(WorldConfig)`. Defaults: gravity (0, -196.2) studs/s^2 (the same as 3D), `tick` 1/60 s, `solver_substeps` 4, `max_frame_time` 0.25 s, `length_unit` 4.
  - **Timestep (assistant's call):** one collision pass per 1/60 s tick, with 4 solver substeps. This is what #115 proposes for 3D (3D currently runs four full 1/240 s steps), so 2D starts where 3D is heading. The leftover time accumulates. `interpolation_alpha()` and `render_pose(id)` give smooth drawing at any refresh rate.
- `sync_from(&Scene2d)`: creates, rebuilds and removes bodies and joints to match the scene. The whole scene is validated first; on error nothing changes.
- `step(&mut Scene2d, dt) -> Result<ticks>`:
  - Poses changed in the scene are applied: a dynamic body teleports; a static or kinematic body becomes a **mover** with a derived velocity, so things on it are carried (like anchored parts in 3D).
  - Then it simulates fixed ticks and writes dynamic and kinematic poses back.
  - **NaN, infinite or negative `dt` returns `Physics2dError::InvalidDt` and changes nothing** (lessons from #33 and #48).
  - Frames longer than `max_frame_time` are clamped.
- Queries:
  - `raycast(origin, dir, &RayFilter)`: the length of `dir` is the maximum distance. Filters: ignore list, collision groups, `include_sensors`.
  - `bodies_at_point(point)`.
- State and impulses: `body_state(id)`, `set_velocity`, `apply_impulse`.
- Events:
  - `drain_touched()`: new pairs only, once each, as in 3D.
  - `drain_sensor_events()`: Enter and Exit, including an Exit when a body is removed.
  - `drain_character_events()`: Jumped, WallJumped, Landed, DroppedThrough.
  - `is_touching(a, b)`.
- Shapes (`Shape2d`):
  - Box, Circle, Capsule (vertical), ConvexPolygon (hull of 3 to 64 points).
  - EdgeChain (open or closed).
  - Tilemap: solid flags, greedy-merged into as few boxes as possible.
- Per collider:
  - `Material2d`: density, friction, restitution.
  - `sensor`.
  - `one_way: Option<Vec2>`: the open direction in body space.
  - `CollisionGroups`: 32-bit memberships and filter.
- Joints: Revolute (limits, motor), Prismatic (axis, limits, motor), Distance (min/max length, so rope or rod, plus an optional spring), Weld (keeps the relative angle the bodies had when it was created).
- Debug view: `debug_shapes()` returns world-space outlines tagged with body kind, sensor, one-way and sleeping.
- Errors: `Physics2dError` (InvalidDt, InvalidConfig, NonFinite, InvalidShape, DuplicateBody, DuplicateJoint, UnknownBody, InvalidJoint, NotDynamic). There is no `unwrap` or `expect` outside tests.

## Character controller
- **Body and movement:**
  - The character is a kinematic box. Each fixed tick it moves by collide-and-slide shape casts and keeps a 0.02-stud skin.
  - After the move it pushes itself out of anything it overlaps, then snaps to the ground (0.6 studs).
  - Walkable slopes keep the horizontal speed; slopes steeper than `max_slope_angle` act as walls.
  - It pushes dynamic bodies.
- **Moving platforms:** the character follows the motion of the body under it, rotation included. That carry runs as its own move that ignores the platform.
- **One-way platforms:** a platform is solid only while the character's bottom is above the platform's top. Measuring along the platform's open direction means rotated platforms also work.
- **Drop-through:** the character ignores that platform for 0.25 s.

Defaults (studs; character 2.4 x 4.8; 4-stud tiles):

| Setting | Default | Why |
|---|---|---|
| `run_speed` | 28 (7 tiles/s) | Between Mario (about 6) and Celeste (about 11 tiles/s) |
| `ground_acceleration` / `deceleration` | 350 / 420 | Full speed in 0.08 s, stop in 0.07 s. Celeste reaches full speed in about 0.09 s; snappy beats floaty |
| `air_control` | 0.65 | Celeste's air multiplier |
| `jump_height` / `min_jump_height` | 13 / 3.5 | About 3.25 tiles held, about 0.9 tiles tapped. Celeste's full jump is about 3.4 tiles |
| `time_to_apex` | 0.36 s | Gives `g = 2h/t^2 = 200.6`, close to world gravity, so the character and props fall alike |
| `apex_hang_speed` / `apex_gravity_multiplier` | 18 / 0.5 | Half gravity near the apex while jump is held (Celeste does this below 40 of 105 px/s) |
| `fall_gravity_multiplier` | 1.6 | Falls faster than it rises (Kyle Pittman, "Math for Game Programmers: Building a Better Jump", GDC 2016) |
| `max_fall_speed` | 100 | About 1.4 x the jump speed (Celeste: 160 / 105) |
| `coyote_time` | 0.10 s | Celeste's jump grace time; the commonly cited range is 0.08 to 0.15 s |
| `jump_buffer_time` | 0.10 s | Celeste uses 0.08 s; 0.1 to 0.15 s is common |
| `wall_slide_speed` | 14 | About 1/7 of max fall speed (Celeste: 20 / 160) |
| `wall_jump_velocity` | (40, 62) | Horizontal is about 1.45 x run speed (Celeste: 130 / 90) |
| `wall_jump_lock_time` | 0.16 s | Celeste's wall-jump force time |
| `max_slope_angle` | 50 degrees | |
| `snap_to_ground` | 0.6 | |
| `drop_through_time` | 0.25 s | |
| `inherit_platform_velocity` | true | Keeps the platform's speed for the whole jump, like jumping off a train |

How the jump numbers are derived:
- `jump_height` stays exact even with apex hang: `v0^2 = 2gh + a^2 - a^2/m`, where `a` is the hang speed and `m` the hang multiplier.
- Releasing jump while rising multiplies gravity by `v0^2 / (2 g h_min)` (3.48 with the defaults), so a tap reaches `min_jump_height`.
- The character uses its own designer gravity and ignores world gravity.
- Only the numbers above are cited from Celeste and the GDC talk. No code was copied.

## How 2D maps to the Instance tree (proposal for the M3 design session; `metallic_core` not edited)
This crate takes plain structs, so either option below works without changing it.

**Recommended: a separate 2D class family under a 2D container.**
- **`Stage2D`** (a service-like container, the 2D counterpart of Workspace). Properties: `Gravity: Vector2`, `PixelsPerStud`, `Tick`. Every `BasePart2D` below it becomes a body.
- **`BasePart2D` (abstract) to `Part2D`.**
  - Pose and size: `Position: Vector2`, `Rotation: number` (degrees), `Size: Vector2`.
  - Shape and material: `Shape2D` enum (Box, Circle, Capsule, Polygon), `Material`.
  - Body type: `Anchored` (Static), `Kinematic: bool`, `CanCollide` (false makes a sensor).
  - Collision options: `OneWay: bool` with `OneWayDirection`, `CollisionGroup: string` (named groups, mapped to bits like Roblox's `CollisionGroup`).
  - Physical settings: `Density`, `Friction`, `Elasticity`, `Bullet`, `FixedRotation`.
  - Drawing: `ZIndex`, `Color`.
  - Polygon and chain points live in a `Points` property (a new `Vector2` list value).
- **`Sprite`**: image, flip, frame, tint and pivot. Either a child of a `Part2D` (follows it) or standalone (decoration, no physics).
- **`Tilemap`**: `TileSize`, a tile palette (`TileSet` asset), grid data. It builds one static body with `Shape2d::Tilemap` per solid layer, plus one-way layers.
- **`EdgeChain2D`**: terrain lines.
- **Constraints:** `HingeConstraint2D`, `PrismaticConstraint2D`, `RopeConstraint2D` (min/max/spring) and `WeldConstraint2D`, with `Attachment2D` children for anchors (Roblox naming).
- **`Character2D` + `Humanoid`**: the controller tuning from `CharacterConfig` becomes properties (JumpHeight, TimeToApex, CoyoteTime...). Events map to `Humanoid.StateChanged` and `Jumping` and `Landed`.
- **`Camera2D`**: follow target, zoom, bounds.
- **Core work this needs:** a `Value::Vector2` (and a `Vector2` list) in `metallic_core`, registry entries, Luau `Vector2` API (Roblox already has the type), and RON save support. The renderer needs a sprite and 2D pass.

**Why not a "2D mode" flag on ordinary Parts:**
- Every 3D system (physics, renderer, Properties panel, scripts) would need special cases for Z and rotation axes.
- Roblox-style 3D properties (`CFrame`, `Size: Vector3`) would carry meaningless values.
- Mixed scenes would be ambiguous.

**In-between games (2.5D) are covered two ways:**
1. 2D physics with 3D rendering: a `Part2D` can have a `MeshPart` child for its look, and `Stage2D.Depth` places the plane in 3D.
2. 3D physics locked to a plane: a later `Workspace.PlaneLock` would use rapier3d's locked axes, for games that want real 3D bodies on a 2D track.

## Tests
- Unit tests (`src/`): maths, validation, tile merging (exact cover), convex hull, contact tracker, jump formulas.
- `tests/physics2d.rs`:
  - A falling body touches the floor at `sqrt(2h/g)` within 2 ticks.
  - A 10-box stack stays stable (no drift or tilt; asleep).
  - One-way platforms: a body launched up passes through and lands; a dropped body is held.
  - Sensors fire Enter and Exit and never block; removing a body inside a sensor fires Exit.
  - Raycasts: nearest hit, distance and normal, ignore list, sensors, zero or NaN direction.
  - A static mover carries a box, with derived velocity.
  - A kinematic body pushes.
  - Determinism: busy scene, character and uneven frame times give bit-identical poses and events.
  - NaN, infinite and negative `dt` are rejected; physics continues afterwards; a huge `dt` is clamped to 15 ticks.
  - Invalid scenes are rejected atomically.
  - Collision groups.
  - Revolute, distance, weld and prismatic joints; a motor.
  - Tilemap and edge chain.
  - CCD bullet.
  - Teleport; interpolation.
- `tests/character.rs`:
  - Stands on the ground.
  - Full and tapped jump heights.
  - **Coyote time:** jumps when pressed 1 and 5 frames after the edge, not at 9.
  - **Jump buffer:** pressed 1 and 5 frames before landing jumps on landing; pressed 10 frames before does not.
  - Climbs a 30 degree slope, not a 70 degree one.
  - Carried by a moving platform horizontally and vertically, and inherits its speed on a jump.
  - One-way: jump up through and land, then drop through.
  - Wall slide caps the fall at 14 and wall jump launches away; both can be turned off.
  - Touches bricks and triggers sensors.
  - Pushes crates.
  - Frame-rate independence: 30, 60 and 120 fps give identical results.

## Benchmark
`cargo bench -p metallic_physics2d --bench step`: 2,000 dynamic bodies (half boxes, half circles) piled in a bin, plus a character, one tick (4 substeps). Result: see "Numbers" below.

## Demo
`cargo run -p metallic_physics2d --example physics2d_demo`
- Contents: a tilemap with a pit and a step, a wall-jump shaft, a 30 degree ramp, one-way platforms, a moving lift and an elevator, an edge-chain hill, a goal sensor, a crate stack, a ball bin, a pendulum and a rope bridge (7 planks on pins).
- Controls: arrows run, Up/Space/Z jump, Down drops, R resets.
- The HUD shows grounded, wall, coyote and buffer state.
- `-- --screenshot out.png` renders headless through egui_kittest and wgpu, with no window.

## Numbers
(filled in below after the runs)

## Next
- Wiring into `metallic_runtime` (after #8).
- Instance classes (M3 design session, needs `Vector2` in core).
- Luau API for 2D.
- Studio 2D viewport.
- Revisit Box2D v3 when `boxdd` reaches 1.0.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/c/Users/alexi/Documents/metallic-studio-worktrees/_target-lead-shared CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_BENCH_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p metallic_physics2d -j 4
cargo bench -p metallic_physics2d --bench step -j 4
cargo run -p metallic_physics2d --example physics2d_demo -j 4 -- --screenshot shot.png
```

## Sources
- Box2D 3.1 release notes: https://box2d.org/documentation/md_release__notes__v310.html and https://box2d.org/posts/2025/04/box2d-3.1
- Box2D 3.2 release: https://github.com/erincatto/box2d/releases/tag/v3.2.0
- boxdd: https://github.com/Latias94/boxdd and https://docs.rs/boxdd
- box2d_sys: https://docs.rs/crate/box2d_sys/latest
- box2d-rs and rapier2d metadata: crates.io API, queried 2026-10-30
- Rapier character controller guide: https://rapier.rs/docs/user_guides/rust/character_controller
- Game-feel numbers: Celeste's published player tuning constants (numbers only) and Kyle Pittman, "Math for Game Programmers: Building a Better Jump", GDC 2016.
