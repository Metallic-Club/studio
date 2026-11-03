# ADR 0003: Scripting, ready for more languages than Luau

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #4

## Context

Luau is the first scripting language, but the engine must take more languages later. The way to stay ready is to keep every Luau-specific type out of the object model, the classes and the scheduler from day one. Forge-Engine got halfway there: its method calling convention (`MethodCall` → `ScriptValue`) was language-neutral, but its script classes were Lua-named (`LuaSourceContainer`), its scheduler and signal delivery lived in a Luau prelude, and `ScriptValue` used Bevy math types.

## Decision

### Layers

| Layer | Crate | Knows about Luau? |
|---|---|---|
| `ScriptValue`, `MethodCall`, `ScriptError`, method and signal descriptors | `metallic_core` | no |
| Script host: `ScriptLanguage` trait, scheduler, signal connections, budgets, the registry-driven binding description, API dump | `metallic_script` | no |
| Luau backend: VM (mlua), native data types, compiling and resuming | `metallic_luau` | yes |

### The `ScriptLanguage` trait (shape, not final API)

- `name()` and `extensions()`: `"luau"`, `[".luau"]`. The engine picks the backend from the script file's extension.
- `compile(source, chunk_name)` → compiled chunk, with errors that carry line numbers.
- `spawn(chunk, script_instance)` → a thread handle. `resume(thread, args)` → yielded, finished or errored.
- `interrupt` support, so the host can enforce a time budget per resume, plus a memory limit per VM.
- Bindings are generated from the registry. The backend asks `metallic_script` for the class, property, method, signal and enum tables and exposes them; it never hard-codes a class.

### The host owns the semantics

So every language behaves the same:

- the task scheduler (`spawn`, `defer`, `delay`, `wait`, `cancel`), resumption order and per-frame budgets;
- signal connections (`Connect`, `Once`, `Wait`, `Disconnect`) and cleanup when a script is destroyed or disabled;
- which services run which scripts.

### Script classes

```
Instance ─► SourceContainer ─► BaseScript ─► Script | LocalScript
                     └──────► ModuleScript
```

- The base is `SourceContainer`, not `LuaSourceContainer`.
- A script object points at a source file in the project (`Source = "Scripts/Door.luau"`). Its language comes from the file extension.
- `require` returns a `ScriptValue` from a ModuleScript in the same language at first. Calls between languages go through `ScriptValue` later and are not part of M2.

### Data types

- `Vector2`, `Vector3`, `CFrame`, `Color3` and `EnumItem` are defined once in `metallic_core` (on top of `glam`).
- Each backend wraps them natively: userdata in Luau, a struct in another language.

### Safety

- Scripts have no file, network or OS access unless an engine API grants it.
- Platform games are untrusted code: strict sandbox, memory cap, and a time budget per resume.
- **No JIT on iOS or consoles** (those platforms forbid it). Luau's interpreter is the baseline there; Luau's native codegen is an opt-in feature on desktop and Android only.

### API dump

- `metallic_script` can write the whole registry as JSON (classes, members, types, docs).
- It feeds the generated Luau type definitions (autocomplete in Studio and in external editors), the online API docs, and the bindings of any future language.

## Proof that the trait is not Luau-shaped

M2 includes a tiny test-only backend in `metallic_script`'s tests: a scripted call sequence, not a real language. It runs through the same trait. If something can only be done through Luau, the trait is wrong.
