# 426: portable shadow cascade depth

Claimed by Codex studio_core on studio/426-portable-shadow-depth from exact main
8699192791825af0c8c44edb9d833339440cc508. Research/source pins and primary links
are separately retained at integration-evidence/426-shadow-coordinate-audit.

WGSL requires reciprocal clip-w for the fragment position builtin. Exact pinned
Naga/wgpu30.0.1 passes raw HLSL SV_Position.w through without the required DX12
normalization; upstream4963 and approved/unmerged10733 corroborate that source path.
No local runtime failure is claimed before executing a probe. No upstream code copy,
dependency fork, product debug UI or new backend selector is included.

The baseline adds one perspective depth varying at VOut location8, assigned by both
vs_main and vs_skinned. Shared fragment_view_depth still returns the original
reciprocal builtin for red evidence; both fs_main and fs_toon use it. Shared
shadow_cascade returns the index or cascade count beyond the last split; shadow_factor
uses it and preserves the existing fully-lit out-of-range behavior. Outline vertex
math and prepass remain unchanged, as do all Rust buffer/uniform layouts.

Root delegated only the new Rust numeric probe module and one module declaration to
platform_backend in this same worktree. studio_core owns shader/docs changes, all
commits and Cargo. The test must use production vertex entries and these shared
helpers; after the complete baseline matrix, the fix changes only the depth helper
return, leaving the probe unchanged. Preserve exact source hashes and DX12/Vulkan
adapter/execution outcomes. First establish an actual constant-depth DX12 red,
then cover skinned routes, split boundaries/equality, far range and tilted perspective
interpolation before the fix. No test, build or GPU result exists at this checkpoint.

PR359's texel snapping/scale work touches no WGSL; PR428 directly touches these shaders
and adds another old reciprocal for debug cascades. Coordinate that pending consumer
through public issue/review after actual proof, without editing the owner's branch.

Next: root review and focused GPU baseline/fix under the granted lane124 guard with
four jobs and a disk check. Report before mandatory full gates; do not merge own PR.


Root's source review caught a baseline typing mismatch before execution: the new
cascade helper returns u32, while the existing shadow_filter layer parameter is i32.
The baseline now explicitly casts the already-range-checked index at that call.
This is a source correction before any compile/GPU result, not a runtime product red.

The first numeric probe executed at b3505a5 on DX12 with an NVIDIA GeForce RTX
3060 Ti. The actual production vs_main route produced depth 0.01888068 and cascade
0 at expected view depth 52.96419 and cascade 1. Coverage passed; the depth assertion
failed, and the focused command exited 101 after a successful build. The raw log,
source fingerprints and result receipt are retained outside the repository under
integration-evidence/426-portable-shadow-depth. This establishes the constant-depth
part regression on that adapter only. The probe owner is expanding skinned, split,
far and sloped controls before the shader fix; Vulkan is not yet executed.
