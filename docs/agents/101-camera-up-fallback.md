# #101 Camera up-vector fallback

## Goal
`Camera::view()` must stay finite when the camera looks straight down/up, at its own position, or has non-finite
coordinates (was NaN -> blank frame, issue #101).

## Done
- `crates/metallic_render/src/camera.rs`: `view()` uses `look_to_rh` with fallbacks:
  - `|forward.y| > 0.999`: up = `Z * sign(forward.y)` (down -> -Z at the top of the screen, up -> +Z). This is where a
    camera facing -Z ends up at -90/+90 degrees pitch, so nothing flips at the limit; never mirrored (+X stays right).
  - zero or non-finite direction (eye == look_at, NaN look_at): forward = -Z.
  - non-finite position: eye = look_at, else origin.
  - Public API unchanged; behaviour documented on `Camera` and `view()`.
- Unit tests in `camera.rs`: `straight_down_is_finite_with_minus_z_at_the_top`, `straight_up_is_finite_with_plus_z_at_the_top`,
  `nearly_vertical_matches_the_fallback_orientation`, `eye_on_look_at_is_finite`, `non_finite_inputs_give_a_finite_matrix`.
- GPU test in `tests/headless.rs`: `straight_down_camera_draws_the_part` (20x20 part from (0,50,0); centre pixel differs
  from the sky-only frame, > 10 colours). All but the continuity test fail with the old `look_at_rh(.., Vec3::Y)`.

## Next
Review / PR (not opened by this session).

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_render -j 4
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Open questions
- `Renderer` still passes `camera.position` raw as the eye uniform and for blended sorting; a NaN position gives a
  finite view_proj but a NaN eye there (renderer.rs is #100's file; not touched here).
- A NaN `fov_deg` still gives a NaN projection.
