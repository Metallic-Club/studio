# Rendering, art styles, VFX and animation: research and design

Date: 2026-10-30. Status: **planned** (research and proposal; nothing here is implemented in the engine).
Scope: how Metallic Engine can make games look good in any style a creator picks (PBR, toon, anime, low-poly, voxel,
pixel art, painterly, sketch, PS1, HD-2D, pure 2D), how creators pick and edit that style, and the animation and VFX
systems that sit next to the renderer. Targets: 60 fps at 1080p on an RTX 3060 Ti, 30 fps at 1080p on Intel Iris Xe.

Related work running in parallel: #37 (lighting research and presets), #40 (`metallic_anim`), #41 (`metallic_vfx`),
the `research-performance` session, and Task 3 / #3 (the M1 renderer). This document links to them instead of
repeating them. Section 13 lists the cheap hooks Task 3 should take now.

**How to read the tags.** "Sourced [Rn]" means a page that was fetched during this research (by me or by the four
research helpers I ran) said it; the URL is in the reference list at the end. "Judgment" is reasoning, mine unless
stated. "Assistant's call" marks a decision I made for the owner, which the owner may override. Cost ratings
(cheap/medium/expensive) are judgment, not measurements.

## 1. Summary

Top decisions (assistant's calls):

1. **Keep a forward renderer and grow it into clustered forward (Forward+).** No deferred G-buffer core. Forward keeps
   MSAA (which pixel art, toon and PS1 looks need), lets every material run its own lighting code (which every
   stylised look needs) and spends the least bandwidth on Iris Xe. Add an optional depth + normal prepass for
   outlines, AO, TAA and occlusion culling. A visibility buffer for dense PBR geometry is a far-later High-tier
   option, never the only path.
2. **A style is data, not a fork.** One renderer with: a per-material **shading model id** (Lit, Toon, Unlit, Flat,
   Retro, Lit Sprite, Custom), an ordered **post stack**, a **render scale + upscale filter**, and per-material
   **outline** settings. A **Look** asset sets all of these at once. Beginners pick a Look; experts edit the Look,
   the materials or the WGSL.
3. **12 Look presets** ship (section 5.5): Realistic, Soft, Toon, Comic, Ink, Painterly, Low-poly, Pixel 3D,
   Retro 32-bit, HD-2D, 2D Pixel, 2D Smooth. Plus 1-bit Dither as a stretch goal.
4. **Materials are parameters in one GPU table, not pipelines.** Pipeline key = (shading model, alpha mode, vertex
   kind, outline hull on/off, MSAA, target formats). Everything else is a uniform. This keeps shader permutations
   small, the lesson of Unreal static switches and Godot defines [R76][R2].
5. **One material language: WGSL**, validated by naga. The node editor (M6, egui-snarl) compiles graphs to readable
   WGSL that a creator can open and fork, the three.js TSL and Godot visual-shader model [R85][R80].
6. **GI ladder:** sky + hemisphere ambient now; Hillaire sky, irradiance probe grid, reflection probes and GTAO in
   M5; DDGI (ray query or voxel tracing) only on High later; radiance cascades or surfels as research. Lumen-class GI
   and virtual shadow maps are out of reach for a small team [R8][R27].
7. **2D lives in the same renderer** as layers of instanced quads with their own sort modes, 2D lights and a pixel
   camera. 2D and 3D layers mix in one frame (HD-2D, side-scroller with 3D depth).
8. **Animation:** own runtime (`metallic_anim`, #40) modelled on ozz-animation's job split; Roblox-named track API on
   top; flipbook and 2D bones reuse the same clip/graph types. Spine is excluded by its licence [R134][R135].
9. **VFX:** one GPU-compute particle runtime (emit, update, draw indirect), Roblox-like emitter properties for
   beginners that map onto a fixed template graph, the same graph open in a node editor for experts.
10. **Proof:** nine WGSL samples (`docs/research/style-shaders/`) validate with naga 30.0.1, translate to SPIR-V,
    HLSL and MSL, build as real pipelines on DX12 (FXC and default compiler) and Vulkan on the RTX 3060 Ti, and
    render the same scene in four looks (section 11).

## 2. What exists now (Task 3)

Read from the `3-renderer` worktree (branch `studio/3-renderer`, commit `f80efcb` plus uncommitted changes),
read-only:
- Forward renderer on wgpu 30.0.1. Passes: shadow (1 or 3 cascades), then one main pass that draws sky, opaque and
  sorted blended Parts. MSAA 4x on High.
- `scene.wgsl` does PBR (GGX), procedural per-material detail, shadows, hemisphere ambient, fog **and tonemapping +
  sRGB encode** (`finish()` in `common.wgsl`) straight into the swapchain format. No HDR target, no post pass.
- Per-instance data `InstanceRaw { model: mat4, color: vec4, material: vec4 }` where `material = (roughness,
  metalness, material id, 0.0)`. **`material.w` is unused**: the natural slot for a shading model id.
- Depth is created per frame size and stored with `StoreOp::Discard`. It cannot be sampled later.
- `RenderSettings { quality: Low | High }`.

This is a sound M1 base. Three small changes (HDR target + finish pass, shading model id, sampleable depth) unblock
every style later. Section 13 lists them.

## 3. Research findings

### 3.1 Renderer architecture

| Engine | Path | What it tells us |
|---|---|---|
| Godot 4 | Forward+ (clustered), Mobile (single-pass forward), Compatibility | Chose forward for flexibility and clean MSAA for "less realistic art styles". Mobile uses RGB10A2 instead of RGBA16F to halve bandwidth. Sourced [R1][R2] |
| Bevy | Forward+ by default; deferred per material; depth/normal/motion prepass | "Forward has the fewest surprises and will work better on more devices"; deferred "uses more texture bandwidth, which can be prohibitive on some devices", no MSAA. Sourced [R3][R4] |
| Unity URP | Forward (9 lights/object), Forward+ (256/camera), Deferred | Deferred: no MSAA, encoded normals, "high performance impact" on mobile. Sourced [R5] |
| Unity HDRP | Deferred + forward | Hair, fabric, StackLit and transparent Lit need forward. "Both" compiles two shader sets per material. Sourced [R6] |
| Unreal | Deferred; Nanite visibility buffer; Lumen | Nanite and Lumen do not support forward shading or MSAA; Nanite needs SM6. Sourced [R7][R8] |
| PlayCanvas | Clustered forward since v1.56 | 255 lights cap, shadows/cookies in atlases; turning features off cuts shader size. Sourced [R10] |
| Roblox | Voxel, ShadowMap, Future technologies; `LightingStyle` Realistic or Soft | `Technology` is now deprecated in favour of `LightingStyle`. Sourced [R11][R12][R13] |
| Teardown | Deferred, 5 targets; voxel objects ray-marched in the fragment shader | A 1-bit world voxel volume drives shadows, AO, reflections and volumetrics. Sourced [R15] |

- Visibility buffer: 4 to 8 bytes per sample instead of a 16 to 32 byte G-buffer, but transparency still needs a
  forward pass. G-buffer bandwidth "can be prohibitive on mobile or integrated GPUs". Sourced [R9]
- Frostbite FrameGraph: passes declare reads and writes; a compile step culls unused passes and aliases transient
  memory (147 MB to about 80 MB at 720p). The WorldRenderer shrank from 15K to 5K lines. Sourced [R14]
- Judgment: every stylised look in section 3.2 is "forward lighting with a different function" plus "a different
  post chain". Forward+ with a prepass covers all of them. Deferred would force toon and painterly into G-buffer
  encodings, as Hi-Fi Rush did with a custom engine branch [R53].

### 3.2 How stylised looks are built

| Look | How it works (sources) | Passes and params it needs | Cost / Iris Xe |
|---|---|---|---|
| Guilty Gear cel | Inverted hull outline: duplicate mesh pushed along normals, back faces only; vertex alpha sets thickness, vertex blue pushes the hull back; width scaled by distance and tan(FOV/2); a second normal set for the outline only. Post-process edges were rejected because lines cannot be tuned per part. Shading is binary lit/shadow with painted shadow colour. Sourced [R50][R51] | Toon shading model (bands = 1), shadow tint or shadow texture, hull pass with front-face culling, per-vertex width, outline normal attribute | Cheap; 2x vertex work |
| Genshin anime | Fan reconstruction (no official source): lightmap texture channels (shadow weight, spec, highlight mask, ramp id), 10-row ramp (day/night), face shadow from a threshold map compared against light angle in head space. Sourced [R52]; Hi-Fi Rush uses the same face method [R53] | Toon with ramp texture and ramp row per material id; Toon Face model with head forward/right uniform; hull outline; rim | Cheap to medium |
| Hi-Fi Rush comic | Deferred toon on custom UE4; step() lighting with art-picked shadow colour; halftone dots for fog, bloom, GI, SSR; hatching for AO and shadows; dots as SDF on a 45-degree grid; post per volume, not per camera. Sourced [R53][R54] | Toon; post edges; screen-pattern pass driven by luminance and AO; local post volumes | Medium; patterns cheap |
| Zelda BotW/TotK | All real-time; three distance layers with different tools; artist parameters cut from over 200 to 50; 1,440 time x weather x colour variations. Sourced [R55]. Exact shading model: no source | Soft toon or soft PBR, strong height and aerial fog, rim, time-of-day LUTs | Cheap to medium |
| Okami painterly | Cel shading plus a full-screen paper filter; chosen partly to save PS2 power. Sourced [R56] | Toon, thick outlines, paper overlay | Cheap |
| Kuwahara painterly | Anisotropic: structure tensor, separable blur, 8-sector elliptical sampling. Sourced [R57] | One post pass, kernel/hardness/sharpness | Expensive; half-res, High only |
| Hatching / sketch | Tonal art maps: mip-mapped hatch textures per tone. Sourced [R58] | Hatch texture array chosen by luminance (screen or world space) | Cheap |
| Moebius ink | Sobel on depth and normals (depth weighted x25), offsets wobbled by noise for a hand-drawn line, luminance bands pick hatch directions. Displaced vertices must be displaced in the normal pass too. Sourced [R59] | Normal prepass, edge pass with wobble, hatch pass | Cheap to medium |
| Wide outlines | Jump flood: silhouette, seed, log2(width) flood passes, composite with anti-aliased distance. Beats brute force above 1 px. Sourced [R61] | 3 to 6 passes at reduced res | Medium; selection and wide lines |
| 3D pixel art (t3ssel8r style) | Texel-snapped camera only fully works for orthographic cameras. Sourced [R62]. Community pipeline: low-res target, depth edges dark, normal edges light, toon ramp, integer upscale shifted by the sub-texel remainder. Sourced [R60]; t3ssel8r's own write-up not found | Fixed low render size, ortho snap, 1 px edge pass, clamp tonemap, palette + dither, upscale with offset | Cheap; low res saves time |
| Dead Cells | 3D models rendered small without anti-aliasing, exported per frame as PNG + normal map, lit by a toon shader in game. Sourced [R63] | "Bake to sprite sheet + normal map" tool; Lit Sprite model | Cheap |
| HD-2D (Octopath) | Pixel sprites as billboards in 3D; point lights make sprites cast shadows; tilt-shift, DoF, bloom, volumetrics. Early tests lost the pixel charm at high resolution and saturation. Sourced [R64][R65] | Lit Sprite model, sprite shadow proxies, DoF/tilt-shift, bloom, fog | Medium; DoF half-res on Iris Xe |
| Obra Dinn 1-bit | Stable dither that "looks like an applied texture". Sourced [R66]. Devlog details: not fetched | Luminance, 2-colour palette, Bayer or blue noise, edges | Cheap |
| Dithering | Bayer is cheap but shows line patterns; blue noise is isotropic; use 64x64 textures, new layer each frame. Sourced [R67] | Bayer from bit maths or a blue-noise texture array | Cheap |
| PS1 | Low internal res (256x240 to 512x240), 15/16-bit colour with dither, fog, vertex snapping, affine textures. Sourced [R68] | Retro model: vertex snap, `@interpolate(linear)` UVs, Gouraud, nearest textures; levels 32 + Bayer | Cheap |
| Dreams | Survey talk on unconventional painterly UGC renderers. Sourced [R69]; internals not read | Inspiration for brush-stroke particles only | Expensive |
| Low-poly / voxel | Judgment: faceted normals from screen derivatives; voxels with greedy meshing and per-vertex AO | Flat model; optional ramp | Cheap |

Pattern across all of them (judgment): five knobs cover every look. (1) Lighting function per material.
(2) Outlines, hull or post. (3) Render scale and upscale filter. (4) Tonemap curve and grading. (5) Screen-space
stylisation passes (quantise, dither, halftone, hatch, paper, Kuwahara).

### 3.3 Material and shader authoring systems

| System | Good | Bad |
|---|---|---|
| Unity Shader Graph + URP | Master Stack with vertex/fragment blocks; Renderer Features inject passes (SSAO, decals) without a new pipeline. Sourced [R70][R72] | Custom lighting needs a Custom Function node with HLSL and preview fallbacks; library code changes between versions. Sourced [R71]. Unity Toon Shader still preview [R73] |
| Unreal Material Editor | Material instances avoid recompiles; material functions are reusable subgraphs; domains include decal, post, UI. Sourced [R75][R76][R77] | 13 shading models, none toon [R74]; static switches multiply shader count, an edit can take a minute to recompile [R76]; Substrate is on by default but "platform support and testing is currently incomplete" [R78] |
| Godot shaders | `render_mode diffuse_toon`, `cull_front`, `vertex_lighting`; a `light()` function per light per pixel is the best custom-lighting hook of all engines reviewed. Sourced [R79] | Visual shaders do not expose all text-shader features [R80] |
| Roblox | MaterialVariant overrides a base material place-wide; SurfaceAppearance PBR maps; LightingStyle Realistic/Soft; post effects Bloom, Blur, ColorCorrection, DepthOfField, SunRays, ColorGrading. Zero code. Sourced [R81][R82][R12][R83] | No custom shaders at all: stylised looks are impossible |
| Bevy | `ExtendedMaterial<Base, Ext>` replaces vertex/fragment/prepass shaders while base PBR functions stay callable. Sourced [R84] | Code only; tied to Bevy |
| three.js TSL | Node graph in code compiled to WGSL or GLSL; per-slot overrides (`colorNode`, `normalNode`, `outputNode`), a lights node, post nodes composed as functions. Sourced [R85] | JavaScript only |
| egui node editors | egui-snarl: MIT OR Apache-2.0, 0.12.0 (2026-08), serde. Sourced [R86]. egui_node_graph: all versions yanked, repo 404 [R87] | |

What Metallic takes (judgment): Roblox's zero-code presets and material overrides for beginners; Godot's `light()`
hook; Unreal's instances and functions; TSL's "graph and code are the same thing"; Unity's idea that a renderer
feature adds a pass without a new pipeline.

### 3.4 GI, shadows, reflections, AO, fog, sky

| Technique | Facts | Fit for Metallic |
|---|---|---|
| Godot GI ladder | Fastest to slowest: ReflectionProbe, LightmapGI, VoxelGI, SSIL, SDFGI. Only probes and lightmaps are "suited for integrated graphics". SDFGI: no dynamic occluders, leaks, 25 frames to converge. Sourced [R16][R17] | Probes first |
| DDGI | Probe grid with octahedral irradiance + depth moments; 32x8x32 probes, 64 rays each: 2.6 ms on an RTX 2080 Ti. Sourced [R18] | High tier, later |
| Radiance cascades | Near field: high spatial, low angular resolution; far field the reverse. Path of Exile 2 uses a screen-space 2D variant for its fixed camera. Works in 2D and 3D. Sourced [R19] | Research; very good for 2D lights |
| Lumen | Screen traces, then SDF or hardware RT; about 8 ms for GI + reflections at 1080p on its target; no forward shading. Sourced [R8] | Out of scope |
| Surfel GI (EA GIBS) | RT + surfels, dynamic and user-made content. Sourced [R20] | Research |
| Ray query in wgpu 30 | `EXPERIMENTAL_RAY_QUERY` documented as Vulkan only; changelog shows DXR and Metal work landing. Sourced [R21][R22][R23] | High tier only, Vulkan first |
| Hillaire sky | Four small LUTs (256x64, 200x100, 32^3, 32^2); 0.31 ms at 720p on a GTX 1080; reference code MIT. Sourced [R24][R25] | All tiers; drives sky, sun colour, aerial fog, time of day |
| Volumetric fog | Froxel grid 160x90x64 or 160x90x128, cost independent of screen res, about 1.1 ms (the extracted slide text does not name the platform). Sourced [R26] | High tier |
| Virtual shadow maps | 16k virtual pages; built for Nanite; non-Nanite geometry "much more expensive". Sourced [R27] | Out of scope; keep CSM |
| XeGTAO | 0.56 ms at 1080p on an RTX 2060; **2.39 ms at 1080p on an i7-1195G7 iGPU** (High); Medium about 2/3 of that. MIT, unmaintained. Sourced [R28] | Half-res Low on Iris Xe |
| Contact shadows | Bend screen-space shadows catch detail the shadow map misses. Sourced [R29] | High tier |
| Tiny Glade | Two developers; own renderer on Vulkan; minimum spec Iris Xe "For 720p". Sourced [R30][R31] | Proof a tiny team can ship good GI on iGPU at 720p |

Lighting detail per preset belongs to #37; this document only fixes which features each Look turns on (section 5.5).

### 3.5 2D rendering

- Godot 2D lights: CanvasModulate ambient; PointLight2D height only affects normal-mapped sprites; LightOccluder2D;
  shadow filters None (fastest, suits pixel art), PCF5, PCF13; blend Add/Subtract/Mix; light and occluder masks.
  Lights are computed at viewport resolution, so pixelated lights need snapping in the shader. Sourced [R101]
- Unity URP 2D: lights target sorting layers, and more layers per light cost more; sprite secondary textures
  `_NormalMap` and `_MaskTex`; Shadow Caster 2D and Composite Shadow Caster. Sourced [R102][R103][R104]
- Tilemaps: Godot batches 16x16 cell quadrants (not when Y-sorted). Sourced [R105]. Unity Chunk mode is fast but
  sprites cannot sit between tiles; Individual mode sorts with sprites and is slower. Sourced [R106]. LDtk is MIT.
  Sourced [R107]. Tiled: editor GPL, libtiled BSD-2; reading its files is fine. Sourced [R108]
- Pixel-perfect camera (Unity): reference resolution, upscale render texture, pixel snapping, crop frame. Sourced
  [R100]. Smooth scroll: render one extra pixel, keep the camera on whole pixels, shift the upscaled image by the
  fraction. Sourced [R109]
- Layers and sorting: Godot CanvasLayer per layer transform; Unity sorting layer, order in layer, then distance;
  Sorting Group renders as one unit. Sourced [R110][R111]
- wgpu texture binding arrays are native only (DX12, Vulkan, Metal), not WebGPU. Sourced [R21]

### 3.6 Animation

- Unity Mecanim: Animator Controller state machine; layers override or add, with avatar masks; synced layers;
  2D blend trees (simple directional, freeform directional, freeform cartesian, direct). Sourced [R113][R114][R115]
- Unreal: one-way transitions, conduits, aliases; blend per bone; inertialization as a cheap crossfade; IK Rig
  retargeting by chains; motion matching database search (trajectory + pose channels). Sourced
  [R116][R117][R118][R119]
- Godot: AnimationTree (BlendTree, BlendSpace, StateMachine with A* travel); BlendSpace2D "Discrete" mode suits
  frame-by-frame 2D; track types include method calls and audio; SkeletonModifier3D runs IK/look-at/spring bones
  after the animation mixer in a fixed order. Sourced [R120][R139][R121][R122]
- Roblox: priorities Idle 0, Movement 1, Action 2 to Action4 5, Core 1000; `AnimationTrack:Play(fadeTime, weight,
  speed)`, `GetMarkerReachedSignal`; IKControl with chain root, end effector, target, pole. Sourced
  [R123][R124][R125][R138]
- ozz-animation (MIT): jobs that never touch memory outside their inputs and outputs; SoA sampling with a cache;
  blending with per-joint weights; local-to-model; two-bone and aim IK as correction quaternions. Sourced
  [R126][R127]. ozz-animation-rs is a pure-Rust port, MPL-2.0. Sourced [R128]
- ACL compression (MIT): accuracy first, then decompression speed, then size. Sourced [R129]
- glTF 2.0: skins with joints and inverse bind matrices; 4 influences per set; morph targets apply before skinning.
  Sourced [R130]. Dual quaternion skinning fixes linear-blend artefacts "at minimal additional cost". Sourced [R131]
- Motion matching: search mocap each frame for the best match to pose + desired trajectory. Sourced [R132]. Learned
  motion matching removes the linear memory growth. Sourced [R133]
- 2D skeletal: **Spine requires every user of an integrating toolkit to own a Spine Editor licence** (section 2.4 of
  its licence), which rules it out for a UGC platform. Sourced [R134][R135]. Rive runtime is MIT [R136];
  DragonBonesJS is MIT, maintenance unclear [R137]

### 3.7 VFX

- Unity VFX Graph: contexts Spawn, Initialize, Update, Output; fixed capacity per system; attributes stored only if
  a later context reads them; bounds Manual/Recorded/Automatic (Automatic costs more). Needs compute; no GLES.
  Sourced [R150][R151][R152][R153]
- Niagara: system of emitters; module stacks for emitter spawn/update (CPU) and particle spawn/update; events; data
  interfaces. Scalability through Effect Types with per-tier culling and spawn scaling; "a simulation of 1 particle
  can potentially take up the same resources as one with 64 particles" on GPU; fixed bounds cheaper; sorting off by
  default. Sourced [R154][R155]
- Godot: GPUParticles3D with fixed FPS, draw order, trails, SDF and heightfield colliders, attractors; CPUParticles
  can convert from GPU particles. Sourced [R156][R157]
- Roblox (beginner reference): Rate, Lifetime, Speed, SpreadAngle, Size/Transparency as NumberSequence, Color as
  ColorSequence, LightEmission, Flipbook, Drag, Acceleration; rate capped at 400/s per emitter on desktop and 100/s
  on mobile, lifetime at 20 s. Beam: cubic Bezier between two attachments. Sourced [R158][R159]
- GPU runtime pattern (AMD "Holy Smoke", Wicked Engine): dead list, two ping-pong alive lists, counters, indirect
  args; emit pops the dead list, simulate writes survivors, draw indirect with 6 vertices per particle. Tiled
  compute rendering cut a smoke view from 27.3 ms to 6.2 ms on an R9 290X. Sourced [R163][R164]
- Destiny: expression graphs compiled to HLSL beside a bytecode interpreter that runs on CPU or GPU, aiming for
  sub-second iteration. Sourced [R161]. PopcornFX moved from script to nodes because artists struggled. Sourced
  [R162]. bevy_hanabi (MIT/Apache, needs Bevy): design reference only. Sourced [R160]
- Low-res particles: render off-screen at reduced size with max-depth downsample; 25 fps to 51 fps in GPU Gems 3's
  test. Sourced [R166]. DOOM 2016: decals in the clustered forward pass from an atlas, up to 256 per cluster;
  particle lighting cached in an atlas. Sourced [R167]. Forward decals must be sorted. Sourced [R165]
- Sorting: FidelityFX parallel sort is MIT, 4-bit radix. Sourced [R168]. `lampshade` (MIT, wgpu 30) offers radix
  sort and compaction; young, single maintainer. Sourced [R172]

### 3.8 wgpu, performance and Iris Xe facts

- `multi_draw_indirect` needs only `INDIRECT_EXECUTION` since wgpu v27; `MULTI_DRAW_INDIRECT_COUNT` is DX12 and
  Vulkan 1.2 only. Push constants are now "immediates". Sourced [R21][R23][R33]
- `PipelineCache` is Vulkan only; DX12 and Metal unimplemented. Sourced [R32]
- **DX12 shader compiler in wgpu 30:** `Dx12Compiler::Auto` (default) uses statically linked DXC if the `static-dxc`
  feature is on, else `dxcompiler.dll` on the PATH, else FXC, which the wgpu source calls "old, slow and
  unmaintained". Sourced: local source of `wgpu-types` 30.0.1, `src/backend.rs` [L1]. Without `static-dxc`, players
  get FXC and slower shader compiles.
- Default limits: 8 storage buffers per stage (downlevel 4), 256 invocations per workgroup, 128 MiB storage
  binding, 64 KiB uniform binding (downlevel 16 KiB). Sourced [R169]. WebGL2 has no compute. Sourced [R170]
- Subgroup sizes: NVIDIA 32, Intel 8 to 32. Do not hard-code. Sourced [R171]
- Bevy 0.16 GPU-driven rendering (multi-draw, GPU cull, bindless) was about 3x faster on a busy scene. Sourced [R34].
  Two-phase occlusion culling against last frame's depth pyramid: Sourced [R35]
- Godot 4.4 ubershaders: one pipeline precompiled with specialization constants changeable at draw time,
  optimised variants built in the background; stutter remains for mid-game feature switches. Sourced [R36].
  Unreal PSO precaching draws a default material until a PSO is ready. Sourced [R37]
- Dynamic resolution from GPU frame time, 50 to 100 percent. Sourced [R38]. FSR 1 and FSR 2 are MIT; FSR 2 needs
  depth, motion vectors, jitter and a reactive mask. Sourced [R39][R40]
- Iris Xe (i7-1185G7): 96 EUs, 1.30 GHz, DDR4-3200 or LPDDR4x-4267. Sourced [R41]. No simultaneous 3D and compute on
  Gen11, so async compute does not help; use few render targets and narrow formats. Sourced [R42]. 128 KB shared
  local memory per subslice. Sourced [R43]
- Judgment: Iris Xe has roughly 1/7 of the 3060 Ti's bandwidth (about 50 to 68 GB/s shared with the CPU, worse on
  single-channel laptops). That number is an estimate from bus width; no fetched page states it.

### 3.9 Crates (permissive only; checked on crates.io or the repo)

| Crate | Licence | Use | Verdict |
|---|---|---|---|
| wgpu, naga, naga-cli 30.0.1 | MIT OR Apache-2.0 | GPU API, WGSL validation | Use (in use) |
| wesl 0.6 | MIT OR Apache-2.0 | WGSL imports and `@if` | Use for shader modules [R173] |
| naga_oil 0.23 | MIT OR Apache-2.0 | WGSL imports (Bevy org) | Fallback |
| glam, bytemuck | MIT/Apache (+Zlib) | Maths, casting | Use (in use) |
| encase 0.13 | MIT-0 | WGSL struct layout | Needs a licence-list decision (MIT-0 is not on the list); else bytemuck with manual padding |
| image, ktx2, ddsfile, half | MIT/Apache | Textures | Use |
| basis-universal 0.3.1 | MIT OR Apache-2.0 | Basis bindings | Stale (2023); evaluate |
| intel_tex_2, image_dds | MIT/Apache | BC compression | Offline tools |
| meshopt 0.7 | MIT OR Apache-2.0 | Simplify, LOD, meshlets | Use |
| gltf 1.4.1 | MIT OR Apache-2.0 | glTF import | Use |
| egui, egui-snarl 0.12 | MIT OR Apache-2.0 | Editor, node graphs | Use |
| etagere, guillotiere | MIT/Apache | Atlas packing | Use (etagere) |
| lyon | MIT OR Apache-2.0 | 2D vector tessellation | Use (2D shapes, UI) |
| ozz-animation-rs 0.11 | MPL-2.0 | Animation runtime | Reference and benchmark for #40 |
| splines 5.0 | BSD-3-Clause | Spline curves | Optional |
| tiled 0.16 | MIT | Tiled map loader | M3 import |
| boxdd 0.6 | MIT OR Apache-2.0 | Box2D v3 bindings | M3 (2D physics) |
| wgpu-profiler, puffin, profiling, tracy-client | MIT/Apache | Profiling | Use |
| lampshade 0.13 | MIT | GPU sort/scan/compact | Evaluate |
| fastnoise-lite | MIT | Noise | Port to WGSL |
| rusty_spine | Spine licence | Spine runtime | **Avoid** |
| egui_node_graph | MIT | Node graph | Avoid (yanked) |
| bevy_hanabi, rend3 | MIT/Apache | Particles, renderer | Read only (Bevy dependency / archived) |

All rows Sourced from `https://crates.io/api/v1/crates/<name>` or the repo [R174]. Upstream C/C++ licences of
basis_universal, meshoptimizer, Tracy and DXC are not verified (judgment: all permissive; check before shipping).

## 4. Render architecture

### 4.1 Frame shape

```
CPU: sync world -> cull (CPU now, GPU later) -> build draw lists per pass
GPU (one render graph, passes declare reads/writes):
  1 shadows         CSM, 1-4 cascades                          depth array
  2 prepass*        depth + normal (+ motion later)            depth, normal
  3 GPU compute*    particle emit/update, skinning, culling    buffers
  4 opaque          forward, shading model per material        HDR colour, depth
  5 outline hull*   per material, front faces culled           HDR colour
  6 sky             Hillaire LUTs (gradient now)               HDR colour
  7 transparent     sorted blended, particles, decals          HDR colour
  8 post (HDR)      edges, AO apply, fog, volumetrics, bloom, DoF
  9 finish          tonemap curve + grade/LUT + encode
 10 post (LDR)      screen pattern, paper, quantise/dither, upscale, CRT, vignette
 11 UI              egui / game GUI at output resolution
 (* optional; the Look and the quality tier switch them)
```
Passes 4-8 run at the **internal size** (output size x render scale, or a fixed pixel-art size). Pass 10's upscale
brings the image to the output size. Without an upscale pass, the finish pass writes the output directly.

### 4.2 Render graph (assistant's call)

A small FrameGraph-style graph [R14], written by us: each pass declares named textures/buffers it reads, writes or
creates; the graph orders passes, skips passes whose outputs nobody reads, and later aliases transient textures.
M1 does not need the graph; it only needs the frame split into named stages (section 13) so the graph can replace the
hard-coded order in M5 without a rewrite. No external render-graph crate.

### 4.3 Clustered forward (M5)

Point and spot lights binned into a froxel grid (16x9x24, log depth slices) by a compute pass; light, reflection
probe and decal indices per cluster in one storage buffer, as Godot and DOOM do [R2][R167]. Toon and pixel looks use
the same light lists; their shading functions quantise the result.

## 5. Look and material system

### 5.1 Material model

```
Material (asset, text RON like scenes)
  shading_model: Lit | Toon | ToonFace | Unlit | Flat | Retro | LitSprite | Custom(shader asset)
  alpha: Opaque | Mask(cutoff) | Blend | Additive | Premultiplied
  base: color, color_map, normal_map, orm_map (occlusion/roughness/metalness), emissive(+map), roughness, metalness
  toon: ramp (bands + softness | ramp texture + row), shadow_tint | shadow_map, spec_size, spec_strength,
        rim_width, rim_color
  outline: hull { width_px, color, vertex_alpha_width, use_outline_normals }, post_edges: on/off
  retro: snap_grid, affine, vertex_lighting
  flags: casts_shadows, receives_shadows, double_sided
MaterialInstance: parent + overridden params (no recompile, like Unreal instances [R76])
```
- GPU side: one storage buffer of packed material records; each instance carries a `u32` material index.
- Pipeline key: (shading model, alpha mode, vertex kind: static/skinned/sprite/particle, hull on/off, MSAA count,
  target formats). Built-in models live in one "uber" surface shader with a `switch` on the model id (the sample in
  `surface.wgsl` does this), so a scene mixing Lit, Toon and Flat needs one pipeline per alpha mode.
- Roblox names stay: `Part.Material = Plastic | Metal | ...` picks a built-in Material; a Look can remap each one
  (the MaterialVariant "override" idea [R81]). New properties: `Material.ShadingModel`, `Lighting.Look`.

### 5.2 Shading models

| Model | Lighting function | Typical Looks |
|---|---|---|
| Lit | GGX + Lambert, IBL/probes | Realistic, Soft, HD-2D |
| Toon | half-Lambert -> bands or ramp row; shadow tint; spec blob; rim | Toon, Comic, Ink, Pixel 3D |
| ToonFace | Toon + face threshold map against head-space light angle | Anime characters |
| Unlit | albedo + emissive | UI-like, 2D, effects |
| Flat | faceted normal from derivatives, Lambert + hemisphere | Low-poly, voxel |
| Retro | per-vertex Gouraud, vertex snap, affine UVs, nearest textures | Retro 32-bit |
| LitSprite | sprite colour + normal map lit by 2D or 3D lights; alpha cutout; billboard modes | HD-2D, 2D Pixel, Dead Cells |
| Custom | creator WGSL implementing `surface()` and optionally `light()` | Anything |

Custom shader contract (judgment, Godot `light()` plus Bevy extension style [R79][R84]):
```wgsl
fn surface(in: SurfaceInput) -> SurfaceOutput   // required: albedo, normal, roughness, metalness, emissive, alpha
fn light(s: SurfaceOutput, l: LightSample) -> vec3<f32>   // optional: replaces the model's per-light function
fn vertex(v: VertexInput) -> VertexInput         // optional: displacement
```
The engine wraps these in its own entry points. Creator shaders never see bindings directly, so the engine can
change bind group layouts without breaking them. Security: creator WGSL runs on players' GPUs, so the engine
validates it with naga, rejects unbounded loops (or injects an iteration cap), caps texture count and instruction
count, and compiles it off the main thread with a fallback material while it builds. A shader that hangs the GPU
must not be able to reach players (proposed issue 13 in section 14).

### 5.3 Material editor (M6)

- **Beginner:** Properties panel with named sliders per model (colour, roughness, bands, outline width, rim) and a
  live preview sphere. Most creators never leave this.
- **Intermediate:** material instance editing; a Look editor with 8 to 15 sliders (BotW cut 200 artist parameters to
  50 [R55]).
- **Expert:** node graph (egui-snarl [R86]) with graph types Surface, Light, Vertex, Post Pass. Subgraphs act as
  functions. The graph compiles to readable WGSL in a side panel; "Convert to code" forks it into a text shader.
  Errors from naga show on the node that caused them.
- Hot reload in under a second (Destiny's target [R161]) while the game runs in Play.

### 5.4 Post-processing stack

An ordered list of passes in two slots, data-driven, each with a minimum tier and a Low-tier fallback:

| Slot | Pass | Params | Cost | Low tier |
|---|---|---|---|---|
| HDR | Edge outlines (depth/normal) | width, depth/normal thresholds, line colour, highlight, wobble | cheap | on |
| HDR | AO (GTAO) | radius, intensity, half-res | medium | half-res Low or off |
| HDR | Height + distance fog | density, height falloff, colour from sky | cheap | on |
| HDR | Volumetric fog/light (froxels) | density, anisotropy | medium | off |
| HDR | Bloom (dual filter) | threshold, intensity, radius | cheap | half-res |
| HDR | DoF / tilt-shift | focus, range, bokeh size | medium | half-res, small kernel |
| HDR | Kuwahara painterly | radius, sharpness | expensive | off (paper overlay instead) |
| finish | Tonemap | curve: ACES fit / PBR Neutral / clamp; exposure (auto later) | cheap | on |
| finish | Grade | contrast, saturation, white balance, 3D LUT + strength | cheap | on |
| LDR | Screen pattern | halftone dots / hatch lines driven by luminance, shadow or AO | cheap | on |
| LDR | Paper / grain overlay | texture, strength | cheap | on |
| LDR | Quantise + dither | levels or palette (up to 64), Bayer 4/8 or blue noise, strength | cheap | on |
| LDR | Pixel upscale | nearest or sharp bilinear, sub-texel camera offset | cheap | on |
| LDR | CRT / scanlines | mask, curvature | cheap | on |
| LDR | Vignette | intensity, smoothness | cheap | on |
| any | Anti-aliasing | MSAA, FXAA/CMAA2 or TAA (+FSR) | varies | MSAA 2x or FXAA |

Custom post passes: creator WGSL fragment function with fixed inputs (colour, depth, normal, params). Local post
volumes with blend distance (Unity/Unreal volumes; Hi-Fi Rush per-volume post [R53]) come after M5.

### 5.5 Look presets

> Superseded for exact values: [`docs/design/look-presets.md`](../design/look-presets.md) (#144) is the source of truth
> for every Look. It changes this section's tonemap choice (AgX Punchy, not ACES, for Realistic) and adds Voxel.

A Look is a text asset: render scale and camera, default shading model, tonemap and grade, outline defaults, post
list, palette/LUT, lighting feature switches (shadows, AO, GI, fog), and a tier table of what drops on Low.
`Lighting.Look = "Toon"` applies one. Changing Look never edits materials; it applies material remaps and defaults
on top, so switching back is lossless.

Pass codes: SH shadows, PRE depth/normal prepass, HULL outline hull, EDGE post edges, AO, FOG, VOL volumetrics,
BLM bloom, DOF, KUW Kuwahara, TM tonemap (A = ACES fit, N = PBR Neutral, C = clamp), GR grade/LUT, PAT screen
pattern, PAP paper, QD quantise + dither, UP pixel upscale, CRT, VIG vignette.

| Look | Default model | Render size / camera | Lighting | Passes in order (Low-tier changes) |
|---|---|---|---|---|
| **Realistic** (default) | Lit | 100% (Low 67% + FSR1) | SH 3 cascades, probes, sky | PRE, AO, FOG, (VOL High), BLM, (DOF opt), TM A, GR, VIG, TAA or MSAA 4x. Low: SH 2, AO half-res, no VOL, MSAA off + FXAA |
| **Soft** (Roblox "Soft" feel) | Lit, roughness floor 0.5 | 100% | SH 2 soft, lifted ambient | FOG, BLM soft, TM N, GR (saturation 1.1). Low: same |
| **Toon** (anime/cel) | Toon (bands 2, soft 0.03, tinted shadows, rim 0.12) | 100% | SH 2, flat ambient, no AO | HULL (2 px), EDGE (intersections only), FOG, BLM, TM N, GR. Low: EDGE off |
| **Comic** (Hi-Fi Rush) | Toon (bands 1) | 100% | SH 2, AO feeds hatching | PRE, EDGE (2 px), AO, BLM, TM N, GR, PAT (dots in light falloff, hatch in shadow/AO). Low: AO half-res |
| **Ink** (Moebius/sketch) | Toon (bands 3) | 100% | SH 2 | PRE, EDGE (wobble on, 1.5 px), TM N, GR (desaturate), PAT (hatch by luminance), PAP |
| **Painterly** | Toon (ramp, soft edges) | 100% | SH 2, soft ambient | PRE, EDGE (soft), FOG, BLM, KUW (half-res, High only), TM N, GR (warm LUT), PAP. Low: no KUW |
| **Low-poly** | Flat | 100% | SH 2, sky ambient | EDGE (1 px, optional), FOG strong, TM N, GR, MSAA 4x (Low FXAA). Voxel variant: Flat + baked vertex AO |
| **Pixel 3D** (t3ssel8r) | Toon (bands 2-3) | fixed 480x270 or 320x180, ortho snapped to texels | SH 1-2, hard | PRE, EDGE (1 px: depth dark, normal light), TM C, QD (palette 16-32, Bayer 4, 0.4), UP (subpixel offset). MSAA off |
| **Retro 32-bit** (PS1) | Retro (snap 320x240, affine 1) | fixed 320x240 | no SH (blob shadows), vertex fog | TM C, QD (levels 32, Bayer 4), UP nearest, optional CRT |
| **HD-2D** | Lit for world, LitSprite for characters | 100% (Low 67%) | SH 2-3 (sprites cast via proxies), probes | PRE, AO, FOG, (VOL High), BLM, DOF tilt-shift, TM A, GR, VIG. Low: DOF half-res, no VOL, no AO |
| **2D Pixel** | LitSprite / Unlit (2D layers) | pixel camera, reference res e.g. 384x216 | 2D lights + normal maps, ambient | TM C, (QD optional), UP (subpixel). MSAA off |
| **2D Smooth** | LitSprite / Unlit (2D layers) | 100% | 2D lights optional | BLM, TM N, GR, MSAA 4x |
| 1-bit Dither (stretch) | Toon (bands 3) | fixed 640x360 | SH 1 | EDGE, TM C, QD (palette 2, Bayer 8 or blue noise), UP |

Each preset is a starting point; every field stays editable in the Look editor.

### 5.6 Shader permutations and pipeline caching

- Pipeline key as in 5.1; post passes are one pipeline each. Estimate: under 100 pipelines for all built-in Looks
  (judgment).
- At place load, collect every pipeline key the place needs and compile them on worker threads behind the loading
  screen. Until a pipeline is ready, draw with the default Lit material (Unreal PSO precaching [R37]).
- Turn on wgpu's `static-dxc` feature on Windows so players never fall back to FXC [L1]. Check the DXC licence with
  `cargo deny` first (judgment: University of Illinois/NCSA, permissive).
- Use wgpu `PipelineCache` on Vulkan only; on DX12 rely on the driver cache [R32].
- WGSL modules assembled with `wesl` imports and `@if` instead of string concatenation once there are more than a
  handful of files [R173].

## 6. One camera and layer model for 2D, 2.5D and 3D

Assistant's call. One world, 3D transforms everywhere. "2D" means a layer that sorts by rules instead of depth.

```
Camera
  projection: Perspective { fov } | Orthographic { size }
  pixel: None | Snap { pixels_per_unit, reference_resolution } | LowRes { size, subpixel_offset: bool }
RenderLayer (ordered list in the place; Workspace = the default 3D layer)
  camera: inherit | own
  sort: Depth (z-buffer, 3D) | Painter { order_in_layer } | YSort { origin_offset } | Iso { axis }
  parallax: Vec2 (1 = moves with camera)
  lights: Light3D | Light2D | Unlit
  space: World | Screen (UI)
SortGroup: collapses a subtree into one sort key (Unity Sorting Group [R111])
Sort key (2D layers) = (layer, order_in_layer, sort group, mode key, stable id)
```
- Frame: 3D layers first (depth tested; HD-2D sprites are alpha-cut billboards inside them), then 2D layers into the
  same HDR target (or a low-res target for pixel looks), then the post chain, then screen-space UI.
- Side-scroller with 3D depth: 3D layer for the world, perspective camera, sprites as LitSprite billboards; or an
  orthographic camera with parallax layers.
- Isometric: orthographic camera at 2:1 dimetric, `Iso` sort for sprites, or real 3D tiles with depth.
- Billboard modes for sprites in 3D: face camera, Y-axis locked (HD-2D characters), fixed.

### 6.1 2D renderer

- Sprites, tiles, 2D shapes and text are instanced quads: one storage buffer of `{transform 2x3, uv rect, layer or
  atlas page, tint, sort key}`; one draw per (layer, material, texture array) batch. Texture arrays on native
  (DX12/Vulkan/Metal), etagere atlas pages as the portable fallback [R21].
- Tilemaps: chunks of 16x16 cells, one instance buffer per chunk, rebuilt on edit; chunk culling; Y-sorted tile
  layers fall back to per-tile instances (the Godot quadrant rule [R105]). Import LDtk (MIT) and Tiled files [R107][R108].
- 2D lights (M3): point, spot, directional, freeform, global ambient; normal and mask secondary textures per sprite;
  occluder polygons into a 1D shadow map per light; light and occluder masks; shadow filter None or PCF (Godot
  model [R101]). Pixel-art lights snap to the art grid. Later: 2D radiance cascades for soft GI in 2D [R19].
- Pixel-perfect camera: reference resolution, integer scale with crop, render to a low-res target one pixel larger,
  shift by the sub-pixel fraction when upscaling [R100][R109]. The `pixel_art.wgsl` upscale sample does this.
- 2D physics: Box2D v3 through `boxdd` (MIT/Apache) per the spec.

## 7. Animation system

Implementation is #40 (`metallic_anim`). This is the target design it should grow into.

### 7.1 Data model

```
Skeleton: bone names, parents (sorted so parents come first), rest pose, retarget tags
Clip: duration, sample rate, tracks, markers (named events), root-motion track
  Track: target (bone TRS | blend shape | Instance property | sprite frame | event | audio)
         keys + interpolation (Step | Linear | Slerp | Cubic/Bezier + easing)
  Compiled form: uniform-sampled, quantised SoA (ozz/ACL style [R126][R129])
Rig: skeleton + chains (for IK and retarget) + constraints + masks (per-bone weights)
Graph: layers (override | additive, mask, weight, sync)
       -> state machines (states, one-way transitions with conditions, conduits, crossfade or inertialize)
       -> blend nodes (Blend2/N, BlendSpace1D/2D incl. Discrete mode for sprites, Additive, OneShot, TimeScale)
       -> modifier stack (look-at, two-bone IK, FABRIK, foot placement, spring bones, ragdoll blend)
Tween: Instance property + easing (Roblox TweenService names and the easings.net set)
```
Roblox-facing API on top: `Animator:LoadAnimation`, `AnimationTrack:Play(fadeTime, weight, speed)`, priorities
Idle..Core, `GetMarkerReachedSignal`, `IKControl` [R123][R124][R125].

### 7.2 Runtime order per frame

1. Graph update on the game thread (parameters, transitions).
2. Sample clips (parallel, SoA). 3. Blend layers and additives in local space.
4. Extract root motion; hand it to the character controller.
5. Local to model space. 6. Modifiers in fixed order; each writes local corrections, then the affected chain is
   recomputed (ozz and Godot SkeletonModifier3D pattern [R127][R122]).
7. Fire markers crossed between the previous and current time.
8. Upload bone palettes; a compute pass skins each mesh once per frame into a buffer that the shadow, prepass and
   main passes all reuse; morph targets apply before skinning [R130]. Linear blend skinning by default, dual
   quaternion as a per-mesh option [R131].

### 7.3 2D animation

- Tier 1 (M3): flipbook clips (a Step track on the sprite frame) and a Discrete BlendSpace2D for 4 or 8 facing
  directions.
- Tier 2 (M6): 2D bones with mesh deformation, reusing Skeleton/Clip/Graph with z = 0, plus draw-order and
  sprite-swap tracks. Import DragonBones (MIT) and possibly Rive (MIT runtime). No bundled Spine runtime: its licence
  requires each user to buy an editor licence [R134][R135].
- Sprite bake tool: render a 3D model's animation to a sprite sheet plus normal map, Dead Cells style [R63].

### 7.4 Editor order (M6, after #40's widgets)

Clip timeline with easing presets and curve editor; sprite-sheet slicer and flipbook preview; state machine editor
with live parameter view during Play; rig setup and 2D weight painting; IK posing; retarget chains; motion matching
database last.

## 8. VFX system

Implementation is #41 (`metallic_vfx`). Target design:

- **One IR, two front ends.** Beginners use `ParticleEmitter`, `Trail`, `Beam` and `Decal` Instances with Roblox-like
  flat properties (Rate, Lifetime, Speed, SpreadAngle, Size/Transparency/Color sequences, Drag, Acceleration,
  Flipbook, LightEmission) [R158][R159]. Each is a fixed template graph whose inputs bind to those properties.
  "Convert to graph" opens it in the node editor (one way), as Godot converts CPU and GPU particles [R156].
- **IR:** contexts Spawn (CPU: rate, bursts, events, tier scaling) -> Init -> Update -> Output(s), each a stack of
  modules whose inputs are expression graphs (Niagara + VFX Graph [R154][R150]). Only attributes a later stage reads
  get stored [R151]. Compiles to WGSL (naga-validated, cached by hash) and to CPU bytecode for tiny emitters and
  previews (Destiny [R161]).
- **GPU runtime:** slab of particle pools in large storage buffers; dead list, ping-pong alive lists, counters and
  indirect args packed into one bookkeeping buffer to stay inside the downlevel limit of 4 storage buffers per stage
  [R169]. Per frame: spawn requests -> emit -> update (depth collision, AABB via integer atomics, since DX12 has no
  float atomics [R21]) -> optional sort (16-bit depth keys) -> draw indirect. Batch small emitters of the same
  variant into one dispatch (Niagara's 1-vs-64 particle note [R155]). `particles_sim.wgsl` and
  `particles_draw.wgsl` prototype emit/update/finish and flipbook + soft-particle drawing.
- **Rendering:** billboards pulled from storage (6 vertices, no vertex buffer); premultiplied alpha so additive and
  blended share a blend state; soft particles by depth fade; flipbook frame blending; translucent particles at half
  or quarter resolution on Low with max-depth downsample [R166]. Ribbons: strips with a fixed particle count per
  strip as a ring buffer, expanded in the vertex shader; Beam = Bezier ribbon with N segments.
- **Decals:** clustered decals in the forward pass from an atlas, sorted (DOOM, Wicked [R167][R165]); a ring buffer of
  short-lived box decals (bullet holes, footprints) against the prepass depth.
- **UGC safety:** hard caps per emitter (capacity, rate, lifetime; Roblox's 400/s and 20 s as defaults [R158]), per
  place and per frame; bounded graph loops; validation before compile.
- **Scalability:** Effect Types per asset (spawn scale, max instances, cull distance, renderer off per tier) [R155].

## 9. Quality tiers and budgets

All numbers are judgment and need profiling (the `research-performance` session may refine them).

| Tier | Target | Internal res | Shadows | AO / GI | AA | Post | VFX |
|---|---|---|---|---|---|---|---|
| Low | Iris Xe, 30 fps at 1080p output | 67% (720p) + FSR1 or bilinear; dynamic 50-100% | 2 cascades, 1024-2048 | sky + probes, GTAO half-res Low or off | FXAA / CMAA2, MSAA 2x max | cheap passes only; bloom half-res | 64k sim, 16k drawn, half/quarter-res translucency |
| Medium | GTX 1650 class, 60 fps at 1080p | 100% | 3 cascades 2048 | probes, GTAO half-res | MSAA 4x or TAA | all cheap + DoF half-res | 256k sim |
| High | RTX 3060 Ti, 60 fps at 1080p-1440p | 100% | 3-4 cascades 2048, contact shadows | probes, GTAO, SSR half-res, volumetrics | MSAA 4x or TAA (+FSR2) | all | 1M sim, 250k drawn |
| Ultra (later) | RTX 3060 Ti+, 60 fps | 100% | + better filtering | DDGI (ray query/voxels) | TAA | all | same |

Frame budgets (judgment):

| Pass | High (16.6 ms) | Low (33.3 ms) |
|---|---|---|
| Shadows | 2.0 | 3.0 |
| Prepass + culling | 1.0 | 1.0 (depth only or off) |
| Opaque + sky | 4.0 | 10.0 |
| AO / GI | 2.0 | 1.0 |
| Transparent + VFX | 2.5 | 3.5 |
| Post + finish + upscale | 1.5 | 3.0 |
| UI | 0.5 | 1.0 |
| Headroom (driver, thermal, spikes) | 3.1 | 10.8 |

Every Look lists its Low-tier fallbacks (section 5.5). Pixel 3D, Retro, 2D Pixel and Low-poly run cheaper than
Realistic because they render at low resolution or skip AO.

## 10. Roadmap mapped to milestones

| Milestone | Render / style work | Animation | VFX |
|---|---|---|---|
| **M1 now** (Task 3) | The section 13 hooks only: HDR target + finish pass, shading model id slot, sampleable depth, render scale field, named stages | none | none |
| M2 (multiplayer; renderer track starts) | Look asset + `Lighting.Look`; presets Realistic, Soft, Toon, Low-poly; post: tonemap curves, grade/LUT, bloom, edge outlines, FXAA; Hillaire sky + time of day (with #37); quality tiers + dynamic resolution | Roblox-named tracks and tweens on #40 | Beginner ParticleEmitter on the GPU runtime (#41) |
| M3 (2D) | 2D layers, sprite batching, tilemaps, pixel camera, 2D lights + normal maps; presets Pixel 3D, Retro 32-bit, 2D Pixel, 2D Smooth, HD-2D (LitSprite) | flipbook clips, Discrete BlendSpace2D | Trail, Beam, flipbooks |
| M4 (creation workflow) | glTF materials into the Material model; texture compression (KTX2/BC) | glTF skins and clips import; GPU skinning | Decals |
| **M5 renderer track (#18)** | render graph; clustered forward lights; GPU-driven culling + multi-draw indirect; probes + reflection probes; GTAO; contact shadows; volumetrics; TAA/FSR2; presets Comic, Ink, Painterly; local post volumes; static DXC + pipeline precompile; custom WGSL materials with sandboxing | | sorting, low-res particles, lighting for particles |
| **M6 modeller/animation/VFX (#19)** | material node editor (egui-snarl -> WGSL); Look editor; sprite bake tool | timeline, curve editor, state machine editor, rigging, IK, 2D bones | node-based VFX editor on the same IR |
| Later (Ultra) | DDGI with ray query or voxel tracing; radiance cascades or surfels research; visibility buffer for dense PBR | motion matching | GPU events, sim stages over grids |

## 11. Proof: style shader samples

Files in `docs/research/style-shaders/` (research samples, not engine code):

| File | What it shows |
|---|---|
| `surface.wgsl` | Task 3's vertex layout with `material.w` as lighting model id: Lit (GGX), Toon (bands, rim, spec), Unlit, Flat (faceted). HDR output + view normal and per-material outline mask in a second target |
| `toon_ramp.wgsl` | Ramp-texture toon (anime) with ramp row per material, spec and rim |
| `outline_hull.wgsl` | Inverted hull, constant pixel width in clip space, per-vertex width, outline-normal attribute, clip-space cap |
| `outline_post.wgsl` | Depth/normal edges: dark line on the near side of silhouettes, light line on creases, per-material mask |
| `tonemap.wgsl` | Finish pass: exposure, white balance, ACES fit / Khronos PBR Neutral / clamp, contrast, saturation, 3D LUT, sRGB handling |
| `pixel_art.wgsl` | Low-res quantise (levels or 64-colour palette in OKLab) with Bayer 4/8 from bit maths; sharp-bilinear upscale with sub-texel camera offset |
| `ps1_retro.wgsl` | Vertex snapping, affine UVs (`@interpolate(linear)`), Gouraud, vertex fog, cutout alpha |
| `particles_sim.wgsl` | GPU particles: emit from dead list, indirect update, survivors to ping-pong alive list, DrawIndirect args |
| `particles_draw.wgsl` | Billboards from storage, flipbook frame blend, soft particles, premultiplied alpha |
| `check/` | Standalone Rust harness (own `[workspace]`, not in the engine workspace): builds every pipeline on a real GPU and renders four looks |

What I ran (2026-10-30, 01:05-01:40 Chicago, this PC: RTX 3060 Ti, driver 610.88):
```
cargo install naga-cli --version 30.0.1 --locked          # matches Task 3's naga 30.0.1
naga <file>.wgsl                                          # each of the 9 files: "Validation successful"
naga <file>.wgsl out.spv | out.hlsl | out.metal           # all 9 translate to SPIR-V, HLSL, MSL
naga --metal-version 2.1 particles_draw.wgsl out.metal    # instance_index needs MSL 2.1; default MSL fails
cargo run --release --manifest-path docs/research/style-shaders/check/Cargo.toml --bin style_shaders_check -- dx12
cargo run --release --manifest-path docs/research/style-shaders/check/Cargo.toml --bin style_shaders_check -- vulkan
WGPU_DX12_COMPILER=Fxc <same binary> dx12                 # 13 pipelines each run, 0 failures on all three
cargo run --release --manifest-path docs/research/style-shaders/check/Cargo.toml --bin render_looks -- <out dir>
```
(`CARGO_TARGET_DIR` pointed at a scratch folder so nothing built lands in the repo.)

`render_looks` draws the same seven-object scene through `surface.wgsl` and the post samples. Output in
`docs/research/style-shaders/shots/`:
- `pbr.png`: Lit + ACES fit.
- `toon.png`: Toon (3 bands, tinted shadows, rim, spec blob) + 2 px post edges + PBR Neutral.
- `flat.png`: Flat model on a low-poly sphere + 1 px edges + PBR Neutral.
- `pixel.png`: Toon at 320x180, 1 px edges, clamp curve, 16-colour palette, Bayer 4x4 at 0.4, sharp upscale to
  1280x720.

I looked at all four images: each look reads as intended. The samples have no shadow map, so no look shows shadows.
The images prove the passes compose and run; they say nothing about performance. The tuning pass showed one lesson:
a global Fresnel rim lights up the far ground in toon looks, so rim must be a per-material setting (it is, in
`toon_ramp.wgsl` and in the Material model).

Not verified: anything on Intel Iris Xe (no such GPU on this PC); Metal (no Mac); DXC through `static-dxc` (the
default `Auto` compiler had no DXC available here, so DX12 runs used FXC both times).

## 12. Could not verify

- Primary sources that could not be fetched: Guilty Gear Xrd GDC slides (used ASW's later toon-line deck instead),
  Ben Golus's wide-outlines article (used his JFA gist), Lucas Pope's Obra Dinn devlog, Praun hatching PDF, Dreams
  slides, t3ssel8r's own write-up, Unreal's Hi-Fi Rush and Octopath interviews, Kyprianidis Kuwahara paper, Unity
  Volume docs, Unreal LUT docs, Niagara simulation stages pages, Bevy render graph docs, three.js/Babylon renderer
  internals, octahedral impostor articles, Wicked Engine compute skinning post, FABRIK paper.
- Genshin details come from fan reverse engineering, not HoYoverse.
- Exact BotW shading model, Okami implementation, Roblox Future lighting internals.
- Whether wgpu 30 ray queries work on DX12 and Metal (docs say Vulkan only; changelog shows progress).
- Iris Xe bandwidth and real costs of every pass on it; Intel `drawIndirectCount` support.
- Upstream licences of DXC, basis_universal, meshoptimizer, Tracy, OIDN; `encase` is MIT-0 (not on the list).
- Spine licence reading should get a legal check; the conclusion (not usable for UGC) is my reading of its text.
- Every budget and cost number marked judgment.

## 13. Urgent: hooks Task #3 (renderer, being written right now) should include so styles are not blocked later

Cheap only; none changes M1's look or tests. Ordered by value.

1. **HDR scene target + separate finish pass.** Render sky, opaque and transparent into an `Rgba16Float` texture
   (MSAA resolves into it). Move `finish()` (exposure, ACES, sRGB encode, dither) from `scene.wgsl` into a
   full-screen pass that writes the swapchain. Cost at 1080p: one 16 MB write + read, well under 1 ms even on
   Iris Xe (judgment). Every style, bloom, TAA and the Studio viewport's grading hang off this.
2. **A post chain slot**: `Vec<PostPass>` (or an enum list) run between the scene and the output, holding only
   `Tonemap` in M1. `Tonemap` takes a `curve` field (ACES fit now; PBR Neutral and Clamp are about 15 lines each in
   `tonemap.wgsl`).
3. **Shading model id in `InstanceRaw.material[3]`.** Name it (`shading_model: f32`, 0 = Lit), add
   `pub enum ShadingModel { Lit = 0 }` in `materials.rs`, and in `scene.wgsl` split `fs_main` into `surface()` (the
   existing `detail()` + colour) and `shade_lit()`, called through a `switch` on the id. Only Lit exists in M1.
   Compute screen derivatives before the switch (naga rejects derivatives in non-uniform control flow).
4. **Sampleable depth.** Create the depth texture with `TEXTURE_BINDING` usage and `StoreOp::Store` when any post
   pass asks for it (none in M1, so keep Discard as the default and make it a flag).
5. **Internal size separate from output size.** `RenderSettings { quality, render_scale: f32 }` (1.0 in M1); the
   HDR and depth targets use `output size x render_scale`; the finish pass samples with a linear filter. This
   unlocks pixel looks and the Iris Xe 67% mode.
6. **Named stages.** Keep `render()` as `shadows() -> scene() -> post() -> finish()` private methods, each taking the
   encoder and named targets. That shape lets a render graph replace the order in M5 without a rewrite.
7. **Leave room in `Globals`** (or add a second uniform for post) instead of packing more flags into
   `params.y/z/w`; the finish pass needs its own small uniform.

Do not add in M1: deferred, a G-buffer, a normal target, outlines, a node editor, a render-graph crate.

## 14. Proposed new issues (not created)

Existing issues cover lighting research (#37), the animation crate (#40) and the VFX crate (#41); these do not
repeat them.

1. **Renderer: HDR target, finish pass and post chain slot** (M1 follow-up if #3 does not take it).
   Render into Rgba16Float, move tonemapping to a full-screen finish pass, add a post-pass list. Unblocks all styles.
2. **Renderer: shading model id and material table.**
   Per-instance material index into a GPU material buffer; built-in models Lit, Unlit, Toon, Flat in one uber shader.
3. **Look presets v1: Look asset and `Lighting.Look`.**
   Text asset for render scale, shading defaults, post list and tier fallbacks; ship Realistic, Soft, Toon, Low-poly.
4. **Post stack v1: edges, bloom, grade/LUT, quantise/dither, pixel upscale, FXAA.**
   Data-driven passes in HDR and LDR slots with per-tier fallbacks; port the samples in `docs/research/style-shaders/`.
5. **Quality tiers, render scale and dynamic resolution.**
   Low/Medium/High with budgets from this doc, GPU timestamp-driven scale 50-100%, FSR 1 upscale (MIT).
6. **Render graph (FrameGraph-lite).**
   Passes declare reads/writes; unused passes skipped; transient textures aliased later. Replaces the fixed order.
7. **Clustered forward lights, reflection probes and decals.**
   Froxel light binning in compute; point/spot lights, box-projected reflection probes, sorted clustered decals.
8. **GPU-driven culling and multi-draw indirect for Parts.**
   Compute frustum + two-phase Hi-Z occlusion culling writing indirect args; CPU path stays as fallback.
9. **GI tier 1: irradiance probe grid baked in Studio + GTAO.**
   DDGI-style probe layout filled by raster captures; XeGTAO-style AO at half res on Low.
10. **2D renderer: sprite batching, tilemaps, pixel camera, 2D lights.**
    Instanced quads with sort modes, chunked tilemaps (LDtk/Tiled import), pixel-perfect camera, normal-mapped 2D lights.
11. **Unified camera and RenderLayer model.**
    Layers with Depth/Painter/YSort/Iso sorting, parallax, per-layer cameras; HD-2D billboards in 3D layers.
12. **Shader pipeline: wesl modules, static DXC, pipeline precompile.**
    Precompile all pipeline keys at load with a fallback material; turn on `static-dxc` after a licence check.
13. **Creator shaders: WGSL contract and sandboxing.**
    `surface()`/`light()`/`vertex()` contract, naga validation, loop and instruction caps, off-thread compile.
14. **Material node editor (egui-snarl to WGSL).**
    Surface/Light/Vertex/Post graph types, subgraphs, generated WGSL view, per-node naga errors, hot reload.
15. **GPU skinning and glTF skinned meshes.**
    Compute skin once per frame into a buffer shared by shadow, prepass and main passes; morph targets first.
16. **Sprite bake tool (3D animation to sprite sheet + normal map).**
    Dead Cells-style pipeline for 2D games made from 3D models; output feeds LitSprite materials.

## References

Fetched during this research. Local: [L1] `~/.cargo/registry/src/*/wgpu-types-30.0.1/src/backend.rs`
(`Dx12Compiler`), [L2] `metallic-studio-worktrees/3-renderer` at `f80efcb` (read-only).

- R1 https://docs.godotengine.org/en/stable/tutorials/rendering/renderers.html
- R2 https://docs.godotengine.org/en/latest/engine_details/architecture/internal_rendering_architecture.html
- R3 https://bevy.org/news/bevy-0-12/
- R4 https://docs.rs/bevy/latest/bevy/pbr/index.html
- R5 https://docs.unity.com/en-us/engine/6000.0/manual/render-pipelines/universal-render-pipeline/introduction/urp-concepts/rendering-paths/comparison
- R6 https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Forward-And-Deferred-Rendering.html
- R7 https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine
- R8 https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- R9 https://jcgt.org/published/0002/02/04/paper.pdf
- R10 https://developer.playcanvas.com/user-manual/graphics/lighting/clustered-lighting/
- R11 https://create.roblox.com/docs/reference/engine/enums/Technology.md
- R12 https://create.roblox.com/docs/environment/lighting
- R13 https://create.roblox.com/docs/reference/engine/classes/Lighting
- R14 https://www.slideshare.net/slideshow/framegraph-extensible-rendering-architecture-in-frostbite/72795495
- R15 https://acko.net/blog/teardown-frame-teardown/
- R16 https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/introduction_to_global_illumination.html
- R17 https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html
- R18 https://jcgt.org/published/0008/02/01/paper-lowres.pdf
- R19 https://github.com/Raikiri/RadianceCascadesPaper
- R20 https://www.ea.com/seed/news/siggraph21-global-illumination-surfels
- R21 https://docs.rs/wgpu/latest/wgpu/struct.Features.html
- R22 https://wgpu.rs/doc/wgpu/documentation/extensions/ray_tracing/index.html
- R23 https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/CHANGELOG.md
- R24 https://sebh.github.io/publications/egsr2020.pdf
- R25 https://github.com/sebh/UnrealEngineSkyAtmosphere
- R26 https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf
- R27 https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine
- R28 https://github.com/GameTechDev/XeGTAO
- R29 https://www.bendstudio.com/blog/inside-bend-screen-space-shadows/
- R30 https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games
- R31 https://store.steampowered.com/app/2198150/Tiny_Glade/
- R32 https://docs.rs/wgpu/latest/wgpu/struct.PipelineCache.html
- R33 https://docs.rs/wgpu/latest/wgpu/struct.RenderPass.html
- R34 https://bevy.org/news/bevy-0-16/
- R35 https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf
- R36 https://docs.godotengine.org/en/stable/tutorials/performance/pipeline_compilations.html
- R37 https://dev.epicgames.com/documentation/en-us/unreal-engine/pso-precaching-for-unreal-engine
- R38 https://dev.epicgames.com/documentation/en-us/unreal-engine/dynamic-resolution-in-unreal-engine
- R39 https://github.com/GPUOpen-Effects/FidelityFX-FSR2
- R40 https://github.com/GPUOpen-Effects/FidelityFX-FSR
- R41 https://www.intel.com/content/www/us/en/products/sku/208662/intel-core-i71185g7-processor-12m-cache-up-to-4-80-ghz/specifications.html
- R42 https://www.intel.com/content/www/us/en/developer/articles/guide/developer-and-optimization-guide-for-intel-processor-graphics-gen11-api.html
- R43 https://www.intel.com/content/www/us/en/docs/oneapi/optimization-guide-gpu/2024-0/intel-xe-gpu-architecture.html
- R50 https://image.docswell.com/s/ASW_Academy/5LVY67-GG-Toonline-Eng
- R51 https://www.blendernation.com/2015/07/26/junya-c-motomura-behind-the-scenes-of-guilty-gear-xrd/
- R52 https://juejin.cn/post/7207631397094948920
- R53 https://www.gcores.com/articles/181912
- R54 https://80.lv/articles/the-making-of-hi-fi-rush-s-3d-toon-rendering-style
- R55 https://thefamicast.com/2017/12/part-5-art-and-implementation-of-3d.html
- R56 https://en.wikipedia.org/wiki/%C5%8Ckami
- R57 https://godotshaders.com/shader/anisotropic-kuwahara-filter/
- R58 https://hhoppe.com/proj/hatching/
- R59 https://blog.maximeheckel.com/posts/moebius-style-post-processing
- R60 https://godotshaders.com/?p=4758
- R61 https://gist.github.com/bgolus/a18c1a3fc9af2d73cc19169a809eb195
- R62 https://arxiv.org/html/2603.14587v1
- R63 https://www.gamedeveloper.com/production/art-design-deep-dive-using-a-3d-pipeline-for-2d-animation-in-i-dead-cells-i-
- R64 https://en.wikipedia.org/wiki/HD-2D
- R65 https://www.siliconera.com/project-octopath-traveler-developers-answer-project-started-troubles-developing-hd-2d/
- R66 https://setsideb.com/?p=5672
- R67 http://momentsingraphics.de/BlueNoise.html
- R68 https://github.com/MenacingMecha/godot-psx-style-demo
- R69 https://www.mediamolecule.com/blog/article/siggraph_2015
- R70 https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Master-Stack.html
- R71 https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Custom-Function-Node.html
- R72 https://docs.unity.com/en-us/engine/6000.0/manual/render-pipelines/universal-render-pipeline/customizing-urp/urp-renderer-feature/feature
- R73 https://docs.unity3d.com/Packages/com.unity.toonshader@0.11/manual/index.html
- R74 https://dev.epicgames.com/documentation/en-us/unreal-engine/shading-models-in-unreal-engine
- R75 https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-material-properties
- R76 https://dev.epicgames.com/documentation/en-us/unreal-engine/instanced-materials-in-unreal-engine
- R77 https://dev.epicgames.com/documentation/en-us/unreal-engine/material-functions-in-unreal-engine
- R78 https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-substrate-materials-in-unreal-engine
- R79 https://docs.godotengine.org/en/stable/tutorials/shaders/shader_reference/spatial_shader.html
- R80 https://docs.godotengine.org/en/stable/tutorials/shaders/visual_shaders.html
- R81 https://create.roblox.com/docs/parts/materials
- R82 https://create.roblox.com/docs/reference/engine/classes/SurfaceAppearance
- R83 https://create.roblox.com/docs/environment/post-processing-effects
- R84 https://docs.rs/bevy/latest/bevy/pbr/struct.ExtendedMaterial.html
- R85 https://github.com/mrdoob/three.js/wiki/Three.js-Shading-Language
- R86 https://github.com/zakarumych/egui-snarl
- R87 https://crates.io/api/v1/crates/egui_node_graph
- R100 https://docs.unity3d.com/Packages/com.unity.2d.pixel-perfect@5.0/manual/index.html
- R101 https://docs.godotengine.org/en/stable/tutorials/2d/2d_lights_and_shadows.html
- R102 https://docs.unity.com/en-us/engine/6000.0/manual/unity2d/2d-urp/2d-index/2d-light-properties-explained
- R103 https://docs.unity3d.com/6000.4/Documentation/Manual/urp/SecondaryTextures.html
- R104 https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@14.0/manual/2DShadows.html
- R105 https://docs.godotengine.org/en/stable/classes/class_tilemaplayer.html
- R106 https://docs.unity.com/en-us/engine/current/manual/unity2d/tilemaps/reference/tilemap-renderer
- R107 https://github.com/deepnight/ldtk
- R108 https://raw.githubusercontent.com/mapeditor/tiled/master/COPYING
- R109 https://yal.cc/gamemaker-smooth-pixel-perfect-camera/
- R110 https://docs.godotengine.org/en/stable/tutorials/2d/canvas_layers.html
- R111 https://docs.unity.com/en-us/engine/6000.6/manual/unity2d/sprite/sort-sprites/sort-sprites
- R113 https://docs.unity.com/en-us/engine/current/manual/animation-section/animation-mecanim/animation-animator-controller/class-animator-controller
- R114 https://docs.unity.com/en-us/engine/current/manual/animation-section/animation-mecanim/animation-animator-controller/animation-state-machines/animation-layers
- R115 https://docs.unity.com/en-us/engine/current/manual/animation-section/animation-mecanim/animation-animator-controller/animation-state-machines/animation-blend-trees/blend-tree-2dblending
- R116 https://dev.epicgames.com/documentation/en-us/unreal-engine/state-machines-in-unreal-engine
- R117 https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-blueprint-blend-nodes-in-unreal-engine
- R118 https://dev.epicgames.com/documentation/en-us/unreal-engine/ik-rig-animation-retargeting-in-unreal-engine
- R119 https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine
- R120 https://docs.godotengine.org/en/stable/tutorials/animation/animation_tree.html
- R121 https://docs.godotengine.org/en/stable/classes/class_animation.html
- R122 https://godotengine.org/article/design-of-the-skeleton-modifier-3d/
- R123 https://create.roblox.com/docs/reference/engine/enums/AnimationPriority
- R124 https://create.roblox.com/docs/reference/engine/classes/AnimationTrack
- R125 https://create.roblox.com/docs/reference/engine/classes/IKControl
- R126 https://guillaumeblanc.github.io/ozz-animation/documentation/animation_runtime/
- R127 https://guillaumeblanc.github.io/ozz-animation/documentation/ik/
- R128 https://github.com/SlimeYummy/ozz-animation-rs
- R129 https://github.com/nfrechette/acl
- R130 https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html
- R131 https://users.cs.utah.edu/~ladislav/kavan07skinning/kavan07skinning.html
- R132 https://www.gdcvault.com/play/1023280/Motion-Matching-and-The-Road
- R133 https://www.theorangeduck.com/page/learned-motion-matching
- R134 https://esotericsoftware.com/spine-runtimes-license
- R135 https://esotericsoftware.com/spine-editor-license
- R136 https://github.com/rive-app/rive-runtime
- R137 https://github.com/DragonBones/DragonBonesJS
- R138 https://create.roblox.com/docs/animation/editor
- R139 https://docs.godotengine.org/en/stable/classes/class_animationnodeblendspace2d.html
- R150 https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/Contexts.html
- R151 https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/Attributes.html
- R152 https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/visual-effect-bounds.html
- R153 https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/System-Requirements.html
- R154 https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-niagara-effects-for-unreal-engine
- R155 https://dev.epicgames.com/documentation/en-us/unreal-engine/scalability-and-best-practices-for-niagara
- R156 https://docs.godotengine.org/en/stable/classes/class_gpuparticles3d.html
- R157 https://docs.godotengine.org/en/stable/tutorials/3d/particles/index.html
- R158 https://create.roblox.com/docs/effects/particle-emitters
- R159 https://create.roblox.com/docs/effects/beams
- R160 https://github.com/djeedai/bevy_hanabi
- R161 https://80.lv/articles/siggraph-the-destiny-particle-architecture/
- R162 https://80.lv/articles/innovating-vfx-in-gaming-the-game-changing-journey-of-persistant-studios/
- R163 https://www.slideshare.net/slideshow/holy-smoke-faster-particle-rendering-using-direct-compute-by-gareth-thomas/35656271
- R164 https://wickedengine.net/category/devblog/2017/11/07/gpu-based-particle-simulation/index.html
- R165 https://wickedengine.net/category/devblog/2017/10/12/forward-decal-rendering/index.html
- R166 https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-23-high-speed-screen-particles
- R167 https://adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/
- R168 https://gpuopen.com/fidelityfx-parallel-sort/
- R169 https://docs.rs/wgpu/latest/wgpu/struct.Limits.html
- R170 https://docs.rs/wgpu/latest/wgpu/struct.DownlevelFlags.html
- R171 https://docs.rs/wgpu/latest/wgpu/struct.AdapterInfo.html
- R172 https://docs.rs/crate/lampshade/latest
- R173 https://github.com/wgsl-tooling-wg/wesl-rs
- R174 https://crates.io/api/v1/crates/<name> (each crate in section 3.9)
