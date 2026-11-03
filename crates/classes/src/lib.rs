//! The API: every built-in class and service, one file per class, grouped by domain.
//!
//! Owns: class declarations (`Object`, `Instance`, `WorldObject`, `Light`, `SpotLight`, `Workspace`, ...), their
//! properties, methods, signals and enums, and per-class behaviour that only touches the data model.
//!
//! Must not depend on: render, physics, scripting or platform crates. A class *names* the components it has
//! (`SpotLightEmitter`); the subsystem that implements a component registers itself under that name.
//! See `docs/adr/0002-object-model.md` and `docs/adr/0007-initial-api.md`.
