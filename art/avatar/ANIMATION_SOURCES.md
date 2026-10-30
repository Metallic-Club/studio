# Animation sources

Where every clip of the default avatar comes from. Only CC0 motion or our own procedural motion
is used. Nothing comes from Roblox, Mixamo, Unity, Unreal or Godot sample content.

## Quaternius Universal Animation Library (CC0)

| Pack | Version | Download | SHA-256 of the zip |
|---|---|---|---|
| Universal Animation Library, Standard | 1 (files dated 2025-03, 45 animations) | https://opengameart.org/sites/default/files/universal_animation_librarystandard.zip | `18ff1a7215f4852b320203e8aaf02a1578b5c8eef9027fbaedfcedc7b85a3ac2` |
| Universal Animation Library 2, Standard | 2 (files dated 2026-01) | https://opengameart.org/sites/default/files/universal_animation_library_2standard.zip | `ec0e40d6d78fe9aaad59e322f40865a8675c22f0745e291622e54520391a9217` |

- **Author:** Quaternius (https://quaternius.com).
- **Licence:** the `License.txt` in both zips reads "CC0 1.0 Universal (CC0 1.0) Public Domain
  Dedication, https://creativecommons.org/publicdomain/zero/1.0/". Credit is not required; we give
  it anyway (see `CREDITS.md`).
- **Files read:** `Godot/AnimationLibrary_Godot_Standard.glb` (UAL 1) and
  `Unreal-Godot/UAL2_Standard.glb` (UAL 2). Only their glTF animation data is sampled. The packs
  are not in the repo.

### Regenerating

1. Download and unzip both packs into one folder outside the repo.
2. Run `blender -b --factory-startup --python art/avatar/retarget_clips.py -- extract --ual DIR`.
   This writes `DIR/cache/ual_sampled.json`.
3. Bake with `METALLIC_UAL=DIR blender -b --factory-startup --python art/avatar/rig_avatar.py`.

`retarget_clips.py` explains the retargeting: bone names, anatomical rest frames, leg-length
scale and foot IK. `anim_clips.py` (`ual_clips`) holds the per-clip polish. The bake writes each
clip's source into `out/default_avatar_clips.json` (`source`) and the glb action
(`source` property).

## Clips

| Clip | Source | What we changed |
|---|---|---|
| `idle` | UAL 1 `Idle_Loop` | Retimed to 3 s. Added two breaths (chest and shoulders) and a hips weight shift. Head levelled for the big head, arms out from the body, feet set to our stance, head and arm overlap (lag). |
| `idle_b` | UAL 2 `Idle_FoldArms_Loop` | Retimed to 3 s, feet set to our stance, head lag. |
| `walk_slow` | UAL 1 `Walk_Loop` (trunk, arms, head) | Legs are procedural (`Locomotion`): exact foot plants, left heel strike at phase 0 and right at 0.5, 0.07-cycle roll-in. The upper body is phase-warped to those contacts. About 6 studs/s. |
| `walk` | UAL 1 `Jog_Fwd_Loop` (trunk, arms, head) | Same leg method, 16 studs/s. Trunk twist toned down, loose hands. |
| `run` | UAL 1 `Sprint_Loop` (trunk, arms, head) | Same leg method, 24 studs/s. Less hunched (spine back 8 degrees), fists. |
| `jump` | UAL 1 `Jump_Start` | Starts from idle's pose, crouch, launch at frame 6 (the runtime starts here), then a faster tuck. Planted feet pinned until launch. |
| `fall` | UAL 1 `Jump_Loop` | Arms raised and paddling, legs apart and pedalling, head levelled. |
| `land` | UAL 1 `Jump_Land` | Shallower, faster squash (deepest at frame 3) and settles into idle's first pose. |
| `land_hard` | UAL 1 `Jump_Land` | Deep squash held longer, settles into idle. Plays for touchdowns faster than 75 studs/s. |
| `swim`, `swim_idle` | UAL 1 `Swim_Fwd_Loop`, `Swim_Idle_Loop` | Retimed, head and arm lag. |
| `sit` | UAL 1 `Sitting_Idle_Loop` | Head levelled. |
| `dance` | UAL 1 `Dance_Loop` | Moves pushed further (arms, spine and hips), hips bounce on the beat (120 bpm). |
| `wave`, `point` | Procedural upper body over UAL 1 `Idle_Loop` legs | Our key poses. UAL Standard has no wave or point. |
| `cheer`, `climb` | Procedural (`anim_clips.py`) | UAL Standard has no cheer. Its only climb (`ClimbUp_1m_RM`) is a one-off ledge mantle, not a ladder/truss loop. |

The `pose_*` clips are still concept poses, not animation.
