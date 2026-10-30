# Task 8: runtime, player and Parkour template (milestone 1A)

Branch `studio/8-runtime-parkour`, worktree `metallic-studio-worktrees/8-runtime-parkour`. Brief:
`.superpowers/sdd/2026-10-30-engine-m1/task-8-brief.md` in the main checkout.

## Goal
`metallic-player` plays the Parkour template: `metallic_runtime::Game` ties world, physics, characters, scripts,
camera and GUI; `metallic_player` is the window/headless app; `templates/parkour` is the game.

## Where things are
- `crates/metallic_runtime/src/`: `game.rs` (Game, frame loop, physics re-sync from the change journal, run boost),
  `input.rs` (FrameInput, InputState), `camera_controller.rs` (orbit, zoom, wall pull-in), `gui.rs` (PlayerGui layout
  and egui painting), `project.rs` (`metallic.toml`).
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
- Run (Shift) = `MoveInput::run` in metallic_physics: 1.5 x WalkSpeed inside the controller; `WalkSpeed` itself never changes.
- StarterGui is copied into PlayerGui once at start (no ResetOnSpawn in M1).
- GUI text uses egui's bundled fonts (Ruling R11; licence texts in `LICENSES/THIRD-PARTY-FONTS.md`, shared with Studio).
- Character touches reach scripts as `part.Touched(HumanoidRootPart)`.
- Lava and checkpoint scripts re-scan their folder every Heartbeat for new parts (the script API has no `ChildAdded`
  yet).
- The level is built per section: sections 2-4 use their own coordinates moved by `SLATE_AT`/`MOSS_AT`/`ICE_AT`
  (`level.rs`); the route test repeats those offsets and the tower numbers.
- Overview screenshots: `metallic-player <project> --frames N --capture N:shot.png --capture-view X,Y,Z,LX,LY,LZ`.

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
