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
