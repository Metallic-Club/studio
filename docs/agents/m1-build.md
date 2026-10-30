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
