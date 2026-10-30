# Architecture

Planned design from the M1 plan; crates appear as their tasks land. When a public interface changes, update this file
and the crate docs in the same PR.

```mermaid
graph LR
  core[metallic_core] --> physics[metallic_physics]
  core --> script[metallic_script]
  core --> render[metallic_render]
  physics --> runtime[metallic_runtime]
  script --> runtime
  render --> runtime
  runtime --> player[metallic_player]
  runtime --> studio[metallic_studio]
```
Arrows point from a dependency to its user. Nothing below `runtime` knows about the apps.

## Crates
- **metallic_core** (Task 2): the Roblox-style world. `World` owns a tree of Instances addressed by generational
  `InstanceId`; `Value` (Nil, Bool, Number, String, Vector3, Color3, UDim2, Ref); a class registry (`classes.rs`) with typed
  properties and defaults; `Signal<Args>`; `save_scene`/`load_scene` for the RON text format (one scene per file,
  only non-default properties, stable order). Also `deep_clone()` used by Play.
- **metallic_physics** (Tasks 4-5): `PhysicsWorld` wraps Rapier behind our own interface: `sync_from(&World)`,
  `step(&mut World, dt)` (1/240 s substeps), `velocity`/`angular_velocity`, `raycast`, `drain_touched`. Anchored parts that
  are moved or rotated become kinematic movers (linear + angular velocity). `CharacterController`, `spawn_character`
  and the Humanoid rules (death, respawn, checkpoints) live here.
- **metallic_script** (Tasks 6-7): `ScriptVm` is a sandboxed Luau VM (mlua; time budget, memory cap, forbidden globals,
  `task` library). `ScriptHost` runs server and client scripts over a shared `World`, exposes the Roblox-named API
  (documented in `docs/luau-api.md`) and RemoteEvent delivery. Output goes through `OutputSink`.
- **metallic_render** (Task 3): wgpu renderer. `Renderer::sync(&World)`, `render(target, camera, size)`; instanced shapes,
  PBR materials, cascaded shadows, sky, fog. `RenderContext` is built for a window (player) or from eframe (Studio).
- **metallic_runtime** (Task 8): `Game::load(project_dir)` (reads `metallic.toml`: `name`, `main_scene`) or
  `Game::open(dir, scene, output)` / `Game::from_world(name, world, output)`, then `start`, `frame(&FrameInput, dt)`,
  `render(&mut Renderer, target, size)`, `paint_gui(&egui::Painter)` and `stop`. Ties world, physics, characters,
  scripts, the third-person `CameraController` and the in-game GUI (`gui.rs`: ScreenGui/Frame/TextLabel drawn with an
  egui painter with egui's bundled fonts; their licence texts are in `LICENSES/`).
  `InputState` turns keys, mouse and gamepad into `FrameInput`s without depending on a window library. Shared by
  player and Studio.
- **metallic_player** (Task 8): `metallic-player <project dir> [--scene F] [--headless --frames N] [--quality low|high]
  [--fps] [--capture FRAME:PNG]`; winit window, wgpu surface, egui-wgpu for the GUI pass, gilrs for gamepads, headless
  mode for smoke tests (no window, no GPU).
- **metallic_studio** (Tasks 9-11, 13-14): egui editor: theme, start screen, templates, viewport, Explorer, Properties,
  Output, gizmos, `History` of `Command`s (undo/redo), `PlaySession`, updater, developer mode.
  Updater (Task 13): `updater.rs` and `github_auth.rs` re-export the shared `metallic_update` crate; Studio builds its
  checker with `UpdateChecker::new(STUDIO_FEED, channel, tokens)`. Release pipeline: `tools/release.py`,
  `.github/workflows/release.yml`, `docs/releasing.md`.
- **metallic_update** (Launcher L1 Task 1): no UI. `http.rs` (`HttpClient`, `UreqClient`, redacting `Token`), `feed.rs`
  (`ReleaseFeed`, `STUDIO_FEED`, `LAUNCHER_FEED`, `UpdateChecker::check/check_quietly/download/apply_and_restart`, GitHub
  Releases feed, Velopack for download and apply), `github_auth.rs` (device sign-in, `GithubAuth`) and `secret.rs`
  (`SecretStore`, `MemorySecretStore`, `KeyringSecretStore::new(service, account)`). Shared by Studio and the Launcher.
- **metallic_launcher** (Launcher L1 Task 2, library part): no window yet. `deep_link.rs` (`DeepLink::parse/to_uri` for
  `metallic://play|studio/open|open|install`, `PlaceId`, `JoinTicket`, `SitePath`), `site.rs` (`SiteOrigin`, `LauncherConfig::from_args`,
  `is_local_page`), `url_policy.rs` (`UrlPolicy::navigation/new_window/origin_of`), `settings.rs` (`LauncherSettings`, `SettingsStore`).
  Site address: `config/launcher_site_origin.txt`.

## Frame loop
`Game::frame(input, dt)` applies camera orbit/zoom, then runs fixed 60 Hz ticks (an accumulator carries the remainder;
`dt` is clamped to 0.25 s), so a headless run of N frames of 1/60 s is deterministic. Each tick:

1. **input -> character controller**: `CharacterController::step` with the camera-relative move wish (Shift sets
   `MoveInput::run`: 1.5 x `Humanoid.WalkSpeed`, which scripts still read unchanged); death, respawn, checkpoints and
   character touches happen here.
2. **physics step**: `PhysicsWorld::step(TICK)` (four 1/240 s substeps; anchored parts moved by scripts become movers).
3. **feed events** to the script host: every `drain_touched` pair as `touched(a, b)` and `touched(b, a)`; a character
   touch as `touched(part, HumanoidRootPart)` so `hit.Parent` is the character Model; `Died` -> `humanoid_died`;
   `Respawned` -> `physics.sync_from` and `character_added(player, model)`.
4. **script step** (`ScriptHost::step`: new scripts, remotes, the inputs above, `RunService.Heartbeat`, resumed waits).
5. **physics re-sync** when the change journal shows a BasePart under Workspace was created, destroyed, reparented or
   reshaped (Size, Shape, Material, Anchored, CanCollide); plain moves need no sync.

Then the camera follows the character (frozen where it died while dead; a ray pulls it in front of walls). The app then
calls `Game::render` (`Renderer::sync` from the change journal, then `render`) and paints the GUI over the 3D view. The
game never holds a `World` borrow while scripts run. `Game::start` runs the scripts (which creates `Player1`), copies
`StarterGui` into the `PlayerGui` once, spawns the character and fires `CharacterAdded`.

## Where to add things
- **A new Instance class or property:** `metallic_core/src/classes.rs` (registry entry with defaults), a test in
  `tests/tree.rs`; the Luau API picks up registry properties by name; Studio's Properties panel too. Add to Insert menu if
  creators should create it.
- **A Luau API member:** `metallic_script/src/api/` (`instance.rs`, `datatypes.rs`, `services.rs`, `remote.rs`), a test
  in `tests/api.rs`, and a line in `docs/luau-api.md`.
- **A material:** the material list in `classes.rs`, its row in `metallic_render/src/materials.rs` (roughness, metalness,
  procedural detail), and density/friction in `metallic_physics`. Test `material_table_has_every_registry_material`
  must still pass.
- **A Studio edit action:** a `Command` in `metallic_studio/src/commands.rs` applied through `History`, never by
  mutating the world directly.

## Quality bar
- Every public item has a `///` doc comment; every crate has a `//!` crate doc.
- Tests sit next to behaviour (write the failing test first).
- No `unwrap()` or `expect()` outside tests; return typed errors (`CoreError`, `ScriptError`, `RuntimeError`, `StudioError`).
- Scripts can never freeze or crash the game: budgets and caps are enforced in `metallic_script`.
- Permissive dependency licences only; `cargo deny check licenses` stays green.
- Benchmarks: criterion benches live in `crates/<crate>/benches/` (`cargo bench -p metallic_core --bench world` exists;
  render sync, physics step and script step follow). Budgets, bench names and the perf-CI plan are in
  `docs/research/2026-10-30-performance-and-scale.md`; record numbers in `docs/agents/`.
