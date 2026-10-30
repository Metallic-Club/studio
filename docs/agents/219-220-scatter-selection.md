# #219 / #220: scatter and selection cores

Branch `studio/219-220-scatter-selection`. Draft PR: "Scatter + selection cores (#219 #220, UI after #10)".

## Goal
Pure, egui-free cores for the brush scatter tool (#219, spec section 9) and selection modes, Select similar and
measure (#220, spec section 10), so the UI can be wired up once Task 10 (#10: viewport, `EditorState`, `History`)
and #213/#215 land. The coordinator scoped this down: no viewport, selection state or undo stack here.

## Done (`crates/metallic_studio/src/tools/`)
- `rng.rs`: SplitMix64 seeded generator; `for_stream(seed, k)` gives one stream per brush dab.
- `scatter.rs`: `BrushSet` (1-16 weighted items, tag `Brush:<name>`), `BrushSettings` (spec ranges, `clamped`),
  `SurfaceProbe` trait (the viewport's raycast), `Stroke` (paint/erase dabs, Poisson-disk dart throwing with a
  spatial hash, density x area / 100 target, dwell allowance per 0.1 s, max 64 per dab, slope and surface filters,
  random yaw/scale, partial alignment to the normal by slerp), `StrokeCommit` (one undo step per stroke, with its
  undo name). `Placement::orientation_degrees` uses the Y-X-Z order `metallic_render` reads.
- `select.rs`: `SelectMode` (Q cycle), `SelectOp` (Shift add, Ctrl subtract), `box_hits` (inside or Alt touching),
  `lasso_hits` (screen-space centre in polygon), `paint_hits` (24 px circle along the trail), `combine`, `invert`,
  `select_similar` with `SimilarKey` (C M L S K N T P), hashed so 50k parts is one pass.
- `measure.rs`: `Measurement` (distance, deltas, horizontal, angle to ground), `MeasureTool` (two clicks, 8 max,
  Esc clears), `box_corners`, `snap_to_vertex`, `Aabb` (oriented box to world box, union), `axis_gaps`,
  `gap_distance`.

## Next (after #10, #213, #215)
- Viewport: implement `SurfaceProbe` with the picker; drive `Stroke::dab` per frame; draw the brush disk.
- History: turn `StrokeCommit` into one command (create `added` under `parent` with the tag; delete `removed`).
- Selection: feed projected `ScreenItem`s; route `combine` into `EditorState`'s selection; Shift+G menu.
- Measure: use #215's snapping for clicks; draw lines and Alt-hover readouts.
- Tags and `LookOverride` are not in `metallic_core` yet; `PartTraits.tags` / `.style` are filled once they exist.

## Commands
```
cargo fmt -p metallic_studio
cargo test -j 2 -p metallic_studio
cargo clippy -j 2 -p metallic_studio --all-targets -- -D warnings
```

## Open questions
- "Same Tags" matches the exact tag set (order and duplicates ignored), not "shares a tag". Owner may overturn.
- Paint select reads "24 px circle" as the diameter (radius 12 px).
