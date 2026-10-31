# #149 Lighting service and presets

Status: implemented and tested. Branch `studio/149-lighting-presets`, stacked on `studio/62-hdr-post-chain` (#62).

## Goal

The `Lighting` service with Roblox property names, applied live in the renderer, and 12 original lighting presets
(`docs/lighting/presets.md`), rendered and tuned on the look-presets reference scene.

## Done

- Core (`classes.rs`): Lighting properties (ClockTime, GeographicLatitude, Brightness, ExposureCompensation, Ambient,
  OutdoorAmbient, ColorShift_Top/Bottom, EnvironmentDiffuse/SpecularScale, GlobalShadows, ShadowSoftness,
  LightingStyle, Fog*, NightAdaptation, MoonBrightness) and classes Atmosphere, Sky, Clouds, BloomEffect,
  ColorCorrectionEffect, ColorGradingEffect. New `class_def` arms are appended at the end of the match and add no name
  of length 4 or 8, so `Part` / `BasePart` / `Instance` lookups are unchanged (the valgrind gate does not run on
  Windows; this is the reasoning, not a measurement).
- Core (`lighting.rs`): `PRESETS`, `find_preset`, `apply_preset` (defaults, then the preset; creates missing
  children, keeps others), `matching_preset`, `lighting_child`.
- Render (`lighting.rs`): `LightingInputs::read(world)` and `evaluate` (sun position, per-elevation sky table, moon,
  pre-exposure, night adaptation, ambient, height fog, legacy fog, shadow filter radius, curve and grade).
  `Renderer::sync` applies it; `Renderer::set_lighting` takes inputs directly and skips unchanged ones.
  `shaders/lighting.wgsl`: soft shadow filter, wrap diffuse, ambient, fog, sky with sun, moon and stars.
  `bloom.rs`: HDR bloom post pass (`BloomEffect`), named `bloom` in the post chain.
- Tests: `metallic_core/tests/lighting.rs` (property round trips, Roblox names and defaults, enum checks, presets,
  save / load of every preset), `metallic_render/tests/lighting_presets.rs` (each preset within luma and chroma
  bounds, presets differ, edits reach the image, GlobalShadows and FogEnd work), unit tests in both `lighting.rs`.
- Previews: `examples/lighting_presets.rs` writes `docs/lighting/presets/*.png` (320 x 180).

## Review fixes

- Ranges: every Lighting and effect Number and Color3 has a registry range (`PropDef::range`, `NumRange`) applied
  in `World::set`, so scene loads and Luau writes are covered: values are clamped (ClockTime wraps), non-finite ones
  were already refused. The renderer also runs `LightingInputs::sanitized` in `evaluate` for values set from code.
  Tests: core `lighting_values_are_kept_in_range_on_write_and_load` (1e39, f64::MAX, inf, NaN; a hand-edited scene
  file), script `lighting_values_from_scripts_stay_in_range`, render `extreme_inputs_give_a_finite_frame`.
- Toon (#320, via #62): toon light is relative to the default lighting's pre-exposed sun and sky, toon characters
  get the same fog and soft shadows; the toon headless test runs without bloom (bloom lifts the 2 px ink by ~15/255).
- Bloom uses the renderer's HDR format (`PostContext::hdr_format`); bloom and lighting shaders validate with naga.

- Merged main after #310: `PropDef` keeps both main's `hidden` (#313) and `range`.
- 8-bit fallback scene target: documented on `HDR_FORMAT` (light above 1 clips, darks band) and tested through
  the chain (`ldr_scene_target_fallback_renders`, forced with `Renderer::force_ldr_scene_target`).
- `RenderContext::from_eframe` takes the host's adapter, so the HDR format check runs in Studio too.
- `FullscreenPass` drops cached bind groups when the renderer rebuilds its targets (`PostContext::targets`),
  so a quality change no longer keeps the old HDR textures alive.

## Tuning notes

Four render / critique rounds on the reference scene. Main changes: exposure key -0.6 EV (sunlit grey baseplate no
longer near white), fog lit by the horizon with half its hue at low sun (so Atmosphere colours lead), night tint and
desaturation reduced, sky fog distance 500 studs, per-preset colour fixes (Overcast on the Neutral curve, Night City
less purple, Horror Fog greener, Cartoon Bright bluer sky, Alien Dusk teal shade).

## Not done (follow-ups)

- Studio picker and thumbnails: Studio has no viewport or properties panel yet (#83). The PNGs can be the thumbnails.
- Point lights (Night City and Sci-fi Interior use Ambient tints instead), `TimeOfDay` string, auto exposure,
  cloud rendering (Clouds.Cover only dims and greys the sky).

## Commands

```
cargo test -j 4 -p metallic_core -p metallic_render
cargo run -p metallic_render --example lighting_presets -- docs/lighting/presets
```
