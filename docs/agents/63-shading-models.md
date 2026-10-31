# #63 Shading models and one GPU material table

Status: implemented, tested.

## What

- `crates/metallic_render/src/shading.rs`:
  - `ShadingModel` {Lit, Toon, Unlit, Flat} (the id the shader switches on).
  - `ModelParams`, with the look-spec 3.1 parameters and the presets `LIT`, `TOON`, `FLAT` and `UNLIT`.
  - `Shading`: a default model, overrides by material, and the Character row.
  - `MaterialRow` (64 B) and `MaterialTable`.
- Table layout follows lead ruling R14: 8 style slots x 12 rows = 96 rows, 6 KB in one uniform buffer (group 0,
  binding 3). Each slot holds the 10 materials in registry order, then the Character row, then one spare row. The
  row stores the model id, roughness after the Look's floor, metalness, wrap, toon bands, softness, flatten, light
  step, shade tint, highlight, rim and the roughness floor.
- `InstanceRaw.material` is now four u32: [0] row, [1] flags (reserved: hull, vertex snap, affine, nearest, style slot
  bits 4-7), [2] material id for procedural detail, [3] reserved. Instances stay 96 bytes.
- `scene.wgsl`: one `fs_main` serves every model, so mixing models adds no pipelines.
  - Every derivative is taken before the model branch: detail filtering, the flat normal and the toon edge widths.
  - Lit is the previous code, with wrap = max(row wrap, `LightingStyle.Soft` wrap).
  - Toon: the sun term (N.L x shadow) cut into bands, a tinted shade side, ambient flattened toward the sky/ground
    average, a highlight blob, an optional rim and optional EV light steps.
  - Flat: the triangle normal from `dpdx`/`dpdy`, with wrapped Lambert and sky light.
  - Unlit: `unfinish(albedo)`, so the part shows its own colour. It writes alpha 0 when opaque, which keeps it out of
    the exposure meter, as toon characters already are.
- Standard skinned meshes take the place Look's Character row and keep their slot's roughness and metalness.
  `SkinStyle::Toon` (#320) keeps its own pipeline and outline untouched.
- API: `Renderer::set_shading(&Shading)` sets slot 0 (the place Look) and `set_style_shading(slot, ..)` sets the
  per-part slots for #223. `material_table()` returns the table. The table uploads only on the frame after a change,
  and an unchanged `set_shading` call writes and allocates nothing.

## For the next lanes

- #64: a Look picks a `Shading` (look spec 3.1) and calls `set_shading`. Look ids match `LookOverride` (#388).
- #223: map each resolved style to a slot (1..7), set it with `set_style_shading`, and write
  `MaterialTable::row(slot, material)` into `InstanceRaw.material[0]` and the slot into flags bits 4-7.
  `LookOverrides::apply` returns only parts whose resolved style changed. When the place Look changes, the parts that
  resolve to `None` follow it, so redraw them too.
- The Retro model (vertex snap, affine) and the flags bits belong to #64/#223. Post edges and outlines belong to #65.

## Verification

- No regression: 53 images rendered on main and on this branch are identical to the byte. They cover the 12
  lighting presets on the reference scene, the avatar review in toon and standard styles (36 views), and the
  Parkour/avatar shots from `metallic_runtime --example avatar_shots`. Realistic shading is unchanged.
- Reference images: `docs/rendering/shading-models/*.png`, from `--example shading_models` (the reference scene in
  lit, lit-soft, toon, toon-stepped, flat, unlit and mixed).
- Frame time at 1920 x 1080 on an RTX 3060 Ti, High quality, measured from submit to idle (median of 200 frames):
  - Main, Realistic: 1.10 ms.
  - This branch: Lit 1.15, Soft 1.03, Toon 1.04, Toon stepped 1.04, Flat 1.07, Unlit 1.01, Mixed 1.18.
  - Every model is within the run-to-run noise of main (about 0.1 ms).
  - Unlit first cost 2.66 ms, because the bisection in `unfinish` ran per pixel. It now runs once per vertex.
- Tests:
  - `tests/shading.rs` runs on the GPU, serialised behind a mutex:
    - Unlit shows the part colour within 3 levels, and other rows are unchanged to the byte.
    - Toon: 3 tones cover 88% of a ball, against 10% for Lit.
    - Flat has under a third of Lit's distinct colours.
    - The Character row drives standard skinned meshes, while the toon style ignores the table.
  - `shading.rs` unit tests: row size, the Realistic rows, overrides and floors, no re-upload of unchanged shading,
    and the WGSL constants matching Rust.
