//! The Luau backend: the first implementation of the `ScriptLanguage` trait from `metallic_script`.
//!
//! Owns: the Luau VM, native wrappers of the engine's data types, compiling and resuming Luau threads.
//!
//! Must not depend on: anything but `metallic_script` and `metallic_core`. A second language is a new crate next to
//! this one. See `docs/adr/0003-scripting.md`.
