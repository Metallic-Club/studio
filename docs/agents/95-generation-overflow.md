# #95 Retire exhausted InstanceId generations

Status: implemented and tested; preparing review.
Branch: `studio/95-generation-overflow`, refreshed onto main `fd164c2`.

## Goal and scope

Preserve stale-handle rejection when an arena slot reaches its last u32 generation. Reuse the clean worktree
whose earlier claim was explicitly released without source edits. This changes only destruction at the boundary;
ordinary free-list reuse, the 64-bit public ID layout and existing tree/signal/history behavior remain intact.

## Done and evidence

- Added three in-crate boundary tests before changing production code. All three failed on the baseline at
  `slot.generation += 1` with the expected debug overflow panic, through the guarded, source-verified lane122 build.
- A checked increment returns reusable slots to the free list only while a fresh generation is representable.
  The last generation remains available for one live allocation; destroying it leaves an empty retired slot.
- Tests cover penultimate-to-last reuse, stale generation-zero and maximum IDs, repeated stale destruction,
  subtree cleanup and signal removal/change history, ordinary free-slot reuse, and retirement in deep clones.
- Public behavior docs and ARCHITECTURE.md describe retirement. No public signature, dependency or ID-layout changes.

## Validation and next

Baseline: `cargo test -p metallic_core --lib generation_tests -j 4`: 0 passed, 3 expected failures.
All required local gates passed. Open the PR and inspect CI; only the
repository lead/owner may merge after review, and this worker must not merge its own PR.

## Focused green and review

- Guarded `cargo test -p metallic_core --lib generation_tests -j 4`: 3 passed after the checked-increment fix.
- Independent read-only engine-research review found no actionable issues in retirement, final-generation reuse,
  copied arena state or subtree signal/history cleanup.
- Guarded workspace `--no-run` prebuild passed with all seven local crate source paths verified.
- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: **215 passed, 0 failed, 0 ignored**.
- Guarded `cargo test -p metallic_core --lib generation_tests --release -j 4`: 3 passed, verifying the boundary
  behavior with optimized overflow semantics as well as debug builds. Release local packages were cleaned first.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed, with the existing unmatched MPL-2.0 allowance warning.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- No runtime budget changes, visual changes, dependencies or new test-only production hooks.
