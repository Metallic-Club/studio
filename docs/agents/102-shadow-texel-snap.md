# #102: shadow texel snapping

## Goal
Stop cascaded shadow edges from shimmering when the camera moves: `fit_cascade` must snap each cascade to whole
shadow-map texels in a light space that does not move with the camera.

## Done
- `crates/metallic_render/src/shadows.rs`, `fit_cascade`:
  - The bounding sphere (centre offset and radius) is computed in camera view space from the projection alone, so the
    radius, and with it the texel size, is bit-identical for every camera pose (only fov, aspect and the slice change it).
  - The ortho half-width carries one texel of padding, so the snapped box still contains the whole slice.
  - The light rotation is a look-at from the world origin along `-to_sun`, independent of the cascade centre. The centre
    is projected into that space, its x/y snapped to whole texels, and the view is that rotation followed by a
    translation to the snapped centre (pulled back toward the sun by radius + caster margin).
- Tests: `shadow_snap_moves_world_point_by_whole_texels` (new; all cascades, camera moved six 0.137-stud steps, world
  origin must keep the same sub-texel offset within 1e-3 texel and the texel size must not change),
  `cascade_covers_its_frustum_slice` (now strict: slice corners inside clip x/y [-1, 1] and depth 0..1, for all three
  cascades of three different camera poses).
- Before the fix the new test failed: world origin drifted off the grid by up to 0.484 texel (e.g. cascade 0:
  (-0.31, -0.48), (0.37, 0.03), (0.06, -0.45) ...). After it every offset is 0.0000.

## Next
- Nothing for this issue. Visual check in the stress example (`--screenshot`) is optional; the behaviour is covered by the CPU test.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_render -j 4 --lib shadow -- --nocapture
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Open questions
- None. Rotating the camera still changes which texels a slice covers (as in any stable-CSM scheme), but no longer
  slides the grid under a fixed world point.
