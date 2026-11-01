# #478 Studio templates: dev fallback, Empty template, hidden 2D filter, new thumbnails

Branch `studio/478-studio-templates` (from `origin/main` e8fa980). Part of #454 (problems 6, 8, 9).

## Goal
- (9) `target/debug/metallic-studio` finds the repo's templates without `METALLIC_TEMPLATES_DIR`.
- (8) An **Empty** template; the 2D filter stays hidden until a 2D template exists (2D is M3).
- (6) Template previews show the current mannequin, for every template.

## Done
- `templates.rs`: `templates_dir()` now resolves, first match wins: env `METALLIC_TEMPLATES_DIR`
  (set and not empty, used as given), installed `<exe dir>/templates` (when it exists), debug
  builds only the repo's `templates/` from `env!("CARGO_MANIFEST_DIR")` at compile time, else
  `<exe dir>/templates` for the warning. Compile-time path, not a walk up from the exe, because
  sessions share one `CARGO_TARGET_DIR`; release builds leave it out (`#[cfg(debug_assertions)]`)
  so a shipped exe carries no build path. Testable core: `resolve_templates_dir(env, exe_dir, dev)`.
  Installer/`tools/release.py` unchanged: it still stages the whole `templates/` beside the exe,
  so the new `templates/empty` ships with it.
- `templates/empty/`: `template.toml` (kind `empty`, order 1), `metallic.toml`, `main.mscene` =
  Baseplate's scene without the floor Part (Workspace with one SpawnLocation, the services,
  default Lighting). Parkour's `order` moved 1 -> 2, so the cards read Baseplate, Empty, Parkour
  and "New game" still picks Baseplate.
- `start_screen.rs`: `filter_options(templates)` drops 2D when no template has `kind = "2d"`; a
  stale 2D filter falls back to All.
- Previews regenerated with the mannequin: `templates/{baseplate,empty,parkour}/preview.png`
  (640 x 360). Baseplate had none before (it showed Theme art); the compiled-in Baseplate copy
  still has none (it is only used when the folder is missing; `tests/baseplate.rs` already skips
  `preview.png` in the byte comparison).
- Tests: `templates::tests::templates_dir_prefers_env_then_installed_then_dev`,
  `debug_builds_fall_back_to_the_repo_templates`; `start_screen::tests::the_2d_filter_hides_until_a_2d_template_exists`;
  `tests/empty_template.rs` (listed as Empty with a preview; scene holds exactly the services plus
  one SpawnLocation; a created game plays with the character standing on the spawn); order tests
  updated to Baseplate, Empty, Parkour.

## Screenshots (in `docs/agents/478-studio-templates/`)
- `start-before.png`: main, with `METALLIC_TEMPLATES_DIR` at main's templates: Baseplate (Theme
  art), Parkour (blocky avatar), filter All 3D 2D Empty.
- `start-before-no-env.png`: main exe copied to a folder without `templates/`, no env: Baseplate only.
- `start-after.png`: this branch, `target/debug/metallic-studio` with no env var: Baseplate, Empty,
  Parkour with mannequin previews, filter All 3D Empty.

## Commands that matter
```
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_TARGET_DIR=<shared target>
cargo build -p metallic_player -p metallic_studio -j 4
# previews: 1920x1080 frame, then scale to 640x360 (System.Drawing HighQualityBicubic was used)
$CARGO_TARGET_DIR/debug/metallic-player templates/<id> --capture 150:<tmp>/<id>.png --frames 160
# start screen
$CARGO_TARGET_DIR/debug/metallic-studio --screenshot out.png --scene start --theme graphite --size 1440x900
```
The debug player exited 0 after these captures (the old exit panic noted in #308 did not show).

## Open questions / next
- None blocking. Seen while capturing (not fixed here): in Parkour the new key-cap hint bar
  ("Space Jump, L Shift Run, Esc Menu") draws on top of the older text hint ("WASD move ...
  Right-drag camera") in the bottom-left corner; it shows in the Parkour preview too.

## Checks (2026-10-31, `-j 4`, shared target)
- `cargo test --workspace -j 4 --no-fail-fast`: 2126 passed, 1 failed:
  `metallic_render --test shading gpu_timing_measures_a_frame_where_supported` ("0 ms" on the AMD
  RX 6650 XT, also alone). This branch does not touch `metallic_render`. The first run also had
  `metallic_player --test smoke parkour_smoke_scene_passes_headless` hit the Luau script time
  limit under load; it passed alone and in the second full run.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: clean.
- `cargo deny check licenses`: `licenses ok`.
