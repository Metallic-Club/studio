# #155: lighting shaders against the numpy reference

Status: tested.

The issue asked for headless renders compared with the #37 reference PNGs. Those PNGs come from a physical
model (Hillaire sky, point lights, neon, its own scenes), and the engine deliberately does not reproduce that
model, so a pixel comparison cannot pass by design. Instead the test checks numbers, and only the quantities the
engine models.

## Pieces

- `tools/lighting/make_refs.py` (numpy only; `python -I`, so install numpy in a venv from
  `tools/lighting/requirements.txt`; about 45 s). It imports `atmosphere.py` and `tonemap.py` from
  `docs/research/lighting-shaders/reference/` and mirrors the few lines of `render.py` it needs, because
  `render.py` imports Pillow. It writes `crates/metallic_render/tests/data/lighting_refs.json` (committed):
  - sky: zenith, horizon and sun colour for 33 sun elevations (the 17 table rows and 16 between them),
    sampled as `render.py m1` sampled them. It reproduces `docs/agents/37-lighting/m1_sky_table.md`
    exactly;
  - fog: 15 optical depths of exponential height fog (3 fog settings, 5 paths);
  - curves: the six tone curves on a 37-step grey ramp from 2^-10 to 2^8, plus 16 saturated colours.
- `crates/metallic_render/tests/lighting_refs.rs`. The GPU checks run the engine's own WGSL
  (`color` + `tonecurve` + `common` + `lighting`) in a compute probe; the `Globals` offsets are read from the
  shader with naga. Without an adapter they skip, like the other GPU tests.
- `height_fog_uses_the_reference_density_and_extinction` (unit test in `src/lighting.rs`). The Atmosphere
  to fog uniform mapping matches `render.py` (sigma0 = 0.012 Density^2, extinction 1 + 3 Absorption).

## Checks and tolerances

| Check | Where | Tolerance | Measured |
|---|---|---|---|
| Sky at the table elevations (zenith, horizon, sun) | CPU `evaluate`, GPU `sky_gradient` | 0.6% per channel (the table has 3 significant figures) | 0.45% |
| Sky hue at all 33 elevations | CPU | same channel order wherever the reference channels differ by more than 5% | all agree |
| Sky brightening or darkening between neighbouring elevations | CPU | same direction wherever the reference changes by more than 10% | all agree |
| Moonlight between -1 and -2 degrees | CPU | under 1e-5 (the moon, not the sunset) | was 2.4e-4, now 1e-6 (fixed) |
| `fog_depth` | GPU | 1e-3 relative (the shader's first-order branch below abs(k dy) = 1e-3 is good to 5e-4) | 4e-4 |
| Tone curves | CPU `ToneCurve::map`, GPU `apply_curve` | 1e-3 absolute, display-linear | pass |

A 1% error injected into `fog_depth` fails the GPU fog check. The moonlight check failed before the fix.

## Deliberate differences (not tested beyond the claims above)

- The sky is a table, not a simulation. Between rows the engine interpolates log-linearly in elevation. By
  day (sun above the horizon) luminance is up to 12% off (sun at +2 degrees); in twilight (0 to -9) the
  zenith is up to 22% off; at night (below -9, under 1e-5 of the day sky) up to about 2x. Hue and the
  direction of change still agree; the measured errors are recorded here, not asserted.
- No azimuth: the reference horizon is a mean of 24 directions around the sky at 3 degrees up. The engine's
  horizon is the same colour all round and is drawn at 0 degrees, and its vertical profile is a smoothstep
  from horizon to zenith (`sky_gradient`), not the physical profile.
- The sun fades out between the horizon and 1 degree below it instead of setting at once. The reference sun has
  set by then.
- At night the reference zenith brightens slightly from -15 to -18 degrees (6%, moon glow), while the
  engine's interpolation darkens it. The step is under the 10% margin.
- Only the default atmosphere is checked. Haze, Glare, Color and AirColor change the reference sky
  physically; the engine tints the table. Clouds use the engine's own overcast mix, not the CIE sky.
- No point lights, neon or scenes: the engine has no point lights yet.
- The engine's Haze also thickens the height fog, by (1 + 0.8 Haze) / 1.8 (1 at the default Haze 1); the
  reference fog ignores Haze.

## Findings

- Fixed: between 1 and 2 degrees below the horizon the moon took the table's interpolated light column, which
  ran toward the setting sun (up to 500x the moon, sunset-red). The moon now holds the -2 degree row.
- Decided (coordinator, 2026-10-30): the engine keeps its clamp for Atmosphere `Offset`. The reference scales
  the opacity, `1 - (1 - T) (1 - 0.75 Offset)`; the engine clamps it, `min(1 - T, 1 - 0.75 Offset)`, so with
  Offset above 0 the engine keeps thin fog at full strength where the reference thins it. Cartoon Bright
  (Offset 0.5) is reviewed with the clamp. **The fog reference holds at Offset 0 only.**
