# Building tools: research

Date: 2026-10-30. Status: research for issue #200. Docs only; no code changed.
Design that uses it: `docs/design/building-tools.md`.

Question: what makes building fast and easy in Roblox Studio, its plugins, and other games and engines, and which
ideas Metallic Studio should adopt.

How to read it:
- **(Sourced: URL)**: a research subagent or I read that page on 2026-10-30. Creator Store vote counts and prices
  come from Roblox's public toolbox search API and the store pages; the store rounds vote counts.
- **(Unverified)**: I could not confirm it from a primary source. Do not build on it without checking.
- **Judgment**: my reasoning, not a sourced fact.
- Section numbers (R1.4 and so on) are what the design doc cites.

Method: four research subagents searched in parallel (Roblox plugins; Roblox Studio's own tools; other games and
engines; colour pickers and precision tools). Where a plugin has a public repository, shortcuts and defaults were
read from its source code rather than from forum posts. The Valve Developer Wiki, the Blender manual site and Adobe's
help pages blocked automated access; facts from them come from search excerpts or the Blender manual's source files
and are marked. A second research document on other engines may land later as
`docs/research/2026-10-30-building-tools-other-engines.md`; it did not exist on any branch when this was written.

No code was copied from Roblox, Unity, Unreal, Godot or any plugin. Licences are listed so nobody copies by mistake:
several plugin repositories are MIT, but F3X has no licence file, Unity's packages forbid use in a competing product,
and WorldEdit and TrenchBroom are GPL.

---

## Summary

1. **Roblox Studio has absorbed many plugin ideas** (pivots, align, tags, soft snapping, a material picker) but still
   spreads them across ribbon tabs, and its Properties colour swatch opens the operating system's colour dialog
   (R2.7). The most loved plugins are single-purpose, pick geometry by clicking it, and close seams automatically
   (R1.17).
2. **The fastest tools share four habits**: the object follows the cursor onto surfaces; one modifier duplicates
   during a drag; one key changes the grid size; and typed numbers work mid-gesture (R2.1, R4.1, R4.4, R4.7, R4.9,
   R4.10).
3. **Vertex-to-vertex snapping on a held key** (Unity and Unreal V) and **surface snapping** are the precision tools
   people praise most outside Roblox (R4.1, R4.4).
4. **Arrays by dragging or scrolling** (Redupe, Axiom, Dreams) beat dialog-based arrays for speed (R1.11, R3.2, R3.5).
5. **Runtime building in Roblox games** (Bloxburg, Build a Boat) uses one-letter tools, a grid-size key, a colour
   eyedropper on a modifier, and server-checked placement (R3.8, R3.9). Roblox's own `Handles`, `ArcHandles` and
   `DragDetector` classes give scripts gizmos (R2.11).
6. **Colour pickers** in creative tools converge on a 2D field plus a hue control, Hex and numeric fields, an
   eyedropper on I or E, recent colours and saved swatches (R5). Oklab is the sourced reason to generate palettes in a
   perceptual space (R5.8).

Ranked ideas to adopt: section R6.

---

## R1. Roblox plugins

### R1.1 Building Tools by F3X (plugin and in-game tool)

- **What**: a docked panel of 14 tools: move, resize, rotate, paint, surface, material, anchor, collision, new part,
  mesh, texture, weld, lighting, decorate. Ships as a Studio plugin and as an in-game tool (Sourced:
  https://github.com/F3XTeam/RBX-Building-Tools).
- **Why it is fast**:
  - One letter per tool: Z move, X resize, C rotate, V paint, B surface, N material, M anchor, K collision, J new
    part, H mesh, G texture, F weld, U lighting, P decorate (Sourced:
    https://github.com/F3XTeam/RBX-Building-Tools/blob/development/Loader/init.lua).
  - Shift actions: Shift+Z / Shift+Y undo/redo, Shift+C clone, Shift+X delete, Shift+K select parts inside the
    selection, Shift+R clear selection, Shift+G group as Model, Shift+F group as Folder, Shift+U ungroup, Shift+P
    export; `[` selects siblings (Sourced:
    https://github.com/F3XTeam/RBX-Building-Tools/blob/development/Core/init.lua).
  - Shift+click adds, Shift+drag draws a selection rectangle; Alt+Z / Alt+Shift+Z scope in and out of models (Sourced:
    https://github.com/F3XTeam/RBX-Building-Tools/blob/development/Core/Targeting.lua).
  - Defaults: move 1 stud, resize 1 stud, rotate 15 degrees. "-" focuses the increment field. Numpad keys nudge by one
    increment (8/2 up and down, 1/9 back and forth, 4/6 left and right), Shift reverses. Enter cycles axis and pivot
    modes. Holding R drags by snap points; T puts the part's bottom on the target surface (Sourced:
    https://github.com/F3XTeam/RBX-Building-Tools/blob/development/Tools/Move/init.lua).
  - Pivot modes: move per part or relative to the last selected; rotate about the selection centre, each part, the
    last part, or an edge (Sourced: README above).
- **Praise**: resizing many parts each about its own centre, colouring unions, anchoring whole models (Sourced:
  https://devforum.roblox.com/t/what-is-your-opinion-on-f3x-btools/1052347;
  https://devforum.roblox.com/t/advanced-guide-to-efficient-building-covers-workflow-and-extremely-effective-plugins/2198562).
- **Complaints**: "slow", a panel that covers the view, "for beginners" (Sourced: 1052347 above); one 2024 post calls
  it "outdated and broken" without detail (Sourced:
  https://devforum.roblox.com/t/looking-for-a-plugin-thats-are-similar-to-f3x-plugin/3163131).
- **Price and licence**: free, about 9,600 up and 400 down votes (Sourced:
  https://create.roblox.com/store/asset/144950355). Source is public, but the repository has **no licence file**
  (Sourced: https://api.github.com/repos/F3XTeam/RBX-Building-Tools), so nothing may be copied.

### R1.2 F3X export

- **What**: Shift+P posts the selection over HTTP to F3X's server and shows a creation id; the separate "Import from
  Building Tools by F3X" plugin pastes that id into Studio. Errors cover disabled HTTP and "Post data too large" ("Try
  splitting up your build") (Sourced:
  https://github.com/F3XTeam/RBX-Building-Tools/blob/development/SyncAPI.lua).
- **Status**: the import plugin was last updated 2019-11-27 (Sourced:
  https://create.roblox.com/store/asset/142485815). A June 2025 user says the export site "has been down for a while"
  (Sourced: https://devforum.roblox.com/t/any-alternative-importexport-plugins-tools/3783018). On 2026-10-30 the
  server did not answer within 15 s (Unverified as a lasting outage).
- **Lesson** (Judgment): in-game builds must leave the game through a format the platform owns, not a third-party
  server. Metallic's build kit saves to the game's data store and can export to a scene file (design 13.2).

### R1.3 Archimedes (v3.1.9)

- **What**: builds arcs and circles by rendering rotated copies of a part: "select a part, select the axis... modify
  your angle, and then hit render" (Sourced: https://create.roblox.com/store/asset/144938633).
- **Why it is fast**: hover a part to preview the face, click to pick it; drag around an angle dial or type in its
  centre; Alignment Inside (default), Middle or Outside sets the rotation point; Render Amount is capped at 360 /
  angle; an experimental Auto-Resize closes overlapping edges; an Enabled checkbox pauses it while you drag parts. No
  shortcuts documented (Sourced:
  https://devforum.roblox.com/t/introducing-archimedes-3-a-building-plugin/1610366).
- **Praise**: the default answer for curves (Sourced: guide 2198562 in R1.1).
- **Complaints**: undo used to delete every generated part; models with differently oriented children land wrongly;
  part counts grow fast; malicious lookalike copies exist (Sourced:
  https://devforum.roblox.com/t/archimedes-plugin-patch-v25-direction-naming-scheme-conundrum/333923;
  https://devforum.roblox.com/t/is-it-just-me-or-is-the-archimedes-plugin-is-buggy/1863884;
  https://devforum.roblox.com/t/removing-a-plugin-from-inventory-should-prompt-to-remove-from-studio/1338797).
- **Price and licence**: free, about 6,720 up and 280 down. No published source or licence (Unverified).

### R1.4 Stravant ResizeAlign

- **What**: click a face on each of two parts; it resizes them so the faces meet, removing seams and overlaps at angled
  joints (Sourced: https://create.roblox.com/store/asset/165534573).
- **Why it is fast**:
  - Seven modes, default **Outer Touch**: Outer Touch, Inner Touch, Wedge Join (adds wedges to a sharp point and
    "always avoids Z-fighting"), Rounded Join (fills with a sphere or cylinder), Butt Joint (right angles only),
    Extend Up To (only the first face moves), Extend Into (first face fully penetrates the second).
  - "Wedge Join for tight corners" is on by default and switches mode at small angles.
  - **Reach Around** threshold 25% (default), 15% or Exact: clicking near an edge selects the face behind it, so you
    rarely move the camera.
  - Hold Ctrl for "Dragger Mode": the normal select dragger without leaving the tool.
  (Sourced: https://github.com/stravant/roblox-resizealign/blob/HEAD/src/Settings.lua;
  https://github.com/stravant/roblox-resizealign/blob/HEAD/src/ResizeAlignGui.lua;
  https://devforum.roblox.com/t/resizealign-v13-rounded-join-dragger-mode/1815331)
- **Praise**: "Lifesaver... VERY REALLY RECOMMENDED" (Sourced: guide 2198562); an update "singlehandedly saved tons of
  time" (Sourced: 1815331).
- **Complaints**: some wedge faces cannot be picked; a resize once extended both ends; Studio updates broke it in the
  past, which produced "fixed" forks (Sourced: 1815331;
  https://devforum.roblox.com/t/100-of-stravants-plugins-are-broken/16487).
- **Price and licence**: free, about 1,960 up and 40 down; **MIT** (Sourced:
  https://github.com/stravant/roblox-resizealign).

### R1.5 Stravant GapFill & Extrude

- **What**: "Click two edges and parts are generated filling the gap. Click the same edge twice to extrude the clicked
  face" (Sourced: https://create.roblox.com/store/asset/165687726). A Polygon mode fills between clicked vertices
  (Sourced: https://github.com/stravant/roblox-gapfill/blob/HEAD/src/GapFillGui.lua).
- **Why it is fast**: a status line says what to click next ("Select the first edge...", "Select second edge... or
  click empty space to cancel"). Thickness: Best Guess (default, matches the adjacent parts), One Stud, Thinnest
  (0.05), or Custom (default 0.2). Flip, union and select-results options. Ctrl borrows the dragger. Works on meshes
  and unions by inferring edges from collision geometry (Sourced:
  https://github.com/stravant/roblox-gapfill/blob/HEAD/src/Settings.lua;
  https://devforum.roblox.com/t/gapfill-v13-now-supports-unions-and-meshparts/1817053).
- **Praise**: "Lifesaver in many cases" (Sourced: 1817053).
- **Complaints**: edge picking is "a bit finicky" on complex meshes (the author); users re-click edges and delete
  stray parts (Sourced: 1817053; https://devforum.roblox.com/t/help-with-gapfill/909391).
- **Price and licence**: free, about 3,840 up and 160 down; **MIT** (Sourced:
  https://github.com/stravant/roblox-gapfill).

### R1.6 Stravant Model Reflect

- **What**: mirrors a selection: click a plane to reflect a copy across it, or flip in place through the pivot on a
  local axis. Handles Parts, Unions, CornerWedges, Decals, Textures and surface types; **not MeshParts**, because a
  MeshPart's mesh cannot be scaled negatively (Sourced: https://create.roblox.com/store/asset/217792838;
  https://github.com/stravant/roblox-modelreflect-2).
- **Why it is fast**: one prompt ("Select at least one Part, Model, or Folder... Then select a plane"). Defaults:
  select the reflected copy on; keep open after reflecting off, after flipping on; a 5 s cut-off avoids Studio hangs.
  A floating panel since November 2025. No shortcuts found (Sourced:
  https://github.com/stravant/roblox-modelreflect-2/blob/HEAD/src/Settings.lua).
- **Praise**: used with Quick Stairs for symmetric stairs (Sourced: guide 2198562).
- **Complaints**: unions and corner wedges needed community forks; MeshParts still unsupported (Sourced:
  https://devforum.roblox.com/t/modified-stravants-model-reflect-to-support-unions/66791).
- **Price and licence**: free, about 752 up and 48 down; v2 repository **MIT**.

### R1.7 Brushtool 2.1 (XAXA)

- **What**: "brushes" clones of chosen parts or models onto the map; pick an object, press Add to put it in the brush
  list (Sourced: https://create.roblox.com/store/asset/2268520847).
- **Why it is fast**: per-object Scale and "Wobble" (random X/Z tilt); placement by bounding box or PrimaryPart;
  Stamp mode places one at a time, Erase removes; ignore water and ignore transparent toggles. No hotkeys documented
  (Sourced:
  https://devforum.roblox.com/t/brushtool-20-plugin-clone-and-place-models-onto-your-map-with-ease/255053).
- **Praise**: "an awesome upgrade", easier tree placement (Sourced: 255053).
- **Complaints**: no import/export of brush sets; no clamping to avoid other objects; fake copies that ask for script
  injection (Sourced: 255053;
  https://devforum.roblox.com/t/why-do-i-get-asked-for-script-injection-in-a-safe-plugin/1426000).
- **Price and licence**: free, about 1,820 up and 180 down; "Feel free to modify and distribute your changes", no
  formal licence (Unverified).

### R1.8 Studio Build Suite (SBS)

- **What**: Fractality's toolset "Forked from CmdUtl v5 by Anaminus": Move Axis (world), Move Object (each object's
  axes), Move First (relative to the first selected), Rotate Pivot / Group / Object, Resize Object and Resize Center
  (each part from its own centre), Scale (duplicate at a scale), Convert (Part to WedgePart), Select Edge (rotate or
  scale about an edge) (Sourced: https://create.roblox.com/store/asset/260276858;
  https://about.roblox.com/newsroom/2017/05/3-must-studio-plugins).
- **Why it is fast**: keys 2 and 3 halve and double the increment (Sourced:
  https://devforum.roblox.com/t/new-option-for-positioning-objects/39609). No full keymap found.
- **Praise**: fixed increments, symmetry and rotating by an edge; "Greatly improved my build quality" (Sourced:
  1052347 in R1.1); two-sided and model scaling (Sourced: 3163131 in R1.1).
- **Complaints**: it moves parts with its own code, so the 2021 Pivot API release required an update (Sourced:
  https://devforum.roblox.com/t/action-required-pivot-api-release-update-f3x-sbs-plugins/1235661).
- **Price and licence**: free, about 900 up and 100 down, updated 2025-10-14. No public repository (Unverified).

### R1.9 Tag Editor, now built into Studio

- **What**: tiffany352's CollectionService tag manager: checkboxes apply a tag to the whole selection; tags get icons,
  colours and groups; World View draws tagged objects as boxes, outlines, spheres, icons or text; Ctrl+A in a tag's
  list selects every tagged object (Sourced: https://github.com/tiffany352/Roblox-Tag-Editor;
  https://devforum.roblox.com/t/tag-editor-plugin/101133).
- **Built in**: Roblox shipped an official Tag Editor from her code on 2022-11-15 (Sourced:
  https://devforum.roblox.com/t/tag-editor-plugin-for-studio/2055202). Since 2023-08-04 Properties has a Tags
  section: "+" opens existing tags sorted by frequency with type-to-filter; arrows and Enter add; tags apply to every
  selected instance (Sourced:
  https://devforum.roblox.com/t/add-and-remove-tags-from-the-properties-panel-beta/2506442).
- **Praise**: "THE plugin to use for CollectionService" (Sourced: 101133). **Complaints**: users want icons and groups
  in the Properties version; Rojo users lost tags on reconnect (Sourced: 2506442; 101133).
- **Price and licence**: free; **MPL-2.0**, archived (Sourced: GitHub above).

### R1.10 Moon Animator 2 (building-relevant parts only)

- **What**: a paid animation suite by xSIXx. Building-relevant: Character Inserter (R6/R15 rigs to pose) and **Easy
  Weld** ("used to clean up complex models, weld them and rig them") (Sourced:
  https://devforum.roblox.com/t/getting-started-with-moon-animator-2-unofficial/476330).
- **Why it is fast**: Easy Weld takes a Base Part (for example LowerTorso) and Target Parts; users mention "join in
  place" and an "Animatable" option (Sourced:
  https://devforum.roblox.com/t/issue-with-custom-rig-animations/2815362). Exact joint type: (Unverified).
- **Praise**: "good for rigging", "200% worth it" (Sourced: https://devforum.roblox.com/t/is-moon-animator-worth-it/2477738).
  **Complaints**: price; a rework made it "confusing and irritating"; cracked and backdoored copies (Sourced: 2477738;
  https://devforum.roblox.com/t/does-my-animation-have-a-backdoor/2871894).
- **Price and licence**: USD 19.99, listed "On sale for 33% off" on the day; about 27,900 up and 2,100 down (Sourced:
  https://create.roblox.com/store/asset/4725618216). Closed source (Unverified).

### R1.11 Stravant Redupe (2025)

- **What**: "similar to an array modifier": drag handles around the selection to add copies; offset or resize copies
  evenly (stairs), rotate between copies, combine for spirals (Sourced:
  https://create.roblox.com/store/asset/73064993918325;
  https://devforum.roblox.com/t/redupe-v10-studio-plugin-create-arrays-of-duplicates-by-dragging/4194569).
- **Why it is fast**: spacing 1.0 puts copies back to back; "Extra Padding" may be negative; typing "/7" in the
  rotation field makes a circle of 7; pivot Outside, Middle or Inside; "Multiply Snap By Count" keeps copies on the
  grid; "Automatic ResizeAlign" closes gaps with wedges; Stamp & Repeat; group as None, Model or Folder (Sourced:
  https://github.com/stravant/roblox-redupe/blob/HEAD/src/Settings.lua).
- **Praise**: "Unbelievably intuitive and powerful"; "might be better than Archimedes" (Sourced: 4194569).
- **Complaints**: fails on MeshParts; too much banking on sloped curves; errors above 300 copies (Sourced: 4194569).
- **Price and licence**: free, about 693 up and 7 down; **MIT**.

### R1.12 Part to Terrain (mkargus)

- **What**: click a part to turn it into smooth terrain of the chosen material (Block, Ball, Cylinder, Wedge; not
  meshes or CSG) (Sourced: https://github.com/mkargus/PartToTerrain).
- **Fast**: one click per part; delete part after converting on by default; searchable material grid.
- **Price and licence**: free, about 4,700 up and 300 down; a USD 4.99 "DEV" build; **GPL-3.0** (Sourced:
  https://create.roblox.com/store/asset/261634767). Relevant to #175 (terrain).

### R1.13 qCmdUtl (Quenty)

- **What**: a fork of CmdUtl 5.0 adding "shortcuts, UNDO, Scaling" (Sourced:
  https://create.roblox.com/store/asset/142314093).
- **Fast**: one key per mode: H move world, G move relative to the first selected, J move on the object's own axes; Z,
  X, C rotate world / about the first selected / each object; R resize on the object's axes, T resize from centre; V
  rotate about an edge picked with Alt+click; Shift+C clone in place. Defaults: move 1 stud, resize 1 stud, rotate 5
  degrees (Sourced: https://github.com/Quenty/qCmdUtl/blob/HEAD/src/init.server.lua).
- **Complaints**: the author warns undo can "glitch". **Price and licence**: free; **MIT**.

### R1.14 Rig Editor (goro7)

- **What**: builds custom Humanoid or AnimationController rigs: create, edit, delete and scale joints with a live
  skeleton preview; multi-joint transforms; custom increments (Sourced:
  https://devforum.roblox.com/t/modern-rig-editor-plugin/260728).
- **Price and licence**: free, about 3,440 up and 560 down; no licence stated (Unverified).

### R1.15 Studio Tweaks (PeZsmistic)

- **What**: adds dragger gizmos: shows the selection's origin and front, labels faces (cylinders for +, cones for -),
  shows welds and Motor6Ds, thinner selection box (Sourced:
  https://devforum.roblox.com/t/extra-dragger-gizmos-disable-selection-box-show-selection-faces-show-selection-origin/735827).
- **Lesson** (Judgment): people want to see which way a part faces and what is welded, without opening Properties.
- **Price and licence**: free; source not released.

### R1.16 Searched for, not found or not notable

"Stravant MoveIt", "Clonetool" and "Polybattle" returned no Creator Store results (Unverified that they exist).
RoBuilder exists with 0 votes (Sourced: https://create.roblox.com/store/asset/8484047558). Bezier/curve plugins exist
with very few votes (Sourced: https://create.roblox.com/store/asset/7871994001). "Easy Weld" as a standalone plugin
has few votes; the popular one is inside Moon Animator (R1.10).

### R1.17 Patterns across the plugins

- **Pick by clicking geometry, with a hover highlight and a status line saying what to click next**: ResizeAlign
  faces, GapFill edges, Model Reflect planes, Archimedes faces, qCmdUtl edges (R1.3 to R1.6, R1.13).
- **Reach-around picking** saves camera moves (ResizeAlign, R1.4).
- **Left-hand single letters for tools, Shift for actions** (F3X, qCmdUtl).
- **An editable increment with keyboard nudges and halve/double keys** (F3X numpad, SBS 2 and 3).
- **Explicit reference frame**: world, each object, first or last selected (F3X, SBS, qCmdUtl).
- **Hold a modifier to borrow the normal dragger** without leaving the tool (ResizeAlign, GapFill: Ctrl).
- **Automatic seam closing**: wedge joins, gap fill, auto ResizeAlign. Gaps and Z-fighting at angled joints are the
  main pain these plugins solve (R1.4, R1.5, R1.11).
- **Preview, then commit**: Redupe ghosts, Archimedes' Render button.
- **Studio absorbs plugins, and Studio changes break plugins**: Tag Editor moved into Studio; the Pivot API forced F3X
  and SBS updates (R1.8, R1.9).
- **Trust**: fake or backdoored copies of Brushtool, Archimedes and Moon Animator exist (R1.3, R1.7, R1.10).
  Judgment: built-in tools remove this risk for Metallic creators.

---

## R2. Roblox Studio's own tools

### R2.1 Move, Scale, Rotate, Transform

- **Keys**: Move 2, Scale 3, Rotate 4 (Sourced: https://create.roblox.com/docs/parts). The Dragger QoL update
  (enabled for everyone in September 2024) dropped the old Ctrl+1 to Ctrl+4: "Selecting the Draggers uses just
  1/2/3/4 instead of Ctrl+1/2/3/4. You can change the shortcuts back in File -> Advanced -> Customize shortcuts"
  (Sourced: https://devforum.roblox.com/t/3161661). 1 is Select (Sourced: https://devforum.roblox.com/t/2661668).
- **Handles**: Move arrows; after release, a numeric distance box stays on screen and can be edited to fine-tune the
  move. Scale shows the current size on the dragged axis. Rotate rings (Sourced: https://create.roblox.com/docs/parts).
  Handles sit at the pivot since 2021; holding **Tab summons the handles to the cursor** (Sourced:
  https://devforum.roblox.com/t/1403027).
- **Modifiers while dragging**: Shift temporarily toggles snapping for every tool. **Alt** is uniform scale (moved from
  Shift in 2024; "There is no setting for this") (Sourced: https://devforum.roblox.com/t/3161661). Ctrl scales about the
  pivot (Sourced: https://devforum.roblox.com/t/1754220). Alt during a cursor drag keeps the starting orientation
  (Sourced: https://devforum.roblox.com/t/2661668).
- **Rotate during a cursor drag**: T tilts toward the camera, R rotates about the hovered surface's normal, 90 degrees
  each, animated (duration setting, 0 turns it off). **Ctrl+T / Ctrl+R** rotate the selection 90 degrees about X or Y
  without a drag (Sourced: https://create.roblox.com/docs/parts; https://devforum.roblox.com/t/3161661).
- **Ctrl+D during a drag duplicates without ending the drag** ("stamping"). H toggles a hotkey help panel during a drag.
  Numpad keys bump the selection one increment in local space (Sourced: https://devforum.roblox.com/t/2661668).
- **Local/World**: Ctrl+L, with an **L** indicator in local mode (Sourced: https://create.roblox.com/docs/parts). API:
  `DraggerService.DraggerCoordinateSpace` (Object default, or World) (Sourced:
  https://create.roblox.com/docs/reference/engine/enums/DraggerCoordinateSpace).
- **Next-Gen Draggers** (beta, 25 June 2026): planar handles, a white centre handle that moves in view space, only
  positive axes by default, snap tick marks on the active axis; rotate draws only the front half of each ring, a
  trackball inside the rings, an outer view-space ring. Settings restore the classic look (Sourced:
  https://devforum.roblox.com/t/4703155).
- **Transform tool**: exists only as `Enum.RibbonTool.Transform` ("combined move, scale, and rotate handles in a single
  gizmo"); current docs do not describe it (Sourced:
  https://create.roblox.com/docs/reference/engine/enums/RibbonTool). Behaviour and key: (Unverified).
- **Geometric vs Physical mode** (2020): Physical drags respect constraints and collisions
  (`DraggerService.DraggerMovementMode`) (Sourced: https://devforum.roblox.com/t/818166;
  https://create.roblox.com/docs/reference/engine/enums/DraggerMovementMode).

### R2.2 Snapping

- Move and Scale snap in studs, Rotate in degrees, set in the toolbar; Shift+2 focuses the move/scale increment, Alt+R
  the rotate increment (Sourced: https://create.roblox.com/docs/parts). Default values are not stated in any source
  read (Unverified; F3X's defaults are 1 stud and 15 degrees, R1.1).
- Toggles: **Shift+X** grid (Move) snapping, **Shift+C** snap to parts (Sourced: https://devforum.roblox.com/t/3161661).
- **Soft snapping**: with grid snapping off, tools still soft-snap to vertices and edge centres; the margin scales with
  zoom ("Soft Snap Margin Multiplier", 0 turns it off). Snapping uses collision geometry for meshes. A **hover ruler**
  shows the grab point and where it will snap on the target (Sourced: https://devforum.roblox.com/t/2661668).
- **Cursor drag placement** (2020 draggers): the dragged bounding box sits on the hovered surface, including walls and
  ceilings; with grid snap, the selection's vertex nearest the click snaps to the nearest target vertex; over empty
  space a "virtual surface" extends the last surface (Sourced: https://devforum.roblox.com/t/683129).
- `DraggerService` properties (exact names): `AlignDraggedObjects`, `AngleSnapEnabled`, `AngleSnapIncrement`,
  `CollisionsEnabled`, `DraggerCoordinateSpace`, `DraggerMovementMode`, `GeometrySnapColor`, `JointsEnabled`,
  `LinearSnapEnabled`, `LinearSnapIncrement`, `PartSnapEnabled`, `PivotSnapToGeometry`, `ShowHover`,
  `ShowPivotIndicator`, `UseBoundingBoxes` and others (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/DraggerService).

### R2.3 Pivots

- Model tab **Pivot** tool: move or rotate the pivot; transforms then happen about it. A **Snap** option snaps the
  pivot to "hotspots" (corners, edges, centres) drawn as small points. **Reset** returns it to the bounding-box centre
  (Sourced: https://create.roblox.com/docs/studio/pivot-tools). Alt snaps to parts inside a Model (Sourced:
  https://devforum.roblox.com/t/1754220). Clicking the pivot drags the object by it as a point; with Ctrl+D that
  stamps copies (Sourced: https://devforum.roblox.com/t/2661668).
- PrimaryPart rules: setting it makes the model's pivot the PrimaryPart's; clearing resets to the bounding-box centre
  (Sourced: pivot-tools page above). A multi-selection pivots about its bounds' centre (Sourced:
  https://devforum.roblox.com/t/1403027).
- API: `PVInstance:GetPivot(): CFrame`, `PVInstance:PivotTo(targetCFrame)` (caches offsets so repeated moves do not
  drift), `BasePart.PivotOffset` with `GetPivot() = CFrame * PivotOffset`, `Model.WorldPivot` (ignored when PrimaryPart
  is set; the bounding-box centre until first set) (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/PVInstance;
  https://create.roblox.com/docs/reference/engine/classes/Model).
- Praise: pivots and handle summoning are "probably the most useful additions in recent times" (Sourced: 1403027).

### R2.4 Align tool

Model tab or Window > 3D. Mode Min / Center / Max; Align In World or Local with X/Y/Z checkboxes (several at once);
Relative To **Selection Bounds** or **Active Object** (the last selected, outlined, does not move; clicking another
selected object changes it); the result is previewed before you confirm (Sourced:
https://create.roblox.com/docs/studio/align-tool). No dedicated shortcut (Unverified).

### R2.5 Solid modeling

- Union Ctrl+Shift+G, Intersect Ctrl+Shift+I, Separate Ctrl+Shift+U, Negate Ctrl+Shift+N (Sourced:
  https://create.roblox.com/docs/parts/solid-modeling). Negation is a tag: the docs say `rbxNegate`, a February 2026
  beta post says `rbxNegated` (Sourced: https://devforum.roblox.com/t/4357808). The two conflict.
- `GeometryService:UnionAsync / IntersectAsync / SubtractAsync` return arrays, cap results at 20,000 triangles, copy
  colour, material and physics from the main part (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/GeometryService). The CSG performance beta (November 2024)
  was fully released 14 July 2025 (Sourced: https://devforum.roblox.com/t/3256495).
- `UsePartColor`; `SmoothingAngle` 30 to 70 degrees recommended; `CollisionFidelity` Default, Hull, Box,
  PreciseConvexDecomposition, Tunable (Sourced: solid-modeling page;
  https://create.roblox.com/docs/reference/engine/enums/CollisionFidelity).
- Historic complaints: "invalid operation" errors, bad shading, slow unions (Sourced: 3256495).

### R2.6 Materials, Material Manager, MaterialVariant

- Material widget on Home and Model; new parts default to Plastic. Material Manager (Window > 3D > Material) has
  "Apply to Selected Parts" and a paint mode (Sourced: https://create.roblox.com/docs/parts/materials).
- **Material Picker** (full release July 2024): visual previews, **hover preview on the selection**, type a name prefix
  and press Enter, paint mode (Sourced: https://devforum.roblox.com/t/3073844).
- `MaterialVariant` under `MaterialService` only; base material, Color/Normal/Metalness/Roughness maps,
  `StudsPerTile`, `MaterialPattern` Regular or Organic (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/MaterialVariant). `BasePart.MaterialVariant` stores the
  variant's **name**, so swapping same-named variants restyles a place and renaming breaks parts (Sourced:
  materials page). "Set as Override" replaces a base material place-wide;
  `MaterialService:GetBaseMaterialOverride / SetBaseMaterialOverride` (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/MaterialService).
- Material Generator (AI): text prompt, pick a tile, Studs Per Tile, Organic toggle (Sourced:
  https://create.roblox.com/docs/studio/material-generator).

### R2.7 Colour picker

- The Color button applies the current colour; its arrow opens a hexagonal picker with a "Click object to apply
  color" paint mode (Sourced: https://create.roblox.com/docs/parts). That the hexagon is the BrickColor palette:
  (Unverified).
- **The Properties swatch opens the operating system's colour dialog**; you can also type an RGB value (Sourced:
  https://create.roblox.com/docs/parts). Users asked for hex in Properties in 2021 (Sourced:
  https://devforum.roblox.com/t/1085982).
- No HSV square, hex field or eyedropper in Studio's own picker was found, and no 2024 to 2026 redesign (Unverified as
  absence).

### R2.8 Collisions, Join Surfaces, Anchor, Lock

`DraggerService.CollisionsEnabled` ("moved object stops against other parts"); `JointsEnabled` joins parts resting
against surfaces at the end of a drag (Sourced: DraggerService page). Anchor Alt+A, Lock Alt+L, each with a "click
object to..." mode (Sourced: https://create.roblox.com/docs/parts). Collision toggle key: (Unverified).

### R2.9 Ribbon and the Model tab

Tabs: Home, Model, Avatar, UI, Script, Plugins. Home: transform tools, Part, Color, Material, Group, Lock, Anchor,
Terrain. Model: transform tools, Pivot, Align, Effects and Constraints inserts, solid modeling (Sourced:
https://create.roblox.com/docs/studio/ui-overview). Group Ctrl+G, group as Folder Alt+Ctrl+G, ungroup Ctrl+U (Sourced:
https://create.roblox.com/docs/parts). The "Next Gen" ribbon became the only UI on 5 January 2026; complaints: "too
spaced out", tools moved into menus, no opt-out (Sourced: https://devforum.roblox.com/t/4085392;
https://devforum.roblox.com/t/3889612).

### R2.10 Selection

Hover outlines; Shift-, Ctrl- or Cmd-click adds; **Alt-click cycles to the object behind** (Sourced:
https://create.roblox.com/docs/studio/ui-overview). Explorer: Select Children / Parent, Ctrl+A, Shift-click ranges,
drag to box-select, and **property queries in Explorer search** (`ClassName = Decal`, `Material == plas`) then Ctrl+A
(Ctrl+Shift+X focuses search) (Sourced: https://create.roblox.com/docs/studio/explorer). Ctrl+D duplicates in place;
Ctrl+Shift+V pastes into; F focuses (Sourced: explorer and ui-overview pages). Viewport box select: (Unverified).

### R2.11 Luau API names

- `BasePart:Resize(normalId: NormalId, deltaAmount: int): boolean`: "just like using the Studio resize tool", returns
  whether it happened; obstruction rules not documented (Unverified) (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/BasePart).
- `Model:ScaleTo(newScaleFactor)`, `Model:GetScale()`: persistent factor starting at 1, scales about the pivot
  including joints, particles and sound; "To" not "By" to avoid drift (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/Model; https://devforum.roblox.com/t/2205722).
- `Model:GetBoundingBox(): (CFrame, Vector3)`; `Workspace:GetPartBoundsInBox(cframe, size, overlapParams?)` (bounding
  boxes) (Sourced: Model and Workspace pages).
- `Selection:Get / Set / Add / Remove`, `SelectionChanged` (plugin security) (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/Selection).
- `ChangeHistoryService:TryBeginRecording(name, displayName?) -> string?`, `FinishRecording(id, operation)` with
  Commit, Cancel, Append; `SetWaypoint` "deprecated soon" (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/ChangeHistoryService).
- **Runtime gizmos**: `Handles` (`Style` Resize or Movement, `Faces`, `MouseDrag(face, distance)`), `ArcHandles`
  (`Axes`, `MouseDrag(axis, relativeAngle, deltaRadius)`) (Sourced:
  https://create.roblox.com/docs/reference/engine/classes/Handles;
  https://create.roblox.com/docs/reference/engine/classes/ArcHandles).
- **`DragDetector`** (released 11 October 2023): `DragStyle` TranslateLine, TranslatePlane, TranslatePlaneOrLine,
  TranslateLineOrPlane, TranslateViewPlane, RotateAxis, RotateTrackball, Scriptable, BestForDevice; `ResponseStyle`
  Geometric (anchors while dragging), Physical, Custom; `PermissionPolicy`; `RunLocally` (false: the server performs
  the drag); `AddConstraintFunction` (for example grid snapping); events DragStart, DragContinue, DragEnd (Sourced:
  https://create.roblox.com/docs/ui/3D-drag-detectors;
  https://create.roblox.com/docs/reference/engine/classes/DragDetector;
  https://devforum.roblox.com/t/announcing-dragdetectors/2641956).

### R2.12 What users praise and complain about

- Praise: drag by pivot with stamping ("one of the most useful things I've seen added to studio") (Sourced: 2661668);
  pivots and handle summoning (Sourced: 1403027); 2020 draggers fixed unpredictable placement and wall/ceiling
  placement (Sourced: 683129).
- Complaints: QoL rotation animation and soft snapping could not be turned off at first (later added) (Sourced:
  2661668; 3161661); users want the hover ruler off (Sourced: https://devforum.roblox.com/t/4698791); new model
  selection visuals disliked (Sourced: https://devforum.roblox.com/t/3170308); Next-Gen handles unreadable or occluded,
  and requests for per-axis model scaling (Sourced: 4703155); people still press Shift for uniform scale (Sourced:
  https://devforum.roblox.com/t/4760031).
- Judgment: two lessons. Every new motion or snap behaviour needs an off switch from day one; and moving a modifier
  (Shift to Alt) costs years of muscle memory, so Metallic should pick its modifiers once.

---

## R3. Games with building

### R3.1 Minecraft: WorldEdit

- **What**: command-driven bulk editing. Wand (wooden axe, `//wand`): left-click pos1, right-click pos2; `//sel`
  cuboid, poly, ellipsoid, sphere, cyl, convex; `//expand 10 n,w` with directions relative to where you look (Sourced:
  https://worldedit.enginehub.org/en/latest/usage/regions/selections/).
- **Fast**: `//set <pattern>`, `//replace [mask] <pattern>`, `//stack <count> [direction]` (Sourced:
  https://worldedit.enginehub.org/en/latest/usage/regions/regionops/); weighted patterns `50%red_wool,10%glass`
  (Sourced: https://worldedit.enginehub.org/en/latest/usage/general/patterns/); `//copy` stores your position relative
  to the selection and `//paste` places relative to where you stand, `-a` skips air, `-n` previews (Sourced:
  https://worldedit.enginehub.org/en/latest/usage/clipboard/); brushes bound to the held item with their own mask and
  size (Sourced: https://worldedit.enginehub.org/en/latest/usage/tools/brushes/).
- **Praise / complaints**: wins on "pure command power"; the commands are the barrier (Sourced:
  https://champbop.com/minecraft/best-minecraft-mod-for-building/).
- **Price and licence**: free; **GPLv3** (Sourced: https://modrinth.com/project/worldedit).

### R3.2 Minecraft: Axiom

- **What**: Builder Mode (hold Left Alt for a context menu of Capabilities and Builder Tools) and a windowed Editor Mode
  (Right Shift) (Sourced: https://modrinth.com/mod/axiom).
- **Fast** (Sourced: https://axiomdocs.moulberry.com/print.html):
  - Capabilities: Replace Mode (R), Tinker (right-click to change block properties), Angel Placement (place in mid
    air), Infinite Reach, Fast Place.
  - Builder tools chosen with Alt + scroll: Move (scroll nudges, Ctrl+F flips, Ctrl+R rotates), **Clone and Stack by
    scrolling** to add copies, Extrude (right-click grows a face), Erase connected, **Setup Symmetry** (a symmetry
    node, Ctrl+F / Ctrl+R).
  - **Magic Select** (M): one click picks all connected blocks of the same type, with match modes and "Surface Only".
    Box (B), Freehand (N), Lasso (L).
  - Stamp (random blueprint scatter with random yaw), Path (Catmull-Rom and Bezier), tool masks combined with AND, OR,
    NOT.
  - Gizmo: axis arrows, plane nodes, rings, a centre node that keeps its distance from the camera; Ctrl enlarges the
    snap step, Shift removes snapping; **hold X, Y or Z to lock the axis**.
- **Praise**: "A game changer for servers, builders and sysadmins/devs alike" (Sourced:
  https://es.trustpilot.com/review/axiom.moulberry.com). **Complaint**: client mod only (Sourced: champbop above).
- **Price and licence**: All Rights Reserved; free for single-player and listed servers; a Commercial License for paid
  builders (Sourced: https://axiomdocs.moulberry.com/other/commerciallicense.html). Price: (Unverified).

### R3.3 Fortnite Creative (in game)

- **Phone tool, PC** (Sourced:
  https://dev.epicgames.com/documentation/en-us/fortnite/hotkey-and-keybinding-shortcuts-in-fortnite-creative):
  left-click copies what you aim at, then pastes; right-click cuts; hold R and sweep to multi-select; X deletes; R and
  E rotate; Tab cycles pitch/yaw/roll; F and C push and pull; **G toggles "Drops"** (a line to where it will land);
  **V cycles grid snap 32, 16, 8, 4, 2, 1, Off**; after copying, hold right mouse and R/E grow or shrink, Tab cycles
  axes.
- **Prefabs and galleries**: prefabs are finished structures that can still be taken apart after placing; galleries are
  themed loose pieces; double-click a prefab to see its parts (Sourced:
  https://dev.epicgames.com/documentation/en-us/fortnite-creative/using-prefabs-and-galleries-in-fortnite-creative).
- Judgment: fast because you aim and click with no separate camera, copy turns into a paint-like paste, and one key
  cycles the grid.
- **Price**: free with Fortnite.

### R3.4 UEFN

Objects are moved by gizmos rather than grabbed; placement drags from the Content Browser; grid snap uses Unreal units
(Sourced:
https://dev.epicgames.com/documentation/en-us/fortnite/guide-to-uefn-controls-for-creative-users-in-unreal-editor-for-fortnite).
Modeling Mode tool groups Create, Shape, PolyEdit, TriModel, MeshOps, Transform, Deform, VoxOps; Epic notes weight maps,
PolyGroups and vertex colours can be lost on import and export (Sourced:
https://dev.epicgames.com/documentation/en-us/fortnite/modeling-mode-in-unreal-editor-for-fortnite). Price: (Unverified
that it is free).

### R3.5 Dreams (PS4)

- **What**: sculpt by stamping primitive shapes with a motion-controlled "imp" cursor; Blend smooths stamps together;
  subtract cuts (Sourced: https://www.gameinformer.com/2019/04/26/the-10-coolest-tools-in-dreams-any-dummy-can-use).
  Live support ended September 2023 (Sourced:
  https://www.videogameschronicle.com/news/media-molecule-is-ending-support-for-dreams-in-september/).
- **Fast**: **Add Clones** inserts evenly spaced copies between objects; **Live Clone** keeps copies in sync;
  Kaleidoscope makes radial copies (Sourced: gameinformer above;
  https://kotaku.com/how-to-get-started-making-things-in-dreams-1841839340). Guides: a grid follows what you grab with a
  plumb line; **an off-grid object turns the grid orange and one button aligns it**; Precise Move; Match Face snaps one
  face flush to another; Stay Upright and Right Angles toggles (Sourced:
  https://docs.indreams.me/en/create/resources/edit-mode-guide/coat/guides).
- **Praise**: "tools that feel intuitive to use" (Sourced: https://www.pushsquare.com/reviews/ps4/dreams).
  **Complaints**: "UI and creation tools can be overwhelming"; "the standard controls are awful" (Sourced:
  https://www.androidcentral.com/dreams-playstation-4-review).

### R3.6 Garry's Mod

- Physgun: hold left mouse to grab, wheel pushes and pulls, E rotates, Shift+E rotates in steps (`gm_snapangles`,
  default 45), right-click freezes (Sourced: https://wiki.facepunch.com/gmod/Using_your_Physgun).
- Addons: **Advanced Duplicator 2** saves contraptions as shareable files (Apache-2.0) (Sourced:
  https://github.com/wiremod/advdupe2); **Precision Tool** "rotate, snap, weld, and align all props" (Sourced:
  https://gamersdecide.com/articles/garrys-mod-best-building-mods); **SmartSnap** snaps to a grid drawn on the target
  prop; a user: "the official gmod snap to grid is horrible" (Sourced:
  https://steamcommunity.com/sharedfiles/filedetails/comments/104815552). Easy Weld: (Unverified).

### R3.7 Rec Room Maker Pen

Pick a shape in the Palette, pull the trigger and move the pen to size it; the free hand grabs; "Done Editing" welds
shapes into one object (Sourced: https://rec-room.fandom.com/wiki/Maker_Pen). The palette shows an **ink (complexity)
meter** that stops building at 100% (Sourced: https://www.gamepur.com/guides/how-to-get-more-ink-in-rec-room). Smart
Ruler and Smart Protractor draw measured guides; the 2025 Maker Pen UI always surface-snaps (Sourced: recroom.com ship
notes via search; the pages now show "under construction", Unverified).

### R3.8 Welcome to Bloxburg (Roblox game)

Build Mode (Sourced: https://welcome-to-bloxburg.fandom.com/wiki/Build_Mode): Q/E rotate the camera, WASD pan, hold
Space for top-down; R rotates the item; G sell, F paint, C clone, V scale; **T cycles placement modes for walls,
floors and fences (Default, Box, Chain, Auto)**; **H toggles the grid, J cycles grid size**, K toggles collision;
**Shift+click with Paint samples a colour**; Ctrl+Z / Ctrl+Y; every change autosaves and nearby players see edits live.
The Advanced Placing pass (200 Robux) turns off collision checks (Sourced:
https://welcome-to-bloxburg.fandom.com/wiki/Advanced_Placing). Selling refunds 100% when removed by undo (Sourced:
Build Mode page).

### R3.9 Build a Boat for Treasure (Roblox game)

Building Tool: translucent preview, R/T rotate, Move grid 1, 0.5, 0.2, 0 or 2 studs (Sourced:
https://build-a-boat-for-treasure.fandom.com/wiki/Building_Tool). Scaling Tool (5,000 Gold): orange orb handles,
custom value 0 to 1000 (Sourced: https://build-a-boat-for-treasure.fandom.com/wiki/Scaling_Tool). **Paint Tool**
(1,500 Gold): 140-colour palette, "More" opens a colour wheel with HSV/RGB entry and an eyedropper, click-drag paints
many blocks (Sourced: https://build-a-boat-for-treasure.fandom.com/wiki/Paint_Tool). Trowel (7,500 Gold): move,
rotate, clone and mirror; rotate steps 90/45/15/5/0 with 15 default (Sourced:
https://build-a-boat-for-treasure.fandom.com/wiki/Trowel_Tool). Binding Tool links inputs (purple) to outputs (yellow)
(Sourced: https://build-a-boat-for-treasure.fandom.com/wiki/Binding_Tool). Judgment: a game sells building tools as
progression, so a runtime kit must let creators gate tools.

### R3.10 Sims 4, Valheim, Teardown, Townscaper (brief)

- Sims 4: Alt for off-grid placement; console cycles full, half, quarter tile and free (Sourced:
  https://ggrecon.com/guides/sims-4-free-build-cheats/). Eyedropper E and Design Tool R from a fan guide (Unverified).
- **Valheim**: hovering a piece colours it from blue (grounded) through green to red by structural support (Sourced:
  https://valheim.fandom.com/wiki/Building; https://valheim.fandom.com/wiki/Building_stability).
- Teardown: MagicaVoxel assets; editor shapes Voxbox, Voxagon, Voxscript; groups become prefabs (Sourced:
  https://teardown.fandom.com/wiki/Level_Editor).
- Townscaper: click blocks onto an irregular grid; the game turns them into houses and bridges with wave function
  collapse (Sourced: https://gamedeveloper.com/blogs/how-townscaper-works-a-story-four-games-in-the-making).

---

## R4. Engines and creative tools

### R4.1 Unity: Scene view transform and snapping

(Sourced:
https://docs.unity.com/en-us/engine/current/manual/working-with-scenes/scenes-manage-gameobjects/positioning-game-objects)
- Tools: W Move, E Rotate, R Scale, T Rect, Y Transform.
- **Vertex snapping: hold V** and drag from a vertex to a vertex; Shift+V toggles it on; with Shift+Ctrl the vertex
  snaps to a surface.
- **Surface snapping: hold Shift+Ctrl** and drag the move handle onto a collider; with Rotate, Shift+Ctrl rotates to
  look at a surface.
- Shift on the move gizmo centre: screen-space plane. Handle orientation Local, Global or Grid.
- **Increment snapping: hold Ctrl** while using a gizmo; sizes in the Grid and Snap overlay (Sourced:
  https://docs.unity.com/en-us/engine/current/manual/working-with-scenes/scenes-manage-gameobjects/grid-snapping/snap-increments).
- Grid: **Ctrl+[ and Ctrl+]** shrink and grow the grid; Ctrl+; moves the grid to the handle (Sourced:
  https://docs.unity.com/en-us/engine/current/manual/working-with-scenes/scenes-manage-gameobjects/grid-snapping/grid-shortcuts).
- Unity's "Z" pivot/centre and "X" local/global toggles: (Unverified on the pages read; widely used defaults).
- Colour field: see R5.5.
- Licence: Unity Editor itself is not a dependency; nothing copied.

### R4.2 Unity ProBuilder

- **What**: build, edit and texture geometry inside Unity (Sourced:
  https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/index.html).
- **Fast**: G toggles Object and Element mode, H cycles Vertex/Edge/Face, Alt+L loop, Alt+R ring; **hold Shift while
  moving, rotating or scaling to extrude** with no dialog (Sourced:
  https://docs.unity3d.com/Packages/com.unity.probuilder@5.0/manual/hotkeys.html;
  https://docs.unity3d.com/Packages/com.unity.probuilder@5.1/manual/Face_Extrude.html).
- **Price and licence**: free since Unity bought ProCore in February 2018 (Sourced:
  https://www.cgchannel.com/2018/02/unity-now-comes-with-probuilder-built-in-for-free/). Unity Companion License: use
  only with Unity, "may not be used... to develop a competing product" (Sourced:
  https://docs.unity3d.com/ja/Packages/com.unity.probuilder@5.0/license/LICENSE.html). Relevant to #136.

### R4.3 Unity Polybrush

Modes Sculpt, Smooth, Color (vertex colour), Texture blend, **Prefab scatter over a mesh**; brush Radius, Falloff,
Strength and a Falloff Curve, saved as Brush Presets (Sourced:
https://docs.unity3d.com/Packages/com.unity.polybrush@1.1/manual/index.html). Same licence as ProBuilder.

### R4.4 Unreal Engine: placement

- **End drops the selected actor onto the surface below.** Surface snapping toggle with "Rotate to Surface Normal" and
  "Surface Offset". **Hold V** for vertex snapping. Grid, rotation and scale snap toggles with increment dropdowns, and
  a power-of-two grid option (Sourced:
  https://dev.epicgames.com/documentation/en-us/unreal-engine/actor-snapping-in-unreal-engine).
- **Alt+drag on the move gizmo duplicates** and leaves the original (Sourced:
  https://docs.unrealengine.com/4.26/BuildingWorlds/LevelEditor/Viewports/ViewportControls).
- Licence: free until USD 1M lifetime gross, then 5% (Sourced: https://www.unrealengine.com/en-US/license).

### R4.5 Unreal Modeling Mode and CubeGrid

PolyGroup Edit (extrude, bevel, edge loop), Boolean, Push Pull (Sourced:
https://dev.epicgames.com/documentation/en-us/unreal-engine/modeling-tools-in-unreal-engine). **CubeGrid** blocks out
from the keyboard: E pulls and Q pushes the selected cell, Ctrl+E / Ctrl+Q resize the grid, Shift+E / Shift+Q slide
the selection one cell, Z corner mode (Sourced:
https://dev.epicgames.com/documentation/en-us/unreal-engine/cubegrid-tool-in-unreal-engine). Complaint: blockouts make
many static meshes (Sourced: https://worldofleveldesign.com/categories/ue5/blockouts-in-ue5.php). Relevant to #136.

### R4.6 Valve Hammer (Source 2)

Mesh-based: "Brushes are no longer used" (Sourced: https://developer.valvesoftware.com/wiki/Hammer_5, via search
excerpt). Shift+B Block tool; selection modes on 1 to 6 (vertices to groups), Space cycles them (Sourced: translated
wiki pages via search, Unverified in English). Source 1 Hammer uses `[` and `]` for grid size (Sourced:
https://www.worldofleveldesign.com/categories/sourcesdk-authoringtools/hammer-source--interface-crash-course.php); the
same keys in Source 2 and its clipping tool: (Unverified).

### R4.7 TrenchBroom (Quake)

(Sourced: https://trenchbroom.github.io/manual/latest/)
- Left-drag on a surface draws a brush; height defaults to the grid size; Shift forces square.
- **Drags move on the horizontal plane; hold Alt to move vertically, even mid-drag; Shift locks the dominant axis.**
- **Shift+hover highlights a face (even behind the brush), drag to extrude**; Ctrl at the start splits a new brush.
- **Ctrl+drag duplicates and moves**; Ctrl+D duplicates in place with a white flash; Ctrl+Shift+D makes a linked
  duplicate.
- Grid 0.125 to 256; keys 1 to 9 set 1 to 256; + and - step. **Esc cancels a drag and undoes it at once.**
- Licence: **GPLv3** (Sourced: https://github.com/TrenchBroom/TrenchBroom.github.io).

### R4.8 Core (Manticore)

W/E/R move, rotate, scale; G toggles snapping; F focuses; default snap 50 units; Core Content, **Community Content**
(other creators' templates) and Project Content browsers (Sourced:
https://development.docs.coregames.com/getting_started/editor_intro/; https://learn.coregames.com/?p=3645). Status:
still listed as Early Access with no recorded shutdown (Sourced: https://en.wikipedia.org/wiki/Core_(video_game));
"discontinued" is (Unverified).

### R4.9 Blender

(Sourced: Blender manual source, https://projects.blender.org/blender/blender-manual/raw/branch/main/manual/ ...)
- **Numeric input during a transform**: type a number; X, Y or Z locks the axis at any time (G X 5); Tab moves to the
  next axis; `=` enables expressions and units (`.../transform/control/numeric_input.rst`; `.../axis_locking.rst`).
- **Snapping**: Shift+Tab toggles, Ctrl during a transform snaps temporarily; targets Increment, Grid, Vertex, Edge,
  Face, Edge Center, Edge Perpendicular, Face Center; **Snap Base** Closest, Center, Median, Active
  (`.../editors/3dview/controls/snapping.rst`).
- **Select Similar Shift+G**; Shift+D duplicates and starts a move; **Ctrl+M then X/Y/Z mirrors**; Array with Line,
  Circle, Curve shapes and Count or Distance; **Measure tool** (Ctrl snaps, click the midpoint for a protractor);
  3D cursor as a pivot and spawn point.

### R4.10 SketchUp

- **Measurements box**: after a move or draw, type a value and Enter; unit suffixes (`10'`, `10cm`); **`3x` makes 3
  copies at the move distance, `3/` divides evenly between the original and the copy** (Sourced:
  https://help.sketchup.com/en/using-measurements-box).
- **Inference**: endpoint, midpoint ("light blue dot" with a "Midpoint" tip), on face, on edge, centre, parallel,
  perpendicular; **Shift locks the inferred direction; arrow keys lock axes (Up blue, Left green, Right red)**
  (Sourced: https://help.sketchup.com/en/article/3000083).
- Move-copy: tap Ctrl to toggle copy, then type `7x` (Sourced via search: https://help.sketchup.com/en/article/3000092).

### R4.11 Figma and Illustrator spacing aids

- Figma: select a layer, **hold Alt and hover another to see distances**; not saved (Sourced:
  https://help.figma.com/hc/en-us/articles/360039956974-Measure-distances-between-layers). Smart selection edits
  spacing with handles; Tidy up uses the most common gap (Sourced:
  https://help.figma.com/hc/en-us/articles/360040450233-Arrange-layers-with-Smart-selection).
- Illustrator **Transform Again (Ctrl+D)** repeats the last transform (Sourced via search:
  https://helpx.adobe.com/illustrator/using/default-keyboard-shortcuts.html). Figma's Ctrl+D repeating an offset:
  (Unverified).

---

## R5. Colour pickers

### R5.1 Blender

Picker types (Preferences): Circle HSV, Circle HSL, Square (SV + H), Square (HS + V), Square (HV + S); a Hex field
accepting shorthand (`FC0`); Ctrl-drag snaps hue to 30-degree steps, Shift-drag is fine adjustment, Backspace resets;
**E** opens the eyedropper over a button, and dragging averages the colours passed over (Sourced:
https://projects.blender.org/blender/blender-manual/raw/branch/main/manual/interface/controls/templates/color_picker.rst;
https://projects.blender.org/blender/blender-manual/raw/branch/main/manual/interface/controls/buttons/eyedropper.rst).

### R5.2 Krita

Advanced Color Selector with HSV, HSL, HSI and HSY' models; Triangle and Wheel shapes confirmed; Shift+I pops up the
selector; H history patches, U common patches (Sourced:
https://docs.krita.org/en/reference_manual/dockers/advanced_color_selector.html). Hold Ctrl with a brush to sample
(Sourced: https://docs.krita.org/en/reference_manual/tools/color_sampler.html).

### R5.3 Photoshop

Eyedropper **I**; holding Alt with a paint tool switches to the eyedropper (Sourced via search: Adobe helpx eyedropper
page and shortcut PDF). Dialog with HSB, RGB, Lab, CMYK and Hex (secondary sources:
https://kb.uwec.edu/articles/adobe-photoshop-cs3-working-with-colors).

### R5.4 Figma

Notation dropdown RGB, HEX, CSS, HSL, HSB; Hex by default; "Current file" colours, styles and a WCAG contrast check
(Sourced: https://help.figma.com/hc/en-us/articles/360041003774-Apply-paints-with-the-color-picker). Eyedropper **I**;
Shift-click applies the sampled style rather than the raw value; sampling outside the app only in the macOS desktop app
(Sourced: https://help.figma.com/hc/en-us/articles/27643269375767-Sample-colors-with-the-eyedropper-tool).

### R5.5 Unity

Colour window: RGB 0-255, RGB 0-1 or HSV, a hex field, a wheel with a centre square, an eyedropper that samples
anywhere on screen; clicking the original swatch reverts; swatch libraries with Delete, Replace and drag to reorder
(Sourced:
https://docs.unity.com/en-us/engine/6000.3/manual/working-with-gameobjects/unity-components/inspector-manage-components/inspector-color-picker).

### R5.6 Godot

Modes RGB, HSV, LINEAR, OKHSL; shapes HSV rectangle, HSV wheel, VHS circle, OKHSL circle, OK HS and HL rectangles;
swatches and recent colours; a screen sampler (Sourced: https://docs.godotengine.org/en/stable/classes/class_colorpicker.html).

### R5.7 Accessible pickers

- WAI-ARIA slider pattern: arrows step, Home/End go to min/max, Page Up/Down optional large steps; `aria-valuetext`
  when the number alone is not meaningful (Sourced: https://www.w3.org/WAI/ARIA/apg/patterns/slider/).
- React Aria's 2D colour area: a group with two hidden range inputs (x and y), "2D slider" role description, per-channel
  value text; arrows step, Shift for a page step, Page Up/Down change Y, Home/End change X (Sourced:
  https://raw.githubusercontent.com/adobe/react-spectrum/main/packages/react-aria/src/color/useColorArea.ts).

### R5.8 Palette generation

Adobe Color rules: Analogous, Monochromatic, Triad, Complementary, Split Complementary, Square, Compound, Shades (Sourced:
https://color.adobe.com/create/color-wheel names five; a secondary source lists the rest:
https://www.videomaker.com/how-to/editing/color-correction/adobe-color/). Illustrator's analogous uses 15 and 30 degree
offsets, triad 120 (Sourced: https://blogs.adobe.com/tonyharmer/2015/03/23/illustrators-colour-harmonies/); Adobe
Color's exact geometry is (Unverified). **Oklab**: HSV "value" does not track perceived lightness (yellow and cyan look
lighter than blue at equal V), and blends to blue drift purple in HSV; Oklab predicts lightness, chroma and hue better
(Sourced: https://bottosson.github.io/posts/oklab/). Judgment: rotate hue in OKLCh so generated swatches look equally
bright.

---

## R6. Ranked ideas to adopt

Ranked by (time saved for a typical builder) x (how often it is used) / (cost to build), my judgment. The design
section that implements each is in brackets.

1. **The object follows the cursor onto surfaces, snapping in the surface's own grid.** The most common edit is "put
   this there" (R2.2, R4.4, R3.3) [design 5.2].
2. **One-step undo per gesture and a preview overlay** so every live preview is safe (R2.12 complaints about
   surprises) [2.1].
3. **Hold V for vertex-to-vertex snapping** with visible glyphs: the precision feature people praise most (R4.1, R4.4,
   R2.2 soft snapping) [5.3].
4. **Alt-drag duplicates, Ctrl+D during a drag stamps**: duplicate and place in one motion (R4.4, R4.7, R2.1) [4.1].
5. **Contextual toolbar next to the selection** with colour, material, style, align, mirror, array: answers F3X's
   reason to exist without covering the view (R1.1, R2.9) [3.2].
6. **Type a number during a drag** (and keep Roblox's editable distance box after release) (R4.9, R4.10, R2.1) [4.5].
7. **`[` and `]` halve and double the grid; the value is always visible** (R1.8, R4.1, R4.6, R3.3) [2.2].
8. **R and T rotate and tilt 90 degrees while dragging**, Ctrl+R and Ctrl+T without a drag (Roblox habit) (R2.1,
   R3.8, R3.9) [4.1].
9. **Own colour popover** with ring and square, Hex, recent and saved swatches; Roblox Properties opens the OS dialog
   (R2.7, R5) [11].
10. **Eyedropper that picks the property, and Ctrl-pick to copy colour, material and style then paint** (R3.8 Shift
    sample, R2.6 paint mode, R5.1 E) [11.3].
11. **Align with active-object reference and hover preview** (R2.4) [6.1].
12. **ResizeAlign-style face-to-face fill with reach-around picking** (R1.4) [6.3].
13. **GapFill between two edges, matching colour and material** (R1.5) [6.4].
14. **Smart duplicate (Ctrl+D repeats the last offset)** (R4.11 Transform Again, R4.10 `3x`) [8.1].
15. **Array by count, radial, grid and path, with ghosts before commit**; "/7" style divide syntax (R1.11, R1.3, R4.9,
    R4.10) [8.2].
16. **Mirror in place and mirror copy, shape-aware for wedges** (R1.6, R4.9 Ctrl+M) [7].
17. **Material picker with search, hover preview on the selection and thumbnails** (R2.6 picker, 2024) [12.1].
18. **Per-part style picker with live thumbnails** (owner request; no sourced precedent in Roblox) [12].
19. **Brush scatter with weights, random scale and yaw, slope limit, erase only what the brush placed** (R1.7, R4.3,
    R3.2 Stamp) [9].
20. **Select similar (Shift+G) and magic select by tag, colour or material** (R4.9, R3.2, R2.10 Explorer queries) [10.1].
21. **Measure tool and Alt-hover distances** (R4.9, R4.11, R3.7 smart ruler) [10.2].
22. **Pivot editing with hotspot snapping and Reset**, Roblox names (R2.3) [4.4].
23. **End drops the selection onto the surface below** (R4.4) [15 keymap].
24. **Off-grid warning plus one-key "snap to grid"** (R3.5 Dreams orange grid) [4.6].
25. **Runtime build kit with `Handles`, `ArcHandles`, `DragDetector` and server checks** (R2.11, R3.8, R3.9) [13.2].
26. **Build in Play, kept on Stop** (R3.8 live edits; #130) [13.1].
27. **Every new behaviour has an off switch from day one** (R2.12) [3.2, 14.1].

Not adopted now, with the reason:
- **Live linked clones and live symmetry** (Dreams, Axiom, TrenchBroom linked duplicates): need an instance-linking
  model (packages); later.
- **CubeGrid keyboard blockout and Shift-drag face extrude** (R4.5, R4.2, R4.7): mesh editing belongs to the modeller
  (#136).
- **Structural integrity colouring** (Valheim, R3.10): a gameplay system, not a Studio tool; a build kit option later.
- **Weighted patterns and masks for bulk replace** (WorldEdit, R3.1): useful for terrain (#175) and voxel games; a
  "Replace material in selection" command covers the Part case.
- **Complexity meter** (Rec Room ink, R3.7): belongs to the publish budget check (#183).

---

## R7. Could not verify

- Roblox Studio's default move and rotate increments; the current Transform tool's behaviour and key; whether the
  Color button's hexagon is the BrickColor palette; the Collisions toggle key; viewport box select (R2).
- The Negate tag spelling (`rbxNegate` vs `rbxNegated`) (R2.5).
- `BasePart:Resize` obstruction rules (R2.11).
- Licences of Archimedes, Brushtool, SBS, Rig Editor, Moon Animator; F3X export server status (R1).
- Moon Animator Easy Weld joint type (R1.10).
- Axiom commercial price; UEFN price; Core's status; Garry's Mod Easy Weld (R3).
- Hammer (Source 2) keys in English, clipping tool, tile meshes (R4.6).
- Unity Z and X toggle defaults (R4.1).
- Adobe Color's harmony angles; Figma's Ctrl+D repeating offsets; Krita shapes beyond triangle and wheel (R5).
- Rec Room ship notes (pages now "under construction") (R3.7).
