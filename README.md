# Metallic Engine + Metallic Studio

The **Metallic Engine** is our own game engine, written in Rust. **Metallic Studio** is the editor creators use to
make 2D and 3D games with it. Games run in the shared **Metallic player**, scripted in sandboxed Luau with Roblox-style names.

Status: milestone 1 is being built: the engine, the player, the Parkour template and the Studio editor run; Play in
Studio (#11) and packaging (#12) are not done yet (see `docs/STATUS.md`). Private repo; the website/backend is `Metallic-Club/plateform`.

## Quick start (Windows 11)
Prerequisites: Git, [Rust via rustup](https://rustup.rs). The exact toolchain installs itself from `rust-toolchain.toml`
on the first `cargo` call. Git Bash: `export PATH="$HOME/.cargo/bin:$PATH"`.

| Do | Command | Status on `main` |
|---|---|---|
| Build | `cargo build --workspace` | implemented |
| Test | `cargo test --workspace` | implemented |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | implemented |
| Licence check | `cargo deny check licenses` (`cargo install cargo-deny --locked` once) | implemented |
| Play Parkour | `cargo run --release -p metallic_player -- templates/parkour` | implemented (keyboard, gamepad; `--emulate phone` for touch) |
| Player smoke test | `cargo run --release -p metallic_player -- templates/parkour --headless --scene tests/smoke.mscene --frames 1200` | implemented |
| Open Studio | `cargo run --release -p metallic_studio` | implemented: start screen (templates, recent projects), the editor (viewport, Explorer, Properties, Output, script tabs, undo, save) and Settings (Ctrl+,). Play/Stop is Task 11 (#11), not merged yet |
| Publish a Studio build | `python tools/release.py --channel dev --version X.Y.Z` (see [`docs/releasing.md`](docs/releasing.md)) | implemented; the install-and-update round trip is not yet tested end to end |
| Package a zip | `tools/package.py` | planned (Task 12, #12); the script does not exist yet |

Daytime tip: add `-j 4` to cargo so your PC stays responsive. The player's options (headless runs, input scripts,
captures, touch emulation) are listed at the top of `crates/metallic_player/src/main.rs`.

## Repo map
- `crates/`: the Cargo workspace.
  - Engine: `metallic_core` (the Instance tree, class registry, scene files, `CFrame`, the `Lighting` service and its
    12 presets), `metallic_render` (wgpu renderer: materials, sun shadows, sky, fog, post and tone mapping, skinned
    characters), `metallic_physics` (Rapier 3D, the Humanoid character controller, death and respawn),
    `metallic_script` (sandboxed Luau with the Roblox-style API), `metallic_runtime` (one running game: world, physics,
    characters, scripts, camera, in-game GUI, input actions for keyboard, mouse, gamepad and touch),
    `metallic_asset` (glTF skinned characters), `metallic_anim` (animation data and state machines),
    `metallic_meshgen` (Part shape meshes).
  - Apps: `metallic_player` (the `metallic-player` binary; its Controls menu has Keyboard, Gamepad, Assist and Touch
    tabs), `metallic_studio` (the `metallic-studio` binary), `metallic_launcher` (the `Metallic` launcher shell around
    the platform website).
  - In `metallic_studio`:
    - Shell: the splash, the start screen (templates, recent projects, Ctrl+N / Ctrl+O), and Settings (theme
      Graphite, Titanium, Light or System; interface size; reduced motion). The theme comes from `metallic_tokens`,
      with bundled Archivo, JetBrains Mono, Noto Sans and Phosphor icon fonts and Studio's own button and plate
      widgets. Update and GitHub sign-in code is here too.
    - Editor: a 3D viewport with click, Ctrl-click, Alt-click and box selection, and move, rotate and scale gizmos
      with grid snapping (Ctrl inverts, Shift for precision). Explorer (tree, rename, drag to reparent), Properties
      (number fields take expressions such as `3*4` or `+2`), the tagged Output panel, and Luau script tabs. Undo and
      redo, with every edit one command, and save, which writes the scene through a temporary file and a rename.
    - Play: `play.rs` holds the Play/Stop seam. No play mode is registered until Task 11 (#11).
    - Not yet docked into the editor: the Input Actions panel (`src/input_actions/`), the colour popover (a demo
      screen, `--scene colour`) and the building-tool cores for arrange, scatter, selection modes and measure
      (`src/tools/`).
  - Libraries not yet used by the player or Studio: `metallic_net` (QUIC and in-memory transports and the wire
    encoding for multiplayer; replication comes later), `metallic_audio`, `metallic_vfx` and `metallic_vfx_editor`,
    `metallic_anim_editor`, `metallic_modeling` (the modeller core), `metallic_physics2d`.
  - Shared: `metallic_update` (update feed and GitHub sign-in for Studio and the Launcher), `metallic_tokens`
    (design tokens).
- `templates/`: game templates as project folders (`baseplate`, `parkour`).
- `branding/`: icon and splash files; the only place they live.
- `docs/`: [`STATUS.md`](docs/STATUS.md), [`luau-api.md`](docs/luau-api.md), [`releasing.md`](docs/releasing.md),
  design specs in `design/` and `superpowers/specs/`, research in `research/`, and `agents/` per-issue progress notes.
- `tools/`: Python scripts: `release.py` (Studio releases), `cargo_lane.py` (see
  [`docs/build-lanes.md`](docs/build-lanes.md)), `perf/` (the CI performance gate), `assets/` (asset fetching and
  previews).
- `.github/`: CI, performance and release workflows; issue and PR templates.

## Working here
- [`CLAUDE.md`](CLAUDE.md) working agreement for AI sessions and people: claim an issue, branch, PR
- [`ARCHITECTURE.md`](ARCHITECTURE.md) how the crates fit together and where to add things
- [`docs/STATUS.md`](docs/STATUS.md) what is in progress now
- [Design spec](docs/superpowers/specs/2026-10-30-metallic-studio-design.md) and [M1 plan](docs/superpowers/plans/2026-10-30-engine-m1.md)
- Issues are the task list: `gh issue list -R Metallic-Club/studio -l status:ready`
