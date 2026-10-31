# Issues #213, #214, #215: building core (branch `studio/213-215-building-core`)

Stacks on #319 (`studio/10-studio-editing`). Spec: `docs/design/building-tools.md` 2.1, 2.2, 3.1, 4.1-4.6, 5.1-5.4,
15, 17.

## Done
- #213 `preview.rs`: `PreviewOverlay` (write-through, originals remembered); `Command::SetProperties` (one step for
  any number of (instance, property) changes, rolls back on error); gizmo drags, body drags and Properties scrubbing
  all go through it; one gesture is one undo step, Esc restores.
- #214 `gizmo.rs`: arrows, plane squares, centre disc (screen plane), rings (outside the arrows for Transform), face
  cubes (Alt: both faces), uniform corner (snaps the largest size), Transform tool (T); body drag with any tool;
  X/Y/Z and Shift+X/Y/Z locks; R/T turn 90 during a body drag; Alt-drag "Duplicate and move"; Ctrl+D stamps; Space
  brings the gizmo to the cursor; typed values (`3*4` Enter) and the editable delta for 5 s after release (replaces
  the step); Ctrl+R / Ctrl+T, End drop, Ctrl+Shift+Home snap to grid with "Off grid" in the header; `[`/`]` and
  Shift+`[`/`]` increments; Pivot/Centre (Z); edit pivot (P) with 27 snap dots; Reset pivot; Model pivots follow
  moves.
- #215 `snapping.rs`: priority vertex > edge midpoint > face centre > point on edge > surface > grid; 12 px radius,
  soft snap 6 px; V forces vertex/edge for one drag, V+Shift puts the grab vertex on a surface; surface placement
  in the hit part's grid (ramps snap along the ramp); virtual surface; glyphs and grid patch; status line names the
  target ("Vertex of Wall_03"). `SnapSource` is the hook for the modeller (5.4).

## Deviations (in the PR)
- Overlay is write-through instead of a render-side overlay, so the renderer is unchanged.
- Pivots use core's `PivotOffset` (BasePart) and `WorldPivot` (Model, #313). A pivot edit (gizmo, pivot dots, Reset, Properties) is one `SetProperties` through the preview overlay; Properties' `WorldPivot` also sets `WorldPivotSet` like `transform::set_world_pivot`. Moving a Model with a set pivot moves `WorldPivot` in the same step. The session-only `SetPivots` store is gone.
- No BVH: candidates come from the part list with a screen-space cull; measured inside budget (see PR).
- No Collisions chip and no outer screen-space rotate ring yet.

## Review fixes (FIX-FIRST on cbe7d1f1)
- Pivots are written on every drag frame (back to the start values at no motion), so no stale `WorldPivot`.
- Resize uses core's 0.001 to 2048 limits; the 0.05 floor is gone (spec 4.3 updated).
- Global keys wait while a Properties scrub or viewport drag is open; Esc cancels a scrub. `EditorState` refuses
  apply, undo, redo and one-shot commits while the overlay holds a gesture.
- Losing window focus cancels a viewport drag (as Esc).
- Virtual surface hits beyond 4096 studs (grazing rays) are rejected; the drag keeps its last placement.
- A `WorldPivot` that cannot be made orthonormal is refused with a status note.
- A set `WorldPivot` moves with its Model even when a `PrimaryPart` gives the pivot (core `pivot_to`).
- Ctrl+D before an Alt-drag has moved does nothing (the selection stays on live parts).
- Spec 2.2 grid rule is now "snap the bounding-box min corner on each axis" (faces on grid lines) for surface
  placement, Snap to grid and Off grid; handle and arrow drags still snap the amount.

## Tests
- `tests/building.rs`: 500 parts x 60 frames = one step, one undo restores, Esc unchanged; 1,000-part commit time;
  Duplicate and move; Ctrl+R/T; End; snap to grid rule; pivot edit and reset; editable delta; body drag on a surface,
  virtual surface, R turn.
- `tests/editor_ui.rs`: body drag in the window; typed `3*4` during a drag.
- Unit tests in gizmo, snapping, pivot, preview, commands, transform, picking.
