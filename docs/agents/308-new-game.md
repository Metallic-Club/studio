# #308 New game button and built-in Baseplate

Status: implemented and tested; pushed for the lead's PR.
Branch: `studio/308-new-game-button`, based on main `e20f0828` (includes #306, which ships `templates/`).

## Problem
Studio 0.1.0 shipped without `templates/`, so the start screen said "No templates found" and had
no way to create a game: the only Create button lives in the selected-template panel.

## What changed
- `templates/baseplate/`: `template.toml` (id `baseplate`, order 0), `metallic.toml`, `main.mscene`
  (RON version 1, modelled on Parkour): an anchored 512x16x512 grey `Part` named `Baseplate` with
  its top at y = 0, a `SpawnLocation`, and the standard builtin services (`Lighting` has no
  properties in the class registry and there is no Sky class, so lighting is the engine default).
- `templates.rs`: `EmbeddedFile`, `TemplateInfo::embedded`, `BASEPLATE_ID`, `builtin_baseplate()`
  (the folder compiled in with `include_bytes!`), `with_builtin()` (adds it when no template with
  that id was listed). `create_from_template` handles both sources through one path: the same
  name validation, empty-folder check, catalog metadata and exclusions (`is_excluded`, shared by
  `copy_tree` and `write_embedded`), `set_project_name` and rollback.
- `start_screen.rs`: `StartScreen::new` calls `with_builtin`, so the screen always offers
  Baseplate. A full-width primary "+ New game" button (ACCENT fill, DEEP text) sits at the top of
  the sidebar above "Recent games"; it selects the first template (Baseplate) and requests focus
  for the game name field, so Enter creates the game. The per-template Create button is unchanged.
- `metallic_core` added as a dev-dependency of `metallic_studio` (tests read `Value`).

## Tests
- `crates/metallic_studio/tests/baseplate.rs`: embedded copy byte-identical to the folder (file
  set and bytes, `preview.png` excepted); disk order Baseplate then Parkour and no duplicate;
  Baseplate offered without a templates folder; the repo template and a game created from the
  embedded copy both open with `Game::open` (Player/Studio path), start and keep the character
  standing on the spawn for 2 s; embedded create refuses a busy folder and writes nothing.
- `start_screen.rs` unit tests: empty list offers Baseplate and `create()` writes `metallic.toml`
  and `main.mscene`; disk templates order Baseplate, Parkour; `new_game()` selects Baseplate and
  sets the focus flag. Two older tests now select the Parkour entry explicitly.

## Screenshots (not in git)
`metallic-studio --screenshot <png> --scene start --location <tmp>` from an exe copied to a folder
without `templates/` and `METALLIC_TEMPLATES_DIR` unset (fallback: Baseplate only), and again with
`METALLIC_TEMPLATES_DIR` at the repo `templates/` (Baseplate, Parkour). Both checked by eye.

## Open points
- No `preview.png` for Baseplate. Parkour's preview is a `metallic-player --capture` start shot at
  640x360, but the debug player panics on exit after a capture (epaint "Dropped TexturesDelta",
  then a wgpu swapchain semaphore panic; Parkour too), and no release player was built. A later
  capture with a release player can add it; the embedded copy would still show the placeholder
  unless the preview is embedded as well.
- The "No templates found" panel is now only reachable if the compiled-in `template.toml` is
  broken, which the byte/parse tests rule out.
