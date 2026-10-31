# Shared math version

The engine's first-party math consumers use workspace glam 0.33.12. The direct
consumers are metallic_anim, metallic_asset, metallic_audio, metallic_meshgen,
metallic_modeling, metallic_render, metallic_runtime and metallic_studio.
Animation retains its serde feature; rendering retains bytemuck. Public animation
and renderer re-exports, asset transforms and audio vectors therefore share the
same active math crate as Rapier/Parry. The core's f64 Vec3 and explicit conversion
to f32 physics coordinates remain unchanged; version deduplication does not remove
that intended precision boundary.

The standalone `docs/research/style-shaders/check` package has an independent
manifest/lockfile and pins the same version. It is checked separately from the
workspace. This change does not upgrade wgpu or change shader code.

## Active dependency policy

Run `python -I tools/test_glam_graph.py` and `python -I tools/check_glam_graph.py`.
The policy reads actual locked Cargo trees for workspace normal/build dependencies,
workspace normal/build/dev dependencies, and the standalone shader checker. Each
must have only glam 0.33.12. The root lockfile can still contain inactive optional
glam 0.30/0.31/0.32 entries from nalgebra's optional conversion features; their
presence in metadata or a lockfile is not evidence of an active duplicate.

The guard also checks a CPU-only runtime tree when the source declares the
Simulation/presentation split. On an earlier source it reports CPU unavailable,
not a CPU-only pass. `--require-cpu` makes that absence an error. When available,
the CPU tree must retain core/physics/scripts and exclude renderer, wgpu, window,
GUI and gamepad dependencies. The standalone research checker intentionally uses
wgpu and is never presented as a CPU-only graph.

MikkTSpace 0.3's unchanged meshgen tangent-reference test uses the supported
nalgebra backend rather than its glam 0.15 backend. This adds nalgebra 0.26.2 and
its older support crates only to the dev graph. It does not claim there is a
single nalgebra version, and it does not replace the production tangent algorithm
or weaken the reference comparison. The upstream backend and array-based Geometry
API are documented in the [published MikkTSpace source](https://github.com/gltf-rs/mikktspace/tree/c1e6f163ec0ac9e77bffcf6144c9b09ab75dc412).

## Compatibility checks

Existing math, animation, asset, tangent-reference, upload-layout and movement
tests and tolerances remain unchanged. The migration is not a claim of bit-exact
floating-point results for every operation: glam's [0.33.12 changelog](https://github.com/bitshifter/glam-rs/blob/0.33.12/CHANGELOG.md)
includes SIMD rounding, inverse/determinant and fused multiply-add changes.

Additional tests load fixed clip/skeleton files emitted by the pre-migration
0.30.10 build. They assert nontrivial vector/quaternion values, hierarchy,
nonuniform scale and emitted field/component order; they do not regenerate the
fixtures with the new dependency. Fixture hashes and provenance live alongside
the files. A CPU-only public-API test joins runtime, animation and asset types,
while an upload test checks nontrivial scale/rotation/translation, column order,
the bytemuck opt-in and the renderer's 96-byte instance offsets without a GPU.

Validation results and any remaining integration limits are recorded in
[the issue106 progress note](agents/106-shared-glam.md). The main-based change
does not absorb pending Simulation, networking or editor foundations.
