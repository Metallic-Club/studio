# Default avatar: modelling research (Phase A)

Date: 2026-10-30. Scope: what we need to know to model the default character from the owner's concept sheet, and
what `art/avatar/build_avatar.py` does with it. Rigging and animation research belongs to Phase B.

## 1. Planar, mannequin-like surfaces

The concept is an artist's mannequin: big flat planes, soft bevels, no smooth organic blob. Game artists get that
look with the **mid-poly / face-weighted-normals** workflow rather than a high-to-low normal-map bake:

- Model the shapes with real bevels (chamfers) on the edges that should catch light, then set vertex normals by
  face area so each big face stays flat and only the narrow bevel strips carry the gradient. This gives clean
  planar shading without a normal map, and with one smoothing group there are no split vertices at hard edges, so
  the engine's vertex count stays low ([80.lv, mid-poly in UE5](https://80.lv/articles/creating-assets-within-the-mid-poly-workflow-in-ue5/);
  [Polycount thread on weighted normals](https://polycount.com/discussion/comment/2459736)).
- Large, silhouette-changing bevels are modelled as geometry; thin one-segment bevels are only for catching light
  ([Polycount](https://polycount.com/discussion/comment/2805354)).
- Blender's Weighted Normal modifier does this ("Face Area" mode, a weight that favours large faces, and "Keep
  Sharp" to respect edges marked sharp). Applying it bakes custom normals that glTF exports as-is.

**Applied:** every part is built from rounded-rectangle rings (2-3 bevel segments per corner) or bevelled convex
hulls. The joined mesh gets a Weighted Normal pass (Face Area, weight 80, Keep Sharp) that is applied before
export. Plane breaks the concept draws as lines (chest break, elbow and knee steps, waist, centre line of chest and
back) are marked sharp so they stay crisp; everything else is one smoothing group. Result: no shading seams and
no visible triangulation in either render pass, at 4,720 triangles.

## 2. Topology that deforms at elbows, knees, shoulders and hips

- Put at least three loops around each joint, closer together on the side that folds; a limb with no joint loops
  hinges with a hard kink or stays stiff ([CG Cookie topology thread](https://cgcookie.com/community/9812-searching-for-topology-and-modeling-advice);
  [Shoot First topology guide](https://www.shootfirst.art/toplogy)). There is no single correct knee layout; it
  depends on the poses the rig needs ([Blender Artists](https://blenderartists.org/t/how-do-i-fix-this-knee-topology/1653202)).
- Our character is faceted and the design doc asks for **rigid or near-rigid weights with a short blend**, so the
  planes keep their shape. That suits separate overlapping shells per body part (the way an artist's mannequin and
  Roblox R15 parts work) better than one continuous skin: volume cannot collapse, and the joint is hidden by overlap.
- Bind in an A-pose: the shoulder then travels about 45 degrees to reach rest instead of about 90 from a T-pose,
  which is where T-pose skinning suffers most. Many pipelines rig in T and skin in A
  ([Neural4D comparison](https://blog.neural4d.com/comparisons/a-pose-vs-t-pose/);
  [Polycount](https://polycount.com/discussion/comment/2582820)).

**Applied:** shells per part (head, neck, torso, pelvis, arm, hand, leg, boot) that overlap at every joint. Each
limb is one loft with a dedicated loop at the joint (ring plane bisecting the two segments) plus a loop on each side
of it: the arm has loops at the deltoid, elbow, forearm step and wrist; the leg at the knee, the lower-leg step
and the hip, where the top rings are tilted so the thigh covers the hip side. Each finger has a knuckle loop and a
middle-joint loop, the thumb three segments, and the boot a loop across the ball of the foot for a later toes bone.
Relaxed A-pose: arms 20 degrees out from vertical and slightly forward, legs about 11 degrees apart.

## 3. Budgets for platform default avatars

| Platform | Published limit / convention |
|---|---|
| Roblox (R15 / Rthro bodies, Avatar Setup) | Head 4,000 tris, each arm and leg 1,248, torso 1,750; whole body at most 10,742 ([Roblox Avatar Setup requirements](https://create.roblox.com/docs/en-us/avatar-setup/auto-setup-requirements)). A-pose or T-pose, a distinct neck, no limbs overlapping in the front view. |
| VRChat | PC "Excellent" up to 32,000 tris and 4 material slots; mobile "Excellent" 7,500 tris and 1 slot, "Good" 10,000 ([VRChat performance ranks](https://creators.vrchat.com/avatars/avatar-performance-ranking-system/)). |
| Fortnite / UEFN | No public character budget. Epic's asset guidance sizes budgets against a 192 cm player (about 3,000 vertices for a character-sized prop) and asks for three LODs, each about half the previous ([Epic, Fortnite-ready assets](https://dev.epicgames.com/documentation/fortnite/fortniteready-assets-best-practices-in-fortnite)). |
| Fall Guys | No official polycount or talk found; only fan models and the official "anatomy" art. Treated as unknown. |

Takeaway: a stylised, textureless default character sits comfortably at 3,000 to 6,000 triangles, well under
Roblox's per-part caps and VRChat's mobile "Excellent" rank. LOD1 at about 50% and LOD2 at about 15% (design doc
3.3) matches Epic's halving rule for the first step.

**Applied:** 4,720 triangles at LOD0 (head 476, neck 92, torso 276, pelvis 220, each arm 284, each hand 632, each
leg 236, each boot 676, as built). Six material slots, which fits VRChat PC "Good" but not mobile; for a mobile
build the slots collapse to one material with vertex colours or a palette texture (Phase B note).

## 4. Reading well at small sizes

- Silhouette first: the character must read as a solid shape before any interior detail; keep only detail that
  survives at the real on-screen size ([readable game art](https://bugnet.io/blog/readable-game-art-why-clarity-beats-detail);
  [shape language](https://rocketbrush.com/blog/shape-language-in-game-character-design-how-to-make-characters-readable-and-consistent)).
- Big heads and simplified bodies read at small sizes and carry expression; too many small elements make the
  silhouette noisy ([mascot design notes](https://mascot-app.vercel.app/blog/game-mascot-design)).
- Roblox keeps the head's scale within 95 to 100% of the body even when proportions change, and offers Classic,
  Rthro and Rthro Slender proportions ([Roblox body scale](https://create.roblox.com/docs/en-us/art/accessories/body-scale)).

**Applied:** the concept's own proportions already follow these rules, and we keep them: about 4.1 heads tall (the
head is 0.37 m of 1.54 m), a thin neck (0.08 m) that separates head from body, big mitten hands (about 0.18 m long,
wider than the forearm) and heavy boots (0.32 m long, 0.22 m wide) so hands and feet stay visible at a distance, arms
and legs clear of the torso in the front view. The silhouette script checks every body view against the concept.

## 5. Ready for the rig (VRM humanoid)

- VRM 1.0 requires hips, spine, head, and upper arm, lower arm, hand, upper leg, lower leg and foot per side; chest,
  upperChest, neck, shoulders, toes and fingers are optional; hierarchy hips > spine > chest > upperChest > neck >
  head, with arms off upperChest via shoulder ([VRM 1.0 humanoid](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/humanoid.md)).
- VRM 1.0 also defines a required rest pose: facing +Z, standing on y = 0, symmetric in X, arms along X in a T,
  feet parallel to Z (no splay), thumbs 45 degrees between X and +Z ([VRM T-pose, Japanese](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/tpose.ja.md)).
  VRChat additionally needs neck and chest ([VRChat rig requirements](https://creators.vrchat.com/avatars/rig-requirements)).

**Applied:** the mesh faces -Y in Blender, which the glTF exporter writes as +Z; it stands on 0, is exactly
symmetric in X, and the feet are parallel (the concept's slight toe-out is left for the idle pose). Every vertex is
in a part vertex group named after its shell (Head, Neck, Torso, Pelvis, Arm.L/R, Hand.L/R, Leg.L/R, Boot.L/R),
which maps one-to-one onto VRM bones for the near-rigid weights (Arm splits at the elbow loop into upperArm and
lowerArm, Torso at the chest break into chest and upperChest). For VRM export the rig poses the A-pose mesh into the
T-pose; the model itself stays in A-pose, which is the better bind pose (section 2).

## 6. Decisions this research changed

1. Separate overlapping shells instead of one welded skin: matches the mannequin look and the near-rigid weighting.
2. Weighted normals plus marked sharp lines instead of auto-smooth by angle: soft bevels, flat planes, crisp
   construction lines, no normal map.
3. Feet modelled parallel (VRM rest-pose rule), not splayed as drawn.
4. Kept the concept's six colour slots for body colours, and noted the one-material path for mobile.
