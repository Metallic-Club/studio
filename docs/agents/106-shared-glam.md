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
sources confirm the new DirectX/WebGPU constructors use RH, Y-up and depth 0..1;
its Vulkan constructors would flip Y and are deliberately not used. Old/new
constructors are separate implementations, not forwarding wrappers, and can
rearrange floating arithmetic. A new exact domain test checks view direction,
up direction, near/far mapping and orthographic corners without warning suppression.
The candidate is refreshed before full/default/private CPU gates.


## Completed main-based gates

Final tested executable source: `36f350e6d16d15ccb764efb55983e9b51f90131e`, pinned
main a0bb1bfc0e84ffd637fe8d1759f4822322601cd8. Final publication appends prose/evidence
only. Backend independently reviewed both the original migration/provenance and
camera-call follow-up without an actionable concern. No source changed during
these final gates.

- Full workspace no-run: passed in 3m21s. Strict full all-target Clippy: passed in
  1m14s. The dev-only nalgebra 0.26.2 future-compatibility note remains visible;
  strict Clippy success is not a claim that all dependency notices are absent.
- Full locked workspace serial test command: 1,852 passed, zero failed, eight
  upstream ignores across 140 result groups including doc tests. All three full
  Parkour routes passed, including keyboard, gamepad and touch. New legacy-file,
  public math interoperability and upload/camera regressions passed. No existing
  assertion, tolerance or lighting golden was changed; only the deprecated
  constructor call in the shadow test was modernized.
- Initial focused animation/asset/meshgen target passed 125 cases, including the
  unchanged Mikk reference. The public interop case passed, and corrected
  camera/upload target passed both cases without first-party warnings.
- Standalone shader checker: guarded `check --bins --locked -j 4` passed in 49.17s
  under its explicit separate workspace owner. Its manifest and lock use 0.33.12;
  no standalone GPU rendering was executed. Its pre-existing compact formatting
  produces differences in an additional non-required standalone rustfmt check;
  it was preserved. The required workspace format check passes.
- Eight Python policy cases pass. Real workspace normal/build, workspace dev,
  and standalone active graphs each have only glam 0.33.12. CPU is explicitly
  unavailable on this main-based source; actual strict `--require-cpu` exits 1 as
  intended, not a CPU-only pass.
- Licenses, workspace format and pinned-base whitespace checks pass. All builds
  used guarded lane116 and four jobs. Actual Lua/GPU execution used its assigned
  sole window. The lane is now idle and released, with no retained performance
  binary requirement.

The exact future-compatibility report was inspected and retained. nalgebra 0.26.2
has trailing semicolons in expression-position macros in base/construction.rs
(lines 888, 891, 894, 897, 900, expanded by transpose_array/componentwise_constructors_impl),
linked to Rust issue 79813. It is accepted by pinned Rust 1.99.1 but may become a
hard error later. No blanket warning allowance or vendored dependency patch was
added. This documented dev-reference tradeoff does not affect the normal runtime
graph; it is not an unqualified future-toolchain compatibility claim.

Raw logs, old generator/fixtures and SHA-256 provenance are kept separately under
external `integration-evidence/106-shared-glam`, using temporary log prefix
`metallic-106-`. The offline lock resolution first found an uncached `paste`
package; normal online resolution added only the six required backend packages,
removed glam 0.15 and did not update unrelated existing versions.

## Separately identified CPU compatibility composition

Root's private composition `b44db1675998d6c22af1e4c04dcf466a8496f74f` contains this candidate and the
pending engine foundations, including PR 309's real presentation/Simulation split.
Its strict four-graph policy passes (including a real CPU-only graph), existing
CPU presentation policy passes, full/CPU prebuilds and strict Clippy pass. This is
separate supporting evidence; it does not deliver CPU mode in the main-based PR.
Actual combined execution is pending at publication. Update only PR/external
metadata when those results arrive, keeping this source and prior evidence fixed.
No own merge, force push, main push or remote-machine action was performed.


## Compatibility refresh on main 995f880

The previous main a0bb1bf/e993e49 results and private b44 composition above remain
historical and unchanged. New main pin:
`995f880ced789284354ccbb1e64b6bf841382abc` (shading/material table, editor wiring,
ModuleScript/require). Object-only merge is textually clean, but source review
identified an actual ABI compile conflict in the added upload regression:
InstanceRaw.material is now [u32;4], not the old float roughness/metalness payload.

Normal append-only main merge preserves all upstream renderer, shader, material
table, editor and require implementations. The added upload test retains every
matrix/color/camera numerical assertion and the 96-byte/0/64/80 offsets, while
initializing a valid material row and checking the 80..96 region as exact u32
words [row,flags,material ID,reserved]. No integer header is reinterpreted as float
material parameters. Shared glam 0.33, legacy fixtures, public interoperability,
CPU-unavailable policy and camera call safeguards are retained. No wgpu upgrade,
assertion relaxation, golden change, force push or own merge is introduced.

Cargo is held pending the coordinator's build-slot release. This checkpoint is
source-only; no refreshed execution or Clippy result is claimed for 995f880 yet.
