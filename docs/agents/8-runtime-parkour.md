# Task 8: runtime, player and Parkour template (milestone 1A)

Branch `studio/8-runtime-parkour`, worktree `metallic-studio-worktrees/8-runtime-parkour`. Brief:
`.superpowers/sdd/2026-10-30-engine-m1/task-8-brief.md` in the main checkout.

## Goal
`metallic-player` plays the Parkour template: `metallic_runtime::Game` ties world, physics, characters, scripts,
camera and GUI; `metallic_player` is the window/headless app; `templates/parkour` is the game.

## Where things are
- `crates/metallic_runtime/src/`: `game.rs` (Game, frame loop, physics re-sync from the change journal, run boost),
  `input.rs` (FrameInput, InputState), `camera_controller.rs` (orbit, zoom, wall pull-in), `gui.rs` (PlayerGui layout
  and egui painting, system font), `project.rs` (`metallic.toml`).
- `crates/metallic_runtime/examples/build_parkour/`: the level as code (`level.rs`), its Luau scripts (`scripts.rs`);
  writes `templates/parkour/main.mscene` and `tests/smoke.mscene`. Re-run after changing the level:
  `cargo run -p metallic_runtime --example build_parkour`.
- `crates/metallic_player/src/`: `main.rs` (headless loop), `args.rs`, `window.rs` (winit, input, fps), `overlay.rs`
  (egui-wgpu pass), `capture.rs` (`--capture FRAME:PNG`).
- Tests: `crates/metallic_player/tests/smoke.rs` (brief's smoke + missing-scene), `crates/metallic_runtime/tests/
  parkour.rs` (template content, walk/run speed, lava death + respawn, script-made parts get physics, determinism),
  `crates/metallic_runtime/tests/parkour_route.rs` (a bot using only player input finishes the course).

## Decisions
- Frame order and fixed 60 Hz tick: see ARCHITECTURE.md "Frame loop".
- Run (Shift) = WalkSpeed x1.5 for the controller step only; restored before scripts run (physics crate untouched).
- StarterGui is copied into PlayerGui once at start (no ResetOnSpawn in M1).
- egui without bundled fonts (OFL/Ubuntu licences are not on the allow list); GUI text uses a system font found at run
  time (Segoe UI Semibold on Windows). Studio (Task 9, eframe) will meet the same licence question.
- Character touches reach scripts as `part.Touched(HumanoidRootPart)`.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=C:/Users/alexi/Documents/metallic-studio-worktrees/_target-8-runtime-parkour
cargo test -j 2 -p metallic_player -p metallic_runtime
cargo run -j 2 -p metallic_player -- templates/parkour --scene tests/smoke.mscene --headless --frames 1200
cargo run -j 2 --release -p metallic_player -- templates/parkour --fps
```

## Status
See the task report (`.superpowers/sdd/2026-10-30-engine-m1/task-8-report.md` in the main checkout) for results.
