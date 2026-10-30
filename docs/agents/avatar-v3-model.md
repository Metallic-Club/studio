# Default avatar v3: model rebuild

Branch `studio/avatar-v3`. Scope: mesh, proportions and shading of the default avatar
(`art/avatar/build_avatar.py`), the rig's spine and shoulder joints (`rig_avatar.py`), the review
renders, and an in-engine review example. Animations are on `studio/avatar-v3-anim`;
`anim_clips.py` is untouched here. The exported `out/default_avatar.glb` contains the clips that
are on main.

## What changed in the mesh

- **Head.** It is no longer a lofted rounded box. It is a cube-sphere grid whose vertices are cast
  from `HEAD_CENTRE` onto an implicit surface. The surface is defined per height by `HEAD_ROWS`
  (half width, front, back, superellipse exponent), and `HEAD_JAW` makes the lower back fuller.
  The result is a soft block with a flatter face whose lower half narrows into a jaw, and the side
  profile tapers. Its normals are analytic (the gradient of the field), so cel bands stay smooth.
  `HEAD_SEG` and `--head-seg` are no longer used. `head_compare.py` was removed.
- **Neck.** About a third of the head's width, and it no longer narrows to a stalk.
- **Torso.** Clear chest with a sharp under-chest break at z 0.952, narrower waist, higher and
  squarer shoulders. It continues 5 cm into the pelvis, so bending the spine never opens a gap.
- **Pelvis.** A V-shaped brief, with a hip line and a sharp belt line.
- **Arms.** Slimmer. The upper arm has a ball top and the forearm tapers. The hands are smaller
  (`HAND_SCALE` 1.24 to 1.04), with longer fingers and a 2 mm groove between them, so the knuckles
  read.
- **Legs.** The thigh has a ball top at the hip and tapers to the knee. There is a knee hinge, and
  the shin has a slight calf and is flush with the end of the thigh.
- **Boot.** Smaller (width factor 1.34 to 1.22, toe 0.206 to 0.196, heel -0.112 to -0.102), with
  a narrower heel, the instep lowered to the concept's slope, and the cuff at about 0.18 m.
- **Shading fixes.**
  - The sole's end rings are marked sharp.
  - At LOD1 and LOD2 (where there is no bevel), hull edges sharper than 35° are marked sharp.
  - Without these, the big caps pulled the normals downward, and the engine drew a dark band
    around the boot.

Triangles (budget LOD0 5744, LOD1 2840, LOD2 876): **5908, 2812, 888**. LOD0 is 3% over the
budget and LOD2 is 1% over, both within the spec's 3–6k. Most of the extra LOD0 triangles are in
the finer head grid (1200 triangles). `crates/metallic_asset/tests/import.rs` pins the new counts.

## Joint changes (metres, left side, front is -Y, from `skeleton()`)

| joint | v2 | v3 | moved (cm) |
|---|---|---|---|
| shoulder | (0.130, 0.004, 1.010) | (0.122, 0.000, 1.046) | 3.7 |
| elbow | (0.197, -0.006, 0.822) | (0.191, -0.009, 0.851) | 2.9 |
| wrist | (0.262, -0.040, 0.642) | (0.270, -0.046, 0.626) | 1.9 |
| hand end | (0.291, -0.076, 0.535) | (0.297, -0.075, 0.528) | 0.9 |
| hip | (0.064, 0.000, 0.730) | (0.070, 0.000, 0.735) | 0.8 |
| knee | (0.126, 0.002, 0.400) | (0.122, -0.004, 0.412) | 1.4 |
| ankle | (0.178, 0.030, 0.105) | (0.175, 0.026, 0.112) | 0.9 |
| ball | (0.184, -0.080, 0.035) | (0.181, -0.076, 0.035) | 0.5 |
| toe | (0.184, -0.172, 0.035) | (0.181, -0.166, 0.035) | 0.7 |
| heel | (0.190, 0.142, 0.000) | (0.187, 0.128, 0.000) | 1.4 |
| thumb 0..3 | (0.231, -0.069, 0.609) .. (0.187, -0.102, 0.497) | (0.244, -0.070, 0.598) .. (0.207, -0.096, 0.504) | 1.7–2.2 |
| index 0..3 | (0.268, -0.103, 0.531) .. (0.308, -0.124, 0.419) | (0.278, -0.097, 0.524) .. (0.315, -0.114, 0.419) | 1.2–1.4 |
| middle 0..3 | (0.291, -0.076, 0.535) .. (0.334, -0.098, 0.415) | (0.297, -0.075, 0.528) .. (0.337, -0.093, 0.415) | 0.5–0.9 |
| ring 0..3 | (0.314, -0.049, 0.538) .. (0.355, -0.070, 0.424) | (0.316, -0.053, 0.531) .. (0.354, -0.070, 0.424) | 0.2–0.9 |
| little 0..3 | (0.337, -0.022, 0.541) .. (0.372, -0.040, 0.446) | (0.335, -0.031, 0.534) .. (0.366, -0.045, 0.444) | 0.8–1.2 |

The spine bones in `rig_avatar.py` move with the new torso (head z to tail z):

| bone | v2 | v3 |
|---|---|---|
| hips | 0.760 → 0.845 | 0.760 → 0.850 |
| spine | 0.845 → 0.935 | 0.850 → 0.950 |
| chest | 0.935 → 1.020 | 0.950 → 1.035 |
| upperChest | 1.020 → 1.100 | 1.035 → 1.115 |
| neck | 1.100 → 1.190 | 1.115 → 1.185 |
| head | 1.190 → 1.420 | 1.185 → 1.420 |
| shoulder (head) | (0.022, 0.004, 1.072) | (0.024, 0.006, 1.088) |

Other skinning changes: the torso blend heights are now 0.815 to 1.076, and the shoulder weight
starts at z 1.000 instead of 0.985.

Notes for the animation branch:
- The arm is 3.7 cm higher at the shoulder and 1.6 cm lower at the wrist. The upper arm is
  0.207 m (was 0.200 m) and the forearm 0.241 m (was 0.194 m).
- Clips that place the hands by rotation alone (hand on hip, sitting) land a little differently.
  Recheck the hand contacts.

Rig: 53 bones **including** the root (52 VRM humanoid bones plus `root`), at most 3 influences
per vertex, and no unweighted vertices. The brief says "53 plus root". The rig was 53 including
root before this change too, and I kept it as is.

## Silhouette IoU (fist renders vs the concept, `silhouette_check.py --raw out/rawB`)

| view | v2 | v3 |
|---|---|---|
| front | 0.851 | 0.852 |
| front 3/4 | 0.836 | 0.835 |
| side | 0.903 | **0.910** |
| back | 0.880 | 0.856 |
| back 3/4 | 0.813 | 0.816 |

IoU barely moved, and it is not a good judge here. The concept's views do not agree with each
other: the back view is narrower and has smaller boots than the front view. v2 had already been
tuned to the outline. v3's gains are inside the outline (head shape, jaw, chest, waist, knee break,
fists) and in the side view. The back view got worse because v3 follows the front and side views
where the sheet contradicts itself.

## Renders (`art/avatar/renders/v3/`)

- `owner-review/turnaround_toon.png`, `owner-review/turnaround_engine.png`: the concept above, the
  model below, five views.
- `owner-review/poses_toon.png`, `owner-review/poses_engine.png`: hand on hip, run, sitting.
- `owner-review/engine_lods.png`: the five views at LOD0, LOD1 and LOD2 in the engine.
- `before_after_toon.png`, `before_after_clay.png`: the concept, then v2, then v3.
- `contact_sheet_*.png`, `deformation_*.png`, `poses_vs_concept_*.png`, `silhouette/`.

The old `renders/phaseA/` files and the phaseB contact, deformation, pose and head-segment sheets
were removed. The phaseB animation strips stay, because they belong to the animation branch.
`render_turnaround.py` is now a quick tool for the rest pose and writes to the gitignored
`out/turnaround/`.

To regenerate:

```text
blender -b --factory-startup --python art/avatar/rig_avatar.py
blender -b --factory-startup --python art/avatar/render_phaseB.py -- turnaround poses deform
python art/avatar/silhouette_check.py --raw art/avatar/out/rawB
python art/avatar/render_phaseB.py --compose
cargo run --release -p metallic_render --example avatar_review -- art/avatar/out/default_avatar.glb art/avatar/out/engine
python art/avatar/owner_review.py [--v2 <rawB folder rendered by the v2 scripts>]
```

## Engine check

`crates/metallic_render/examples/avatar_review.rs` renders the glb headlessly. It uses High
quality with sun, shadows and fog, and no outline. It writes the five views at each LOD, using
the first frame of idle, plus the three pose clips.

The only engine-only problem was the dark band around the boot at LOD1 and LOD2. It was fixed in
the mesh normals, so there are **no engine shader changes**. LOD selection, skinning and the
analytic head normals all come through correctly.

## Still off

- The back view is wider than the concept's back view (IoU 0.856), because the sheet is
  inconsistent and v3 follows the front view.
- The shoulders still slope slightly. The concept's shoulder line is squarer.
- The lower leg and boot together read a little like tall boots, because the shin is wide at the
  bottom to match the outline.
- The feet point straight ahead in the rest pose. In the concept the left foot is turned out,
  which is left to posing.
- In the toon look the fists still show small outline specks where fingers touch. The hull is
  thinner on the fingers, which helps, but some specks remain.
- At the waist in the engine, the edge of the pelvis casts a thin dark line that reads as a belt.
- LOD0 and LOD2 are slightly over the per-LOD triangle budget (see above).
- The pose clips (anim branch) were written for v2 proportions. In hand on hip, the hand sits
  slightly off the hip.
