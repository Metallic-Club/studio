# 136 modeling: `metallic_modeling` (modeller core)

Issue #136, branch `studio/136-modeling`, epic #19. Status: implemented; partly tested (see "Verification state").
Builds were frozen by the coordinator (C: disk full) before the full checks could run.

## Goal
An engine-agnostic modeller core for Studio's built-in, beginner-friendly modeller: an editable polygon mesh with
stable ids, vertex/edge/face selection, modelling operations as reversible commands, triangulation to `MeshData`,
basic UVs, glTF/OBJ export, plus an egui demo.

## Done
- Crate `crates/metallic_modeling` (deps: glam 0.30, serde_json, thiserror; dev: criterion, proptest, gltf, eframe,
  egui_kittest, image). No dependency on `metallic_render` or `metallic_meshgen`.
- Mesh: BMesh-style radial edges in id arenas (see "Representation"); journal-based `ChangeSet` undo/redo.
- Ops: transform (with X/Y/Z symmetry editing), extrude faces (region / individual), edges, vertices; inset (region /
  individual, depth); bevel edges with segments (quarter-ellipse profile, vertex polygons, profile merged into the
  closing face for single-edge ends); loop cut (n cuts, n-gon ends get the split points); edge loop / ring select;
  bridge edge loops; merge at centre / by distance; dissolve verts / edges / faces; delete (verts, edges, faces, only
  faces); fill and grid fill (Coons patch); mirror with seam weld; subdivide; Catmull-Clark with sharp-edge creases;
  flip normals, recalculate outside; mark sharp / seam; box and planar UV projection; LSCM unwrap along seams with
  shelf packing; `MeshData` triangulation (ear clipping) with flat / smooth / auto-smooth normals that split at sharp
  edges; glTF (`.glb` and `.gltf` with data URI) and OBJ (n-gons) export; primitives (cube, plane, grid, cylinder,
  UV sphere).
- Booleans: interface only (`ops::boolean`: `BooleanBackend` trait, `BooleanOp`, `TriMesh`, `boolean()`), no backend
  (see "Booleans").
- `Op` enum (plain data), `MeshCommand` (run / undo / redo), `History`, `contextual_extrude`.
- Demo `cargo run -p metallic_modeling --example modeling_demo` (and `-- --screenshot out.png faces|edges|verts`,
  headless through egui_kittest + wgpu).

## Commands that matter
```
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=<worktrees>/_target-lead-shared CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_BENCH_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p metallic_modeling -j 4
cargo bench -p metallic_modeling --bench edit -j 4
cargo run -p metallic_modeling --example modeling_demo -j 4
cargo run -p metallic_modeling --example modeling_demo -j 4 -- --screenshot docs/agents/136-shots/faces.png faces
```

## Next
- Switch `MeshData` and the primitives to `metallic_meshgen` (#35) once merged (field names already match:
  positions, normals, uv0, indices, bounds; meshgen adds tangents, uv1, AO).
- Boolean backend (owner decision below).
- Bevel: grid patches for corners with three or more rounded edges (now one n-gon), clamp-overlap option, profile
  shape parameter; bevel of vertices.
- Bridge face loops (delete the two faces first), bridge with unequal counts, twist/offset UI.
- Symmetry for topology ops (extrude/bevel on both halves); today symmetry applies to transforms, and Mirror builds the
  other half.
- Knife, spin, screw, edge slide, shrink/fatten, proportional editing.
- Studio integration: modeller workspace, Part <-> editable mesh link (live in the game), own gizmos (PR #127 5.5).
- UV editor view; pinning in LSCM; better packing (rotation).

## Open questions (owner)
1. Booleans: allow `BSL-1.0` (needed by `clipper2-rust`, a dependency of the pure-Rust `manifold-rust`, Apache-2.0)
   in `deny.toml`, or accept a CMake + C++ build step for `manifold-csg` (MIT/Apache bindings to manifold3d,
   Apache-2.0)? Recommended: BSL-1.0 is OSI-approved and permissive (no attribution in binaries); allowing it keeps the
   build pure Rust.

## Representation (assistant's call)
Compared, for interactive editing of n-gons:
- **Indexed face-vertex** (faces = index lists, adjacency rebuilt on demand): simplest, any topology, but every
  local query (what is across this edge, what is around this vertex) needs a rebuilt map; ops become O(mesh).
- **Half-edge** (OpenMesh, CGAL `Surface_mesh`): O(1) local traversal and natural for manifold n-gons, but cannot
  hold an edge with three faces ("complex edges"; the OpenMesh paper says complex vertices are handled, complex edges
  are not), and wire edges and loose vertices need special cases. Modelling routinely passes through such states
  (extruding an interior edge, importing messy meshes), so the structure would refuse valid user actions.
- **BMesh radial edge** (Blender's design, read from its developer docs only, never its GPL code): vertices, edges,
  loops (face corners) and faces; a "disk cycle" of edges around each vertex and a "radial cycle" of loops around each
  edge, so any number of faces per edge, wire edges and loose vertices are all fine, while local traversal stays O(1).

Chosen: **BMesh-style radial edge, stored in index arenas instead of linked lists.** Primary data (positions, edges
with sharp/seam flags, faces as vertex loops with per-corner UVs) lives in tombstoned arenas with stable, never-reused
ids; the disk (vertex -> edges), radial (edge -> faces) and pair lookup are derived, kept sorted, and updated only by
three raw slot writers. That makes undo simple and exact: every op runs inside `Mesh::record`, which captures the
before/after state of each touched element (`ChangeSet`); undo writes the befores, redo the afters, in an order
(faces out, edges out, vertices, edges in, faces in) that keeps adjacency consistent. Undo/redo never re-run an op,
and ids come back identical (tested by content hash). Manifold-only algorithms ask `Mesh::vert_fan` for an ordered
fan and fail with `NonManifold` when there is none.

## Research notes
- Beginners and Blender (informal sources, no survey found): mode switching (object/edit, vertex/edge/face), a large
  hidden hotkey surface, the dense UI and the 3D cursor are the most cited hurdles. Demo response: visible mode
  buttons, every operation on a labelled button with its shortcut, a status bar that always says what the mouse and
  keys do now (including during modal drags), cancel with Esc or right-click, and plain-language errors ("Bevel works
  on edges: switch to edge mode (2)").
  - https://you.stonybrook.edu/jadams/2025/08 ; https://en.fmyly.com/article/why-is-blender-so-hard-for-beginners/
- Beginner-first tools: Blockbench (simple, voxel/low-poly), Womp (deliberately limited feature set), Spline (web
  scenes), Plasticity ("CAD for artists", hard surface). Lesson for Metallic: a small, discoverable core (extrude,
  inset, bevel, loop cut, mirror, smooth) covers most game props; advanced tools can hide behind a search palette.
  - https://www.womp.com/blogs/getting-started-with-3d-modeling-why-womp-is-the-best-tool-for-beginners ;
    https://80.lv/articles/beginner-s-guide-to-3d-modeling-in-plasticity
- Half-edge limits: Botsch et al. 2002 (OpenMesh) https://cg.cs.tu-dortmund.de/publications/2002-openmesh.pdf ;
  OpenVolumeMesh notes index-based structures avoid the manifold restriction.
- Blender BMesh design doc (developer.blender.org/docs/features/objects/mesh/bmesh/) returned HTTP 403 to the fetcher;
  the design summary above is from general knowledge of the published BMesh docs, not from Blender code.
- Booleans (checked on crates.io, 2026-10-30):
  - `manifold-rust` 0.19.1, Apache-2.0, pure-Rust port of Manifold 3.5 with an exact and a "robust" engine; depends
    on `clipper2-rust` (**BSL-1.0**, not on our allow-list), `dashu-*` (MIT/Apache), `rustc-hash`, optional rayon.
    Very young (first release 2026-07, several releases a week).
  - `manifold-csg` 0.1.4 / `manifold3d` 0.4 (MIT OR Apache-2.0), bindings that clone and build manifold3d (C++,
    Apache-2.0) with CMake at build time; offline builds need a prebuilt library.
  - Decision: interface now, backend after the owner decides (open question 1). The interface takes and returns
    closed triangle meshes, which both options use.
- UV unwrapping: xatlas (MIT, C++) and the pure-Rust `uvunwrap` crate (MIT, "inspired by xatlas", LSCM + packing,
  optional faer solver) exist; implemented LSCM directly from Lévy et al. 2002 (conformal residual per triangle, two
  pins, CG on normal equations) to avoid a dependency; `uvunwrap` is a candidate if we need its segmentation.
- Catmull-Clark: standard rules (face point = average, edge point = average of ends and adjacent face points, vertex
  = (F + 2R + (n - 3)P) / n), boundary/sharp edges as creases ((a + 6p + b) / 8, corners fixed). Checked against the
  textbook cube result (corner to 5/9).
- All new dependencies are MIT OR Apache-2.0 (glam, serde_json, thiserror; dev: proptest, gltf, eframe, egui_kittest,
  image). egui's `default_fonts` stays off (OFL/Ubuntu-font licence); the demo loads system fonts at run time.

## Verification state (2026-10-30, at the build freeze)
Verified (ran green on this machine):
- `cargo build -p metallic_modeling --lib` and `--example modeling_demo`: no errors, no warnings.
- `cargo test -p metallic_modeling`: lib 4/4, `tests/ops.rs` 39/39, `tests/props.rs` 3/3 (two proptests at 48 cases
  plus a deterministic check that every random op kind succeeds), `tests/uv_export.rs` 12/12 (GLB parsed by `gltf`).
- Headless screenshots rendered (`docs/agents/136-shots/faces.png`, `edges.png`, `verts.png`) and looked at.

NOT verified yet:
- Doctest in `src/lib.rs`: it failed on a borrow error; the fix (split the `Op::Select` into a `let`) is written but
  was not compiled because of the freeze.
- The bevel fix for a single beveled edge ending at a vertex with more than three faces (new test
  `bevel_edge_ending_at_a_valence_four_corner_leaves_no_overhang`) passed together with all other tests, but the
  screenshots were rendered BEFORE that fix and still show the old overhanging flaps at the top corners of the base.
  Re-render them.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: not run.
- `cargo test --workspace -j 4`: not run (only this crate).
- `cargo deny check licenses`: not run (new deps are MIT/Apache; `image` dev-dep has only `png`).
- Bench numbers: not run.
- Rebased on origin/main (d004bc5) with git only. `Cargo.lock` had two conflicts, both pure insertions of different
  packages; resolved by hand as a union. Not yet checked by cargo: the first build may rewrite `Cargo.lock`; commit
  that if it does.
- The interactive demo (window) was not run; only the headless screenshot path.

## Checks to run after the reboot
```
cd <worktrees>/136-modeling   # <worktrees> = the metallic-studio-worktrees folder
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=<worktrees>/_target-lead-shared CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_BENCH_DEBUG=0 CARGO_INCREMENTAL=0
git fetch origin && git rebase origin/main   # only if main moved again
cargo test -p metallic_modeling -j 4                      # includes the doctest
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo test --workspace -j 4
cargo deny check licenses
for s in faces edges verts; do cargo run -q -p metallic_modeling --example modeling_demo -j 4 -- --screenshot docs/agents/136-shots/$s.png $s; done   # then look at them: no flaps at the base's top corners
cargo bench -p metallic_modeling --bench edit -j 4        # record numbers below
cargo clean -p metallic_modeling                          # only this crate, never the shared target
```

## Numbers
Bench not run yet (build freeze).
