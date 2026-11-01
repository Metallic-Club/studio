# #105 Renderer::sync uploads

Status: implemented and tested locally; not pushed, no PR yet.
Branch: `studio/105-sync-uploads`, stacked on `studio/108-renderer-small-items` (`a2a5cde`). Session metallic-1e-work.
Merge #108 first, or open this PR against that branch.

## Goal

Make the `Renderer::sync` doc match the behaviour, upload only the changed instance ranges when a few parts change,
and batch the blended draws if that is simple and safe. Acceptance: a test that counts uploaded bytes for one change.

## Done

- **Partial uploads.** `InstanceBuffer` keeps a CPU mirror of what the GPU buffer holds. `upload` diffs the new list
  against it byte for byte (`changed_ranges`; runs less than `MERGE_GAP` = 4 instances apart become one write) and
  writes only those ranges; a buffer that had to grow is written in full. Same for the blended buffer, so a still
  camera uploads nothing for transparent parts either. When the set or order changes (add/remove, shape change,
  Transparency crossing 0) the later entries shift and most of that buffer is written again, which the issue allows.
- **Counter.** `#[doc(hidden)] Renderer::uploaded_instance_bytes()` (integration tests cannot see `cfg(test)`
  items; `force_ldr_scene_target` uses the same pattern).
- **Blended draws batched.** `draw_blended_instances` binds the blended buffer once and issues one `draw_indexed`
  per run of same-shape neighbours in the far-to-near list, with the run as the `first_instance` range
  (`kind_runs`). Safe: primitives (and instances) inside one draw are rasterised and blended in submission order on
  every backend, and wgpu's GL backend emulates `first_instance` for direct draws (no feature needed; only indirect
  draws need `INDIRECT_FIRST_INSTANCE`). The shadow pass uses the same function.
- **Docs.** `Renderer::sync` doc rewritten (journal check, uploads in the next `render`, when most of a buffer is
  rewritten); ARCHITECTURE.md metallic_render bullet.
- **Tests.** Unit: `changed_ranges_cover_exactly_what_differs`, `blended_runs_group_same_shape_neighbours_in_order`.
  GPU (`tests/renderer_state.rs`): `one_changed_part_uploads_one_instance`: 200 parts (20 transparent); the first
  frame uploads 200 x 96 bytes, an unchanged frame 0, a colour change on one opaque part 96, on one transparent
  part 96.

## Deferred

- Nothing from the issue. Not done: sorting the blended list by shape within equal depth to make longer runs (would
  change the draw order of overlapping parts).

## Next

Rebase after #108 merges; the lead opens the PR (`Closes #105`).

## Checks run (AMD RX 6650 XT, Windows 11)

- `cargo test -p metallic_render -j 4`: lib 46 passed; headless 4; lighting_presets 4; post_chain 7;
  renderer_state 3; all 0 failed.
- `cargo test --workspace -j 4 --no-fail-fast`: 1618 passed, 1 failed, 7 ignored. The failure was
  `metallic_player` `smoke::parkour_smoke_scene_passes_headless`, a Luau "Script timeout: exhausted allowed
  execution time" under load (headless, no rendering). It passes alone (`cargo test -p metallic_player --test smoke`:
  3 passed). The same wall-clock flake hit `metallic_script` tests on the #108 branch.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: exit 0.
- `cargo deny check licenses`: `licenses ok`.
- `cargo fmt --all --check`: exit 0.
- Built with `CARGO_TARGET_DIR` set to the #108 worktree's `target` to reuse its dependency builds.

## Commands

```
cargo test -p metallic_render -j 4
cargo test -p metallic_render -j 4 --test renderer_state one_changed_part -- --nocapture
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all --check
```
