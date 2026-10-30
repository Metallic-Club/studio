# #62 HDR target, finish pass and post chain

Status: implemented and tested. Branch `studio/62-hdr-post-chain` from main `7c238f8`. #149 (lighting presets) stacks on it.

## Goal

Render the scene into an HDR target, move tone mapping and sRGB encoding into their own pass, and give the renderer an
ordered, extensible list of post passes (styles research section 13 hooks 1, 2, 4, 6, 7; look-presets 7.2 step 1).

## Done

- `Rgba16Float` scene target; MSAA resolves into it. Sky, opaque, skinned and blended parts write linear light.
  Blending now happens in linear HDR light instead of on tone-mapped values.
- Finish pass (`shaders/finish.wgsl`, `src/tonemap.rs`): exposure, scene-linear grade (tint, saturation, contrast),
  six curves (`ToneCurve`: Filmic = the old Narkowicz fit and the default, AgX, AgX Punchy, PBR Neutral, ACES Hill,
  Clamp), vignette, brightness, sRGB encode, dither. Own uniform (`FinishSettings` on `Renderer::finish`).
  Pre-scales are solved so scene grey 0.18 lands on Filmic's display value under every curve; the doc's pre-scales
  normalise to AgX and do not all match these implementations (Clamp, AgX), so they are not used.
- Post chain (`src/post.rs`): `PostPass` trait (name, slot `Hdr` or `Ldr`, `reads_depth`, `enabled`, `record`),
  `PostChain` (push, insert_before, remove, `get_mut::<T>`), ping-pong targets made on demand, `FullscreenPass`
  helper (prelude `shaders/post_prelude.wgsl`; `post_input`, `post_aux`, `post_sampler`, `post_output`; sRGB output
  via a pipeline override constant, so one pass may draw several times a frame; `draw_to` for own textures).
  An empty chain costs one full-screen pass (finish writes the target directly).
- Depth: always `TEXTURE_BINDING`; stored only when a pass reads it; offered at the scene sample count.
- Colour helpers moved from `common.wgsl` to `color.wgsl` (no bindings) so every module can include them.
- Tests: `tests/post_chain.rs` (grey under every curve on the GPU, flat image without dither, exposure, grade,
  vignette, HDR and LDR passes in order through both ping-pong targets, depth offered, Low and High), unit tests in
  `tonemap.rs` and `post.rs`. Default image matches main (reference scene: max 9/255 at MSAA edges, mean 0.04).

## Not done (follow-ups)

- Render scale (internal size separate from output size) and the separate 1x depth/normal prepass (#109).
- Bloom, edges and other passes: #65 and the Looks. The lighting presets PR (#149) adds bloom.

## Commands

```
cargo test -j 4 -p metallic_render
cargo clippy -j 4 -p metallic_render --all-targets -- -D warnings
```

Shared `CARGO_TARGET_DIR` caveat: worktrees of this repo hash to the same artifact names, so a build in one worktree
can be reused stale by another (mtime fingerprints). Use a per-session target dir or touch the crate before building.
