# ADR 0002: Object model and class system

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #3

## Context

Metallic's API is class-based, like Roblox. A class inherits its parent's properties, methods and signals and adds its own:

```
Object ─► Instance ─► WorldObject ─► Light ─► SpotLight
```

`Light` has `Enabled`, `Brightness`, `Color` and `Range`; `SpotLight` adds `Angle`; a script sees all five on a SpotLight. Forge-Engine solved this well, and we keep its core idea: **a class describes itself once, and everything else reads that description.**

## Decision

### 1. Classes are declarations

Each class is one file with a static `ClassDesc`. The parent is named by string, so class files never depend on each other in Rust. Illustration only; #13 settles the exact API:

```rust
// crates/classes/src/world/lights/spot_light.rs
static PROPS: &[PropMeta] = &[
    PropMeta::number("Angle", 90.0).group("Spotlight").range(1.0, 179.0).visual()
        .desc("Full angle of the cone, in degrees."),
];

pub static CLASS: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
    .describe("A cone of light.")
    .creatable()
    .components(&["SpotLightEmitter"])
    .props(PROPS);
```

From that one file, the class is available to `Instance.new("SpotLight")` in any script language, to scene files, and to Studio's Insert menu and Properties panel. None of those systems change when a class is added.

### 2. The registry

- At startup the registry collects every `ClassDesc` and resolves each inheritance chain: parents first, then the child's additions.
- It refuses broken data with a clear error: an unknown parent, a cycle, a duplicate property that does not mark itself as an override, a component nobody declares.
- A child may change an inherited default (`defaults`) or hide an inherited member (`hide`).
- Classes are found by a `build.rs` in `metallic_classes`, which generates the list of class files. This works on every target. Linker tricks (`inventory`, `linkme`) are not used because they are unreliable on mobile and consoles.

### 3. The data model is the only source of truth

- `DataModel` owns every instance: its class, name, parent, children and property values.
- Ids are an index plus a generation, so a stale id is detected and never mistaken for a new object. Ids are never reused.
- Every write goes through one function. It checks the type, clamps to the declared range, rejects NaN, marks dirty bits, runs `on_changed` hooks and queues `Changed`. Writing the value a property already has does nothing.
- Renderer and physics are **mirrors**. They read dirty instances once per frame or step, and nothing is stored only on their side.

### 4. Behaviour: shared logic in the base class, specifics in components

- **API sharing comes from inheritance:** properties, methods and signals.
- **Behaviour sharing comes from components:** named bundles of behaviour (`Transform`, `SpotLightEmitter`) that a class lists. The **subsystem implements the component**. `metallic_render` registers "how to draw a `SpotLightEmitter`", `metallic_physics` registers "how to simulate a `Rigidbody`".
- This is the main change from Forge. There, `core/hooks.rs` named Bevy, render and physics types, so the object model could not compile without them. Here the object model compiles alone, and a headless server simply has no render implementations.
- Optional per-class hooks (`on_created`, `on_prop_changed`, `on_parent_changed`, `on_destroying`, `on_started`) stay in the class file.

### 5. Methods and signals are language-neutral

- A method is `fn(&mut MethodCall) -> Result<ScriptValue, ScriptError>`. `ScriptValue` is a plain Rust enum (nil, bool, number, string, Vector3, CFrame, Color3, instance, enum item, list, map). See #4.
- A signal is declared with `SignalDesc`, fired from Rust, queued, and delivered after the change that caused it (deferred). Only instances that something listens to queue anything.

### 6. Property metadata drives Studio

- `PropMeta` holds the type and default, flags (transform, visual, physics, runtime-only, not saved, read-only) and inspector hints (label, group, order, range, widget, conditional visibility, description, old names).
- Studio's Properties panel, Insert menu and API docs are generated from it. Nothing is hand-listed twice.

### 7. Services

- A service is a class with flags: `service(order)`, `persisted`, `runs_scripts`, `hidden_in_explorer`.
- The root is `DataModel` (`game` in scripts). Its services are created once and can never be re-parented.

### 8. Threads

- The data model is single-threaded and owned by the runtime loop. Scripts see one consistent world in a fixed order.
- Render and physics may run on other threads by working on data extracted from the dirty list. They never hold references into the tree.

## Not taken from Forge

Its Roblox compatibility shims (`PVInstance`, aliases kept only for Roblox scripts, `unsupported(...)` stubs). Its physics bridge, UI classes and its own class families (constraints, movers, effects, audio effects, characters). Its Bevy dependency inside the object model.
