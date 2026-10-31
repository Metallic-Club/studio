# #216: PivotTo, GetPivot, PivotOffset, WorldPivot, Resize, ScaleTo, GetBoundingBox

Branch `studio/216-pivot-api`, built on `studio/80-cframe` (PR #304, the CFrame part of #80).

## Goal
Roblox names and behaviour for the transform API the building tools and BuildKit use (spec
`docs/design/building-tools.md` sections 4.3, 4.4, 16).

## Done
- `metallic_core`: `Value::CFrame(Box<CFrame>)` and `ValueType::CFrame` (scene files store it); `PropDef::hidden`;
  `BasePart.PivotOffset` (last BasePart property, so older lookups scan no further); `Model.PrimaryPart`,
  `Model.WorldPivot`, hidden `WorldPivotSet` and `ScaleFactor`. `transform` module: pivots, `pivot_to`,
  `bounding_box`, `scale_to`, `resize` with an oriented-box overlap test, `NormalId`.
- `metallic_script`: `GetPivot`, `PivotTo`, `Resize`, `ScaleTo`, `GetScale`, `GetBoundingBox`, `GetExtentsSize`
  (`api/pivot.rs`), `Enum.NormalId`, `WorldPivot` read/write rules, hidden properties, `IsA("PVInstance")`.
- Tests: `crates/metallic_script/tests/pivot.rs`, unit tests in `crates/metallic_core/src/transform.rs`.

## Decisions
- `ScaleTo` is absolute, as in Roblox. The issue's "ScaleTo 2 then 0.5 returns the original sizes" does not hold under
  Roblox semantics (0.5 is half the original); the test checks `ScaleTo(2)` then `ScaleTo(GetScale() * 0.5)` and
  `ScaleTo(0.5)` separately.
- `PVInstance` is not added to the class chain: it would add a step to `find_property` for every inherited lookup.

- Review follow-ups: `ScaleTo`/`PivotTo`/`Resize` plan every write and check it before applying (atomic). Part sizes
  are capped at 2048 per axis (Luau `Size` writes clamp, `Resize` clamps, `ScaleTo` errors). Stored CFrames are
  orthonormalised (`transform::orthonormal`) and degenerate ones are rejected.

## Next
- Studio gizmos (#200 follow-ups) call `metallic_core::transform` directly.
- CFrame in remote payloads.

## Commands
```
cargo test -j 2 -p metallic_core
cargo test -j 2 -p metallic_script
cargo clippy -j 2 -p metallic_core -p metallic_script --all-targets -- -D warnings
```
