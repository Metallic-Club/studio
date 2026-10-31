# Issue 106: shared glam dependency

## Goal and ownership

Branch `studio/106-shared-glam`, base exact main
`a0bb1bfc0e84ffd637fe8d1759f4822322601cd8`. Claimed after a fresh ready/claim check;
no blocker or other claim was present. Codex owns this task, with a read-only
backend audit and one new runtime test contributed by studio_core. The root worker
coordinator approved the supported MikkTSpace nalgebra backend tradeoff. Neither
this work nor review constitutes lead/owner acceptance.

## Scope and baseline

All eight direct first-party manifests now use shared workspace glam 0.33.12,
preserving animation serde and renderer bytemuck opt-ins. The standalone shader
research checker and its independent lockfile are updated separately. The Mikk
reference uses only the supported nalgebra feature, so the unchanged meshgen test
does not pull glam 0.15. Its nalgebra 0.26.2 plus five support dependencies are
dev-only; no existing unrelated package version changed. No wgpu upgrade,
production algorithm, existing test, tolerance or lighting golden is changed.

Before changing manifests, actual main/glam 0.30.10 emitted frozen clip and skeleton
RON files using a temporary example. Its source/hash and logs are retained outside
git in `integration-evidence/106-shared-glam`; the example was removed. Both new
legacy-file tests passed against the old dependency before migration. Fixtures
contain nontrivial Vec3/Quat values, parent/child joints and nonuniform scale.

The new active-tree guard rejected the baseline as expected: normal/build had
glam 0.30.10 and 0.33.12; dev added0.15.2; standalone had0.30.10. Candidate active
graphs now each contain only 0.33.12. Optional inactive 0.30/0.31/0.32 entries remain
in nalgebra metadata/lock and are deliberately not counted as active edges.
Eight standard-library Python policy tests pass, including partial/malformed
graphs, old dev edges, strict CPU absence, and a presentation dependency in CPU.

## CPU boundary and validation plan

This main pin does not yet contain pending PR 309's Simulation/presentation split.
The guard reports CPU unavailable on this branch; strict `--require-cpu` fails.
Do not call `--no-default-features` on this main-based runtime a graphics-free
success. Root will prepare a separate identified composition after the executable
source freezes, using the current foundation inputs, for actual CPU graph/gates.
Do not modify the existing integration worktree or absorb PR 309 into this PR.

Core contributed the new renderer-free public-API interoperability test (blob
24fcfda3f29189c0c8c88300b6214362e18eeb1a). Root independently reviewed the graph
guard, fixtures/tests, GPU layout test and CI hooks without an actionable concern.
Next: format and checkpoint; run focused unchanged
animation/asset/meshgen/reference and renderer-layout checks, standalone bins check,
full required workspace/Clippy/license gates with guarded lane116 and four jobs.
Keep actual Lua/GPU windows coordinated, record source hashes and failures, preserve
old fixture provenance, then publish a review PR without own merge. Disk must stay
above the coordinator's 5 GiB pause threshold. Ownership cleanup reclaimed 6.2 GiB;
the anim-only baseline compiled in 11.39s; the legacy target compiled in 1.11s
and its two tests passed.
No migrated Rust execution result is claimed yet.


## First migrated feedback and camera API update

Initial candidate 3d87ca1 passed 125 animation/asset/meshgen checks, including the
unchanged Mikk reference and both legacy fixtures, then the public interoperability
and GPU upload tests. Render compilation exposed five deprecated camera-helper
warnings; the initial private composition also identified example call sites.
Those logs remain tied to the first candidate and are not strict final gates.

A complete source sweep replaces deprecated `look_at_rh`, `perspective_rh` and
`orthographic_rh` call paths in render, modeling examples, the existing shadow
projection test, and the standalone renderer with recommended glam camera APIs.
No assertion, input value, tolerance or golden is changed. Pinned glam 0.33.12
sources confirm the new DirectX/WebGPU constructors use RH, Y-up and depth0..1;
its Vulkan constructors would flip Y and are deliberately not used. Old/new
constructors are separate implementations, not forwarding wrappers, and can
rearrange floating arithmetic. A new exact domain test checks view direction,
up direction, near/far mapping and orthographic corners without warning suppression.
The candidate is refreshed before full/default/private CPU gates.
