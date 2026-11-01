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
  - The depth is clip w passed as a varying, not read from the fragment `position.w`: that builtin held 1/w on
    Vulkan and w on DX12 here, which flipped the near-side test on DX12 (CI on WARP) and drew lines around toon
    figures.
  - The `edges` HDR pass (before bloom, off by default) draws:
    - silhouettes from the second difference of 1/z, which is 0 on any plane, so floors seen at grazing angles get
      no lines; only the near side of a depth step draws;
    - creases where the normal turns by more than `crease_threshold` between neighbours of about the same depth.
  - A neighbour past the frame border continues the surface linearly in 1/z, so a plane seen edge-on at the frame
    border gets no line there (test `grazing_floor_gets_no_line_at_the_frame_border`).
  - The line multiplies the scene colour by `color` (a dark warm brown, never black or neon).
  - Width is in 1080p pixels and scales with the frame height.
  - **No double outlines**: pixels with negative depth are skipped, and their neighbours draw no crease against
    them. A toon character keeps exactly its hull line (test `toon_characters_keep_one_outline`). Standard avatars
    and parts get post lines.
- **Anti-aliasing: MLAA-style edge AA using SMAA's local contrast test** (`mlaa.rs`, `MlaaPass`, pass name
  `mlaa`, LDR slot, last). It is not SMAA: no area or search textures, no diagonal patterns, no corner rounding.
  The renderer turns it on whenever MSAA is off (`Quality::Low`); High keeps MSAA 4x and leaves it off. It runs
  three passes:
  1. Luma edges, kept with SMAA's local contrast test (at least half the largest step around).
  2. Blend weights. An orthogonal line search (up to 32 px each way) finds the line ends and their crossing edges.
     The coverage area of the reconstructed line (L, Z and U shapes) is computed analytically per pixel, as in
     MLAA, instead of read from a precomputed area texture.
  3. A neighbourhood blend.

## Why MLAA-style AA over TAA (and over FXAA)

- TAA needs motion vectors and a history buffer. Every skinned, procedural and vertex-snapped surface would have to
  write correct velocities, or it ghosts.
- TAA smears toon lines, pixel-3D and low-poly looks, and those Looks are core to Metallic.
- TAA makes captures depend on frame history. Screenshots and the image tests here are deterministic per frame.
- Morphological AA runs on every tier, including WebGL2 (fragment passes only), and costs a fixed amount per pixel.
- `docs/design/look-presets.md` row 5 planned FXAA for #65. FXAA blurs along every local luma step, including
  toon bands, texture detail and pixel-3D texels; line reconstruction only blends across found edges and leaves
  the rest of the image untouched (the AA test checks that smoothed pixels stay within their neighbours' range).
  It also needs no lookup textures, the reason that row gave for FXAA. Cost: 0.2 ms at 1080p on the test GPU.
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
  - A floor seen under a degree from edge-on gets no line at the frame border, on High and Low.
  - Toon characters keep one outline.
  - AA softens stair steps only where MSAA is off, and only mixes neighbour colours.
  - The GPU timer measures a frame.
- Unit tests:
  - `lut.rs`: `.cube` parsing and errors (NaN and infinite entries are refused), identity order, half-float
    conversion (subnormals, values flushed to 0 below the smallest half, NaN, infinities, negatives).
  - `edges.rs`: shader validates, width scaling.
  - `mlaa.rs`: shaders validate.
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
| `low-mlaa.png` | Low with the default AA |

## GPU time, 1920 x 1080 (timestamp queries; median, p10..p90)

RTX 3060 Ti:

| Setting | ms |
| --- | --- |
| High default | 0.751 (0.746..0.842, bimodal clocks) |
| High + edges (toon avatar) | 0.837 (0.831..0.842) |
| High + edges (standard avatar) | 0.787 (0.783..0.793) |
| High + LUT | 0.755 (0.752..0.760) |
| Low, AA off | 0.653 (0.648..0.656) |
| Low + MLAA | 0.859 (0.853..1.223) |
| Low + MLAA + edges | 0.998 (0.992..1.350) |

- Edges: the prepass plus the pass cost about 0.09 ms.
- LUT: free within noise.
- MLAA: about 0.2 ms at 1080p.

WARP (software, 20 frames, noisy):

| Setting | Median ms |
| --- | --- |
| High default | 1226 |
| High + edges | 1344 (+10%) |
| High + LUT | 1199 |
| Low, AA off | 1209 |
| Low + MLAA | 1216 |

GPU times come from `Renderer::enable_gpu_timing` / `read_gpu_time_ms` (#63).

## Per-frame cost

Nothing allocates per frame:

- The prepass targets, MLAA targets and LUT texture are made on resize or on change.
- Per-frame uniforms are written into existing buffers.
- With the passes off, the prepass does not run.

## Follow-ups

- #64 Looks pick the edge settings, the LUT and the AA per Look (pixel Looks turn AA off).
- A diagonal search for the MLAA pass, if diagonal edges look stepped on Low in review.
