# #111: rendering research samples, fixes from the #61 review

Status: tested (research samples in `docs/research/style-shaders/`, not engine code).

| Review point | Change |
|---|---|
| Hook 5 breaks `Eq` (`render_scale: f32`) | The research doc now proposes `render_scale_percent: u8` (100 in M1). |
| Particle buffer budget (7 bound, 5 in `cs_update`, doc says at most 4) | `particles_sim.wgsl` packs alive A, alive B and the dead list into one `lists` buffer (a `parity` field picks the current half), and uses one `counters` buffer and one `args` buffer in group 1. No entry point binds more than 3 storage buffers; there are 4 in all. The doc's claim that counters and args share one buffer was wrong: a dispatch may not read its indirect args from a buffer it binds for writing. The doc now says so. |
| Compute and the hull, ramp and PS1 samples are only compiled | `style_shaders_check` runs the particle simulation for 8 frames and checks the read-back counters, indirect args, lists (every slot exactly once) and particles against a CPU model, including recycling. The doc lists what is run, drawn, or only compiled. |
| Flipbook wraps to frame 0 at end of life | By age: `f = clamp(t, 0, 1) * (total - 1)`, and the next frame is clamped, so the flipbook plays once and holds the last frame. With `fps > 0` it still loops. |
| `bands = N` gives N + 1 tones | Documented in `surface.wgsl` and the doc: `bands` counts steps (1 = two-tone lit/shadow), which matches how the presets already use it. |
| `check/Cargo.lock` gitignored | Committed. `cargo deny check licenses` with the repo's `deny.toml` passes on it. |
