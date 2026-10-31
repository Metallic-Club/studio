# #151 Auto exposure and tonemappers per Look

Status: implemented and tested. Branch `studio/151-auto-exposure`, on main after #62 and #149.

## Goal

Histogram auto exposure as a compute pass separate from the tone curve, and the AgX Punchy (default), PBR Neutral and
Clamp curves chosen per Look (`docs/research/2026-10-30-lighting.md` 3.9, 5.5; `docs/design/look-presets.md` 3.3).

## Done

- `metallic_render::exposure` and `shaders/exposure.wgsl` (after the research prototype): `build_histogram` (16x16
  tiles, 256 log2 bins over -12..8, centre weight 1..4, workgroup atomics) and `resolve_exposure` (prefix sum, mean
  of the 30-95% band, EV100 clamp to `ExposureMin`..`ExposureMax`, exponential adaptation at 3/s toward light and
  1/s toward dark, night bias up to -2 EV below EV100 10, compensation). It runs after the finish pass on the HDR
  image the finish read, which is independent of the multiplier, so there is no feedback loop. The next frame copies
  the multiplier on the GPU into the finish uniform (`mode.w`) and the globals (`light_toon.y`, so toon's `unfinish`
  stays exact). No CPU readback.
- Units: the CPU fixed exposure (sky table, compensation, night bias) stays as the pre-exposure. EV100 15 is defined
  as the default lighting's fixed exposure, and the key (-2.22, log2) was measured on the reference scene, so Clear Day
  meters to its fixed exposure (multiplier 0.998).
- Core: `Lighting.AutoExposure` (true), `ExposureMin` (-4), `ExposureMax` (16), both clamped to -10..20, appended to
  the Lighting props (no `class_def` change). `ExposureCompensation` range widened to Roblox's -5..5.
  `TonemapperPreset` gains `AgXPunchy` (look-presets conflict 14).
- Looks: `LookTone::for_look(id)` gives curve, exposure mode and compensation from look-presets 3.3 (ids or display
  names). `LightingInputs.look` reads `Lighting.Look`, which arrives with #64; until then a place is Realistic.
  `Default` follows the Look; `Fixed` turns metering off; `Display` also drops the curve pre-scale.
- NaN and infinity: the meter puts NaN and negatives in the unmetered bin 0 and +inf in the top bin. An empty frame
  holds the last exposure. The finish maps NaN/negatives to 0 and inf to 65504 using bit tests (compilers can fold
  `x != x`). `ToneCurve::map` mirrors this, and the settings are made finite before upload.
- `Renderer`: public `auto_exposure`, `auto_exposure_active`, `reset_exposure`, `set_frame_time`, `read_exposure`.
  `capture` resets and meters the view once, then renders, so a still is deterministic.
- Presets: `ExposureCompensation` retuned per preset so the reference scene keeps its #149 brightness under the meter.
  Previews and the table in `docs/lighting/presets.md` were regenerated.

## Tests

- `tests/exposure.rs`:
  - Convergence on bright (0.3), dark (0.01) and night (0.0005) scenes: EV100 and multiplier within 0.06 EV of the
    model, bright and dark on the same display grey, night darker.
  - Adaptation rates over time: 0.78 of the way after 0.5 s toward light, 0.39 toward dark; cuts reset.
  - `ExposureMin` / `ExposureMax` clamps.
  - Every preset meters within 1 EV of its fixed exposure (Clear Day within 0.1).
  - NaN / inf pixels and settings; all-NaN frames.
  - Off / Fixed Looks.
  - GL backend.
  - CPU and GPU curves against f64 references written from the public AgX minimal-fit and Khronos PBR Neutral
    descriptions (GPU worst error 1/255 over a 14-EV ramp in grey, orange and blue).
  - The Look table.
- Existing finish-only tests turn auto exposure off. The avatar coverage test fixes the exposure, because it counts
  changed pixels.

## Open

- Bloom's threshold sees the image before the multiplier. Brightened dark scenes therefore bloom a little less.
- The histogram floor is 2^-12 of pre-exposed light. Scenes darker than that relative to the fixed exposure meter as
  that floor.
- `Lighting.Look` and the rest of the Look (shading, post stack) are #64.
