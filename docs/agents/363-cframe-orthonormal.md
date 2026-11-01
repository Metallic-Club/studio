# #363 Stored CFrames stay orthonormal

Status: implemented and tested locally; committed on the branch, not pushed, no PR yet.
Branch: `studio/363-cframe-orthonormal` (from `origin/main` `8bc3cd7`).

## Goal

`World::set` accepted any finite `Value::CFrame`, including zero, scaled or skewed rotations, and
`CFrame::inverse` (used by `get_pivot`, `pivot_to`, `scale_to`) assumes an orthonormal one. Decided rule
(lead's call, following Roblox): re-orthonormalise on write keeping the right axis (the rule `Orthonormalize` already had since #304; the coordinator chose this over keeping the look vector so merged behaviour does not change), refuse what cannot be fixed.

## Done

- `CFrame::try_orthonormalize` (`crates/metallic_core/src/cframe.rs`): the one rule.
  - Already orthonormal and right-handed within `ORTHONORMAL_TOLERANCE` (1e-6 on `R^T R - I`): returned bit for bit.
  - Otherwise: look = -back.unit(), right = (look x up).unit(), up = right x look (the `look_at` basis).
  - `None` when not finite, an axis column is under `MIN_AXIS_LENGTH` (1e-9), or
    `|det R| / (|right| |up| |back|)` is under `DEGENERATE_EPSILON` (1e-6): parallel or coplanar columns.
- `CFrame::orthonormalize` (Luau `Orthonormalize`) now uses the same rule; degenerate input comes back unchanged.
  Before, it kept the right axis.
- `World::set` (`instance.rs`): after the finite check, every `Value::CFrame` goes through
  `try_orthonormalize`; `None` is `CoreError::InvalidValue` ("the rotation is degenerate"), the NaN error path.
- Scene loader: goes through `World::set`, so it gets the rule; doc comment on `load_scene` says so.
- `transform::orthonormal` delegates to `try_orthonormalize` (keeps its finite/degenerate error messages).
- Luau `CFrame.new` with 12 numbers (`metallic_script/src/api/cframe.rs`, `twelve_numbers`) normalises the
  same way. Roblox's behaviour for a degenerate matrix there is not documented, so a finite degenerate matrix
  gets the identity rotation at the given position; a non-finite one is kept so property writes still error.
  `CFrame.fromMatrix` still keeps its axes as given.
- Tests: `crates/metallic_core/tests/cframe_orthonormal.rs` (`zero_rotation_cframe_is_refused`,
  `scaled_rotation_cframe_is_normalized`, `skewed_rotation_cframe_is_normalized`,
  `nearly_orthonormal_cframe_is_kept`, `nan_cframe_is_still_refused`, `scene_load_normalizes_cframe`);
  `crates/metallic_script/tests/cframe.rs` `cframe_new_twelve_numbers_is_orthonormal`.
- `crates/metallic_script/tests/pivot.rs` `pivot_offset_is_made_orthonormal` updated: the skewed and zero targets are built
  with `fromMatrix` because `CFrame.new` already fixes them; expected values are unchanged.
- Docs: ARCHITECTURE.md metallic_core bullet, `docs/luau-api.md` (CFrame constructors, CFrame properties).

## Next

- Push, open the PR (`Closes #363`), set the issue to `status:review`.

## Commands

```
cargo test -p metallic_core -j 4 --test cframe_orthonormal
cargo test -p metallic_script -j 4 --test cframe --test pivot
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all --check
```

## Open questions

- Roblox keeps mirrored (determinant -1) matrices; this rule makes them right-handed by flipping the right axis.
  Chosen on purpose (stored rotations are always proper); revisit only if a Roblox-compat case needs mirrors.
