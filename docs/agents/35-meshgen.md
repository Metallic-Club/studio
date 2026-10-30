# Task 35: metallic_meshgen (better default shapes)

Branch `studio/35-meshgen`, crate `crates/metallic_meshgen`. Pure Rust, depends only on `glam` and `thiserror`
(`bytemuck` behind the `bytemuck` feature). Nothing else in the workspace uses it yet: Task 3 (render) adopts it
after merge.

## What it does
- 17 shapes: Block, Wedge, CornerWedge, Ball, Cylinder, Cone, Pyramid, TriangularPrism, Torus, Pipe, Ramp,
  Stairs, Arch, RoundedBlock, Capsule, Hemisphere, HexPrism. `Shape::from_name` also accepts Roblox names
  (`"Sphere"` gives `Ball`).
- Bevels in studs (round or chamfer, default 0.05 round), clamped on thin parts so they never eat a face.
- Normals: smooth (exact surface normals, hard edges between faces), flat, or crease angle.
- MikkTSpace tangents (checked against the `mikktspace` crate in a test), world-scale `uv0` (1.0 = 1 stud at
  every size), per-chart `uv1` (0..1), baked AO for concave shapes (convex shapes get 1.0 for free).
- LODs: Low / Medium / High. Segment counts also grow with the real size, so big curves stay smooth.
- `collider_hint`: box / sphere / cylinder / cone / capsule / convex hull, or a compound of convex pieces
  for concave shapes (Torus, Pipe, Ramp, Stairs, Arch).
- `MeshKey` (quantised to 1/4096 stud, unused parameters zeroed), `MeshCache` (one `Arc<MeshData>` per key),
  `MeshKey::stable_hash` for on-disk caches (salted with `MESHGEN_VERSION`).

Conventions (units, axes, winding, orientation of each shape) are in the crate docs at the top of
`src/lib.rs`. The one that matters most for the renderer: **meshes are built at their real size**, so the
instance transform carries rotation and translation only, never scale.

## Adopting it in metallic_render (Task 3)
1. Add `metallic_meshgen = { path = "crates/metallic_meshgen", features = ["bytemuck"] }` to
   `[workspace.dependencies]` and depend on it from `metallic_render`.
2. Per Part: `MeshDesc::new(shape, size)` (plus bevel / params from the Part), then `desc.key()?` and look the
   key up in a `MeshCache` (or a GPU-buffer map keyed on `MeshKey`).
3. Upload `mesh.interleaved()` with `bytemuck::cast_slice`. `Vertex` is 60 bytes: position @0, normal @12,
   tangent @24, uv0 @40, uv1 @48, ao @56. Index format `Uint32`, `FrontFace::Ccw`, back faces culled.
4. Pick `Lod` by screen size; LOD only changes curved shapes, so flat ones share one key across LODs.

## Commands
Build with `-j 2` while other agents share the machine, and only when C: has at least 4 GB free.
- Tests: `cargo test -p metallic_meshgen -j 2`. `MESHGEN_SEED=<n>` reruns the random-size test with another
  seed.
- Contact sheet of every shape (CPU rasterizer, no GPU):
  `cargo run -p metallic_meshgen --example preview -j 2 -- out.png [--lod low] [--bevel 0.2] [--chamfer] [--flat]`
- OBJ files for Blender: `cargo run -p metallic_meshgen --example export_obj -j 2 -- out_dir [--lod low]`
- Benchmarks: `cargo bench -p metallic_meshgen -j 2`.

## Status at handoff (2026-10-30): checks NOT run yet
All cargo builds were frozen (C: full, page file could not grow) until the owner reboots. This commit is
**unverified**. None of the following has run against it:
- The crate tests on this exact tree. The last session was chasing a failure in
  `random_sizes_bevels_and_params_stay_valid` that it had not confirmed fixed. The repro it was using:
  `MeshDesc::new(Shape::Pyramid, [32.95206, 0.4850101, 16.605331]).with_bevel(Bevel::round(0.04650396)).with_lod(Lod::Medium)`.
  Look at the triangles around the apex, near (0.0020, 0.2424, 0.0027). A flat, wide pyramid with a round
  bevel almost as tall as the apex region is suspected. If the test fails, turn this repro into a named test
  in `tests/shapes.rs`.
- `examples/preview.rs` and `examples/export_obj.rs` have never been compiled or run, so no preview PNG has
  been made yet.
- Clippy, the `bytemuck` feature build, the benchmarks, and every workspace-wide check.
- `cargo deny check licenses` for the new dependencies (glam, thiserror, bytemuck; dev: criterion, png,
  mikktspace).

After the reboot, with at least 4 GB free on C:, run in this order (stop at the first failure):
```sh
cargo test -p metallic_meshgen -j 2
cargo test -p metallic_meshgen --all-features -j 2
cargo clippy -p metallic_meshgen --all-targets --all-features -j 2 -- -D warnings
cargo run -p metallic_meshgen --example preview -j 2 -- docs/agents/35-meshgen-preview.png
cargo run -p metallic_meshgen --example preview -j 2 -- docs/agents/35-meshgen-preview-low.png --lod low
cargo run -p metallic_meshgen --example export_obj -j 2 -- target/meshgen-obj
cargo bench -p metallic_meshgen -j 2      # copy the per-shape times into "Timings" below
cargo fmt --all --check
cargo test --workspace -j 2
cargo clippy --workspace --all-targets -j 2 -- -D warnings
cargo deny check licenses
```
Open the preview PNGs and check each shape's orientation against the crate docs: wedges rise toward +Z,
the CornerWedge peak sits above (+X, +Z), and round extrusions run along X. Commit the PNGs if they look
right.

## Timings
Not measured yet (see above).

## Known limits
- `MeshCache` is not thread-safe on its own (wrap it in a `Mutex` or keep one per thread).
- Sharp angles recede slightly under a round bevel: a rounded knife edge sits a little inside the box.
- AO is a per-vertex bake from the mesh itself; it does not see neighbouring Parts.
