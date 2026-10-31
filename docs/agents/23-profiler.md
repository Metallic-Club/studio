# #23 Task 16: engine profiler and debug views

Status: in progress, split into three PRs (all branches from main).

1. **Core scopes and capture** (`studio/23-profiler-core`, Part of #23): `metallic_profiler` crate, per-script time,
   GPU stage timing, `Game` and host frame marks.
2. **Studio Profiler panel and the player's F3 overlay** (Part of #23).
3. **Debug views** (Closes #23): wireframe, collision shapes, touch volumes, contacts, light and exposure meter,
   overdraw.

## PR 1

- `crates/metallic_profiler`: `Profiler::scope`, `frame`, `begin_frame`/`end_frame`, `charge_script`, `record_gpu`,
  `with_frames`/`latest`; `RingBuffer`; `FrameRecord` (fixed size). Thread-local, off by default.
- Plan tests: `profiler_nests_scopes_and_sums_children`, `disabled_profiler_records_nothing`,
  `ring_buffer_keeps_last_600` (profiler crate), `per_script_time_attributed` (`metallic_script/tests/profiler.rs`).
  Also: no allocation while recording (counting allocator), dropped scopes past the limits, late GPU results,
  `metallic_render/tests/gpu_timing.rs` (scopes and four GPU stages on a real adapter), and
  `metallic_runtime/tests/profiler.rs` (a profiled Parkour game).
- Cost when off: a scope reads one thread-local flag; the frame marks read one more. No clock reads, no queries,
  no allocation. `metallic_core` (the `instructions` gate's benchmark) is unchanged.

## PR 2

- Studio: `profiler/profiler_view.rs` (`ProfilerPanel`), a Profiler tab beside Output (View > Profiler). Record,
  pause or resume, stop, clear; a frame graph stacked by subsystem with 60 and 30 fps lines (a click pauses on that
  frame); subsystem and GPU stage times, a scope timeline, and script times sorted by time or name. Colours come
  from theme tokens. `--scene profiler` plays the demo Parkour and pauses at frame 150 for review screenshots.
- Player: `dev_overlay.rs`, F3 (never passed to the game as a control) or `--dev-overlay`. fps, mean and max frame
  time, GPU time and time per subsystem over the last 60 frames, plus a 180-frame graph. Showing it enables the
  profiler; hiding it disables and clears it, so it costs nothing while hidden.
- Tests: `metallic_studio/tests/profiler_panel.rs` (kittest: records, clicks the graph to pause, resumes);
  `dev_overlay` unit tests (`summary_averages_recent_frames`, `toggling_starts_and_stops_the_profiler`).
- Screenshots: `23-shots/studio-profiler.png`, `23-shots/player-f3.png`.

## Review fixes (#401 FIX-FIRST)

- `with_frames` no longer holds the recording state: the history is a separate thread-local, and a frame that ends
  inside a read is held and joins after it (test `any_profiler_call_works_inside_with_frames`).
- `GpuFrame` (RAII) ends the GPU timer's frame on every return path, so an early return can no longer leave the
  timer active for a later, unprofiled frame (test `a_frame_that_returns_early_is_never_read_back`).
- Ruling: #401 is the profiler of record for #23; #210 is not merged. Follow-ups from #210 that #401 lacks: budget
  tables and checks per subsystem, opt-in simulation profile reports on `Game`, puffin/Tracy export.

## Decisions

- **Not built on #210.** Codex's open #210 adds `metallic_profiling`, whose `Frame` is borrowed from a
  `FrameProfiler` and passed to each scope. Reaching the renderer and the script scheduler that way needs a new
  parameter through their public calls, and it has no frame history. Task 16 wants `Profiler::scope(name)` and a
  600-frame ring, so this is a thread-local collector instead. Ideas taken from #210: subsystems, exclusive
  time charged to the innermost scope, and missing GPU time reported as missing, not zero. Nothing was copied from
  it. If both land, `metallic_profiling`'s budget checks could read `FrameRecord`.
- GPU timestamps come from empty compute passes between stages, so only `TIMESTAMP_QUERY` is needed (not
  `TIMESTAMP_QUERY_INSIDE_ENCODERS`, which some backends lack).
- A Studio frame runs from one `ui` call to the next (it includes rendering and the wait for the next frame).
