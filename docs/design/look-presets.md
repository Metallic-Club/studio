# Look presets: unified spec

Date: 2026-10-30. Status: **design** (docs only; nothing here is implemented). Issue #144.

This spec merges three research documents into one definition that #62 to #66 can build from:

- Render styles, PR #61: `docs/research/2026-10-30-rendering-styles-vfx-animation.md` ("styles" below) and
  `docs/research/style-shaders/`.
- Lighting, PR #139 (branch `studio/37-lighting`): `docs/research/2026-10-30-lighting.md` ("lighting" below),
  `docs/research/lighting-shaders/` and `docs/agents/37-lighting/`.
- Templates, PR #60: `docs/research/2026-10-30-templates-and-onboarding.md` and `docs/templates/README.md`.
- Studio UI, PR #127: `docs/research/2026-10-30-studio-ui-design.md`.
- The renderer on `origin/main`: `crates/metallic_render`.

The 14 Look files live next to this document in `docs/design/looks/*.look.ron`. They are the exact values; the tables
below summarise them. If a table and a file disagree, the file wins.

Decisions I made for the owner say "assistant's call". Numbers marked "start value" are starting points that the
acceptance renders in section 6 will tune. Iris Xe costs are judgment: nobody has profiled on one yet.

## 0. Decisions in short

1. A **Look** is a data file that sets how a place is drawn: shading per material class, a default lighting preset,
   tonemap curve, exposure mode, post stack, render scale, outlines, palette and fog. `Lighting.Look` picks one.
2. **14 Looks ship**: the 13 the styles doc listed plus **Voxel** (assistant's call; cheap, popular, and it only needs
   one new shader parameter).
3. **Realistic uses AgX Punchy**, not ACES. Stylised looks use PBR Neutral. Palette looks use Clamp or Neutral
   depending on how they quantise light (section 2).
4. **A Look names its default lighting preset; a lighting preset never sets a Look.** Choosing a time of day must not
   change the art style.
5. The templates research's "look presets" (Titanium, Toybox and the rest) become **Themes**: a Theme picks a Look, a
   lighting preset and colours for tagged parts and UI. One name per idea.
6. Creators customise a Look by **diff**: Studio writes a small project file with `base: Some("toon")` and only the
   changed fields. Switching Looks never edits Parts or materials, so it is always reversible.
7. Every Look runs on the **Low tier (Iris Xe)**. Each file lists only its tier deltas from one shared baseline.

## 1. Definitions

### 1.1 What a Look sets

| Field | What it controls | Values |
|---|---|---|
| `shading` | Shading model per material class (1.2) | `Lit`, `Toon`, `Flat`, `Retro`, `LitSprite`, `Unlit` with parameters |
| `lighting` | Default lighting preset, `LightingStyle`, shadow filter, shadow softness floor, ambient scale, AO mode, volumetrics | Section 3.2 |
| `exposure` | How scene light maps to the display before the curve | `Auto` (histogram), `Fixed` (EV from sun elevation), `Display` (1:1 for 2D) |
| `tonemap` | Tonemap curve | `AgXPunchy`, `AgX`, `Neutral`, `Aces`, `Filmic`, `Clamp` |
| `outlines` | Inverted-hull lines and post edges | `hull`, `edges`, each `Off` or `On(...)` |
| `hdr` | Ordered passes on the HDR image | `Bloom`, `Dof`, `Kuwahara` |
| `grade` | Contrast, saturation, brightness, tint, LUT, vignette | Roblox `ColorCorrectionEffect` ranges |
| `ldr` | Ordered passes after the curve | `Pattern` (halftone, hatch), `Paper` |
| `palette` | Colour quantisation with dither | `Off`, `Levels`, `Auto`, `Colors` |
| `render` | Internal size, upscale filter, snapping, anti-aliasing, blocky detail | `Relative(f)`, `Fixed(f)`, `PixelHeight(n)` |
| `output` | Passes at window size after the upscale | `Crt` |
| `fog` | Fog mode, strength and colour | `Height`, `Vertex(start, end)`, `Off` |
| `tiers` | Changes per quality tier on top of the shared baseline | Section 3.6 |

### 1.2 Material classes and how a surface picks its model

The renderer resolves each draw's shading model in this order; the first match wins:

1. `shading.materials["<Material name>"]`: the Roblox material of the Part (`Plastic`, `SmoothPlastic`, `Metal`,
   `Wood`, `Concrete`, `Glass`, `Grass`, `Brick`, `Ice`, `Sand`; the list in `metallic_core::classes::MATERIALS`).
2. `shading.classes[...]`: `Character` (any Part with a `Humanoid` in its ancestor Model) or `Sprite` (2D sprites and
   billboards, from #71).
3. `shading.default`.

The Part keeps its own colour, roughness and metalness from `MATERIAL_TABLE`. The Look only picks the lighting
function and its parameters. Particles follow the Look's render scale, palette and post passes but keep the shading
their emitter sets (#41).

### 1.3 Frame order

The renderer runs a Look in this order. Each step reads the named Look field.

```
 1 shadows            lighting.shadows, tier cascades
 2 prepass*           depth + normal at 1x; only when edges or AO need it and MSAA > 1 (#109)
 3 opaque + hull      shading, outlines.hull, fog (forward)
 4 sky, transparent   fog
 5 edges*             outlines.edges (reads prepass, or the main pass normal target when MSAA = 1)
 6 AO apply*          lighting.ao
 7 hdr list           in file order: Dof, Kuwahara, Bloom ...
 8 finish             exposure -> grade (scene-linear) -> tonemap -> brightness, LUT, vignette -> FXAA if aa = Fxaa
 9 ldr list           in file order: Pattern, Paper
10 palette            quantise + dither at internal size
11 upscale            render.upscale to window size, sub-texel shift when render.subpixel
12 output list        Crt
13 UI                 at window size, never styled by the Look
(* skipped when the Look or tier turns it off)
```

Steps 3 to 10 run at the internal size. Grading before the curve and the LUT after it follows the lighting doc
(section 3.10); section 2 explains the merge.

### 1.4 Look and Lighting

`Lighting` keeps Roblox names (lighting doc section 5). The Look and the `Lighting` service split the work like this:

- **`Lighting` properties are the place's lighting.** `ClockTime`, `Brightness`, `Ambient`, `ExposureCompensation`,
  `ShadowSoftness`, the `Atmosphere`, `Sky` and `Clouds` children: the creator and scripts own them.
- **The Look modifies how that lighting reaches the screen**, at render time, without writing to `Lighting`:
  - effective shadow softness = max(`Lighting.ShadowSoftness`, `lighting.softness_min`);
  - sky and probe ambient x `lighting.ambient_scale`;
  - `Lighting.GlobalShadows = false` turns shadows off whatever the Look says;
  - fog density x `fog.density_scale`; `fog.color` set to a colour replaces the sky colour;
  - `ExposureCompensation` adds to the Look's `exposure.compensation`.
- **The Look writes two things into `Lighting`, and only when a creator picks it in Studio** (5.1): `LightingStyle`
  (from `lighting.style`) and, if the place still uses an unedited preset, the Look's default lighting preset. A
  script that sets `Lighting.Look` at run time changes neither (assistant's call: scripts own their lighting).
- **Post effect instances under `Lighting`** (Roblox classes) adjust the Look:

| Instance | Effect while `Enabled` | Why |
|---|---|---|
| `ColorCorrectionEffect` | Adds to the Look's grade: contrast, saturation and brightness add; `TintColor` multiplies | Its defaults are neutral, so an untouched instance changes nothing |
| `ColorGradingEffect.TonemapperPreset` | `Default` follows the Look's curve; any other value replaces it | Lets the lighting doc's `Default` keep meaning AgX Punchy under Realistic |
| `BloomEffect` | Replaces the Look's bloom (absolute values) | Roblox scripts animate `Bloom.Intensity` for flashes; they expect absolute numbers |
| `DepthOfFieldEffect` | Replaces the Look's `Dof`, or adds a focus DoF if the Look has none | Roblox semantics |
| `BlurEffect` | Adds a Gaussian blur at the end of the HDR list | Roblox semantics (pause menus) |
| `SunRaysEffect` | As in the lighting doc 5.4 | |

The lighting doc's `ColorGradingEffect.Vignette` is dropped; vignette lives in the Look's `grade` only
(assistant's call: one home per parameter). New places get no `BloomEffect`, so the Look's bloom applies.

### 1.5 Themes

PR #60 called its one-click restyles "look presets". This spec renames them **Themes** (assistant's call), because
they set more than the render style and because `Lighting.Look` already names the style. A Theme is a project or
install file `themes/<id>.theme.ron` that sets:

- `look`: a built-in Look id or a custom Look file;
- `lighting`: a lighting preset name plus Roblox-named property overrides;
- `roles`: colour and material for Parts tagged `Theme:<Role>` (roles from `docs/templates/README.md` section 3);
- `ui`: the UI theme for objects tagged `Theme:UI`.

The tags change from `Look:<Role>` to `Theme:<Role>` for the same reason. Section 5.3 maps the nine shipped Themes.

### 1.6 What a creator can override per place

| Layer | How | Kept when the Look changes |
|---|---|---|
| `Lighting`, `Atmosphere`, `Sky`, `Clouds` properties | Properties panel, scripts | Yes |
| Post effect instances | Insert under `Lighting` | Yes |
| Any Look field | Customize (5.2): a project Look file with `base` and the changed fields | The file stays; `Lighting.Look` points elsewhere |
| Shading for one Roblox material | `shading.materials` in a custom Look | Same |
| Per Part | Not in v1. `MaterialVariant` with a `ShadingModel` comes with #63's material assets | |

## 2. Conflicts resolved

| # | Conflict | Decision | Reason |
|---|---|---|---|
| 1 | Realistic tonemap: ACES fit (#61) or AgX Punchy (#139) | **AgX Punchy** for Realistic and HD-2D | ACES turns blue ramps purple and pushes reds to orange (`docs/agents/37-lighting/tonemap_hues.png`); Punchy restores the saturation plain AgX loses |
| 2 | Pixel 3D curve: Clamp (#61) or Neutral (#139 "Pixel Dusk") | **Neutral** for Pixel 3D; **Clamp** for Retro 32-bit, 1-bit and 2D Pixel | Pixel 3D quantises light in 1 EV steps, so its top band sits above 1.0; Clamp would merge the two brightest bands. The other three need exact colours or only a luminance threshold |
| 3 | Grade space: display after the curve (#61 `tonemap.wgsl`) or scene-linear before it (#139) | Contrast, saturation and tint **before** the curve; brightness, LUT and vignette **after** | Highlights roll off through the curve instead of clipping; LUTs are authored on display images |
| 4 | Bloom: dual filter with threshold (#61) or CoD pyramid, no threshold, 0.04 (#139) | **CoD pyramid, threshold 0**; shipped Looks keep intensity at or below 0.08 | Energy-conserving and firefly-safe; the cap keeps defaults tasteful (brand) |
| 5 | Low-tier AA: FXAA, CMAA2 or MSAA 2x (#61), SMAA 1x with FXAA fallback (#139) | **FXAA** in #65; swap in SMAA 1x later if it fits 1.0 ms on Iris Xe | FXAA needs no lookup textures; both docs accept it |
| 6 | Low-tier shadows: 1 x 1024 (Task 3), 2 cascades 1024-2048 (#61), 2 x 1536 tent, far cascade every 2nd frame (#139) | **2 x 1536 tent**, pending owner decision #110 | The most specific proposal, inside both docs' 3.0 ms shadow budget |
| 7 | Lighting presets that set a Look (#139: Cartoon Bright sets Toon, Pixel Dusk sets Pixel 3D) | **Reversed**: the Look names a default preset; presets never set a Look | Picking a time of day must not restyle the game |
| 8 | "Look preset" means two things (#60 Titanium, Toybox; #61 Toon, Pixel) | #60's become **Themes**; files are RON, not TOML | One name per idea; RON matches the scene format |
| 9 | AO in stylised looks: Comic's hatching reads AO (#61); stylised looks skip AO darkening (#139) | AO mode **`PatternOnly`** | Comic and Ink compute AO to drive hatching but never darken with it |
| 10 | Toon shadows: "SH 2" (#61) or shadow combined with N.L before banding, tent not PCSS (#139) | **#139's method** for every Toon model | Cast shadows and the terminator then share one edge |
| 11 | Soft: roughness floor 0.5 and lifted ambient (#61), or wrap 0.3 and a 1.5 degree light radius floor (#139) | **Both**: wrap 0.3, roughness floor 0.5, `softness_min` 0.5, ambient x 1.25 | They change different terms and do not fight; `ShadowSoftness` 0.5 gives exactly the 1.5 degree radius |
| 12 | Pixel 3D: palette in post (#61) or light quantised in EV (#139) | **Both**: `light_step_ev` 1.0 in shading, then a 32-colour palette in post | Light steps keep hues; the palette fixes the colour count |
| 13 | Post edges under MSAA (#109) | Edges need the **1x depth + normal prepass when MSAA > 1**; with MSAA off the main pass writes the normal target itself | wgpu 30 cannot resolve depth; Low tier (FXAA) and pixel looks skip the prepass |
| 14 | `TonemapperPreset.Default` = AgX Punchy (#139) | `Default` = **the Look's curve**; a new `AgXPunchy` value names it explicitly | Under Realistic nothing changes; under Toon, `Default` gives Neutral as #139 itself wants |
| 15 | Exposure: auto everywhere (#139) | **`Fixed`** for Pixel 3D, Retro and 1-bit; **`Display`** for 2D | Auto exposure drifts, and palette looks then flicker as pixels swap palette entries |
| 16 | Toon rim: global 0.12 (#61 table) or per material (#61's own test lesson) | **Character class only** | A global rim lit the far ground in #61's renders |
| 17 | Saturation scale: 1.1 multiplier (#61) or -1..1 offset (#139) | **Roblox `ColorCorrectionEffect` scale**, -1..1, applied as 1 + s | Matches the creator API |
| 18 | Retro size 320 x 240 (#61) on 16:9 windows | `PixelHeight(240)`, width from the aspect (427 at 16:9) | No pillarbox by default; a 4:3 option can come later |
| 19 | Vignette on `ColorGradingEffect` (#139) | Vignette lives in the Look | One home per parameter (1.4) |

## 3. Preset table

All values come from `docs/design/looks/`. Colours are sRGB 0-255, like `Color3` in scene files. Widths in pixels
refer to a 1080p output for `Relative` and `Fixed` scales (the renderer scales them by internal height / 1080) and to
internal pixels for `PixelHeight` looks.

### 3.1 Shading

| Look | Default model | Class and material overrides |
|---|---|---|
| Realistic | Lit (wrap 0, roughness floor 0) | none |
| Soft | Lit (wrap 0.3, roughness floor 0.5) | Metal: floor 0.35. Glass: wrap 0, floor 0. Ice: floor 0.2 |
| Toon | Toon (bands 2, softness 0.03, shadow tint (230,220,255), flatten 0.85, spec 0.08 / 0.5) | Character: rim 0.12 (255,250,240), tint (235,210,230). Metal: bands 3, spec 0.2 / 1.0. Glass: Lit wrap 0.3 |
| Comic | Toon (bands 1, softness 0.02, tint (205,210,255), flatten 0.9, spec 0.1 / 0.8) | Glass: Lit wrap 0.3 |
| Ink | Toon (bands 3, softness 0.05, tint white, flatten 0.9, no spec) | none |
| Painterly | Toon (bands 3, softness 0.12, tint (205,195,255), flatten 0.7, no spec) | Glass: Lit wrap 0.3 |
| Low-poly | Flat (wrap 0.1) | none |
| Voxel | Flat (wrap 0) | none; `detail_texels_per_stud` 8 makes material detail blocky |
| Pixel 3D | Toon (bands 3, softness 0, tint (215,205,255), flatten 0.85, spec 0.06 / 0.6, `light_step_ev` 1.0) | none |
| Retro 32-bit | Retro (affine 1.0, vertex lighting, alpha cutoff 0.5) | Glass: cutoff 0 (blended) |
| HD-2D | Lit (wrap 0.1, roughness floor 0.3) | Sprite: LitSprite (nearest, cutoff 0.5, normal maps, Y-locked billboard, casts shadows) |
| 2D Pixel | Unlit | Sprite: LitSprite (nearest, cutoff 0.5, normal maps, fixed) |
| 2D Smooth | Unlit | Sprite: LitSprite (linear, blended, normal maps, fixed) |
| 1-bit Dither | Toon (bands 3, softness 0.02, tint white, flatten 0.8, no spec) | none |

Model parameters:

- **Lit**: GGX + Lambert as in `scene.wgsl` today. `wrap` 0..0.5 bends the terminator (`soft_diffuse` in
  `stylized_lighting.wgsl`); `roughness_floor` raises every material's roughness to at least that value.
- **Toon**: `toon_sun`, `toon_ambient` and `toon_shade` from `stylized_lighting.wgsl`. `bands` 1..8 lit steps;
  `softness` edge width in N.L units; `shadow_tint` multiplies the shade side; `flatten` 0..1 mixes ambient toward the
  sky/ground average (0.85 equals the prototype's `mix(..., 0.15)`); `spec_size` and `spec_strength` draw a highlight
  blob; `rim_width` 0 turns rim off; `light_step_ev` > 0 quantises incoming light in EV steps (`quantise_light`).
- **Flat**: faceted normal from screen derivatives, computed before any branch; Lambert with `wrap`.
- **Retro**: `ps1_retro.wgsl`. Vertex snap to the internal pixel grid, affine UVs blended by `affine`, per-vertex
  light when `vertex_lighting`, nearest filtering, cut-out alpha at `alpha_cutoff` (0 means blended).
- **LitSprite**: sprite colour, optional normal map, lit by 2D or 3D lights; `filter` Nearest or Linear;
  `billboard` Fixed, Face or YLocked; `casts_shadows` draws a shadow proxy.
- **Unlit**: albedo + emissive.

### 3.2 Lighting fields and default pairing

| Look | Default preset | Also good with | `style` | Shadows | Softness min | Ambient x | AO | Volumetrics |
|---|---|---|---|---|---|---|---|---|
| Realistic | Clear Day | the other 9 (all but Cartoon Bright, Pixel Dusk) | Realistic | Auto | 0 | 1.0 | Darken 1.0 | Auto |
| Soft | Clear Day | Cartoon Bright, Golden Hour, Overcast, Moonlit, Desert Noon | Soft | Tent | 0.5 | 1.25 | Darken 0.6 | Off |
| Toon | Cartoon Bright | Clear Day, Golden Hour, Moonlit, Night City | Soft | Tent | 0 | 1.0 | Off | Off |
| Comic | Cartoon Bright | Clear Day, Golden Hour, Night City | Soft | Tent | 0 | 1.0 | PatternOnly | Off |
| Ink | Clear Day | Overcast, Golden Hour, Horror Fog | Soft | Tent | 0 | 1.0 | PatternOnly | Off |
| Painterly | Golden Hour | Clear Day, Overcast, Moonlit, Alien Dusk | Soft | Tent | 0.4 | 1.1 | Darken 0.5 | Off |
| Low-poly | Clear Day | Golden Hour, Desert Noon, Overcast, Moonlit, Alien Dusk | Realistic | Tent | 0 | 1.0 | Darken 0.7 | Off |
| Voxel | Clear Day | Golden Hour, Overcast, Moonlit, Night City | Realistic | Hard | 0 | 1.0 | Darken 0.8 | Off |
| Pixel 3D | Pixel Dusk | Clear Day, Golden Hour, Moonlit, Night City | Soft | Hard | 0 | 1.0 | Off | Off |
| Retro 32-bit | Clear Day | Horror Fog, Golden Hour, Overcast, Moonlit, Underwater | Realistic | Blob | 0 | 1.0 | Off | Off |
| HD-2D | Golden Hour | Clear Day, Overcast, Moonlit, Night City, Horror Fog | Realistic | Auto | 0.2 | 1.0 | Darken 1.0 | Auto |
| 2D Pixel | Clear Day | Golden Hour, Moonlit, Night City, Pixel Dusk | Realistic | Off | 0 | 1.0 | Off | Off |
| 2D Smooth | Clear Day | Golden Hour, Moonlit, Night City, Cartoon Bright | Realistic | Off | 0 | 1.0 | Off | Off |
| 1-bit Dither | Clear Day | Overcast, Moonlit, Horror Fog | Soft | Hard | 0 | 1.0 | Off | Off |

Lighting presets are the twelve in the lighting doc section 6 with its exact values, minus their `Look` field
(conflict 7). Shadow filters: `Off`; `Blob` (a soft disc under each `Character`, no shadow map); `Hard` (one tap,
crisp pixel edges); `Tent` (tent PCF, scaled by softness); `Auto` (tent on Low and Medium, PCSS on the two near
cascades on High and Ultra). Stylised looks never use PCSS (lighting doc 3.14). Volumetrics `Auto` means froxel fog on
High and Ultra only.

In 2D Looks the sun becomes one global directional 2D light for normal-mapped sprites, with the preset's sun colour;
`Ambient` and `OutdoorAmbient` light the rest. The sky is a background gradient from the preset's sky colours.

### 3.3 Tonemap, exposure and grade

| Look | Curve | Exposure | Comp. (EV) | Contrast | Saturation | Tint | Vignette |
|---|---|---|---|---|---|---|---|
| Realistic | AgXPunchy | Auto | 0 | 0 | 0 | white | 0.15 |
| Soft | Neutral | Auto | 0 | -0.05 | +0.1 | white | 0.1 |
| Toon | Neutral | Auto | 0 | 0 | +0.1 | white | 0 |
| Comic | Neutral | Auto | 0 | +0.1 | +0.2 | white | 0 |
| Ink | Neutral | Auto | +0.3 | +0.1 | -0.6 | (255,250,240) | 0 |
| Painterly | Neutral | Auto | 0 | -0.05 | +0.15 | (255,244,230) | 0.1 |
| Low-poly | Neutral | Auto | 0 | 0 | +0.1 | white | 0.1 |
| Voxel | Neutral | Auto | 0 | +0.05 | +0.05 | white | 0.1 |
| Pixel 3D | Neutral | Fixed | 0 | 0 | +0.1 | white | 0 |
| Retro 32-bit | Clamp | Fixed | 0 | 0 | 0 | white | 0 |
| HD-2D | AgXPunchy | Auto | 0 | +0.05 | +0.1 | (255,250,242) | 0.25 |
| 2D Pixel | Clamp | Display | 0 | 0 | 0 | white | 0 |
| 2D Smooth | Neutral | Display | 0 | 0 | 0 | white | 0 |
| 1-bit Dither | Clamp | Fixed | 0 | +0.15 | 0 | white | 0 |

- Every curve gets the lighting doc's pre-scale so scene 0.18 lands on one display value: AgX 1.0, AgX Punchy 1.7316,
  Neutral 1.4137, ACES (Hill) 1.6752, Filmic (Narkowicz) 0.8306, Clamp 1.0. Switching curves changes contrast and hue,
  not brightness.
- `Auto`: the lighting doc's histogram metering (section 3.9), EV100 clamped to `ExposureMin`..`ExposureMax`.
- `Fixed`: EV100 15 by day, blended to 4 by night from the sun elevation (lighting doc 5.5).
- `Display`: no metering and no curve pre-scale; scene value 1.0 becomes display 1.0. An unlit sprite then shows its
  texture colours exactly under Clamp, and up to linear 0.76 under Neutral.
- No shipped Look uses a LUT. The `lut` slot takes a 32^3 creator LUT later (#65).

### 3.4 Post stack

| Look | Edges | Hull | HDR list | LDR list |
|---|---|---|---|---|
| Realistic | off | off | Bloom 0.04 / 24 | none |
| Soft | off | off | Bloom 0.05 / 32 | none |
| Toon | intersections only, 1 px, (40,36,52) | 2 px, (40,36,52), clip cap 0.02 | Bloom 0.03 / 24 | none |
| Comic | all, 2 px, (26,24,32) | off | Bloom 0.03 / 24 | Halftone (luminance 0.35-0.65, 6 px cells at 45 degrees, 0.5); Hatch (shadow, 5 px at 45 degrees, 0.6); Hatch (AO, 5 px at -45 degrees, 0.5) |
| Ink | all, 1.5 px, (30,28,34), wobble 0.6 px | off | none | Hatch luminance 0-0.6 at 45, 0-0.38 at -45, 0-0.18 at 0 degrees (4 px, 0.8 each); Paper fibre 0.35, (242,236,222) |
| Painterly | all, 1 px, (60,48,52), strength 0.4, wobble 0.4 px | off | Kuwahara r 6 px, sharpness 8, hardness 8, half res; Bloom 0.05 / 32 | Paper canvas 0.2, (250,244,232) |
| Low-poly | off | off | Bloom 0.03 / 24 | none |
| Voxel | off | off | Bloom 0.02 / 16 | none |
| Pixel 3D | all, 1 px, (28,24,36), crease highlight 0.5 | off | none | none |
| Retro 32-bit | off | off | none | none |
| HD-2D | off | off | DoF tilt-shift (centre 0.55, band 0.3, max 8 px); Bloom 0.06 / 32 | none |
| 2D Pixel | off | off | none | none |
| 2D Smooth | off | off | Bloom 0.03 / 24 | none |
| 1-bit Dither | all, 1 px, (27,26,32) | off | none | none |

Edge parameters not shown: depth threshold 0.05 and normal threshold 0.3, except Ink (0.04, 0.25) and Painterly
(0.06, 0.4). `IntersectionsOnly` draws lines where two objects meet and skips silhouettes the hull already covers.
Bloom is written as intensity / size. Built-in hulls on boxes and wedges need the smoothed outline normals that
`outline_hull.wgsl` reads from its location 2; #63 generates them for the built-in meshes.

### 3.5 Render, palette and fog

| Look | Scale | Upscale | Snap | Sub-texel | AA | Palette | Fog |
|---|---|---|---|---|---|---|---|
| Realistic | Relative 1.0 | Bilinear | off | no | Auto | off | Height x 1.0 |
| Soft | Relative 1.0 | Bilinear | off | no | Auto | off | Height x 1.0 |
| Toon | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 0.8 |
| Comic | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 0.8 |
| Ink | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 0.5, paper colour |
| Painterly | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 1.2 |
| Low-poly | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 1.6 |
| Voxel | Relative 1.0 | Bilinear | off | no | MSAA 4x | off | Height x 1.2 |
| Pixel 3D | PixelHeight 270 | Sharp | Texel | yes | off | Auto 32, Bayer 4, 0.4 | Height x 0.8 |
| Retro 32-bit | PixelHeight 240 | Nearest | Vertex | no | off | Levels 32, Bayer 4, 1.0 | Vertex 40-200 studs |
| HD-2D | Relative 1.0 | Bilinear | off | no | Auto | off | Height x 1.2 |
| 2D Pixel | PixelHeight 216 | Sharp | Texel | yes | off | off (the art is the palette) | off |
| 2D Smooth | Fixed 1.0 | Bilinear | off | no | MSAA 4x | off | off |
| 1-bit Dither | PixelHeight 360 | Nearest | Texel | yes | off | Colors (27,26,32) and (232,228,216), Bayer 8, 1.0 | Height x 0.5 |

Scale modes (assistant's call):

- `Relative(f)`: internal size = window x f x the tier's scale (Low 0.67).
- `Fixed(f)`: window x f; tiers do not change it. 2D Smooth uses it because sprites blur badly when upscaled.
- `PixelHeight(n)`: integer scale k = max(1, round(window height / n)); internal size = ceil(window / k), plus one
  pixel in each axis when `subpixel` is on. A 1080p window gives Pixel 3D 480 x 270 at k = 4; a 720p window gives
  k = 3 and 427 x 240. Tiers never change it; these looks are already the cheapest.

`Texel` snap moves an orthographic camera in whole internal texels and hands the remainder to the upscale pass
(`pixel_art.wgsl`). It does nothing for perspective cameras; those still get the low-res target, edges and palette,
with some shimmer (assistant's call: a Look never changes the camera, gameplay does). `Vertex` snap is the Retro
vertex grid. `Sharp` upscale is sharp bilinear: exact at integer scales, clean when the sub-texel shift moves the
image. `AA: Auto` means the tier baseline (3.6).

Palette modes:

- `Levels(n)`: n steps per channel in display space (32 = 15-bit colour).
- `Auto(size)`: built from the place's Part colours in OKLab, expanded by the Look's light bands, then reduced with
  k-means to `size`. Studio rebuilds it one second after colours stop changing; a running game builds it once at
  load and again only when `Lighting.Look` changes, so scripted recolours never make the palette pop.
- `Colors([...])`: a fixed list, up to 64.
- Dither `Off`, `Bayer4`, `Bayer8` (bit maths, stable per internal pixel) or `BlueNoise` (64 x 64 texture, later).

### 3.6 Quality tiers

The baseline below applies to every Look. Each Look file lists only its deltas in `tiers`. Tier names follow #66;
Low is the Iris Xe tier.

| Tier | Target | Relative scale | Shadows | AO | Specular | AA when `Auto` | Volumetrics |
|---|---|---|---|---|---|---|---|
| Low | Iris Xe, 30 fps at 1080p output | 0.67 + FSR 1 | 2 x 1536, tent, far cascade every 2nd frame | GTAO half res, 2 slices | sky only | FXAA | off |
| Medium | GTX 1650, 60 fps at 1080p | 1.0 | 3 x 2048, tent | GTAO half res | + reflection probes | MSAA 4x | off |
| High | RTX 3060 Ti, 60 fps at 1080p | 1.0 | 4 x 2048, PCSS on near 2 when `Auto` | GTAO full res | + SSR half res | MSAA 4x | froxels 160 x 90 x 64 |
| Ultra | RTX 3060 Ti and up | 1.0 | + cached static cascade | same | + ray-query reflections | TAA (FSR 2 native AA) | 128 slices |

Looks that set `aa: Msaa4` keep MSAA on Ultra; TAA would smear their lines. Looks with `aa: Off` never get AA.

Per-look deltas and estimated post cost on Iris Xe (Low, 1080p output; judgment from the ms figures in both
research docs: bloom half res 0.5, grade 0.4, FXAA 1.0, FSR 1 0.5, GTAO half 0.5, edges 0.3, pattern 0.3). The post
budget on Low is 3.0 ms.

| Look | Low | Medium | High / Ultra | Low post cost (ms) |
|---|---|---|---|---|
| Realistic | baseline | baseline | baseline | 2.9 |
| Soft | AO off | baseline | baseline | 2.4 |
| Toon | FXAA, edges dropped | baseline | MSAA 4x kept | 2.4 + hull vertex cost |
| Comic | FXAA, AO off (hatching follows shadows only) | baseline | MSAA 4x kept | 3.0 |
| Ink | FXAA | baseline | MSAA 4x kept | 2.6 |
| Painterly | FXAA, Kuwahara dropped, paper 0.3 | Kuwahara dropped, paper 0.3 | Kuwahara on | 2.8 |
| Low-poly | FXAA | baseline | baseline | 2.4 |
| Voxel | FXAA | baseline | baseline | 2.4 |
| Pixel 3D | none | none | none | 0.5 |
| Retro 32-bit | none | none | none | 0.4 |
| HD-2D | AO off, DoF half res with max 4 px | DoF half res | baseline | 3.0 |
| 2D Pixel | none | none | none | 0.3 |
| 2D Smooth | AA off | baseline | baseline | 0.9 |
| 1-bit Dither | none | none | none | 0.5 |

With FXAA on Low, edges read the main pass normal target directly, so no Low look pays for a prepass (conflict 13).
Low shadows and the 0.67 scale wait on owner decision #110; if the owner picks 1 cascade or native 1080p, only this
baseline table changes.

## 4. Look asset format

### 4.1 Files

- Built-in Looks: `looks/<id>.look.ron` in the Studio and player install, read-only. Ids are kebab-case
  (`pixel-3d`, `retro-32`); names are display strings (`Pixel 3D`).
- Custom Looks: `looks/<name>.look.ron` in the project folder, next to the scene files.
- Themes: `themes/<id>.theme.ron`, same split.
- `Lighting.Look` stores a built-in id (`"toon"`) or a project path without the extension (`"looks/MyToon"`).

RON matches the scene format (`metallic_core::scene_file`): one field per line, trailing commas, comments with `//`.
A one-field change is a one-line diff.

### 4.2 Schema

```
Look (
    format: "metallic.look",              // required, exact string
    version: u32,                         // 1 for this spec
    id: String,                           // [a-z0-9-], unique per install or project
    name: String,                         // 1..40 chars, shown in Studio
    base: Option<String>,                 // None for a full Look; Some("toon") for a diff
    dimension: ThreeD | TwoD | Mixed,     // picker grouping only
    shading: (default: Model, classes: { Character | Sprite: Model }, materials: { "<Material>": Model }),
    lighting: (preset: String, also_good: [String], style: Realistic | Soft,
               shadows: Off | Blob | Hard | Tent | Auto, softness_min: f32, ambient_scale: f32,
               ao: Off | PatternOnly | Darken(intensity: f32), volumetrics: Off | Auto),
    exposure: (mode: Auto | Fixed | Display, compensation: f32),
    tonemap: AgXPunchy | AgX | Neutral | Aces | Filmic | Clamp,
    outlines: (hull: Off | On(width_px, color, max_clip_width),
               edges: Off | On(mode: All | IntersectionsOnly | SilhouettesOnly, width_px, color, strength,
                               depth_threshold, normal_threshold, highlight, wobble_px)),
    hdr: [Bloom(intensity, size, threshold) | Dof(mode: TiltShift | Focus, centre, focus_band, max_blur_px)
          | Kuwahara(radius_px, sharpness, hardness, half_res)],
    grade: (contrast, saturation, brightness, tint: Rgb, lut: Option<String>, lut_strength, vignette),
    ldr: [Pattern(kind: Halftone | Hatch, source: Luminance | Shadow | Ao, from, to, cell_px, angle, ink, strength)
          | Paper(texture: Fibre | Canvas, strength, tint)],
    palette: Off | Levels(levels, dither, strength) | Auto(size, dither, strength) | Colors(colors, dither, strength),
    render: (scale: Relative(f32) | Fixed(f32) | PixelHeight(u32), upscale: Bilinear | Nearest | Sharp,
             snap: Off | Texel | Vertex, subpixel: bool, aa: Auto | Off | Fxaa | Msaa2 | Msaa4,
             detail_texels_per_stud: u32),
    output: [Crt(scanlines, mask, curvature)],
    fog: (mode: Height | Vertex(start, end) | Off, density_scale: f32, color: Sky | Rgb),
    tiers: { Low | Medium | High | Ultra: TierDelta },
)

Model = Lit(wrap, roughness_floor) | Toon(bands, softness, shadow_tint, flatten, spec_size, spec_strength,
        rim_width, rim_color, light_step_ev) | Flat(wrap) | Retro(affine, vertex_lighting, alpha_cutoff)
        | LitSprite(filter: Nearest | Linear, alpha_cutoff, normal_maps, billboard: Fixed | Face | YLocked,
          casts_shadows) | Unlit
TierDelta = (scale: Option<f32>, upscale: Option<Upscale>, aa: Option<Aa>, shadows: Option<Shadows>,
             ao: Option<Ao>, drop: [PassKind], half_res: [PassKind], replace: [Pass])
PassKind = Edges | Hull | Bloom | Dof | Kuwahara | Pattern | Paper | Crt
Rgb = Rgb(u8, u8, u8)                     // sRGB, converted to linear on load like Color3
```

Field rules: every field is required when `base` is `None`. Struct-like variants use RON's `Name(field: value)` form,
so no `Some(...)` wrappers appear outside `TierDelta` and `base`. `replace` swaps the parameters of the pass of the
same kind; `drop` removes a pass; `half_res` runs it at half the internal size.

### 4.3 Full example: Realistic

The other 13 full files are in `docs/design/looks/`: `soft`, `toon`, `comic`, `ink`, `painterly`, `low-poly`,
`voxel`, `pixel-3d`, `retro-32`, `hd-2d`, `pixel-2d`, `smooth-2d`, `one-bit`.

```ron
(
    format: "metallic.look",
    version: 1,
    id: "realistic",
    name: "Realistic",
    base: None,
    dimension: ThreeD,
    shading: (
        default: Lit(wrap: 0.0, roughness_floor: 0.0),
        classes: {},
        materials: {},
    ),
    lighting: (
        preset: "Clear Day",
        also_good: ["Golden Hour", "Overcast", "Moonlit", "Night City", "Horror Fog", "Desert Noon", "Sci-fi Interior", "Underwater", "Alien Dusk"],
        style: Realistic,
        shadows: Auto,
        softness_min: 0.0,
        ambient_scale: 1.0,
        ao: Darken(intensity: 1.0),
        volumetrics: Auto,
    ),
    exposure: (mode: Auto, compensation: 0.0),
    tonemap: AgXPunchy,
    outlines: (hull: Off, edges: Off),
    hdr: [
        Bloom(intensity: 0.04, size: 24.0, threshold: 0.0),
    ],
    grade: (contrast: 0.0, saturation: 0.0, brightness: 0.0, tint: Rgb(255, 255, 255), lut: None, lut_strength: 0.0, vignette: 0.15),
    ldr: [],
    palette: Off,
    render: (scale: Relative(1.0), upscale: Bilinear, snap: Off, subpixel: false, aa: Auto, detail_texels_per_stud: 0),
    output: [],
    fog: (mode: Height, density_scale: 1.0, color: Sky),
    tiers: {},
)
```

### 4.4 A customised Look

What Studio writes after a creator opens Toon, sets the outline to 3 px and drops the bloom:

```ron
// looks/ChunkyToon.look.ron
(
    format: "metallic.look",
    version: 1,
    id: "chunky-toon",
    name: "Chunky Toon",
    base: Some("toon"),
    outlines: (
        hull: On(width_px: 3.0, color: Rgb(40, 36, 52), max_clip_width: 0.02),
    ),
    hdr: [],
)
```

Merge rule: struct fields merge one level deep (`outlines.hull` replaces only `hull`); lists (`hdr`, `ldr`, `output`,
`also_good`) and enum values replace whole. A `base` may itself have a `base`, up to 4 levels, no cycles.

### 4.5 Theme example

```ron
// themes/meadow.theme.ron
(
    format: "metallic.theme",
    version: 1,
    id: "meadow",
    name: "Meadow",
    look: "soft",
    lighting: (preset: "Golden Hour", set: [("ClockTime", Number(16.6))]),
    roles: {
        "Ground": (color: Rgb(98, 140, 72), material: "Grass"),
        "Wall": (color: Rgb(150, 112, 78), material: "Wood"),
        "Hazard": (color: Rgb(214, 92, 52), material: "SmoothPlastic"),
        "Goal": (color: Rgb(72, 150, 214), material: "SmoothPlastic"),
    },
    ui: (panel: Rgb(36, 40, 32), text: Rgb(240, 238, 228), accent: Rgb(150, 190, 110), corner_radius: 6),
)
```

Theme `set` values use the scene file's `SceneValue` forms (`Number`, `Color3`, `Bool`). The M3 art pass picks the
role colours; the four above only show the shape.

### 4.6 Versioning

- `version` is the schema version. This spec is 1. A loader reads every version up to its own and migrates in memory;
  Studio rewrites the file at the new version on the next save and says so in Output.
- A file with a newer `version` than the engine knows does not load. The place falls back to Realistic and Studio
  shows "This Look needs a newer Metallic".
- Additive changes (a new optional field with a default, a new enum value) keep the version; renames and meaning
  changes bump it and ship a migration plus a test file per old version.
- Built-in Looks may change values between releases. A place that must not change copies the Look with Customize.

### 4.7 Validation

The loader rejects a file and names the field and line when:

1. `format` is wrong, a field is unknown (`deny_unknown_fields`), or a required field is missing when `base` is
   `None`.
2. `base` names no Look, forms a cycle or nests deeper than 4.
3. A number leaves its range: `wrap` 0..0.5; `roughness_floor`, `flatten`, `strength`, `softness_min` 0..1;
   `bands` 1..8; `softness` 0..0.5; `light_step_ev` 0..4; `ambient_scale` 0..4; `compensation` -5..5; contrast,
   saturation, brightness -1..1; `vignette` 0..1; bloom intensity 0..1, size 4..64; `width_px` 0..8; `wobble_px`
   0..4; `cell_px` 2..32; `radius_px` 1..16; `PixelHeight` 90..1080; `Relative` and `Fixed` 0.25..2.0;
   `Levels` 2..256; `Auto` size and `Colors` count 2..64; `detail_texels_per_stud` 0..64.
4. A pass kind appears twice in `hdr`, except `Pattern`, which may appear up to 4 times in `ldr`.
5. `snap: Texel` is set without `PixelHeight`, or `subpixel: true` without `Texel`.
6. A material key is not in `MATERIALS`, or a class key is not `Character` or `Sprite`.
7. `lighting.preset` names no known preset (warning only: the place keeps its lighting).

Warnings, shown in Studio but never blocking:

- A palette Look with `exposure.mode: Auto` (palette flicker, conflict 15).
- `aa: Fxaa` or `Msaa*` on a `PixelHeight` Look (smears pixels).
- Estimated Low-tier post cost above 3.0 ms, from the per-pass costs in 3.6.

Shipped Looks pass one more check in CI: bloom intensity at or below 0.08, and no Unlit emissive boost (brand: no
neon, no glow in Metallic's own defaults). Creator Looks may go to 1.0.

## 5. Studio behaviour

### 5.1 The Look picker

- **Where**: a **Look** button in the viewport header next to the view switch, and the `Look` row at the top of
  `Lighting`'s Properties. Both open the same popover. Ctrl K also lists every Look as "Look: Toon".
- **Layout**: a popover (the Studio UI doc gives floating surfaces radius and `bg.raised`) with three rows of tiles:
  **Style** (14 Looks, grouped 3D, then 2D and Mixed), **Lighting** (the 12 presets, the current Look's `also_good`
  first), and **Colours** (Themes; shown only when the place has `Theme:<Role>` tags). Tiles are 160 x 90 thumbnails
  with the name below in primary text. The current choice has the 2 px accent outline used on the start screen. No
  glow, no animated borders.
- **Thumbnails**: static PNGs of the reference scene (6.1), rendered by the acceptance test at 320 x 180 and shipped
  with Studio. The same images serve as review evidence, so a thumbnail never lies about a Look.
- **Live preview**: hovering a tile for 150 ms renders the viewport with that Look; moving off restores the current
  one. Nothing is written. Opening the picker starts compiling every built-in Look's pipelines in the background (#73);
  a tile whose pipelines are not ready shows a small progress ring and previews when ready.
- **Commit**: a click applies the choice as **one undo step** named "Look: Toon". The step sets `Lighting.Look`,
  `Lighting.LightingStyle`, and the default lighting preset when the place's lighting still equals a preset
  unchanged. If the creator edited lighting by hand, Studio keeps it and shows one line under the tiles: "Kept your
  lighting. Use Cartoon Bright?" with a button that applies the preset as its own undo step.
- **Esc** closes the popover without changes. Changing a Look during Play changes the play copy only, like any edit
  made in Play.

### 5.2 Customize

- The popover's **Customize** button opens the **Look** section in Properties for the current Look.
- **Simple view** shows only the knobs the current Look uses, at most 12: outline width and colour, bands, shadow
  tint, bloom, contrast, saturation, vignette, pixel height, palette size, dither strength, fog, paper strength. BotW
  cut its artist knobs from over 200 to 50 (styles doc 3.2); 12 is enough for a beginner.
- **Advanced** shows every field of section 4.2, grouped as the file is.
- The first edit to a built-in Look creates `looks/<Look name> (custom).look.ron` with `base` set and points
  `Lighting.Look` at it. Later edits write only changed fields. Each slider release is one undo step, previews live
  while dragging (Studio UI doc 5.3 rule), and right-click "Reset to Toon" removes the field from the diff.
- **Save as Theme** writes the current Look, lighting and role colours as a Theme file.

### 5.3 Templates

- `template.toml` gets `theme = "<id>"` (replacing PR #60's look preset field). The scene file still stores
  `Lighting.Look` and all lighting properties, so the place renders the same without the template file.
- The picker shows a "Template default" chip on the template's Theme; clicking it restores the Theme in one undo step.
- A public publish of an unchanged template counts a Look, lighting or Theme change as "changed something" (PR #60
  section 4.1).

The nine Themes from `docs/templates/README.md` map as follows (assistant's call; colours from the M3 art pass):

| Theme | Look | Lighting preset | Overrides | Used by |
|---|---|---|---|---|
| `titanium` | Realistic | Clear Day | none | Parkour, Empty 3D, Hangout, Ball Arena |
| `toybox` | Soft | Cartoon Bright | none | Tycoon, Party Rounds, Obby Tower, Base Wars |
| `meadow` | Soft | Golden Hour | `ClockTime` 16.6 | Build Sandbox, Simulator, Tower Defense, Survival Nights, Roleplay Town, Fishing, Garden, Empty isometric |
| `dusk` | Realistic | Golden Hour | `ClockTime` 17.9; `Atmosphere.Decay` (120,96,150) | Battle Royale, Kart Racing |
| `noir` | Realistic | Moonlit | `Atmosphere.Density` 0.7, `HeightFalloff` 0.01; `ColorCorrectionEffect.Saturation` -0.3 | Horror, Escape Room |
| `frost` | Soft | Overcast | `Atmosphere.Color` (220,232,245); `ColorCorrectionEffect.TintColor` (235,245,255) | Side-on Runner, Empty side-on |
| `foundry` | Realistic | Overcast | `Atmosphere.Haze` 2; `Ambient` (40,32,24); warm work lights from the template | FPS Arena, Arena Fighter |
| `paper` | custom `paper`: base 2D Smooth, `Paper(Fibre, 0.3, (246,242,232))`, saturation -0.25 | Clear Day | none | Platformer, Idle Clicker, Puzzle Grid, Card Battler, Story, Empty 2D |
| `pixel` | custom `pixel16`: base 2D Pixel, `palette: Auto(size: 16, dither: Off, strength: 0.0)` | Clear Day | none | Horde Survivor, Top-down Adventure |

Two themes ship their own small Look diffs, which also proves the customise path end to end. Paper's "ink outlines"
on 2D shapes need a stroke on `Part2D` shapes from #71; until then Paper has no outlines.

### 5.4 Luau

Roblox-style naming; Metallic additions are marked +.

```lua
local Lighting = game:GetService("Lighting")

Lighting.Look = "toon"                 -- + built-in id, display name ("Pixel 3D") or "looks/MyToon"
print(Lighting.Look)                   --   reads back the stored form: "toon"
Lighting:GetPropertyChangedSignal("Look"):Connect(function() end)

for _, info in Lighting:GetLooks() do  -- + {Id, Name, Dimension, Builtin}
    print(info.Id, info.Name)
end

Lighting:PreloadLookAsync("ink")       -- + yields until the pipelines exist, so a later switch does not hitch
Lighting:ApplyLightingPreset("Golden Hour") -- + sets the preset's Lighting, Atmosphere, Sky, Clouds and effect values

Lighting.LightingStyle = Enum.LightingStyle.Soft   -- Roblox property; the Look also sets it when picked in Studio
local grade = Instance.new("ColorCorrectionEffect")
grade.Saturation = -0.4                -- adds to the Look's grade (1.4)
grade.Parent = Lighting
```

- `Lighting.Look` replicates from server to clients. A LocalScript change affects only that client, as Roblox
  `Lighting` edits do. Flashbacks, dream scenes and "photo mode" filters work this way.
- Switching to a Look whose pipelines are not compiled draws the previous Look until they are ready, never a blank
  frame. `PreloadLookAsync` avoids the wait.
- The quality tier is the player's setting. Scripts cannot read or force it in v1.
- Scripts cannot edit Look fields in v1. They use `Lighting` properties and effect instances. Typed Look properties
  from Luau come later (section 8).

## 6. Acceptance

A future headless test (`crates/metallic_render/tests/looks.rs`, built in #64) renders each Look and runs the checks
below. Thresholds are start values: the first accepted renders tighten them, and the PR that changes one says why.

### 6.1 Reference scenes

**3D scene** (`tests/looks/look_test.scene.ron`). Units are studs; colours sRGB.

| Object | Class, material, colour | Size | Position | Notes |
|---|---|---|---|---|
| Baseplate | Part, Concrete, (163,162,165) | 128 x 1 x 128 | (0, -0.5, 0) | |
| Red ball | Part Ball, Plastic, (196,40,28) | 6 | (-8, 3, 0) | band and hue checks |
| Blue box | Part, SmoothPlastic, (13,105,172) | 6 x 6 x 6 | (0, 3, 0), yaw 30 degrees | edges, halftone |
| Metal ball | Part Ball, Metal, (200,200,205) | 5 | (8, 2.5, 0) | specular |
| Wood wedge | WedgePart, Wood, (160,95,53) | 6 x 4 x 8 | (-4, 2, -10) | facets |
| Glass slab | Part, Glass, (180,210,230), Transparency 0.5 | 4 x 8 x 1 | (6, 4, -8) | transparency |
| Brick column | Part, Brick, (155,90,70) | 4 x 20 x 4 | (14, 10, -16) | long shadow on the ground |
| Grass pad | Part, Grass, (75,151,75) | 20 x 1 x 20 | (-20, 0.5, -20) | detail texels |
| Dummy | Model with Humanoid: head ball 2, torso 2 x 2 x 1, limbs 1 x 2 x 1; skin (234,184,146), shirt (52,142,64) | 5 tall | feet at (-2, 0, 6) | Character class, rim, blob shadow |
| Swatch row | 8 Parts, SmoothPlastic: (220,40,40), (240,140,30), (240,220,40), (60,180,70), (40,190,200), (40,80,220), (190,60,190), (240,240,240) | 2 x 0.2 x 2 each | x = -7 to +7 step 2, z = 10 | hue and chroma checks |
| Grey ramp | 6 Parts, SmoothPlastic, grey 10% to 100% sRGB in equal steps | 2 x 0.2 x 2 | x = -5 to +5 step 2, z = 13 | luminance curve |
| Fog row | 5 Parts, Plastic, (120,120,130) | 6 x 12 x 6 | (0, 6, -60) to (0, 6, -180), step 30 | fog |

- **Perspective camera**: eye (0, 9, 26), looking at (0, 2.5, -4), vertical FOV 70 degrees.
- **Orthographic camera** (Pixel 3D, 1-bit, Voxel): eye (20, 20, 20) looking at (0, 2, 0), view height 24 studs.
- Output 1280 x 720, Medium tier (and a second run on Low). Lighting: the Look's default preset. Auto exposure runs
  120 frames at a fixed 1/60 s so it converges; TAA off; ordered dither only.
- A test-only Look `debug-ids` renders every object Unlit in a unique flat colour from the same camera; its image
  gives the object masks the checks use. The test also captures the depth target for the silhouette mask.

**2D scene** (`tests/looks/look_test_2d.scene.ron`) for 2D Pixel, 2D Smooth and the HD-2D sprites: Camera2D at
(0, 4), 16 pixels per stud; a ground row of 16 px tiles from a 16-colour test tileset; a 16 x 24 px player sprite at
(-4, 1.5) with a normal map; a 16 x 16 crate at (2, 1); an 8 x 8 coin at (5, 3); two parallax layers (0.3, 0.6); one
2D point light at (3, 4), (255,200,150), range 6; `Ambient` (180,180,200); and an unlit screen-space strip of the 16
palette colours, 8 x 8 px each, along the top edge. HD-2D places the player sprite as a Y-locked billboard at the
dummy's spot in the 3D scene.

### 6.2 Metrics

All metrics read the final 8-bit sRGB image.

- **Luma** Y' = 0.2126 R' + 0.7152 G' + 0.0722 B' on values / 255. Reported as mean, p1 and p99.
- **Internal image**: for `PixelHeight` Looks, the window image sampled at the centre of each k x k block.
- **Block purity**: share of k x k blocks with one colour. Camera static, so the sub-texel shift is 0.
- **Colours**: unique RGB values in the internal image.
- **Line coverage**: share of silhouette pixels (depth jump above 5%, dilated by 1 internal pixel) whose luma is at
  or below the line colour's luma + 0.08.
- **Bands**: peaks holding at least 3% of the red ball's pixels in a 64-bin luma histogram.
- **Hue error**: OKLCh hue of a swatch's centre half against the hue of its source colour.
- **Chroma ratio**: mean OKLCh chroma of the swatch row divided by the same value under Realistic with the same
  lighting preset.
- **Fog**: Michelson contrast of each fog-row box against the sky behind it; must fall with distance.

### 6.3 Per-Look checks

| Look | Must be visible | Numeric checks |
|---|---|---|
| Realistic | Soft-edged column shadow on the baseplate; sun reflection on the metal ball; fog row fading out; no lines | Luma mean 0.35-0.55, p1 >= 0.02, p99 <= 0.97; blue swatch hue error <= 10 degrees (the ACES test); line coverage <= 0.10; fog falls |
| Soft | Gentle terminator on the red ball; wide shadow penumbra; no hard specular | Mean 0.40-0.60; p1 >= 0.04; chroma ratio >= 0.95; hue error <= 10 degrees |
| Toon | Graphite outline around every object; two-tone ball with violet-tinted shade; rim on the dummy only | Mean 0.42-0.65; line coverage >= 0.85; red ball bands 2-3; no rim pixels on the baseplate (dummy mask excluded) |
| Comic | 2 px ink lines; halftone dots on the blue box's falloff; hatching inside the column's shadow | Mean 0.40-0.65; line coverage >= 0.85; red ball bands 2; ink pixels in the column shadow 15-60% |
| Ink | Near-monochrome image on warm paper; wobbly lines; crossed hatching in dark areas | Mean 0.50-0.78; chroma ratio <= 0.45; line coverage >= 0.80; darkest swatch has 2 or more hatch angles (FFT peaks) |
| Painterly | Smeared brush-like flat areas on High; warm grade; canvas grain | Mean 0.35-0.60; on High, mean 5 x 5 local standard deviation inside objects <= 0.6 x Realistic's; red-minus-blue mean > Realistic's (warmth) |
| Low-poly | Visible facets on the ball (flat triangles); strong fog | Mean 0.38-0.60; red ball bands >= 6 (one per facet group); fog row box 3 contrast <= 0.5 x Realistic's |
| Voxel | Blocky 1/8-stud squares in grass and brick detail; hard shadows | Mean 0.36-0.58; inside the grass pad, colour changes only on an 8-per-stud grid (>= 95% of changes on grid lines) |
| Pixel 3D | Square art pixels; 1 px dark silhouettes; light crease lines; dusk palette | Block purity >= 99.5%; colours <= 32; line coverage >= 0.85; mean 0.22-0.50 |
| Retro 32-bit | Jittery vertex grid (visible on the wedge), 15-bit banding with 4 x 4 dither, vertex fog, blob shadow under the dummy | Block purity >= 99.5%; every channel within 1 of a multiple of 255/31; mean 0.30-0.58; no shadow-map shadow from the column (luma behind it >= 0.9 x the lit baseplate's) |
| HD-2D | Sharp centre band, blurred top and bottom; warm bloom on bright edges; pixel sprite casting a shadow | Mean 0.30-0.55; Laplacian variance in the top 20% of rows <= 0.5 x centre band's; blue hue error <= 10 degrees; sprite shadow present (luma under the sprite <= 0.8 x neighbours) |
| 2D Pixel | Crisp square pixels; unlit palette strip exact | Block purity >= 99.5%; unlit strip matches the source colours exactly; colours in the lit area <= 16 x light levels |
| 2D Smooth | Smooth filtered sprites; unlit strip near exact | Unlit strip within 2/255 of source per channel; no k x k block structure |
| 1-bit Dither | Two colours only; Bayer 8 pattern in mid-tones; 1 px lines | Colours == 2; dark share 20-60%; block purity >= 99.5% |

Every Look also checks: the image is identical across two runs; the Low-tier image passes the same checks with the
luma band widened by 0.05; and the HDR target holds no NaN or infinite values.

Performance (manual until CI has an Iris Xe): on an Iris Xe laptop at 1920 x 1080 output, Low tier, the 5,000-Part
stress scene stays at or below 33.3 ms GPU time per frame with each Look, and post + finish + upscale at or below
3.0 ms (GPU timestamps, #23).

## 7. Implementation order

### 7.1 What Task #3 already has (`crates/metallic_render` on `origin/main`)

- Forward wgpu renderer: shadow pass (1 x 1024 on Low, 3 x 2048 on High), then one main pass for sky, opaque and
  sorted transparent Parts. MSAA 4x on High.
- `Quality` is `Low` or `High` inside `RenderSettings`.
- `finish()` in `common.wgsl` applies exposure (`params.z`), the Narkowicz ACES fit and sRGB with dither, straight
  into the swapchain. No HDR target, no post pass.
- `InstanceRaw.material` = (roughness, metalness, material id, unused). The unused slot is the shading model id.
- `MATERIAL_TABLE` has the 10 Roblox materials with roughness and metalness; surface detail is procedural.
- `Sky` is a three-colour gradient with sun, fog density (exp2) and exposure. `Lighting` exists in `metallic_core`
  with no properties.
- Depth uses `StoreOp::Discard`, so no later pass can sample it.
- Headless tests capture frames and count distinct colours (`tests/headless.rs`), which the Look tests extend.

### 7.2 Order and issue mapping

| Step | Issue | Delivers | Done when |
|---|---|---|---|
| 1 | **#62** HDR target, finish pass, post chain | `Rgba16Float` target; finish pass with the six curves and pre-scales; grade uniform (scene-linear part); exposure uniform fed from `m1_sky_table.md` until the histogram exists; render scale with bilinear upscale; sampleable depth flag; post slots `hdr`, `finish`, `ldr`, `output`; normal target when MSAA is off; optional 1x depth + normal prepass (#109) | Realistic renders through it; a 0.18 grey lands within 1/255 of the same value under every curve |
| 2 | **#63** Shading model and material table | `ShadingModel` {Lit, Toon, Flat, Unlit} in `InstanceRaw.material[3]`; GPU material table with per-model parameters; class resolution (1.2) incl. `Character`; smoothed outline normals for built-in meshes; hull pass in the pipeline key. Retro and LitSprite follow in step 6 | Toon and Flat render the 3D reference scene |
| 3 | **#65** Post stack, part A | CoD bloom, grade with LUT slot, vignette, edges, FXAA | Toon and Realistic pass 6.3 except Look-file loading |
| 4 | **#66** Tiers | `Quality` becomes Low, Medium, High, Ultra; baseline table 3.6; Look tier deltas; dynamic resolution; FSR 1 | Each Look's Low image passes 6.3; owner decision #110 applied |
| 5 | **#64** Look presets v1 | RON loader and validator (4.2, 4.7); `Lighting.Look` in `metallic_core`; built-in Realistic, Soft, Toon, Low-poly; acceptance test and thumbnails; Luau surface (5.4) | 6.3 green for the four Looks |
| 6 | **#65** part B, then **#64** v2 | Palette (Levels, Auto, Colors; Bayer 4 and 8), Sharp and Nearest upscale with sub-texel shift, Pattern, Paper; Retro model; then Looks Pixel 3D, Retro 32-bit, 1-bit, Comic, Ink, Voxel | 6.3 green for the six |
| 7 | M5 (#18) with #65 part C | DoF, Kuwahara, volumetrics (#70), CRT; LitSprite on 3D layers (#72); then Painterly and HD-2D | 6.3 green for both |
| 8 | M3 with #71 | 2D layers, pixel camera, 2D lights; then 2D Pixel and 2D Smooth, Themes `paper` and `pixel` | 2D checks green |

Studio work runs beside steps 5 to 8: the picker and Customize belong to #83 (Studio: look preset picker), live
preview needs pipeline precompile from #73, and creator WGSL passes wait for #74. The lighting presets and the
`Lighting` properties they set come from PR #139's proposed issues, not yet created; until they land, every Look's
`lighting.preset` resolves to the current `Sky` defaults and the pairing column is inert.

## 8. Gaps and open questions

- **Owner decision #110** (Low internal scale and cascade count) changes only the baseline tier table.
- Auto palette building (k-means in OKLab, rebuild rules) has no prototype; #65 part B should prototype it first.
- `Blob` shadows, `detail_texels_per_stud`, `PatternOnly` AO and the `IntersectionsOnly` edge mode are new in this
  spec; no research sample implements them yet.
- `ToonFace` (anime face shadows) from the styles doc has no Look field yet; it needs face threshold maps on
  imported characters (M4).
- Luau cannot edit Look fields in v1. A later typed surface (for example `Lighting.LookSettings.OutlineWidth`) needs
  its own design.
- Paper's 2D outlines need shape strokes from #71.
- Every threshold in 6.3 and every Iris Xe cost in 3.6 is a start value until the first renders and an Iris Xe
  profile exist.
- Roblox `Atmosphere` defaults used by the lighting presets come from memory (lighting doc 3.13).
