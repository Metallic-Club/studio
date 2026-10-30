# Default avatar: design

Date: 2026-10-30. Status: **design**. Replaces the "blocky six-part figure" in section 4 of
`2026-10-30-metallic-studio-design.md` ("Character like Roblox").

## 1. What the owner asked for

The owner drew a concept sheet (`art/avatar/concept/default-avatar-concept.png`) and asked for it to become Metallic's
default character, built "one to one as much as possible" in our engine. It must look perfected, not like AI slop.
Improvements are welcome. The owner's words, in short:

- A 3D model of the concept. The whole character may be grey.
- Full rigging and default animations. The owner asked for deep research into modelling, rigging and animation for
  default game characters.
- Players can import their own characters. The default characters are the ones we make, and a player can swap to any
  character.
- When a game uses another style, the character is restyled to fit and still looks right. For a 2D game it becomes
  2D. For a PS1-style game it looks PS1. PS1 graphics should also be an option for games and templates.

## 2. Reading the concept sheet

- **Proportions:** stylised and chunky, about 4.5 heads tall. The head is a large rounded box, slightly wider at the
  top, with a short neck. The torso is a tapered, faceted shape with a visible chest plane and a pelvis "brief" line.
  Arms and legs are long tapered prisms with clear elbow and knee breaks. The feet are big, flat, sole-heavy boots
  with a bevelled toe. The hands are blocky mittens with a separate thumb, four finger blocks when open, and a fist
  pose.
- **Surface:** planar facets with soft bevels (an artist's mannequin, not a smooth organic body), and no face. The
  thin cross on the head is a construction line, not a feature.
- **Views:** front, 3/4 front, side, back and 3/4 back, plus head views (front, side, 3/4), hands (back, fist,
  open), feet (front, side, sole) and three poses (hand on hip, run, sitting back on the hands).

## 3. Decisions

Each item marks whose decision it is: the owner's, or my call.

1. **Colour:** neutral grey by default (owner). There are separate material slots for head, torso, arms, legs, hands
   and feet so games and players can colour them, like Roblox body colours (my call).
2. **Source of truth:** a Blender Python script in `art/avatar/`, run headless (`blender -b --python ...`). It
   regenerates the mesh, rig and animations from scratch, so the model can be reviewed and rebuilt. Exports go to
   glTF 2.0 (`.glb`). Blender 5.2 is installed (my call).
3. **Budget:** about 3,000 to 6,000 triangles for LOD0, then LOD1 at about 50% and LOD2 at about 15%. One UV set,
   laid out for a later texture or decal (my call).
4. **Rig:** one humanoid skeleton with VRM-humanoid bone names (hips, spine, chest, upperChest, neck, head; shoulder,
   upperArm, lowerArm, hand per side; thumb and four fingers with 2 to 3 bones each; upperLeg, lowerLeg, foot, toes
   per side), plus a root bone. VRM names make importing and retargeting players' own characters straightforward.
   Skinning uses at most 4 influences per vertex, which fits glTF and our GPU path. Faceted limbs use rigid or
   near-rigid weights with a short blend at the joints, so the planes keep their shape the way the concept shows
   (my call).
5. **Default animations:**
   - Locomotion, matching the Roblox defaults: idle (two variants), walk, run, jump (start), fall loop, land, climb,
     swim and swim idle, sit.
   - Extras: wave, point, cheer and dance (emotes), plus the concept's three poses as still poses.
   - Root motion is off: the character controller moves the character.
   - Clips loop cleanly at 30 fps, and walk and run are authored to the controller's speeds (WalkSpeed 16, run
     ×1.5) so the feet don't slide (my call).
6. **Engine:**
   - Issue #76 (GPU skinning and glTF skins) moves forward to land right after milestone 1A: glTF import with the
     `gltf` crate (MIT OR Apache-2.0), linear-blend skinning, and clips sampled on the CPU.
   - The animation system itself (lane #40, `metallic_anim`) supplies blending and the locomotion state machine. A
     small built-in controller (idle / walk / run / jump / fall / land / climb / swim) ships first, and #40 replaces
     it.
   - The Parkour template's avatar switches from parts to this character. The collision capsule does not change.
7. **Restyling for each game's style:** this is already designed as *Looks* (`docs/design/look-presets.md`, issues
   #62 to #66). The character needs no per-style model, because a Look changes how it is drawn:
   - `toon` matches the concept itself (cel bands plus a brown inverted-hull outline).
   - `retro-32` is the PS1 look: low internal resolution, vertex snapping, affine textures and dither.
   - `pixel-2d` and `smooth-2d` cover 2D games: an orthographic side camera, plus the sprite bake tool (#77). The
     bake tool turns the rigged 3D character and its clips into 8-direction sprite sheets in the game's palette.
   - Acceptance: the same character, rendered in Standard, Toon, Retro-32 and Pixel-2D, reads clearly in all four.
8. **Importing your own character:** players import a `.glb` or `.vrm` with a humanoid skeleton. Bones are mapped by
   VRM names, or by a name-matching table for Mixamo and Blender Rigify. The default animations then retarget to it.
   This is in milestone 2 or later; the default character is the reference rig for it.

## 4. Phases

| Phase | Output | Gate |
|---|---|---|
| A | Blender script plus mesh; turnaround renders matching the concept's views, in grey clay and in a toon/outline render | Lead and owner see the stills before any rigging |
| B | Skeleton, weights, LODs, the default clips, `.glb` export; renders of the concept's three poses and a contact sheet of every clip | Lead reviews the poses and deformation at the joints |
| C | Engine: glTF and skinning (#76), clip playback, built-in locomotion controller; the Parkour avatar swapped | Tests, plus in-engine screenshots and a capture of walk, run and jump |
| D | Look acceptance: the character in Standard, Toon, Retro-32 and Pixel-2D | Screenshots |

## 5. Quality bar (anti-slop)

- **Silhouette:** in each view, the outline must match the concept's silhouette when the two are overlaid at the same
  height. Lead and owner check by eye against the sheet.
- **Planes:** no smooth-shaded blob. Facets and bevels must read the way the concept's planes do. No visible
  triangulation artefacts and no shading seams.
- **Hands and feet:** match the concept's details (thumb, finger blocks, sole plane, toe bevel).
- **Deformation:** elbows, knees, shoulders and hips keep their volume in the poses. Nothing pokes through, and the
  wrists don't twist like candy wrappers.
- **Motion:** weight and overlap: anticipation on jump, contact poses on walk and run, and no foot sliding at the
  controller's speeds.
