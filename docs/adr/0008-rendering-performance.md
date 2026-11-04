# ADR 0008: Rendering performance architecture

- **Status:** Accepted (2026-11-02)
- **Issue:** #32
- **Refines:** [ADR 0004](0004-runtime-rendering-platform.md). That record chose *what* we build on (own loop, `winit`, `wgpu`); this one fixes *how* the renderer stays fast.

## Context

The owner's requirement: **the best performance possible without compromising quality or customization.** Games look like Roblox games: thousands to hundreds of thousands of mostly static parts, a few moving ones, scripted lights and effects. They run on everything from phones to gaming PCs and consoles.

The previous version of this project (`Metallic-Club/studio-achive-`) built a `wgpu` renderer. Its results show both what to keep and what breaks at scale. The facts below were checked in its source:

- **What worked:**
  - One instanced draw per primitive mesh, with materials selected through a per-instance row into a material table, so the draw count stays constant (`renderer.rs:1742-1754`).
  - Syncing only the parts the change journal names (5,000 parts with 50 changed: 97.5 µs).
  - Texel-snapped cascade fitting.
  - An open HDR post chain where disabled passes cost nothing.
  - MSAA stored with `Discard` and resolved in-pass.
  - Exposure metering that stays on the GPU.
  - 5,000 parts at 1080p High on an RTX 3060 Ti: p50 2.88 ms, p99 4.29 ms.
- **What does not scale:**
  - **No culling of any kind.** Every part is drawn in the main pass *and* in every shadow cascade (`renderer.rs:1375-1380`).
  - Any single change rebuilt all instance lists by scanning every slot, then diffed and copied a full CPU mirror (`renderer.rs:1129-1139`, `246-261`).
  - 96-byte instances (a full `mat4`).
  - No LOD for parts, and no point or spot lights.
  - Only two presets, with no render scale or dynamic resolution.
  - Shader pipelines compiled lazily, so turning on an effect mid-game hitched.
  - Standard-Z depth with a 0.3 / 4000 near/far, which invites z-fighting.

## Decision

### 1. Budgets come first

Every renderer PR states its effect against these budgets, measured on the stress scenes (§10).

| Tier | Reference | Target |
|---|---|---|
| Low | Intel Iris Xe laptop, mid-range phone | 30 fps (aim 60 at reduced render scale), Low preset |
| Mid | GTX 1650 / RX 6500 class, current consoles at their own presets | 60 fps, Medium |
| High | RTX 3060 Ti / Ryzen 5 5500 (dev PC) | 60 fps with ≥ 2× headroom, High; uncapped is a bonus |

Per-frame **render CPU** budget (sync + cull + encode + submit), main thread or render thread:

| Tier | Budget |
|---|---|
| High | 1.5 ms |
| Mid | 2.5 ms |
| Low | 4.0 ms |

**Scale target:** a scene of **100,000 parts with 10,000 in view** meets the Mid budget at Medium.

### 2. A GPU-resident scene with stable slots

- Every renderable instance gets a **persistent slot** in a GPU instance buffer when it enters the world, and keeps it until it leaves. A free list reuses holes; nothing is compacted on the CPU.
- **Only dirty slots are uploaded.** The data model's change tracking says which ones; writes to neighbouring slots are coalesced into ranged `write_buffer` calls through a staging ring. There is no full-scene scan or mirror diff on change.
- Static and dynamic instances live in separate regions, so a moving part never touches the static data.
- **Compact instance format, ≤ 64 bytes:** a 3×4 affine transform (48 B), colour packed into `u32`, material row and flags in `u32`, and the rest reserved. A storage buffer is used where supported; the vertex-buffer fallback is for GLES.

### 3. Culling is mandatory and layered

- **CPU path (every backend):**
  - static instances live in a spatial structure (BVH or loose grid), refit incrementally when parts move;
  - each view (main camera, every shadow cascade, every local-light shadow) culls against it;
  - per-preset distance culling, plus screen-size culling of tiny distant parts.
- **GPU-driven path (where compute and indirect draws exist: desktop and consoles):**
  - a compute pass culls slots and writes compacted visible-instance lists and indirect draw arguments;
  - `multi_draw_indexed_indirect` where available;
  - the CPU path stays the fallback and the reference for tests.
- **Occlusion culling** (two-phase Hi-Z) is optional and on only where it measurably wins. Forge measured it losing on CPU-bound scenes and with MSAA.
- **Shadow casters are culled per cascade**, against the cascade's own volume extended toward the light.

### 4. A constant number of draws

**Kept from the archive:** instancing by mesh with a material table and one uber-pipeline per shading model. Draw count depends on the number of *mesh kinds* and *shading models*, never on part count. Mesh assets join the same scheme through a shared vertex/index pool, so they can be drawn indirectly too.

### 5. Level of detail

- Primitive meshes (sphere, cylinder, wedge …) have 3–4 LOD levels, chosen by projected screen size. LOD bias is a quality setting.
- Imported meshes get an LOD chain at import time (`meshopt` simplification).
- Impostors and merged clusters for far static geometry come later, behind the same interface.

### 6. Lights and shadows

- **Clustered forward lighting** (a froxel grid) for point and spot lights. The light budget per cluster and per frame is set by the preset; lights beyond the budget fade by importance (brightness × screen size), never pop.
- **Sun cascades:**
  - count, resolution and distance per preset (Low: 1 × 1024; High: 3 × 2048, as the archive measured);
  - texel-snapped (kept);
  - far cascades that contain only static casters are **cached** and redrawn only when the sun moves or a static caster in them changes.
- **Local-light shadows** come from a shadow atlas with a per-preset budget, assigned by importance.

### 7. Overdraw and bandwidth

- **Reverse-Z** with `Depth32Float` (`Depth24Plus` or `Depth16Unorm` for shadow maps on mobile where it measures better).
- The sky is drawn **last**, at the far plane with a depth test, not first with a full-screen overdraw.
- Opaque instances are roughly sorted front to back by cell. An optional depth prepass on desktop, chosen per preset, applies when the scene has heavy overdraw.
- **Tile-friendly on mobile:**
  - attachments use `Discard` stores where the result is not read;
  - MSAA resolves in-pass;
  - few full-screen passes;
  - a compact HDR format (`Rg11b10Ufloat`) where quality allows.

### 8. Quality never removed, only scaled: the post chain and settings

- **Presets change *how much*, never *what*.** A game's look (materials, shading models, lighting, post effects, styles) is its developer's choice and is never switched off by a preset. Presets scale cost: render scale, distances, sample counts, cascade and light budgets, LOD bias, effect resolution.
- **Settings** (each one overridable by the player, readable by scripts, and previewable in Studio):

  | Setting | Values |
  |---|---|
  | Preset | Low / Medium / High / Ultra / Auto |
  | Render scale | 50–200% |
  | Dynamic resolution | on/off, with a target frame time |
  | Frame cap / vsync | |
  | View distance | |
  | Shadow quality and distance | |
  | Local-light budget | |
  | LOD bias | |
  | Anti-aliasing | off / MLAA / MSAA 2×, 4×, 8× (TAA later) |
  | Effect quality | bloom resolution, exposure metering resolution |
  | Texture quality | |

- **Auto** runs a short GPU-timed calibration on first launch and after driver or hardware changes, then dynamic resolution holds the frame time.
- **The open post chain is kept** (passes can be added, removed and reordered; disabled passes are free). Passes are **fused** where possible: the bloom composite and grading/LUT/tone curve go in the single finish pass, and exposure metering reads a bloom mip instead of full resolution.
- **Upscaling:** bilinear first; a higher-quality upscaler (FSR 1 class) is planned behind the render-scale setting.

### 9. No hitches by construction

- **All pipeline permutations are known up front.** Pipelines for the enabled feature set are compiled at load on worker threads, never on the frame. Enabling an effect mid-game uses a pipeline that already exists, or compiles in the background and switches when ready.
- Use the Vulkan `PipelineCache` where available, kept on disk. On DX12, DXC is an option rather than the default (#86, measured in `docs/perf.md`): it needs `dxcompiler.dll` shipped beside the executable, and `static-dxc` downloads a prebuilt library while building, so the default build keeps FXC, whose cost the background compiles take off the first frame.
- **Large textures are made off the frame too** (#152): the OS clears a new device memory block on the thread that makes it (about 0.12 ms a MiB measured), so the shadow maps, atlas and attachments a settings change needs are made one a frame on a worker while frames draw with the ones they replace, and each system switches once all of its own exist. A scene's first frame still makes everything at once.
- **No per-frame heap allocation** in sync, cull or encode. Buffers and lists are reused; a test with a counting allocator guards it.
- **No CPU waits on the GPU in the frame** (`device.poll(Wait)`, map-read stalls). Readbacks (timestamps, picking) are asynchronous, a frame or more late.
- **Presentation:** `Fifo` (vsync) by default with `desired_maximum_frame_latency = 2`, and frame pacing for capped frame rates.

### 10. Measurement is part of "done"

- Stress scenes are committed with the renderer:
  - 10k / 50k / 100k static parts with 1% moving;
  - many lights;
  - a heavy-overdraw interior;
  - a mobile-sized scene.
- **GPU timestamps per pass**, plus an in-app overlay (frame time graph; p50 / p99 / 1% low; draws, visible instances and triangles per pass).
- CPU-side renderer workloads (sync, cull, encode) join the instruction-count PR gate (#29). GPU timings run on a controlled machine, not on shared CI.
- Every renderer PR reports before/after numbers on the stress scenes.

### 11. Threads and large worlds

- The main thread produces a frame packet (dirty slots, camera, lights). The render thread culls and encodes it while the main thread runs the next frame's scripts and physics.
- **Camera-relative rendering:** transforms are made relative to the camera before upload, so f32 precision holds far from the origin.

## Consequences

- Culling, stable slots, LOD and pipeline prewarming are **in the first renderer milestone (M3)**, not "later". The M3 epic (#19) is ordered so the stress scenes and the profiling overlay come before any optimisation, to be measured against.
- The GPU-driven path doubles some logic (CPU and GPU culling); the CPU path is the reference that tests compare against.
- Customization stays fully available on every tier. A game can look the same on Low as on Ultra, only at lower resolution, shorter distances and with fewer samples.
