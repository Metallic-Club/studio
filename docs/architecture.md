# Architecture

This is the map. Each section links the decision record (ADR) that explains *why*.

## One idea

**A class describes itself once, and everything else reads that description.** Scripts (in any language), scene
files, Studio's Insert menu and Properties panel, and the API docs all come from the same class registry. Adding a
class is one file; none of those systems change. ([ADR 0002](adr/0002-object-model.md))

## Crates

| Crate | Folder | Owns |
|---|---|---|
| `metallic_core` | `crates/core` | values and math, ids, the language-neutral calling convention, the class registry, the `DataModel` tree, signals |
| `metallic_classes` | `crates/classes` | the API: one file per class and service |
| `metallic_script` | `crates/script` | the language-neutral scripting host (`ScriptLanguage` trait, scheduler, bindings, API dump) |
| `metallic_luau` | `crates/luau` | the Luau backend |
| `metallic_render` | `crates/render` | the `wgpu` renderer and graphics quality settings |
| `metallic_physics` | `crates/physics` | the bridge to the Metallic Box3D fork |
| `metallic_project` | `crates/project` | project manifest, scenes, assets, `game.pak` |
| `metallic_platform` | `crates/platform` | window, input, paths, OS glue (the only crate with OS-specific code) |
| `metallic_engine` | `crates/engine` | the runtime loop; wires everything above; what apps depend on |
| `metallic_player` | `apps/player` | runs games; a library (embedded by Studio and Launcher) plus a binary |
| `metallic_studio` | `apps/studio` | the editor (desktop only): egui on its own winit loop and the engine's device ([ADR 0009](adr/0009-studio-foundation.md)) |
| `metallic_launcher` | `apps/launcher` | the website shell that runs Player and Studio |

## Dependency rules

([ADR 0001](adr/0001-project-layout.md))

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

- Dependencies point down only. Subsystems never depend on each other; `metallic_engine` joins them.
- `metallic_core` depends only on small portable crates.
- Classes *declare* components (`SpotLightEmitter`); subsystems *implement* them by name. That is why the object
  model builds without a GPU, and why a server is simply the engine without the `render` feature.
- OS-specific code lives only in `metallic_platform` (and app packaging). Console code lives in private repos behind
  the platform and render seams.

## Data flow

([ADR 0002](adr/0002-object-model.md))

1. The **`DataModel`** owns every instance and property. It is the only source of truth.
2. Every write goes through one function. It validates the value, marks dirty bits and queues signals.
3. Each frame, **subsystems mirror** what changed: the renderer redraws dirty instances, and physics syncs dirty
   bodies before its fixed step and writes results back.
4. **Scripts** reach the data model only through the registry's methods and properties, with values passed as
   `ScriptValue`. ([ADR 0003](adr/0003-scripting.md))

## Runtime, rendering, platforms

There is no Bevy. The engine runs its own loop on `winit` and renders with `wgpu`; consoles plug in private backends.
Graphics quality presets and dynamic resolution are part of the engine.

The loop is `metallic_engine`'s `Engine`: each frame runs input, then `RenderStepped` (clients only),
`PreSimulation`, the fixed simulation steps (60 Hz accumulator, capped catch-up), `PostSimulation`, `Heartbeat`, and
last the render hook. Its crate docs give the table and the time rules; a dedicated server runs the same loop with
`Engine::tick` and no window (the `render` feature off, so no winit).

How the renderer stays fast at 100k parts without giving up quality or customization (budgets, stable GPU slots,
layered culling, LOD, clustered lights, cached shadows, no hitches) is fixed by
[ADR 0008](adr/0008-rendering-performance.md).
([ADR 0004](adr/0004-runtime-rendering-platform.md))

## Physics

Box3D, through a Metallic-Club fork. 1 unit = 1 metre. ([ADR 0005](adr/0005-physics.md))

## Projects

A project is one `.mtproj` file, a zip archive holding `metallic.toml`, JSON scenes, `.luau` scripts, assets and the
asset index. Exported games are packed into `game.pak`. Only 3D exists for now.
([ADR 0006](adr/0006-project-format.md))

## The starting API

The first classes and services, and what is deliberately left out, are listed in
[ADR 0007](adr/0007-initial-api.md).

## Adding a class

A class is one file in `crates/classes/src/`. Take `SpotLight`, in
[`world/lights/spot_light.rs`](../crates/classes/src/world/lights/spot_light.rs):

```rust
pub static COMPONENT: ComponentDesc = ComponentDesc::new("SpotLightEmitter")
    .describe("Lights a cone that opens Angle degrees along the light's forward direction (-Z), up to Range metres.")
    .requires(EMITTER_REQUIRES); // ["Transform"]

static PROPS: &[PropMeta] = &[PropMeta::number("Angle", 90.0)
    .range(1.0, 180.0).widget(Widget::Slider).visual().group("Spotlight").order(0)
    .desc("How wide the cone is: the full angle from one edge to the other, in degrees.")];

pub static CLASS: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
    .describe("A cone of light shining forward from the light, like a torch or a stage light.")
    .category(CATEGORY) // "Lights", shared by every light
    .creatable()
    .components(&["SpotLightEmitter"])
    .props(PROPS);
```

- **`ClassDesc`** names the class and its parent (by string, so class files never import each other), describes it
  for Studio and the API docs, puts it in the Insert menu (`category`), lets scripts create it (`creatable`), and
  lists its components and its own properties.
- **`PropMeta`** declares one property: its type and default, the range writes are clamped into, the change flag
  (`visual()`: the renderer redraws it), where Studio shows it (`group`, `order`, `widget`), and its description.
  Defaults are in metres (ADR 0005).
- **Inheritance** gives it the rest. `Light` ([`light.rs`](../crates/classes/src/world/lights/light.rs)) declares
  `Enabled`, `Brightness`, `Color`, `Range`, `Shadows` and `CFrame` once; the registry copies them into every
  subclass first, so they keep the same `PropId` in `SpotLight`, `PointLight` and `DirectionalLight`, and a script
  sees all of them plus `Angle`. A subclass can change an inherited default (`defaults`) or hide a member
  (`DirectionalLight` hides `Range`; the slot stays, only the name disappears). Shared constants for the family
  (`CATEGORY`, `EMITTER_REQUIRES`) and helpers (`world_cframe`) live in the parent's file.
- **The component** is only a name and a description here. The renderer (M3) will register "how to draw a
  `SpotLightEmitter`" under the same name and read `Angle`, `Range` and the rest from the data model. A headless
  server simply has no implementation, and the class still works. `requires` makes the registry check that every
  class with the emitter also has `Transform`, which `Light` provides.
- **Discovery is automatic.** `crates/classes/build.rs` scans `src/` for `pub static CLASS`, `COMPONENT` and `ENUMS`,
  so this file is the only change: no list to edit, no registration call. Folders become modules
  (`metallic_classes::world::lights::spot_light`).

Enums go in the file of the class that owns them (`pub static ENUMS`, such as `Material` in `base_part.rs`). Every
class, member, component and enum needs a description; a test fails otherwise.
