# #80 (CFrame part): CFrame datatype and BasePart.CFrame

Branch `studio/80-cframe`. Part of #80; #216 (pivots) builds on it.

## Goal
Roblox-compatible `CFrame` in Luau and `BasePart.CFrame`, so the pivot API (#216) and later Camera/Raycast work have a
transform type.

## Done
- `metallic_core::CFrame` and `RotationOrder` (`crates/metallic_core/src/cframe.rs`): constructors, Euler angles in all
  six orders (decomposition handles gimbal lock), axis angle, look-at, quaternion, inverse, lerp (slerp), spaces.
- Luau `CFrame` (`crates/metallic_script/src/api/cframe.rs`), `Enum.RotationOrder`, `BasePart.CFrame` (reads and writes
  Position/Orientation, degrees YXZ).
- Tests: `crates/metallic_core/src/cframe.rs` unit tests, `crates/metallic_script/tests/cframe.rs`.

## Left of #80
- `workspace:Raycast` with `RaycastParams`/`RaycastResult`: `metallic_script` does not depend on `metallic_physics`, so
  it needs a raycast hook that the runtime installs on `ScriptHost`.
- `workspace.CurrentCamera` (`Camera` class with `CFrame`, `FieldOfView`) wired to the runtime camera controller.
- Raw `UserInputService` events (`InputBegan`, `InputEnded`, `InputChanged`, ...); the enums and last-input members
  exist from #251.
- `CFrame` as a stored property type (scene files, remote payloads). #216 adds it for `PivotOffset`/`WorldPivot`.

## Commands
```
cargo test -j 2 -p metallic_core
cargo test -j 2 -p metallic_script
cargo clippy -j 2 -p metallic_core -p metallic_script --all-targets -- -D warnings
```
