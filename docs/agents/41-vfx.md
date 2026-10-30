# #41 VFX: progress note

Branch `studio/41-vfx`, worktree `metallic-studio-worktrees/41-vfx`.

## Goal
`metallic_vfx` (effect model, deterministic CPU simulator, presets) and `metallic_vfx_editor` (egui editor: module
stack, node graph, curve/gradient/timeline widgets, live preview).

## Status: implemented, NOT tested
- `crates/metallic_vfx`: committed (effect model, simulator, 26 `.vfx.ron` presets, `benches/update.rs`).
- `crates/metallic_vfx_editor`: committed (src, `examples/vfx_editor_demo.rs`, `examples/vfx_screenshots.rs`).
- `Cargo.lock`: updated for egui 0.37 / eframe / egui_kittest / wgpu / image.
- Nothing in this branch has compiled yet. Every build attempt stopped with "There is not enough space on the disk"
  (os error 112) on C:. That is a disk problem, not a code error, but compile errors may still exist.
- All cargo builds are frozen until the owner reboots the PC (page file filled C:).

## Checks NOT run yet (none have run)
1. `cargo build` / `cargo check` of `metallic_vfx` and `metallic_vfx_editor`: not run.
2. Crate tests: `cargo test -p metallic_vfx -p metallic_vfx_editor`: not run.
3. Workspace tests: `cargo test --workspace`: not run.
4. Clippy: `cargo clippy --workspace --all-targets -- -D warnings`: not run.
5. Licences for the new deps (egui, eframe, egui_kittest, wgpu, image): `cargo deny check licenses`: not run.
6. Screenshots: `vfx_screenshots` example never ran, so no PNGs exist and nobody has looked at the editor.
7. Bench `metallic_vfx/benches/update.rs`: not run.

## Commands to run after the reboot
Only when C: has at least 4 GB free. Use `-j 2` and the shared target. Run in this order, in a background shell:
```
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=C:/Users/alexi/Documents/metallic-studio-worktrees/_target-lead-shared
cd C:/Users/alexi/Documents/metallic-studio-worktrees/41-vfx
cargo test -j 2 -p metallic_vfx -p metallic_vfx_editor
cargo run -j 2 -p metallic_vfx_editor --example vfx_screenshots -- docs/agents/41-vfx
cargo test -j 2 --workspace
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo deny check licenses
```
The screenshot example writes `editor-stack.png`, `editor-graph.png`, `presets.png`, `styles.png` into
`docs/agents/41-vfx/`. Look at them yourself, then commit them.

## Next
1. Run the checks above and fix what fails.
2. Commit screenshots, rebase on `origin/main`, then open the PR (`Closes #41`) with the check output pasted in.

## Open questions
- `metallic_vfx_editor/Cargo.toml` declares egui/eframe/egui_kittest/image directly. `docs/agents/m1-build.md` says
  third-party deps go in root `[workspace.dependencies]`. Not moved yet because that can't be verified without a build.
