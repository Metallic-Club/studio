# Issue #144: unified Look presets spec (branch `studio/144-look-presets`)

Goal: one spec that merges render styles (#61), lighting (#139), templates (#60) and Studio UI (#127) into exact
Look presets that #62 to #66 can build. Docs only; no cargo run (disk nearly full).

## Done
- `docs/design/look-presets.md`: definitions, 19 resolved conflicts, preset tables, tier baseline and deltas, RON
  schema, versioning, validation, Studio picker and Customize, Themes mapping, Luau surface, acceptance scenes and
  numeric checks, implementation order mapped to #62-#66.
- `docs/design/looks/*.look.ron`: 14 full Look files (the 13 from #61 plus Voxel). Brackets checked with a small
  Python script; not parsed by `ron` (no build allowed).

## Next
- #62/#63/#65/#66/#64 follow section 7.2 of the spec.
- When #64 adds the loader, parse every file in `docs/design/looks/` in a unit test.
- Update `docs/templates/README.md` section 3 to say Themes and `Theme:<Role>` tags (rename from look presets).

## Open questions
- Owner decision #110 (Low internal scale, cascade count) changes only the tier baseline table.
- Thresholds in section 6.3 and Iris Xe costs are start values until real renders and a profile exist.
