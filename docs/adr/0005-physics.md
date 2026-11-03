# ADR 0005: Physics on a Box3D fork

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #6

## Context

3D physics uses Box3D, the lightweight open-source engine. Metallic forks it so features it lacks (soft bodies, for example) can be added later. **For now the fork stays default.** Forge-Engine used Box3D through the `box3d` and `bevy_box3d` crates; we do not take its integration (the Bevy plugin, its physics bridge, constraint and mover classes).

## Decision

- **Fork:** `Metallic-Club/box3d` is a fork of the upstream Box3D C library and also holds the Rust `-sys` bindings, which compile the C code with `cc`. Upstream changes are merged in on purpose, not automatically. The only changes allowed now are build and binding fixes.
- **Studio repo:** `metallic_physics` depends on that fork by git tag. It contains no C code. This keeps this repo clean and lets the fork be reused.
- **Ownership:** `metallic_physics` owns the Box3D world. It mirrors simulated instances from the data model (anchored, dynamic, collision shape from `Part.Shape` and `Size`), steps at a fixed 60 Hz with sub-steps driven by `metallic_engine`, and writes results back with the change feed muted, so nothing feeds back on itself.
- **First scope (M7):** rigid bodies (anchored and dynamic), collisions, `Workspace:Raycast`, `Touched` / `TouchEnded` (only for parts something listens to), and the basic physical properties on `BasePart`. No joints or constraints yet; they get their own decision when needed.
- **Units: 1 unit = 1 metre.** This is Box3D's native scale and glTF's, so imported models are right-sized with no scale factor. *The owner can overrule this in #9* (Roblox and Forge use 0.28 m studs). It must be fixed before M3, because it affects cameras, lights and assets.
- **2D:** later, with its own physics library and decision. Not part of this fork.
