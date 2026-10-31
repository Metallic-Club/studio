# #426 Portable view depth for the shadow cascade choice

Status: implemented, tested.

## Problem

`scene.wgsl` (`fs_main`) and `skinned.wgsl` (`fs_toon`) chose the shadow cascade from `1.0 / in.clip.w`. The
fragment `position.w` builtin is not portable:

- On Vulkan it is 1/w, so the result is the linear view depth.
- On DX12 it is w, so the result is 1/w: a tiny value.

On DX12, every pixel therefore picked cascade 0, and the shadow fade never started. That covers Windows players on
DX12, and CI on WARP. Shadows beyond the first split (about 42 studs at High) disappeared. The test below measured,
with the old shaders on an RTX 3060 Ti:

| Backend | Floor under a roof past split 0, lit / open |
| --- | --- |
| Vulkan | 0.25, 0.28, 0.52 |
| DX12 | 0.97, 1.00, 1.00 |

#413 found the same issue in the post-edge prepass and fixed it there.

## Fix

- `VOut` gets `@location(8) view_depth`, set to `clip.w` in `vs_main` and `vs_skinned`.
- `fs_main` and `fs_toon` read that value instead of the builtin. It stays within WebGL2's inter-stage limit (9 of 16
  used).
- Vulkan output is byte-identical to main: 30 images, the lighting presets and the avatar review.

## Audit of fragment `position` use

| Shader | Use | Status |
| --- | --- | --- |
| `scene.wgsl` `fs_main` | `.w` for the cascade | fixed |
| `skinned.wgsl` `fs_toon` | `.w` for the cascade | fixed |
| `prepass.wgsl` | depth | fixed in #413 (varying) |
| `scene.wgsl`, `skinned.wgsl` | `.xy` for the shadow filter's dither | portable (pixel centre on every backend) |
| `skinned.wgsl` `vs_outline` | `clip.w` | vertex stage, portable |
| `finish.wgsl`, `bloom.rs`, `edges.rs`, `mlaa.rs`, `post.rs` | `.xy` as a pixel index, vignette, dither | portable |
| `sky.wgsl` | not read | fine |

No other fragment shader reads `position.z` or `position.w`.

## Test

`tests/cascades.rs`, `shadows_hold_across_cascade_splits_on_every_backend`:

- **Scene:**
  - A long floor under noon sun with no sky or ambient light.
  - A roof over floor points at 0.6 x split 0, at 0.93 and 1.07 x each inner split, and in the middle of the last
    cascade.
- **Each sample** must be at most 0.8 of its unroofed brightness.
- **Backends:** the test runs on every backend the machine has (Vulkan, DX12, Metal, and the software adapter: WARP
  on Windows). They must agree within 0.1.
- **Now:** Vulkan, DX12 and WARP give the same ratios (0.03, 0.07, 0.10, 0.25, 0.28, 0.52).
