# ADR 0001: Project layout (repository and crates)

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #2

## Context

One repository (`Metallic-Club/studio`) holds the engine, Studio, Player and Launcher. It has to:

- build for Windows, Linux, macOS, Android, iOS, PlayStation and Xbox, with as little platform-specific code as possible;
- let a game server build and run without a GPU or a window;
- let more scripting languages than Luau be added later;
- stay quick to build and easy to find your way around, because many agents work in it.

Forge-Engine (the reference) put the object model, every class, rendering, physics, audio, scripting and the debug UI into one crate (`forge_engine`). Any change rebuilt everything, and class files imported Bevy and renderer types, so the API could not be built without the renderer.

## Decision

```
studio/                         Metallic-Club/studio (the one repo)
├── Cargo.toml                  workspace: members, shared dependency versions, build profiles
├── rust-toolchain.toml         pinned stable Rust
├── README.md · AGENTS.md · CONTRIBUTING.md · LICENSE
├── .github/                    CI workflows, issue and PR templates
├── docs/
│   ├── architecture.md         the map: crates, dependency rules, data flow
│   └── adr/                    decision records (0001-project-layout.md, ...)
├── crates/                     the engine: libraries only, they build for every platform
│   ├── core/                   metallic_core      values, math, ids, class registry, data model, signals
│   ├── classes/                metallic_classes   THE API: one file per class and service
│   ├── script/                 metallic_script    language-neutral scripting host, scheduler, bindings
│   ├── luau/                   metallic_luau      Luau backend (the first language)
│   ├── render/                 metallic_render    wgpu renderer, graphics quality settings
│   ├── physics/                metallic_physics   Box3D bridge (3D)
│   ├── project/                metallic_project   project manifest, scenes, assets, packaging
│   ├── platform/               metallic_platform  window, input, paths, OS glue
│   └── engine/                 metallic_engine    runtime loop; wires the crates above; what apps depend on
├── apps/                       executables, kept thin
│   ├── player/                 metallic_player    runs games: library (embedded by Studio and Launcher) + binary
│   ├── studio/                 metallic_studio    the editor (Windows, Linux, macOS only)
│   └── launcher/               metallic_launcher  website shell that runs Player (and Studio on PC)
├── templates/                  starter projects (3D Baseplate)
└── xtask/                      `cargo xtask`: packaging, API dump, repo checks
```

### Who depends on whom

```
 apps         metallic_studio ──► metallic_player ◄── metallic_launcher
                                        │
 runtime                         metallic_engine
                ┌──────────┬──────────┬─┴────────┬───────────┬────────────┐
 subsystems  classes     script     render     physics     project     platform
                │          │ ▲        │          │           │
 backends       │          │ luau     │          │           │
                ▼          ▼ ▼        ▼          ▼           ▼
 model     ─────────────── metallic_core ──────────────────────────
```

### Rules (CI checks them from #11 on)

1. **Arrows only point down.** A subsystem never depends on another subsystem. `metallic_engine` is the only crate that knows all of them.
2. **`metallic_core` stays pure:** only small portable crates (`glam`, `serde`, `thiserror` and similar). No GPU, window, VM or OS code.
3. **`metallic_classes` declares, it does not draw or simulate.** A class names the components it has (`SpotLightEmitter`), and the subsystem that implements the component registers itself under that name (#3). Classes never import render, physics, scripting or platform types.
4. **`render` gets a window handle (`raw-window-handle`) and decoded assets from `engine`.** It does not depend on `platform` or `project`.
5. **`#[cfg(target_os = ...)]` only appears in `metallic_platform`** (and in app packaging). Everything else is platform-free.
6. **A language backend depends only on `metallic_script` and `metallic_core`.** A new language is a new crate next to `luau/`; nothing else changes.
7. **`metallic_engine` features:** `render`, `physics`, `luau` (all on by default). A dedicated server is `--no-default-features --features physics,luau`.
8. **Studio builds for desktop targets only.** Player and Launcher build everywhere.
   `xtask` is host tooling that never ships; it may read `metallic_core`, `metallic_classes` and `metallic_script`
   (the API dump, #48).
9. **Console code is under NDA.** It lives in private repos (`Metallic-Club/platform-<console>`) that plug into the `metallic_platform` and `metallic_render` seams. This repo holds only the seams, never console SDK code.

### Naming

- Folder = short name (`crates/core`), package = `metallic_<name>`.
- One class per file under `crates/classes/src/<group>/`, file name = the class name in snake_case (`spot_light.rs`).
- Branches `<issue#>-<slug>`. Decision records `docs/adr/NNNN-title.md`.

### Reserved for later (do not create now)

| Crate | When |
|---|---|
| `crates/net` | M10, multiplayer |
| 2D physics and 2D classes | when 2D starts. Nothing in this layout has to move for it |
| `crates/audio`, game UI | not planned yet |
| `platforms/<os>/` packaging (Gradle, Xcode) | M9, when that platform is worked on |

### Not split, on purpose

- Asset loading lives in `metallic_project` ("everything about files"), not in its own crate.
- Values, registry and data model share `metallic_core`; they always change together.
- No `utils` or `common` crate. Shared code belongs in the lowest crate that needs it.

## Consequences

- 9 library crates, 3 apps and `xtask`. Most issues touch one crate, so agents rarely collide and builds stay incremental.
- A server, a test or a tool can use the object model and the API without building wgpu.
- Adding a platform touches `metallic_platform` (and a render backend for consoles); adding a language adds a crate; adding a class adds one file.
