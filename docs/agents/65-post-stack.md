# #65 Post stack v1: bloom, LUT grading, outlines, anti-aliasing

Status: implemented, tested.

## What

- **Bloom**: unchanged from #149 (`bloom.rs`). It is already the tasteful kind: an energy-preserving lerp toward a
  dual-filter mip blur, with a soft threshold and a firefly clamp in the prefilter, driven by `Lighting.Bloom*`. Nothing
  adds glow on top.
- **LUT grading** (`lut.rs`, `shaders/finish.wgsl`):
  - `Lut3d` is an N x N x N colour cube (2..=65): `identity`, `from_fn` and `parse_cube` (the common `.cube` text
    format, 3D only; a 1D LUT, a bad line or a wrong count is a `LutError`).
  - The finish pass samples it as an `Rgba16Float` 3D texture with linear filtering, after brightness and before
    dither, on display-encoded colour, and mixes by a strength.
  - `Renderer::set_lut(Option<&Lut3d>, strength)` uploads it once. It lives outside `FinishSettings` so that
    `set_lighting` does not clear it. Off by default (strength 0 skips the sample).
- **Outlines** (`edges.rs`, `shaders/prepass.wgsl`):
  - When an enabled pass `reads_geometry`, a 1x geometry prepass draws parts and skinned meshes into one
    `Rgba16Float` target: world normal in rgb, linear view depth in a. Background is 0.
  - Toon skinned meshes, which #320 already outlines with a hull, write negative depth.
  - The `edges` HDR pass (before bloom, off by default) draws:
    - silhouettes from the second difference of 1/z, which is 0 on any plane, so floors seen at grazing angles get
      no lines; only the near side of a depth step draws;
    - creases where the normal turns by more than `crease_threshold` between neighbours of about the same depth.
  - The line multiplies the scene colour by `color` (a dark warm brown, never black or neon).
  - Width is in 1080p pixels and scales with the frame height.
  - **No double outlines**: pixels with negative depth are skipped, and their neighbours draw no crease against
    them. A toon character keeps exactly its hull line (test `toon_characters_keep_one_outline`). Standard avatars
    and parts get post lines.
- **Anti-aliasing: SMAA-style morphological AA** (`smaa.rs`, LDR slot, last). The renderer turns it on whenever
  MSAA is off (`Quality::Low`); High keeps MSAA 4x and leaves it off. It runs three passes:
  1. Luma edges with local contrast adaptation.
  2. Blend weights. An orthogonal line search (up to 32 px each way) finds the line ends and their crossing edges.
     The coverage area of the reconstructed line (L, Z and U shapes) is computed analytically per pixel, instead of
     with the reference implementation's precomputed area texture.
  3. A neighbourhood blend.

## Why SMAA over TAA

- TAA needs motion vectors and a history buffer. Every skinned, procedural and vertex-snapped surface would have to
  write correct velocities, or it ghosts.
- TAA smears toon lines, pixel-3D and low-poly looks, and those Looks are core to Metallic.
- TAA makes captures depend on frame history. Screenshots and the image tests here are deterministic per frame.
- Morphological AA runs on every tier, including WebGL2 (fragment passes only), and costs a fixed amount per pixel.
- Costs: no temporal stability on sub-pixel detail (a later TAA or upscaler can take that up), and v1 has no
  diagonal search or corner rounding.

## Deviation from the plan

- The research note suggested reusing a normal/depth MRT from the main pass when MSAA is off. v1 always uses the 1x
  prepass instead:
  - one code path for both qualities;
  - the main pass and its pipelines stay byte-identical;
  - it only runs when an enabled pass reads geometry.
  An MRT path can replace it later if the prepass shows up in profiles.

## Checks

- `tests/post_stack.rs` (GPU tests, serialised):
  - The LUT grades by strength: identity within 1 level, a red/blue swap at full and half strength, off at 0.
  - Edges outline silhouettes and creases but not planes or sky, on High and Low.
  - Toon characters keep one outline.
  - AA softens stair steps only where MSAA is off, and only mixes neighbour colours.
  - The GPU timer measures a frame.
- Unit tests:
  - `lut.rs`: `.cube` parsing and errors, identity order, half-float conversion.
  - `edges.rs`: shader validates, width scaling.
  - `smaa.rs`: shaders validate.
- `tests/headless.rs` toon ink test and `tests/post_chain.rs` remove the new passes where they assert exact colours or an
  empty chain.
- Regression: 53 images (lighting presets, avatar review toon and standard, Parkour avatar shots) are byte-identical
  to main. All post stack features are off at High by default.

## Screenshots (`docs/rendering/post-stack/`, `examples/post_stack.rs`)

| File | Setting |
| --- | --- |
| `high-default.png` | High, no new passes (same as main) |
| `high-edges.png` | High + edges; the toon avatar keeps only its hull line |
| `high-edges-standard-avatar.png` | High + edges, standard avatar outlined by the post pass |
| `high-lut.png` | High + a warm 33^3 LUT at full strength |
| `low-no-aa.png` | Low (no MSAA), AA off |
| `low-smaa.png` | Low with the default AA |

## GPU time, 1920 x 1080 (timestamp queries; median, p10..p90)

RTX 3060 Ti:

| Setting | ms |
| --- | --- |
| High default | 0.835 (0.744..0.840, bimodal clocks) |
| High + edges (toon avatar) | 0.831 (0.827..0.838) |
| High + edges (standard avatar) | 0.782 (0.778..0.786) |
| High + LUT | 0.755 (0.750..0.760) |
| Low, AA off | 0.652 (0.648..0.655) |
| Low + SMAA | 0.857 (0.853..0.864) |
| Low + SMAA + edges | 0.994 (0.989..1.227) |

- Edges: the prepass plus the pass cost about 0.08 ms against the default's low mode.
- LUT: free within noise.
- SMAA: about 0.2 ms at 1080p.

WARP (software, 20 frames, noisy):

| Setting | Median ms |
| --- | --- |
| High default | 1226 |
| High + edges | 1344 (+10%) |
| High + LUT | 1199 |
| Low, AA off | 1209 |
| Low + SMAA | 1216 |

`Renderer::enable_gpu_timing` / `read_gpu_time_ms` (from #63) are carried here too, so this branch measures on its own.

## Per-frame cost

Nothing allocates per frame:

- The prepass targets, SMAA targets and LUT texture are made on resize or on change.
- Per-frame uniforms are written into existing buffers.
- With the passes off, the prepass does not run.

## Follow-ups

- #64 Looks pick the edge settings, the LUT and the AA per Look (pixel Looks turn AA off).
- A diagonal search for SMAA, if diagonal edges look stepped on Low in review.
