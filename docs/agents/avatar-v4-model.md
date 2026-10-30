# Default avatar v4: crisp concept model and toon look

This continues `avatar-v3-model.md` on `studio/avatar-v3`. The owner's verdict on v3 was that the
proportions were already close. What looked wrong was the style. For v4 the owner chose:

1. **Crisp concept.** Build the model the way the drawing is drawn: planes, chamfers and tapered
   prisms.
2. **A cartoon look in the game.** An ink outline and warm two-tone shading, as on the concept
   sheet.

This doc covers the model changes, the engine's new toon style, where the review stills are, and
what is still off.

## Model (`art/avatar/build_avatar.py`)

- **Head.** It is a chamfered block lofted from horizontal sections (`HEAD_ROWS`). The implicit
  surface, its analytic normals and `HEAD_JAW`/`HEAD_GRID`/`HEAD_CENTRE` are gone.
  - The front plane and the side planes are flat. The front corners have small bevels (about 13%
    of the width), and the back corners are rounder.
  - The top plane is broad, and the front-view outline rounds over into it.
  - The sides angle in over the lower third into a jaw, and there is a flat chin underside.
  - In side view, the underside runs up from the chin to the neck at an angle.
- **Neck.** A short prism with a slight taper, about a third of the head's width.
- **Torso and pelvis.** Chamfered sections (`seg=1`) with faceted sides (the new `loft(...,
  facet=True)` marks every lengthwise edge sharp). The waist is narrower than v3: a half width of
  0.086 m at z 0.885, where v3 had 0.092 m.
- **Limbs.** Faceted, tapered, 8-sided prisms.
  - The upper arm tapers to the elbow, from a half width of 0.047 m to 0.036 m.
  - The forearm widens to 0.050 m just below the elbow, then tapers to 0.036 m at the wrist.
  - The thigh tapers from 0.073 m to 0.049 m at the knee.
  - The shin is narrower: 0.046 to 0.053 m. In v3 it widened to 0.070 m toward the boot, which made
    the lower leg read as a tall boot.
- **Hands.** `HAND_SCALE` is 1.04 → 1.00, and the palm is chamfered.
- **Boot.** `BOOT_W` is 1.22 → 1.14, and the cuff is lower and narrower (z 0.164–0.168 m, half
  widths 0.060–0.066 m). Hull edges above 30° are now marked sharp at every LOD, so the wedge reads
  crisp.
- **Default colour.** Every slot's base colour is now the concept's skin tone, #e9d8b6 (linear
  0.814, 0.686, 0.468). It was grey.

### Joint changes from v3 (metres, left side)

| joint | v3 | v4 |
|---|---|---|
| wrist | (0.270, -0.046, 0.626) | (0.284, -0.046, 0.628) |
| knee | (0.122, -0.004, 0.412) | (0.127, -0.004, 0.412) |
| ankle | (0.175, 0.026, 0.112) | (0.186, 0.026, 0.112) |

The elbow and the hand and foot points follow from these. The arms hang a little further out and
the feet stand a little further apart, which gives the concept's relaxed A-stance.

## Engine: toon style (`crates/metallic_render`)

- **`SkinStyle { Standard, Toon }`** is a new field on `SkinnedInstance`. `Toon` is the default,
  and the runtime avatar (`metallic_runtime::avatar`) uses it. A future per-object style picker
  only has to set this field.
- **Toon shading** (`fs_toon` in `shaders/skinned.wgsl`):
  - The lit tone is the slot colour itself. The shade is warmer and darker (×0.62/0.60/0.66), and
    the core facing away from the sun is darker again (×0.45/0.42/0.49).
  - Band edges are soft (smoothstep over about 0.08 of N·L). The tones still follow the sun
    direction and the cascaded shadows.
  - The brightness and a little of the hue come from the sun and sky colours, and fog still
    applies.
  - The colours are chosen in display space and passed through `unfinish` (the inverse of the
    tone curve). Under the default sky, the lit tone therefore comes out as #e9d8b6 on screen.
- **Ink outline** (`vs_outline`/`fs_outline`):
  - It is an inverted hull: the back faces are pushed out along a smoothed normal in clip space.
  - The width is at most `OUTLINE_MAX_PX` = 2.2 px. Below that cap it is `OUTLINE_STUDS` = 0.035
    studs, so the line thins with distance once the figure is under about 350 px tall.
  - The colour is #5b3b33.
- **Smoothed normals** are computed once at upload: per vertex, the sum of the normals at the same
  position, to 0.1 mm. The skinned vertex grew from 52 to 64 bytes. Nothing is allocated per frame.
- **`Globals`** gained a `viewport` vector (the pixel size and its reciprocal), which the outline
  uses to set its width.
- **Draw order.** The main pass draws standard skinned instances, then toon instances, then the
  toon outlines, using two new pipelines (`skinned_toon` and `skinned_outline`).
- **Test.** The new headless test `toon_avatar_has_ink_outline_and_warm_shading` checks two
  things: ink-coloured pixels appear around the toon avatar (and not around the standard one), and
  the torso is warm.

## Stills (`art/avatar/renders/v4/owner-review/`)

- `turnaround_engine.png`: the concept above, the in-engine toon look below. It is drawn in the
  rest pose with the fingers curled.
- `poses_engine.png`: hand on hip, run and sitting, in the engine.
- `v3_vs_v4.png`: the concept, then v3 as the owner reviewed it (standard shading), then v4.
- `engine_lods.png`: LOD0, LOD1 and LOD2 in the engine.

To regenerate:

```text
blender -b --factory-startup --python art/avatar/rig_avatar.py
cargo run --release -p metallic_render --example avatar_review -- art/avatar/out/default_avatar.glb art/avatar/out/engine
python art/avatar/owner_review.py --version v4 --prev <folder of v3 engine renders> v3
```

The v3 renders come from the same example run on the v3 glb (`git show 7833d65e:art/avatar/out/default_avatar.glb`),
with `--standard`.

## Critique rounds (engine, toon, side by side with the concept)

1. The head was a hard box with tight top corners and a flat back, and the boots read as soft
   loaves. I rounded the back of the skull and made the hull edges crisp.
2. Seen at the same scale, the concept's front-view head is a rounded block: the top rounds over
   about a quarter of the width, the jaw narrows from mid-height, and the underside slopes steeply.
   I reprofiled `HEAD_ROWS` to match.
3. Two changes to the review renders and one to the model:
   - The turnaround was drawn in the first frame of idle (bent arms). It now uses the rest pose,
     which is the model's actual A-stance.
   - The camera was lowered to remove the high-angle distortion.
   - The arms were moved out and the feet apart.
4. The idle curl read as open claws, so the turnaround now closes the fists with the run pose's
   fingers. I also added a knee step (the shin's rim stands out of the thigh's end), raised
   `HAND_SCALE` to 1.00, tucked the torso top inside the neck, and reframed the poses.

Triangles: **LOD0 4360, LOD1 2756, LOD2 896** (budget 5744 / 2840 / 876). LOD2 is 2% over.

## Still off

- The concept's interior ink lines (chest line, pelvis V, centre line) are not drawn. An inverted
  hull only inks silhouettes and the places where parts overlap (elbow, wrist, knee). Interior
  lines would need a separate crease-line feature.
- A short dashed ink line shows at the base of the neck in the front view. It is the trapezius
  edge fighting the neck surface for depth.
- The head's top corners are still a little squarer than the drawing's.
- The poses come from the animation branch's clips, which were made for v3. Sitting in particular
  does not match the concept's pose.
- The Blender toon renders (`render_phaseB.py`) were not redone for v4. The stills that count are
  the in-engine ones.

## After the owner's approval: fixes and integration

The owner approved v4 with four fixes:

1. **Head crown.** The top and back of the skull are now a dome with a radius of about 0.15 m (the
   first eight rows of `HEAD_ROWS`). The flat face plane starts at z 1.435, and the sides and jaw
   are unchanged.
2. **Neck.** It is slimmer: half widths go from 0.058 to 0.049 m, about a third of the head's
   width. The trapezius rows are narrower too (half widths 0.098 and 0.068 m), and the torso's
   top ring sits well inside the neck.
   - The "dashed line" at the neck base was not ink. It was the stair-stepped edge of the head's
     cast shadow on the chest.
   - The toon shader now uses the filtered shadow term directly, without a hard step on it.
   - The outline hull is also pushed 0.02 studs back along the view ray, so it never ties with
     the surface where parts meet.
3. **Fists.** `HAND_SCALE` is 1.12. On the concept's front view, the fist is about 0.37 of the
   head's width; v4 was about 0.33.
4. **Toon shading.**
   - The shade and core tones are warmer and darker (×0.66/0.54/0.46 and ×0.50/0.39/0.33).
   - The lit band starts at N·L 0.12–0.20, so planes turned partly away read as shade.
   - A 7% darker terminator line runs just inside the lit band.
   - A faint warm rim on the shade side separates the form from the ink.

Then I merged `studio/avatar-v3-anim`, which brings the CC0 Quaternius UAL clips (see
`art/avatar/ANIMATION_SOURCES.md` and `CREDITS.md`).
- Every clip was re-baked on v4 with `METALLIC_UAL=<packs> rig_avatar.py`. The floor check
  passes: no sole point is more than 5 mm below the floor.
- The standing-clip slip numbers in `out/default_avatar_clips.json` are essentially the same as on
  the animation branch.
- The in-engine strips in the toon look are in `art/avatar/renders/v4-anim/`, from
  `cargo run --release -p metallic_render --example avatar_clips -- art/avatar/renders/v4-anim
  idle walk run jump fall land sit wave cheer dance`.
- The animation branch's v3 strips (`renders/v3-anim/`) were removed. `render_anim_review.py` now
  writes to the gitignored `out/anim-review/`.
- `v2_vs_v4.png` and `v3_vs_v4.png` in `renders/v4/owner-review/` compare the concept, the old
  version (standard shading) and v4 (toon).
