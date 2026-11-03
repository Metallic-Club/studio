//! The engine runtime: the one crate that knows every subsystem, and the one apps depend on.
//!
//! Owns: the game loop (fixed physics step plus render step), `RunService`, and the wiring of the registry,
//! scripting backends, renderer, physics and project loading.
//!
//! Features: `render`, `physics` and `luau` (all on by default). A dedicated server builds with
//! `--no-default-features --features physics,luau`. See `docs/adr/0001-project-layout.md`.
