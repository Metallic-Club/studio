//! 3D physics: the bridge between the data model and the Metallic fork of Box3D.
//!
//! Owns: the physics world, mirroring simulated instances, the fixed step, write-back, raycasts and touch events.
//!
//! Must not depend on: the render, scripting, project or platform crates. See `docs/adr/0005-physics.md`.
