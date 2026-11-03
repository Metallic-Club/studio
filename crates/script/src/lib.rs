//! The language-neutral scripting host.
//!
//! Owns: the `ScriptLanguage` trait that language backends implement, the task scheduler, signal connections, time
//! and memory budgets, the registry-driven binding description and the API dump.
//!
//! Must not depend on: any specific language or VM, or the render, physics, project or platform crates.
//! See `docs/adr/0003-scripting.md`.
