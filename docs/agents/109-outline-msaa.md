# #109 Post-edge inputs with MSAA

Status: implemented and tested; preparing review.
Branch: `studio/109-outline-msaa`, initially based on main `12a6a6b`; rebased onto `fd164c2` before publication.

## Goal and decision

Correct the styles research's implication that storing a multisampled depth attachment makes it readable by
`texture_depth_2d`. Align the older research with the unified Look design's existing decision 13: a separate 1x
depth/normal prepass when EDGE and MSAA are enabled, and stored opaque 1x depth plus normal/mask MRT without MSAA.

## Evidence and changes

- Verified wgpu 30.0.1 primary API docs: depth/stencil attachments have no resolve target, while color attachments do.
- Verified the current WGSL specification's distinct single-sample/multisampled depth types and indexed loads.
- Added an explicit resource/sample-count table, matching camera/size/coverage rules, alpha-tested geometry behavior,
  background-mask clearing, target lifetime and separate output requirements.
- Updated the affected Toon/Low-poly and general EDGE Look rows, hook 4, frame prepass description, shader comments,
  and a cross-reference in the unified spec. Existing exact Look values and scene renderer code are unchanged.
- Explained the extra prepass cost, 1x silhouette coverage limit and deferred coherent depth/normal resolve alternative.
- Defined the future complete GPU execution/readback acceptance for scene sample counts 1 and 4. Existing research
  pipeline creation is not treated as evidence that an MSAA frame executes.

## Validation and next

The shader changes are comments only; no renderer hook, public interface, dependency, visual UI or GPU-cost claim.
Publish the PR and inspect CI. The historical research remains design evidence, not a renderer implementation claim.
Only the repository lead/owner may merge after review and CI; do not merge this PR here.

## Completed validation

- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: **212 passed, 0 failed, 0 ignored**.
  The guarded prebuild cleaned the previous checkout's local packages; logs verify all seven local source paths.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed, with the existing unmatched MPL-2.0 allowance warning.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Local Markdown references resolve. Removing line comments/whitespace leaves the outline WGSL tokens byte-identical.
- Independent read-only engine-research review found no actionable technical issues and rechecked primary API sources.
- No GPU integration run or screenshot: this change adds no renderer hook and changes no executable shader tokens.
- Main's intervening `fd164c2` change adds only input-action design/progress/HTML files; no Rust, manifests or tests changed
  since the completed gates. CI will run on the published final head.
