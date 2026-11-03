# ADR 0006: Project format and project types

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #7

## Context

The engine saves and manages projects. A project is 3D or 2D, each with its own physics, rules and editor tools. **Only 3D is built now.** A project is either a **standalone** game (shipped on its own) or a **platform** game (runs inside the Metallic platform, loads assets from platform servers, shows the platform UI). Projects are edited by people and by agents, often with git, so files should diff and merge.

## Decision

### A project is a folder

```
MyGame/
├── metallic.toml        manifest (below)
├── Scenes/Main.scene    scenes: versioned JSON, one instance per object, stable key order
├── Scripts/*.luau       script sources; script objects in a scene point at these files
├── Assets/              models (glTF/GLB first), textures (PNG/JPEG), other files
└── .metallic/           machine-local cache (asset ids, thumbnails). Never committed or shipped
```

### `metallic.toml`

```toml
[project]
name = "My Game"
description = ""
tags = []
kind = "3d"            # "2d" is reserved and refused until 2D exists
mode = "standalone"    # or "platform"
start_scene = "Scenes/Main.scene"
engine = "0.1"         # engine version the project was last saved with
```

### Scenes

- Every scene has a `format` version. An unsupported version is refused before anything loads.
- Unknown classes or properties (from a newer or older engine) are skipped and reported as warnings, never crash, and do not shift references.
- References between instances are stored by a stable per-scene id, not by position, so a merge does not silently re-point them.
- Default values are not written, which keeps files small and diffs meaningful.
- Loading never leaves the data model half-loaded.

### Settings live in two places

| What | Where |
|---|---|
| Project settings: name, description, tags, standalone/platform, start scene | `metallic.toml` (Studio's Project Settings window edits it) |
| Studio settings: keybinds, theme, layout, editor behaviour | the user's config folder, per user, never in the project |
| Player settings: graphics, sensitivity | the user's config folder, per game (#5) |

### Export

- **Standalone:** the project is packed into one binary file (`game.pak`, read-only, built by `xtask` or Studio) and shipped next to the player executable.
- **Platform:** the project is uploaded to the platform and the player streams it. Designed in M10, but the pak format is shared.

### Mode changes player behaviour, not the file format

Standalone and platform projects use the same files. Only the player's behaviour differs (platform UI, server connection, asset source).
