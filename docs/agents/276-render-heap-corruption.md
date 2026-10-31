# #276 Renderer headless tests crash now and then (heap corruption)

Status: fixed. Branch `studio/276-render-heap-corruption`.

## Symptom

Render test binaries died now and then on Windows with `STATUS_HEAP_CORRUPTION` (0xc0000374), an access violation
(0xc0000005) or exit 139, with no failing assertion. One `post_chain` run also hung.

## Root cause

`RenderContext::new_headless` used `wgpu::Instance::default()`, which asks for every backend. wgpu-core then sets up
every HAL, OpenGL included, even when Vulkan or DX12 is picked. On Windows, each GL instance (wgpu-hal `gles/wgl.rs`)
makes a hidden window on its own thread, chooses and sets a pixel format and creates a WGL context in the vendor's
OpenGL driver. Tests run on parallel threads, so several threads did this at once, and the driver corrupted the heap
or faulted. Upstream: gfx-rs/wgpu#9760 (access violation in `ChoosePixelFormat` when Vulkan and GL are set up) and
#6211 (WGL context activation is unsound). No `unsafe` in `metallic_render` is involved.

## Evidence (Windows 11, AMD GPU, `tests/context_stress.rs`, 10 runs per row unless noted)

| What each thread does (GL only) | Threads x rounds | Crashes |
|---|---|---|
| Create and drop an instance | 8 x 40 | 7, 8 (two series) |
| Same, one thread | 1 x 320 | 0 |
| Instance and adapter | 8 x 30 | 5 |
| Whole device (old code) | 8 x 20 | 2 |
| Whole device, one thread | 1 x 160 | 0 |
| Instance, creation under a process-wide lock | 8 x 40 | 0 |
| Instance, drop under the lock | 8 x 40 | 8 |
| Whole device (fixed code), 20 runs | 8 x 20 | 0 |

So creating GL instances in parallel races; dropping them in parallel does not matter.

## Fix

- `context.rs`: one process-wide `setup_lock` around every instance, adapter and device it creates
  (`new_for_window`, `new_headless_with_backends`). Rendering and teardown stay parallel.
- `new_headless` tries `Backends::PRIMARY` first and falls back to GL only if that finds no adapter, so most machines
  never set up GL for headless work. Explicit GL tests (`post_chain`, `exposure`) still work and are serialised by the
  lock.
- Tests: `gl_devices_set_up_on_many_threads_at_once` (8 threads at once through `RenderContext`) and
  `headless_prefers_a_primary_backend`. The stress run stays `#[ignore]`.

## Not covered

- eframe (Studio, Launcher) creates its own wgpu instance on one thread; it is not affected.
- Several test processes using the GPU at the same time: one `exposure` run crashed (0xc0000005) while three
  `headless` loops and `post_chain` ran in other processes; the 75 `headless` and 25 `post_chain` runs in that load
  were clean, and so were 30 `exposure` runs alone. That cross-process load is not what `cargo test` does (it runs
  test binaries one after another).
