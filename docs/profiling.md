# CPU scopes and frame budgets

`metallic_profiling` implements bounded CPU recording without depending on the world, renderer, scripts or runtime.
It is re-exported at `metallic_runtime::profiling` for the future game loop. Optional external backend scopes already
instrument implemented script and renderer entrypoints. The current runtime remains a scaffold; CPU budget reports
are not automatically populated by those entrypoints or by unimplemented game stages.

## Record a frame

```rust
use metallic_profiling::{BudgetTable, FrameProfiler, Subsystem, SystemClock, cpu_scope};

let mut profiler = FrameProfiler::<SystemClock, 256>::default();
let frame = profiler.begin_frame();
{
    cpu_scope!(frame, Subsystem::Scripts, "scripts.step");
    // Run the script stage here.
}
let report = frame.finish();
let checks = report.check_budgets(&BudgetTable::proposed_dev_60());
```

Use a literal scope name and a lexical block enclosing the measured work. `cpu_scope!` retains a CPU guard and an
optional backend guard until that block ends. `frame.scope(stage, static_name)` returns just the CPU guard, useful
for explicit lifetimes. Close guards in reverse opening order. Normal Rust borrowing prevents finishing a frame
while a guard remains live. A deliberately forgotten guard is reported as unclosed; it does not turn into a passing
budget check. Dropping an unfinished frame discards it.

Each collector measures one thread. Create separate collectors for worker threads; this API does not merge their
times or add them to the main-thread budget. The frame duration is elapsed CPU wall time between begin and finish,
including unscoped work. It is not CPU utilization, GPU execution time or a sum of subsystem timings.

Subsystem totals are **exclusive**: a physics callback nested inside a script scope is charged to physics for the
callback's duration and to scripts for the rest. Recursion within one subsystem does not double-count its time.
`ScopeSample::inclusive` retains the whole nested duration for timeline consumers; inclusive trace durations can
overlap and must not be summed as the frame's CPU cost. Samples are retained in completion order, with start offset
and nesting depth available for later presentation.

## Bounded storage and honest availability

The sample capacity is a const generic, default 256. Nesting is capped separately at 64 open scopes. The collector
uses fixed arrays, static labels and no heap allocation during default recording, report iteration or budget
assessment. There is no retained history or per-frame background stream. Optional external backends have their own
allocation and overhead characteristics; the zero-allocation test covers the default collector with an explicit
test clock, not the backends or arbitrary user-supplied clocks.

When the trace fills, `dropped_samples()` increases but complete scope durations continue contributing to stage
budgets. Trace capacity zero is supported for accounting without retaining samples. Nesting overflow rejects scopes
and marks attribution incomplete. Bad nesting, forgotten guards or non-monotonic timestamps also prevent complete
stage assessments. Counters expose why. `unscoped_time()` is available only when attribution is complete.

`Measurement` distinguishes:

- `Unavailable`: the stage was not entered in this frame.
- `Complete(duration)`: valid accounting, including a measured duration of zero.
- `Incomplete(duration)`: partial or unreliable observations. This value must not certify a performance target.

Frame elapsed time can remain complete when only stage attribution overflowed; that does not certify the individual
stage budgets. A bad clock makes the frame measurement incomplete too. A `Clock` implementation supplies monotonic
durations since a fixed origin. Tests advance a manual clock instead of sleeping; production uses `SystemClock`
backed by `Instant`. Resolution can yield zero for genuinely short scopes without meaning the stage is free.

## Policies, not achieved targets

`BudgetTable::new(frame_limit).with_subsystem(stage, limit)` configures explicit inclusive upper limits on exclusive
CPU time. Equality passes; strictly larger complete durations exceed the limit. Unbudgeted, unmeasured and incomplete
states remain distinct. There is deliberately no aggregate pass flag hiding missing stage measurements.

The four `proposed_*` tables reproduce the CPU-only proposal in
[`2026-10-30-performance-and-scale.md`, section 3.2](research/2026-10-30-performance-and-scale.md): Dev/60, Mid/60,
Low/30 and Server/60. The frame limits use rounded microseconds, 16,667 or 33,333. These are configurable proposals,
not accepted requirements, device detection or measured hardware performance. Server render/UI limits are `None`,
not zero. A platform or user policy can supply its own table. No GPU budget is evaluated by this crate.

Run the synthetic headless smoke example:

```sh
cargo run -p metallic_profiling --example frame_report
```

Its CPU loop demonstrates report availability and formatting. It is not a Parkour frame benchmark and establishes
no FPS, hardware-tier or profiler-overhead result. The headless Parkour benchmark and Studio developer-mode timeline
remain dependent on the runtime/editor work; #23 is not completed by this foundation.

## Optional backends

Both features use [`profiling` 1.0.18](https://docs.rs/profiling/1.0.18/profiling/), which supports one backend at a time.
No backend is enabled by default. Do not combine `puffin` and `tracy` or their runtime forwarding features.

```sh
cargo check -p metallic_profiling --all-targets --features puffin
cargo check -p metallic_profiling --all-targets --features tracy
```

With `puffin`, the host calls `set_puffin_enabled(true)` to capture scopes; `Frame::finish` marks its frame boundary.
This crate does not add puffin HTTP transport or an in-app view.

Tracy is configured with `manual-lifetime` and `only-localhost` in the direct optional dependency on the same
`tracy-client` version used by `profiling`. A host must call `start_tracy()` at startup, retain the returned handle
and initialize **before any engine or third-party profiling work**. That explicit call can start worker threads
and accept local profiler connections. Dropping the handle does not stop the upstream client: this crate exposes
no shutdown API, and upstream shutdown is unsafe while another thread can still instrument work. Do not infer
automatic shutdown from the handle's lifetime. Ordinary tests must not start the client.

The leaf's Tracy bridge retains an `Option<Span>` in the caller's lexical block; it does not enter a temporary `if`
block that would end the span immediately. Leaf scopes/frame marks are safely inactive before client initialization.
Other libraries' direct `profiling` macros can require a running client, so a complete engine app built with Tracy
must still perform host initialization. Enabling the Cargo feature alone is not a complete app integration.

In runtime builds, forward features via `profiling-puffin` or `profiling-tracy`. Cargo unifies backend features in
other libraries using `profiling`, so external traces may also include their scopes. Applications choose when to
create CPU frames; default CPU accounting is always active for frames/scopes they enter.

External profiler samples and the CPU collector share lexical work boundaries but not a clock implementation or
exact overhead. They should not be treated as bit-identical timelines. No measured timing cost is claimed here.

## Implemented entrypoint scopes and remaining integration

`backend_scope!` exports a static label without needing a frame collector. It is compiled out with no backend.
These are currently instrumented:

| Entry point | Static CPU label |
|---|---|
| `ScriptVm::run_script_owned` (all `run_script` variants route here) | `script.run` |
| `ScriptVm::step` | `script.step` |
| `Scene::sync` | `render.scene_sync` |
| `Renderer::sync` | `render.sync` |
| `Renderer::render` | `render.cpu` |
| `Renderer::prewarm` | `render.prewarm` |

Prewarm can include CPU waiting on device work; that span is not a GPU query. Scope payloads contain no dynamic
script names, source text or user-supplied data. Existing script/render behavior tests cover default-build execution;
individual backend builds are also compiled, and inactive Tracy behavior is tested without starting a client.

This foundation references #121 and leaves it open for:

- Physics entrypoint scopes after the shared leaf crate lands, coordinated with the physics work.
- Actual `Game::frame` scopes, frame boundaries and a headless Parkour frame benchmark after #8 supplies the runtime.
- Host backend initialization and developer-mode budget display after #14.
- Timeline/per-script integration and diagnostics in #23, with its #8/#10 dependencies.

No full-app Tracy host, developer-mode UI, complete per-subsystem budget integration or game benchmark is delivered
by this substrate. The first complete run must be verified through those application integration tasks.
