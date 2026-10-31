# #23 Task 16, part 3: debug views

Status: in review (Closes #23). Parts 1 (#401, profiler core) and 2 (#416, Studio panel and the Player's F3 overlay)
are separate PRs; this branch starts from main and does not depend on them.

## What it adds

- `metallic_render::debug_views`: `DebugView` {Off, Wireframe, Normals, Overdraw, ShadowCascades, Exposure} and
  `DebugShape` (a unit mesh, any transform, a colour; `segment` and `capsule` helpers).
  - The view is `Globals.debug.x`. `shaders/debug.wgsl` holds `debug_scene` (normals, cascade colours), `fs_wire`
    and `fs_overdraw`. `common.wgsl` darkens the sky behind wireframes and overdraw.
  - Wireframe: each unit mesh gets a line-list index buffer of its edges (`edge_indices`; edges inside a flat
    face are dropped). Characters stay shaded.
  - Overdraw: an additive pipeline with no depth test. Each layer adds a little warm light, so more layers read
    brighter and warmer.
  - Exposure (the light meter): the finish pass shows the exposed light in false colour, one band per stop around
    the light the tone curve shows as middle grey. The meter keeps running.
  - Views that replace the scene's light skip metering, so the exposure holds instead of chasing its own output.
  - Debug shapes are drawn with coarser spheres and cylinders, over everything (no depth test).
- `metallic_physics::debug_draw`: `PhysicsWorld::debug_draw(DebugDrawOptions, &mut DebugDraw)` returns one
  `DebugShape` per collider (kind, pose, size, body state: fixed, kinematic, dynamic or sleeping; sensor), the
  contact points and normals, and the velocities. `CharacterController::debug_touch_volumes` adds the 15 bone
  capsules per character (#370), marked while touching.
- `metallic_runtime`: `Game::set_debug_draw` / `debug_shapes`; `sync_renderer` hands the shapes to the renderer.
  Colours are in `debug_draw.rs`.
- Player: F4 cycles the views, F6 toggles the physics overlay; `--debug-view NAME`, `--debug-physics`.
- Studio: View > Debug views (`debug_views/views_menu.rs`) for the render view, a light meter reading (EV100 and
  multiplier) and the physics overlays while playing.

## Cost when off

The frame is the same as before, apart from one uniform read per pixel in the scene, sky and finish shaders.
`set_debug_shapes(&[])` returns at once when nothing was drawn. `Game::sync_renderer` checks one flag.

## Tests

- `metallic_physics`: `debug_draw_emits_one_box_per_collider` (the plan's test) and
  `a_resting_body_shows_contacts_and_a_falling_one_its_velocity`.
- `metallic_render`: `flat_faces_keep_their_outline_but_not_their_diagonals` (a box: 4 edges per side, 24), `views_round_trip_by_name_and_cycle`,
  `a_segment_spans_its_ends`; `tests/debug_views.rs` on a GPU: every view changes the image and Off restores
  it exactly; debug shapes draw and clear.
- `metallic_runtime/tests/debug_draw.rs`: Parkour shows a wireframe per collider, 15 touch volumes and contacts.
- `metallic_studio/tests/views_menu.rs` (kittest): the menu sets the viewport view and the physics overlays.

## Screenshots (Parkour, Player, `docs/agents/23-shots/`)

`view-wireframe.png`, `view-normals.png`, `view-overdraw.png`, `view-cascades.png`, `view-exposure.png`,
`physics.png`.

## Not done

- A memory panel per subsystem and per script VM (listed in the plan's interfaces) is not part of these PRs.
- Overdraw skips the characters (their skinned pipeline has a second bind group).
