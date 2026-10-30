# Issue #37: lighting research, presets and WGSL prototypes (branch `studio/37-lighting`)

Goal: lighting that beats Roblox Future and heads toward Unreal, at 60 fps on an RTX 3060 Ti and 30 fps on Iris Xe.
Deliverable: `docs/research/2026-10-30-lighting.md`. Research and prototypes only; `metallic_render` is untouched.

## Done
- Research doc: findings marked Sourced/Judgment, pipeline per tier with ms budgets, creator Lighting API with Roblox
  names and defaults, 12 presets with exact values, 8 urgent hooks for Task #3, phased plan, 12 proposed issues.
- 12 WGSL prototypes in `docs/research/lighting-shaders/` (atmosphere LUTs, sky, sky ambient SH9, tonemap, auto
  exposure, GTAO, CSM with PCF/PCSS, height fog, stylised lighting). All valid under naga 30.0.1 and translate to
  SPIR-V, HLSL and MSL.
- numpy reference in `docs/research/lighting-shaders/reference/` (Hillaire LUTs, tonemappers, presets, a box-scene
  renderer). Outputs in `docs/agents/37-lighting/`: `presets.png`, `sky_elevations.png`, `tonemap_curves.png`,
  `tonemap_hues.png`, `tonemap_scenes.png`, `sky_table.md`, `m1_sky_table.md`. I looked at each and tuned.
- Decision (assistant's call): Default tonemapper = AgX Punchy for Realistic, replacing render-styles' ACES fit.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo install naga-cli --version 30.0.1 --locked      # once
sh docs/research/lighting-shaders/validate.sh          # validate + translate all shaders
python docs/research/lighting-shaders/reference/render.py              # all reference renders (~3 min, numpy)
python docs/research/lighting-shaders/reference/render.py m1           # Task #3 sky table only
python docs/research/lighting-shaders/reference/render.py "preset:Night City"
```

## Next
- Task #3 owner: take the section 8.1 hooks (HDR target + finish pass, CPU sky from `m1_sky_table.md`, exposure,
  AgX curve, height fog, ShadowSoftness + blend band, sampleable depth, `Lighting` struct).
- Owner or lead decides which proposed issues (section 10) to create.
- Run the LUT shaders on a GPU and compare with the numpy reference (proposed issue 12).

## Open questions
- Roblox `Atmosphere` defaults, `Brightness` 2 and `ClockTime` 14 come from memory; the docs pages list none.
- Iris Xe costs are extrapolated; nobody has profiled on one.
- AgX "Golden" look values are from memory (unused by default).
- Fixed equinox date: fine for games, but creators may want seasons later (`+ DayOfYear`).
- Disk: C: ran out of space during the session. The safety hook blocked deleting my scratch clones outside the
  worktree; about 60 MB of them remain in the session scratchpad.
