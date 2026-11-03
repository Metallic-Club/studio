# ADR 0004: Runtime, rendering and platform layer (no Bevy)

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #5

## Context

Games have to run smoothly on everything from gaming PCs to phones, and on PlayStation and Xbox. Forge-Engine used Bevy for the window, the loop, the ECS and rendering. Its own notes show what that cost:

- **Bevy entities were only mirrors of the data model**, so the ECS stored a second copy of state the engine already owned.
- **Every Bevy upgrade was work and lost performance.** It needed a `compat.rs` shim, and going from Bevy 0.16 to 0.19 raised the player's p50 frame time from 1.29 ms to 2.29 ms (editor 1.61 → 3.53 ms) before manual fixes.
- **Bevy has no console support** and little control over mobile-specific trade-offs.
- **Bevy dominated compile times and binary size.**

## Decision

### No Bevy. Own loop and renderer on proven low-level crates.

| Concern | Choice | Where |
|---|---|---|
| Window, input, events | `winit` on desktop, Android and iOS; private platform crates on consoles | `metallic_platform` |
| GPU | `wgpu`: Vulkan, Metal, DX12, and GLES as a fallback for old Android. Shaders in WGSL (naga) | `metallic_render` |
| Consoles | a private backend behind the same `metallic_render` and `metallic_platform` seams (NDA, private repo) | outside this repo |
| Loop | fixed physics step (60 Hz, sub-steps) plus a variable render step; `RunService` signals around them | `metallic_engine` |

**Alternatives considered:** Bevy (Forge's path; rejected for the reasons above), Fyrox (same coupling problem, no consoles), raw Vulkan/Metal/DX12 (too much work before it pays off; `wgpu` covers them).

**Cost we accept:** we write the renderer ourselves (PBR materials, lights, shadows, culling, batching). It is planned as a sequence of small issues in M3, starting with "parts and lights on screen" and adding features behind quality settings.

### Performance is a design input, not a later pass

The renderer is built around these from the start:

- **Batching / instancing:** parts that share a mesh and material draw in one call, and colour travels per instance.
- **Culling:** frustum culling always; distance and LOD culling per quality level. Occlusion culling only where measured to win (Forge measured it losing on CPU-bound scenes and with MSAA).
- **Lights:** clustered forward lighting. The shadow cascade count and resolution come from the quality level (Forge measured the sun's cascades as the largest single cost of big scenes).
- **Dynamic resolution** with a target frame time, plus a frame cap. Mobile adds battery and thermal-aware limits.
- **Only what changed is re-uploaded**, driven by the data model's dirty bits.

### Graphics settings are part of the engine

- **Presets:** Low, Medium, High, Ultra, and Auto (a short benchmark picks the preset, then dynamic resolution holds the frame time).
- Each preset is a set of individual settings (render scale, shadows, view distance, anti-aliasing, effects), and each can be overridden.
- Settings are stored per user by the player, changeable while the game runs, previewable in Studio, and readable by scripts (read-only).

### Headless

- With the `render` feature off, the engine runs the same loop with no window and no GPU (dedicated servers, CI tests).
