//! The renderer: a mirror of the data model, drawn with `wgpu`.
//!
//! Owns: the GPU device and surfaces (from a raw window handle), meshes, materials, lights, shadows, culling,
//! batching, graphics quality presets and dynamic resolution. Implements the render side of components
//! (`SpotLightEmitter`, ...).
//!
//! Must not depend on: the platform, project, scripting or physics crates. The engine hands it a window handle and
//! decoded assets. See `docs/adr/0004-runtime-rendering-platform.md`.
