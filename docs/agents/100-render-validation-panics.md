# #100 Renderer: wgpu validation panics on size mismatch or >8192 px

## Goal
`Renderer::render` with a stale `size`, or a capture/window above 8192 px, must not panic.

## Done
- `RenderContext` (window and headless) requests `Limits::default().using_resolution(adapter.limits())`, so the
  max texture size is the adapter's (16384 on the RX 6650 XT) instead of 8192. Other limits stay at defaults.
- Devices we create log uncaptured wgpu errors with `log::error!` instead of wgpu's default panic.
  `RenderContext::from_eframe` leaves the host's limits and handler alone (the host owns the device).
- `Renderer::render` draws at the target texture's real size (mip 0); `size` only matters for the zero check
  (minimised window). A mismatch is logged at debug level.
- `Renderer::capture` returns `RenderError::InvalidSize` for a side above `max_texture_dimension_2d` or a readback
  above `max_buffer_size`; `Renderer::prewarm` clamps to `max_texture_dimension_2d`.
- Tests in `crates/metallic_render/tests/headless.rs` (run inside a validation error scope, so they fail on any
  wgpu validation error): `render_with_a_size_other_than_the_target_uses_the_target`,
  `capture_wider_than_8192_does_not_panic`, `capture_above_the_device_limit_is_invalid_size`. All three fail
  without the fix.

## Next
- Review and merge. No public signature changed.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_render -j 4 --test headless
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Open questions
- Studio's eframe device keeps eframe's limits and error handler; a Studio viewport wider than that device's
  limit is still the host's call (render itself no longer mismatches sizes).
