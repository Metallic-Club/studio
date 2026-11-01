# #76: glTF skinning and the default avatar in the engine (Phase C)

Branch `studio/76-gltf-skinning-avatar`. Design: `docs/superpowers/specs/2026-10-30-default-avatar-design.md`
(sections 3.6, 4, 5). Asset: `art/avatar/out/default_avatar.glb` (Phase B).

## Goal
The engine imports, skins, animates and draws the default avatar, and the Parkour player is that avatar instead of
the six-part figure.

## Where things are
- `crates/metallic_asset/`: new crate. `import.rs` (`SkinnedModel::from_glb`, all validation), `accessor.rs`
  (bounds-checked accessor reads), `skeleton.rs`, `anim.rs` (Transform, Pose, Track, Clip, sampling and blending).
  Tests: `tests/import.rs` (real asset, hostile files, byte-flip fuzz).
- `crates/metallic_render/src/skinned.rs` + `shaders/skin_common.wgsl`, `shaders/skinned.wgsl`, `vs_skinned` in
  `shaders/shadow.wgsl`; wiring in `renderer.rs` (`set_skinned`, `skinned_lods`, skinned main and shadow pipelines).
  Test: `tests/headless.rs::skinned_avatar_draws_and_picks_its_level_of_detail`.
- `crates/metallic_runtime/src/avatar.rs` (placement, facing, joints), `locomotion.rs` (built-in controller),
  `game.rs` (`update_avatar`, `sync_renderer`). `examples/avatar_shots.rs` writes the review screenshots.
- `crates/metallic_physics/src/avatar.rs`: the old part figure is now invisible collision/touch proxies (names kept).
  `character.rs`: `relative_velocity` (velocity minus what the floor carries).
- `crates/metallic_player`: `--autopilot` (run in a circle and jump; used with `--fps`).
- Screenshots: `docs/agents/76-gltf-skinning-avatar/`.

## Decisions
- New crate `metallic_asset` rather than a module in render: render needs the meshes, runtime the skeleton and clips,
  and the animation system (#40) and player avatars later; keeps file formats out of the renderer.
- gltf 1.4 without default features; our own accessor reader. gltf-json's validation can panic on some broken
  references, so references are checked first, validation runs after, and `catch_unwind` is the last guard.
- Units: 1 stud = 0.28 m, so the 1.54 m avatar is 5.5 studs. Feet = root centre - `ROOT_HEIGHT` (3 studs).
- Model faces +Z; `Orientation.Y = 0` faces -Z: the avatar is turned by yaw + pi. Facing is smoothed (time constant
  0.06 s).
- Animation speed comes from the velocity relative to the floor, so standing on a moving platform plays idle.
- Gaits: idle / walk_slow (6) / walk (16) / run (24 studs/s), blended between neighbours; a shared phase advances by
  distance / blended stride so feet do not slide. Jump starts at its launch frame (the physics jump is instant, so the
  crouch is skipped); fall after 0.12 s airborne; land; crossfades 0.1-0.2 s with smoothstep.
- Climb and swim: the character controller has no such states yet; TODO in `locomotion.rs`, no code.
- LOD by projected height: LOD1 below 220 px, LOD2 below 80 px.
- Phase B export bug fixed: `rig_avatar.py` exported at Blender's default 24 fps, so the clip times were 30/24 too
  long. The scene fps is now set before export and before validation; the glb was re-exported.

## Measurements (2026-10-30)
- Frame rate, `metallic-player templates/parkour --fps --autopilot` (1920x1080, High, no vsync, RTX 3060 Ti, avatar
  running and jumping on the spawn island): average 1267 fps, p50 0.69 ms, p99 2.32 ms, max 63 ms (a single frame; not traced).
- Foot slip of planted foot vertices (test `planted_feet_do_not_slide_at_the_authored_speeds`, median / p90):
  6 studs/s: 0.13 / 0.41; 16: 0.46 / 1.08; 24: 0.94 / 1.99 studs/s (under 5% / 10% of the speed).
- Screenshots: `docs/agents/76-gltf-skinning-avatar/` (regenerate with
  `cargo run --release -p metallic_runtime --example avatar_shots -- docs/agents/76-gltf-skinning-avatar`).

## Open points
- The invisible proxy parts (old figure sizes) are the touch volumes; they do not follow the animated limbs.
- Jump anticipation (crouch) is skipped because the physics jump starts at once.
- Climb and swim wait for controller states.
