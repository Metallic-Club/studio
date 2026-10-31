# #103: Ball and Cylinder render scale matches physics

## Goal
The renderer drew Ball and Cylinder stretched to the full `Size`; physics (`build_collider` in
`crates/metallic_physics/src/sync.rs`) and Roblox use a sphere of the smallest axis and a cylinder
along X (length `Size.X`, diameter `min(Size.Y, Size.Z)`). Make the drawn shapes match.

## Done
- `crates/metallic_render/src/scene.rs`: private `mesh_scale(kind, size)` used by `read_part`.
  Sphere = uniform `min(X, Y, Z)`; Cylinder = `(X, min(Y,Z), min(Y,Z))`; Box/Wedge unchanged.
  The unit cylinder mesh (`meshes.rs`) already runs along local X, so no rotation change.
- Tests: `ball_render_scale_matches_physics`, `cylinder_render_scale_matches_physics`,
  `block_and_wedge_keep_full_size` (expected values written out from `sync.rs`; render does not
  depend on physics). Both shape tests fail without the fix.

## Next
- Shipped in batch PR `studio/1e-batch-20261031-1` with #102. `docs/agents/m1-shots/stress.png` regenerated there
  (`cargo run -p metallic_render --example stress -- --screenshot <png>`, RX 6650 XT): balls are round now.

## Commands
```
cargo test -p metallic_render -j 4
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Open questions
- None.

## Gate (2026-10-30, rebased on origin/main 96aceb0)
- render lib 26 passed; clippy, `deny check licenses`, `fmt --check` clean.
- Full workspace under load: headless GPU binary crashed once (STATUS_HEAP_CORRUPTION), then passed alone
  (3 passed); metallic_script `api` / `vm_round2` hit script execution-time budgets under load, pass alone.
