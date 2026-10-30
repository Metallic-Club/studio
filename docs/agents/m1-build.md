# Metallic Engine M1: build notes

## Versions (2026-10-30)
- rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1, toolchain stable-x86_64-pc-windows-msvc, pinned by `rust-toolchain.toml` (channel "1.99", components clippy + rustfmt).
- cargo-deny 0.20.2 (`cargo install cargo-deny --locked`, about 3m30s).
- Edition 2024, resolver 3.

## Timings (empty skeleton, cold)
- `cargo test --workspace`: about 3 s. `cargo clippy --workspace --all-targets -- -D warnings`: about 2.6 s. `cargo deny check licenses`: a few seconds.
- These will grow sharply once wgpu, rapier, mlua and egui land.

## Notes for later tasks
- Dependency direction: core <- physics, script, render <- runtime <- player, studio. Shared path deps live in root `[workspace.dependencies]`; crates use `x.workspace = true`.
- Add third-party deps to `[workspace.dependencies]`, then rerun `cargo deny check licenses`. Allowed: MIT, Apache-2.0 (also WITH LLVM-exception), BSD-2/3-Clause, Zlib, ISC, Unicode-3.0, MPL-2.0. Add exceptions in `deny.toml` only deliberately.
- Workspace crates are `LicenseRef-Metallic-Proprietary` and `publish = false`; `[licenses.private] ignore = true` makes deny skip them.
- deny currently warns "Zlib not encountered" (no dependency uses it yet); harmless.
- PC load: 08:00-24:00 America/Chicago use `cargo ... -j 4`; before 08:00 all cores.
- `/target` and `/dist` are git-ignored; `Cargo.lock` is committed.

## Renderer (Task 3, `metallic_render`)
- Stack: wgpu 30.0.1, glam 0.30.10 (glam 0.34 moved `Mat4::look_at_rh`/`perspective_rh` into a new `camera` module; pinned to 0.30 for the familiar API), bytemuck, pollster. winit 0.30 and png are dev-dependencies of the stress example only.
- Stress example: `cargo run --release -p metallic_render --example stress -- --seconds 30` (flags: `--vsync`, `--low`, `--screenshot PATH` for a windowless offscreen frame).
- Measured 2026-10-30 on NVIDIA GeForce RTX 3060 Ti, 1920x1080, High (3 cascades 2048, 4x MSAA), uncapped present, 5,001 parts (5,000 random + baseplate), orbiting camera, 30 s:
  - 5 s windows, average fps: 319.6, 436.7, 322.7, 565.2, 577.0. Overall average 463.8 fps.
  - Per-window minimum fps (worst single frame) was 1.8, 19.8, 11.6, 41.9, 68.1: isolated frame-time spikes, worst in the first seconds (shader/driver warm-up), also seen while the PC was busy with other builds. Not investigated further.
- Screenshot: `docs/agents/m1-shots/stress.png` (offscreen frame of the same scene via `--screenshot`).
- Headless GPU tests live in `crates/metallic_render/tests/headless.rs` and skip when no adapter exists.

### Renderer fix round 1 (2026-10-30)
- Incremental sync: `World::changes_since(&self, ChangeCursor)` journal in metallic_core (bounded at 16,384 entries, `Changes::Overflow` triggers one full re-read). `Scene::sync` of 5,000 parts with 50 changed: best of 20 = 97.5 us in release (`cargo test --release -p metallic_render --lib sync_of_5000 -- --nocapture`; asserted < 1 ms).
- Stress now moves 500 parts per frame through `World::set` + `sync` (sync cost included in fps; separate sync column). 30 s, RTX 3060 Ti, 1920x1080 High, uncapped, no cargo build running at start (other lanes started rustc during the run): window avg fps 347.8 / 357.9 / 355.0 / 351.2 / 337.3, overall avg 349.2; window minimums 31.8 / 47.8 / 80.7 / 50.4 / 62.2; frame ms p50 2.88, p99 4.29, p99.9 13.71, max 31.48; 0 frames over 33 ms; sync 1.30 ms/frame (this includes the 500 `World::set` calls).
- Spike investigation: frame #0 costs 24-32 ms with or without pre-warm; a cold shader (edited WGSL, so no driver cache) and warm one behave the same, and `Renderer::prewarm` takes only 4-5 ms, so pipeline compile and buffer growth are not the cause. The earlier 1.8 fps window minimum was one ~550 ms first frame of a freshly built binary. Mid-run spikes of 15-35 ms (1-3 frames per 30 s) coincide with other lanes' rustc/link processes competing for the CPU; with the machine quiet the p99.9 stays near 6-8 ms. Vsync is not involved (AutoNoVsync). `Renderer::prewarm(size)` added and used by the example (`--no-prewarm` to compare).
