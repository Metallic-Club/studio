//! The object model every other crate stands on.
//!
//! Owns: the value types and math (`Id`, `Value`, `Vector3`, `CFrame`, `Color3`), the language-neutral calling
//! convention (`ScriptValue`, `MethodCall`), the class registry and its descriptors (`ClassDesc`, `PropMeta`, ...),
//! the `DataModel` instance tree and signals.
//!
//! Must not depend on: any other Metallic crate, or any GPU, window, scripting VM or OS-specific code. Only small
//! portable crates are allowed (`glam`, `serde`, `thiserror`). See `docs/adr/0002-object-model.md`.
