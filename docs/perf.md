# Performance checks

Two kinds of numbers: **instruction counts**, which gate every pull request that touches the engine, and
**wall-time measurements**, which are recorded by hand on a stated machine and are not a gate. Renderer frame budgets
(GPU and render CPU per tier) are set in [ADR 0008](adr/0008-rendering-performance.md); this page covers the CPU
hot paths of `metallic_core` and of the Luau bindings (`metallic_luau`).

## The perf gate

`.github/workflows/perf.yml` runs on every pull request that changes `crates/`, `Cargo.toml`, `Cargo.lock` or the
workflow itself. It counts the instructions (Callgrind's `Ir`) each workload executes, once for the PR's base and
once for its head, on the same runner. **The PR fails when any workload grows by more than 5%** (exactly 5%
passes). There are five suites, each with its own table in the job summary:

- [`crates/core/benches/perf_gate.rs`](../crates/core/benches/perf_gate.rs): the data model (below);
- [`crates/luau/benches/perf_gate.rs`](../crates/luau/benches/perf_gate.rs): Luau scripts doing 10,000 `part.Position`
  reads (`luau_reads`), `part.Transparency` writes (`luau_writes`) and `part:IsA("BasePart")` calls
  (`luau_method_calls`) through the script host, after a warm-up run that compiles the chunk and fills the member
  cache. Each measured run includes one script start, which the 10,000 operations dominate;
- [`crates/engine/benches/perf_gate.rs`](../crates/engine/benches/perf_gate.rs): the frame loop's own overhead.
  `engine_empty_frames` is 100 headless frames (`Engine::tick`) of a started server game with no scripts and the
  Luau backend registered: the clock, the four script steps with nothing due, the `RunService` watch checks, and the
  empty simulation and render steps. It builds with the dedicated server's features
  (`--no-default-features --features physics,luau`), so no renderer or window code is compiled in. At introduction
  (#73; ubuntu-24.04 runner, Rust 1.99.1, gungraun 0.20.0, release profile) it counted 173,536 `Ir`: about 1,700
  instructions per empty frame;
- [`crates/engine/benches/perf_gate_client.rs`](../crates/engine/benches/perf_gate_client.rs): the frame loop's
  client steps, which a dedicated server does not compile. It builds with the engine's default features (`render`,
  so winit is compiled in), but opens no window and needs no GPU: the engine stays headless. The input queue and
  `UserInputService` only exist in that build, so they cannot be a feature-gated workload of the server suite
  without also changing what `engine_empty_frames` measures. `engine_input_mouse_unwatched` queues 1,000 mouse
  events (cursor moves and raw motion, alternating) with `Engine::push_input` into a started standalone game with no
  scripts, so nothing is connected to `UserInputService`, then runs one `tick()`; the `setup` warms it with four
  frames of the same input. It measures the path #74 made allocation-free (`crates/engine/tests/input_alloc.rs`).
  At introduction (#99; ubuntu-24.04 runner, Rust 1.99.1, Valgrind 3.22.0, gungraun 0.20.0, release profile) it
  counted 75,759 `Ir`: about 76 instructions per event, queuing and the frame included;
- [`crates/render/benches/perf_gate.rs`](../crates/render/benches/perf_gate.rs): the GPU scene's CPU side (#76), with
  no GPU, over 10,000 parts. `render_sync_moving` and `render_sync_scattered` are one frame of `InstanceScene` with
  1% of the parts changed in the `setup` (drain, repack, coalesce) plus the upload's CPU work written into a plain
  buffer: `render_sync_moving` (100 parts that move every frame, so they sit together in the dynamic region: one
  range, the copy path) and `render_sync_scattered` (100 parts spread over the scene recoloured: 100 ranges, the
  scatter path's layout). Since #79 a sync also notes which slots moved, for the culling grid (from the change
  record's `TRANSFORM` bit: a flag and, the first time, a queue entry per moved slot); moving their grid items is the
  cull's work, so it is gated by `render_cull` (#114): the CPU cull (`InstanceScene::cull`, #79) for a fixed camera
  (above one corner of the 400 m square the parts fill, looking across it, 45° vertical field of view, 1920×1080)
  with `CullSettings::default()`, into a `VisibleSet` warmed in the `setup`. About 60% of the parts are in view,
  some of the 172 grid cells wholly and some crossing the frustum's edges. 1% of the parts (100) moved since the
  last cull: half cross into another 32 m grid cell and half stay inside their cell, and by then they sit in the
  dynamic region, as moving parts do in a game. The sync and coalesce that queue them run in the `setup`, so the
  measured call pays for moving the 100 grid items and then walking the view, as a frame does. The camera, the
  movers and the parts are the workload's definition: changing any of them is a gate change. At introduction (#114;
  ubuntu-24.04 runner, release profile) it counted 345,288 `Ir`: the 100 moved items and the walk of the view.

A suite the base does not have yet (the PR that introduces it) cannot be compared: its head counts are measured,
checked (each must be present and above zero) and reported as `NEW (not gated)`, and the next PR is gated against
them. Whether the base has a suite is decided from the base commit before any file is copied into it.

### Why instruction counts

Wall time on shared CI runners varies by up to ±50% between runs (other tenants, CPU frequency, cache state), so a
wall-time gate would either miss real regressions or fail at random. Valgrind runs the code on a simulated CPU and
counts every instruction, so the same binary on the same input gives the same count to within a few instructions,
on any runner. Instructions are a proxy: they do not see cache misses, branch mispredictions or SIMD width, so a
change that trades instructions for memory traffic can look better than it is. The gate catches the common case
(more work per operation); wall time on a known machine (below) is still the judge for tuning.

### What it measures

Each workload starts from a fixture its `setup` function builds outside the measured function: 1,000 `Part`s with
four non-default properties (`CFrame`, `Size`, `Color`, `Material`) in a `Folder` under `Workspace`, using the
shared test class set (`crates/core/tests/support/classes.rs`), with a renderer and a physics change consumer
registered and drained, as in a running game. Only the workload function's own instructions count; building the
fixture and dropping the data model afterwards do not.

| Workload | Measured work | `Ir` at introduction | `Ir` after #49 |
| --- | --- | ---: | ---: |
| `reads` | Ten passes reading seven properties (`CFrame`, `Size`, `Color`, `Transparency`, `Anchored`, `CanCollide`, `Material`) of every part: 70,000 typed reads, three of the seven properties at their default | 8,188,839 | 8,188,489 |
| `writes` | Ten passes setting every part's `CFrame`; every write changes the value: 10,000 writes | 5,266,817 | 4,496,817 |
| `create` | 1,000 more parts, each created and parented into the folder | 689,364 | 627,505 |
| `destroy` | The 1,000 parts destroyed one by one, in creation order | 11,725,942 | 709,200 |
| `clone` | `clone_tree` of the folder with its 1,000 parts | 1,270,279 | 1,235,766 |
| `drain_changes` | One consumer draining 50 changed parts of 5,000 | 2,028 | 2,028 |

The first column is from the PR that added the gate (#29), measured on `main` at `8b2caff` (data model, signals and
the first classes): ubuntu-24.04 runner, Rust 1.99.1, Valgrind 3.22.0, gungraun 0.20.0, release profile. The second
is from #49, on the same runner image and versions. #49 made removing a child O(1): `destroy` had cost about 11,700
instructions per part, against about 690 to create one, because removing a child shifted and renumbered every later
sibling. It also brought an unwatched write back under its cost before signals (`writes` was 4,746,793 then). Both
columns are reference points, not targets; the gate always compares a PR with its own base.

### How the workflow compares

1. It checks out the PR's merge commit (the head) and, in `base/`, that merge's first parent: the base branch as
   the head was merged onto it. (The event's `pull_request.base.sha` can be older than that when the base branch
   has moved on, and comparing against it would charge other PRs' changes to this one.)
2. It copies the head's workload into the base: `perf_gate.rs`, the shared test classes it includes, and
   `rust-toolchain.toml`. If the base has no `perf_gate` bench entry yet (the PR that introduced the gate), it adds
   the head's entry and the `gungraun` dev-dependency. Both sides therefore run the same workload with the same
   compiler; only the engine code differs.
3. It runs the base with `--save-baseline=base`, then the head with `--baseline=base`. gungraun compares each
   workload and exits with an error when `Ir` grows past the limit set in `perf_gate.rs` (`IR_LIMIT_PCT`).
4. `cargo xtask perf-report` then checks every workload in the bench's `library_benchmark_group!` list: each must
   have a base count and a head count, both above zero, compared against the `base` baseline. **A missing or zero
   count fails the job**: gungraun on its own treats a missing baseline as "nothing to compare" and passes, and a
   zero count means the measured function was not found. The comparison table goes to the job summary.

Base and head build into separate target folders. Cargo names a workspace crate's artifacts by its path relative to
the workspace root, which is the same on both sides, so a shared folder would let the second build reuse the first
one's binary and the gate would compare the base with itself.

The Luau suite follows the same steps with `cargo bench -p metallic_luau --bench perf_gate` and reports with
`cargo xtask perf-report --source crates/luau/benches/perf_gate.rs --suite metallic_luau` (plus `--new-suite` when
the base lacks it). Its workloads only use public APIs, so the bench source is the only file copied to the base.
The engine suite does the same with
`cargo bench -p metallic_engine --bench perf_gate --no-default-features --features physics,luau` and
`--source crates/engine/benches/perf_gate.rs --suite metallic_engine`, the render suite with
`cargo bench -p metallic_render --bench perf_gate` and `--source crates/render/benches/perf_gate.rs --suite
metallic_render`, and the engine's client suite with `cargo bench -p metallic_engine --bench perf_gate_client` and
`--source crates/engine/benches/perf_gate_client.rs --suite "metallic_engine (client)"`. The client suite runs after
the render suite, so it reuses that build of `metallic_render` and wgpu and adds only winit and the engine's `render`
build; winit's X11 and Wayland backends are loaded at run time, so the runner needs no system packages to compile it.
Workload names are unique across the suites, because the report matches gungraun's summaries by
function name.

The job takes several minutes, most of it compiling (the Luau suite builds Luau's C++ sources for both sides). Pushes to `main` that touch the same paths only build,
so the cargo cache is saved from `main` and restored by PRs, as in `ci.yml`.

### When the base cannot build the workloads

If a PR changes an API the workloads call, the base cannot compile the head's workloads and the job fails with
"base cannot build the workloads". This is deliberate: skipping would let any API change hide a regression. Split
the change: first add the new API next to the old one (the base still builds the old workload, and the gate
measures the rest), then switch the workloads and remove the old API in a second PR.

### Running it locally

It needs Linux (or WSL) with Valgrind and the `gungraun-runner` binary of exactly the same version as the `gungraun`
crate in `Cargo.lock` (0.20.0 at the time of writing). It cannot run on Windows or macOS.

```sh
sudo apt-get install valgrind
cargo install gungraun-runner --version 0.20.0 --locked
cargo bench -p metallic_core --bench perf_gate                          # measure and show the counts
```

To compare a change with `main` the way CI does:

```sh
git switch main
cargo bench -p metallic_core --bench perf_gate -- --save-baseline=base
git switch my-branch
cargo bench -p metallic_core --bench perf_gate -- --baseline=base --save-summary=json
cargo xtask perf-report --home target/gungraun --baseline base
```

(When `main` does not have the workload you changed, copy your `perf_gate.rs` into the `main` checkout first, as the
workflow does.) The Callgrind profiles land in `target/gungraun/`; open them with `callgrind_annotate` or KCachegrind
to see where the instructions went.

**On Windows**, a plain `cargo bench` skips the target (`bench = false` in `crates/core/Cargo.toml`), and so does
`cargo clippy --all-targets`. To lint it, run `cargo clippy -p metallic_core --bench perf_gate -- -D warnings`; the
perf workflow does this on every run. Running it by name fails at once with "Is gungraun-runner installed?".

### Adding a workload

- **Only for an implemented hot path**: code that runs per frame, per part or per change, which a regression would
  make users feel. Not for code that does not exist yet, and not for one-off startup work.
- **Setup outside the measurement.** Build the fixture in a `setup` function (`#[bench::id(args = (...), setup =
  ...)]`) and return whatever the workload changed, so neither building nor dropping it is counted.
- **Each workload's body must differ from every other's.** The compiler merges identical functions into one symbol,
  and the second workload would then measure nothing (the report fails on its zero count).
- **Deterministic input.** No randomness, clocks or thread timing; the count must be the same on every run.
- Add the function to the `benchmarks = [...]` list; `perf-report` reads that list to know what must be present.
- The base must be able to build the new workload with its own API (see above): add a workload in a PR after the API
  it calls has landed.
- **Put it in the suite whose build has the code.** A workload added to an existing suite is measured on the base
  too (the head's bench source is copied there) and gated in the PR that adds it, so the base must run it as the
  code it measures. A workload that needs other features than every suite (as the input step needed `render`) gets
  its own bench file and `[[bench]]` entry with those features, and the workflow gets that suite's steps, decided
  from the base commit by the file's presence like the others: the PR that adds the file has no baseline, so its
  workloads are reported `NEW (not gated)` and the next PR is gated against them.

### Changing the gate is reviewed like code

The threshold (`IR_LIMIT_PCT`), a workload's size (`PARTS`, `PASSES`, `DRAIN_PARTS`, what the body does), the
fixture, the compiler settings and the workflow are all part of the gate. A PR that changes any of them says so in
its description and why, and the reviewer checks it as carefully as engine code: loosening the threshold or
shrinking a workload makes every later regression easier to miss. Never change the gate in the same PR as the code
it would otherwise fail; a regression the gate catches is fixed, or accepted explicitly by the reviewer with the
reason written in the PR.

## Measured

Wall-time numbers from the criterion benches (`cargo bench -p metallic_core --bench data_model --bench registry`,
release profile), recorded by hand. They are for spotting trends against the targets set in the issues, on a stated
machine; they are not a gate. Each milestone that adds a hot path adds rows here.

**2026-11-02**, Ryzen 5 5500, 16 GB, Windows 11, Rust 1.99.1. Two runs back to back on the same machine, kept as
quiet as could be arranged: `main` at `b756236` (data model, signals, first classes, perf gate), then #49 at `f1c2abc` (O(1) child
removal, cheaper unwatched writes). Every 95% confidence interval is within 1.5% of its median.

| Bench | `main` `b756236` | #49 `f1c2abc` | Target |
| --- | ---: | ---: | ---: |
| `set_cframe` | 35.0 ns | 27.8 ns | ≤ 100 ns |
| `set_cframe_watched` | 75.0 ns | 63.8 ns | |
| `get_ref_stored` | 7.3 ns | 7.3 ns | ≤ 20 ns |
| `get_ref_default` | 11.0 ns | 11.1 ns | ≤ 20 ns |
| `cframe_typed` | 10.3 ns | 10.4 ns | |
| `create_1000_parts_4_props` | 392 µs | 356 µs | ≤ 1 ms |
| `large/create_100k_set_one` | 10.5 ms | 9.2 ms | < 50 ms |
| `read_7_props_5000_parts` | 249 µs | 248 µs | ≤ 0.3 ms |
| `drain_50_of_5000` | 147 ns | 146 ns | ≤ 20 µs |
| `drain_1000_events` | 27.2 µs | 27.0 µs | |
| `build_100_classes` | 365 µs | 366 µs | |
| `prop_id_own` / `prop_id_inherited` / `prop_id_missing` | 20.1 / 19.2 / 14.5 ns | 20.2 / 19.1 / 14.5 ns | |
| `is_a_true` / `is_a_false` | 1.9 / 1.9 ns | 1.9 / 1.9 ns | |

An earlier run (2026-11-02, same machine, sharing it with other builds) had `read_7_props_5000_parts` at 330 µs, over
its target; on the quieter machine it is 249 µs on `main`, so that was load, not a regression.

### Luau bindings

`cargo bench -p metallic_luau --bench api`, **2026-11-03**, same machine, #46. Each `api/*_1m` bench is one script
doing the operation a million times through the script host, so the time in ms is the cost of one operation in ns.
`vm/hot_loop` is 20 million iterations of a small loop with the interrupt (the time budget) installed;
`vm/hot_loop_no_interrupt` is the same without it.

| Bench | Interpreter | `--features jit` | Target |
| --- | ---: | ---: | ---: |
| `api/part_property_get_1m` (`part.Position`) | 144 ns | 141 ns | ≤ 150 ns |
| `api/part_property_get_stored_1m` (`part.Transparency`) | 107 ns | 96 ns | |
| `api/part_property_set_1m` (`part.Transparency = x`) | 106 ns | 104 ns | ≤ 200 ns |
| `api/method_call_1m` (`part:IsA("BasePart")`) | 233 ns | 213 ns | |
| `vm/hot_loop` / `vm/hot_loop_no_interrupt` | 721 / 634 ms (1.14×) | 162 / 74 ms (2.19×) | ≤ 1.5× |

An installed interrupt costs a fixed few nanoseconds per safepoint, which is 14% of an interpreted loop iteration but
doubles a native one. About 90 ns of every member access is Luau calling the metamethod, and about 40 ns of
`part.Position` is the `Position` getter looking up `CFrame` by name.

Since #64 no interrupt is installed while a call is within its budget: a watchdog thread installs it once the deadline
has passed (`crates/luau/src/watchdog.rs`), so `vm/hot_loop` now measures the budget enforced and
`vm/hot_loop_no_interrupt` the budget turned off. **2026-11-03**, same machine, shared with other builds, so each pair
is the minimum of 21 (jit) or 15 (interpreter) interleaved runs of each, from a throwaway harness:

| `vm/hot_loop` / `vm/hot_loop_no_interrupt` | Interpreter | `--features jit` | Target |
| --- | ---: | ---: | ---: |
| `main` `0f98e88` (interrupt always installed) | 578 / 524 ms (1.10×) | 160 / 73 ms (2.19×) | |
| #64 (watchdog) | 520 / 522 ms (1.00×) | 73 / 73 ms (1.00×) | ≤ 1.14× / ≤ 1.2× |

Criterion's own medians for #64 agree: 562 / 574 ms (interpreter) and 74.1 / 74.5 ms (jit).

Perf-gate instruction counts of the Luau suite at introduction (#46; ubuntu-24.04 runner, Rust 1.99.1, gungraun
0.20.0, release profile): `luau_reads` 13,859,424, `luau_writes` 15,281,853, `luau_method_calls` 21,521,695 (10,000
operations each, plus one script start).

### Script lifecycle

`cargo bench -p metallic_script --bench lifecycle`, **2026-11-03**, same machine, #68. One frame creates 1,000 parts
under `Workspace` and destroys them; the host variants then run a whole frame (four `step`s) of a started game with
the minimal backend. `no_host_connected` has one unrelated connection, which is what a running script's `Enabled`
watch costs the data model (destroyed instances stay readable until their events are delivered). The machine was
shared with other builds, so these are the 10th percentile of 4,000 frames per variant, the variants interleaved
(criterion's medians moved by up to 30% between runs; the ratios held).

| Bench | `main` `c9deb3f` | #68 | Target |
| --- | ---: | ---: | ---: |
| `create_destroy_1000/no_host` | 47.8 µs | 48.0 µs | |
| `create_destroy_1000/no_host_connected` | 94.2 µs | 97.3 µs | |
| `create_destroy_1000/host_empty_game` | 272.4 µs | 50.8 µs | ≈ `no_host` |
| `create_destroy_1000/host_one_script` | 301.5 µs | 94.6 µs | ≈ `no_host_connected` |

Before #68 the host connected `DescendantAdded`/`DescendantRemoving` on each service that runs scripts, so every part
queued two signal events (with argument vectors) that the host discarded after a class test. It now drains a change
consumer: a dirty-list entry and one class test per part.

### GPU scene

`cargo bench -p metallic_render --bench gpu_scene`, **2026-11-03**, same machine (Ryzen 5 5500; AMD Radeon RX 6650 XT
on Vulkan), #76. 100,000 parts in `Workspace`, 1,000 of them (1%) changed per frame, the edits outside the timing.
`moving`: the same parts move every frame, so after their second frame they sit together in the dynamic region.
`scattered`: a different spread of parts is recoloured every frame, one lone static slot each, the worst case for
coalescing. `sync` is the CPU side alone (drain, repack, coalesce); `upload` adds `GpuScene::upload` and the
submission that carries it.

| Bench | Time | Target |
| --- | ---: | ---: |
| `sync/100k_1pct_moving` | 74 µs | ≤ 0.5 ms |
| `sync/100k_1pct_scattered` | 115 µs | ≤ 0.5 ms |
| `upload/sync_upload_100k_1pct_moving` (1 range, copy path) | 127 µs | |
| `upload/sync_upload_100k_1pct_scattered` (1,000 ranges, scatter pass) | 242 µs | |
| `upload/sync_upload_100k_1pct_scattered_copies` (1,000 copies) | 680 µs | |
| `upload/per_range_write_buffer_100k_1pct_scattered` (1,000 `write_buffer`s, rejected) | 1.51 ms | |
| `rebuild/100k` (every part placed again) | 9.95 ms | |

wgpu creates a staging buffer for every `write_buffer` call, which is why one staging write per frame beats a write
per range by 6×, and records each `copy_buffer_to_buffer` for about 0.45 µs, which is why many ranges go through the
scatter pass instead (from `SCATTER_MIN_RANGES`, 32).

With #77 the coalesce also keeps the draw lists current (O(1) per dirty slot): `sync/100k_1pct_moving` measured
75 µs and `sync/100k_1pct_scattered` 114 µs, the same as before within noise.

With #79 the sync also feeds the culling grid: a slot that changes draw list is placed in the grid in the
coalesce, and one that moved is flagged and queued once for the next cull, which moves its item (in place while
it stays in its cell, O(1) otherwise). Whether a part moved comes from its change record (the `TRANSFORM` dirty
bit: `CFrame`, `Position`, `Orientation`, `Size`) or a new mesh kind (`Shape`); a recolour or a new material
leaves the grid alone. Perf-gate history of this PR (`render_sync_moving` / `render_sync_scattered`, base
128,226 / 139,941 `Ir`): items moved in the coalesce, 155,313 (+21%, about 270 instructions per moved part);
queued, but found by comparing the packed transform, 136,356 / 147,668 (+6.3% / +5.5%, about 80 per written
part); from the change record, the version kept. `sync/100k_1pct_moving` measured 75 µs on `main` and 80 µs with
it, `sync/100k_1pct_scattered` 110–120 µs on `main` and 116 µs (that bench varies by about 10% between runs on
this machine). The cull itself, `cull/*` (one `InstanceScene::cull` of the 100,000 parts from a 1920 × 1080
camera in the middle of the field at head height, looking along it, nothing queued):

| Bench | Visible | Time |
| --- | ---: | ---: |
| `cull/100k_30pct` (70°) | 27,758 | 82 µs |
| `cull/100k_10pct` (26°) | 9,351 | 37 µs |

### Drawing parts

`cargo bench -p metallic_render --bench draw_parts`, **2026-11-03**, same machine (Ryzen 5 5500; AMD Radeon RX 6650
XT), #77. A grid of parts, all in view and nothing culled (70% blocks, 10% each of balls, cylinders and wedges, 1%
see-through blocks), drawn at 1920×1080 offscreen. CPU is the wall time of `Renderer::render_scene` (upload, frame
uniform, transparent sort, recording the passes, submit) with no scene changes; GPU is the timestamps of the whole
frame and of the opaque pass. "Moving" changes the camera every frame, so the frame uniform is written and the
see-through parts are sorted again. Every frame makes 4 opaque draws and 1 transparent draw. This is the baseline
culling (#79) is measured against; there is no target yet.

| Parts (triangles) | Camera | Vulkan CPU p50 / p99 | Vulkan GPU frame / opaque | DX12 CPU p50 / p99 | DX12 GPU frame / opaque |
| --- | --- | ---: | ---: | ---: | ---: |
| 10,000 (1.18 M) | still | 0.078 / 0.121 ms | 0.141 / 0.134 ms | 0.091 / 0.158 ms | 0.117 / 0.112 ms |
| 10,000 (1.18 M) | moving | 0.087 / 0.105 ms | 0.140 / 0.133 ms | 0.117 / 0.200 ms | 0.118 / 0.114 ms |
| 100,000 (11.8 M) | still | 0.091 / 0.124 ms | 1.069 / 1.060 ms | 0.109 / 0.128 ms | 0.778 / 0.775 ms |
| 100,000 (11.8 M) | moving | 0.126 / 0.172 ms | 1.066 / 1.057 ms | 0.136 / 0.169 ms | 0.797 / 0.793 ms |

The CPU cost hardly depends on the part count: the draws are per mesh kind, and nothing per part runs unless it
changed (the moving rows add the sort of 1,000 see-through parts).

## Stress scenes

The scenes every renderer change is measured against (ADR 0008 §10, #78). They are generated by code from a seed
(`metallic_engine::stress`, whose docs say what each holds), and 1% of the parts move every frame, by frame
number, so every run draws the same frames. **Every renderer PR reports its before and after numbers on these
scenes**, run the same way, on the same machine, with the commit it measured.

```sh
cargo run --release -p metallic_engine --example stress -- --scene parts_100k   # one scene, JSON on stdout
cargo run --release -p metallic_engine --example stress -- --scene all > results.json
cargo run --release -p metallic_player -- --scene parts_100k --overlay           # look at one, F3 toggles the overlay
```

The example opens a window of the scene's size, presents without vsync (`AutoNoVsync`), drops 120 warm-up frames,
summarises the next 1,000 and exits; `--frames`, `--warm-up`, `--seed`, `--size`, `--vsync` and `--overlay` change
that, and `WGPU_BACKEND` picks the backend. `metallic_player --scene <name> --bench <frames>` prints the same
statistics, but with vsync until graphics settings land (#84), so its frame times show the display's rate, and the
GPU, running at low clocks, takes longer per frame (about 3.5× on `see_through_10k` here). Use the example for
these tables.

What the numbers are (`metallic_engine::perf`, which also feeds the performance overlay):

- **Frame**: wall time from one drawn frame to the next, so 1000 / fps. p50 and p99 of the measured frames; the
  **1% low** is the mean of the slowest 1% of frames.
- **Render CPU**: `SceneRenderer`'s own work in a frame: scene sync, overlay, the camera, `Renderer::render_scene`
  (upload, recording, submit). Waiting for a swapchain image and presenting are left out (with vsync they are
  mostly waiting). Script and physics CPU time are measured too (`Engine::set_cpu_timing`, on for windowed engines);
  the stress scenes have no scripts and no physics yet, so both are about 1 µs and left out of the table.
- **Cull CPU** (#79, `cpu_ms.cull` in the JSON): the part of render CPU spent culling; 0 in a frame that kept the
  last cull because nothing changed.
- **GPU**: timestamps around each pass, p50 over every timed frame; the frame column is the sum of the passes.
- **Draws, instances, triangles** per pass, and the instances culled (`draws.culled`), from the last frame.

**2026-11-03**, Ryzen 5 5500, 16 GB, AMD Radeon RX 6650 XT (driver 32.0.21045.16010), Windows 11, **DX12**, branch
`78-stress-scenes` at `0471e58`. Preset: none yet (#84), so the renderer's defaults: no MSAA, the opaque, sky,
transparent and UI passes, no culling (#79), no depth prepass. Present mode `AutoNoVsync`, 1,000 frames after 120
of warm-up, seed 1, window at 100% scaling. Vulkan could not be measured: on this PC no Vulkan adapter accepts a
window's surface (#110; offscreen Vulkan works, see "Drawing parts").

| Scene | Parts (in view) | Size | Frame p50 / p99 / 1% low | Render CPU p50 / p99 | GPU p50: frame (opaque / transparent) | Draws opaque / transparent | Instances | Triangles |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `parts_10k` | 10,000 (3,010) | 1920 × 1080 | 0.284 / 0.359 / 0.400 ms | 0.155 / 0.215 ms | 0.200 (0.200 / 0.000) ms | 4 / 0 | 10,000 | 1.19 M |
| `parts_50k` | 50,000 (15,179) | 1920 × 1080 | 0.613 / 0.710 / 0.744 ms | 0.195 / 0.260 ms | 0.530 (0.530 / 0.000) ms | 4 / 0 | 50,000 | 5.88 M |
| `parts_100k` | 100,000 (30,206) | 1920 × 1080 | 1.027 / 1.148 / 1.186 ms | 0.247 / 0.318 ms | 0.908 (0.907 / 0.000) ms | 4 / 0 | 100,000 | 11.77 M |
| `many_lights` | 3,652 + 1,152 lights (3,296) | 1920 × 1080 | 0.336 / 0.410 / 0.430 ms | 0.145 / 0.188 ms | 0.255 (0.255 / 0.000) ms | 4 / 0 | 3,652 | 1.41 M |
| `overdraw` | 3,266 (1,453) | 1920 × 1080 | 0.624 / 0.711 / 0.729 ms | 0.150 / 0.219 ms | 0.548 (0.547 / 0.001) ms | 1 / 1 | 3,266 | 0.04 M |
| `mobile` | 5,000 (1,466) | 1280 × 720 | 0.220 / 0.303 / 0.313 ms | 0.149 / 0.209 ms | 0.104 (0.099 / 0.006) ms | 4 / 1 | 5,000 | 0.59 M |
| `see_through_10k` | 10,001 (10,001) | 1920 × 1080 | 0.996 / 1.085 / 1.163 ms | 0.919 / 0.989 ms | 0.641 (0.050 / 0.586) ms | 1 / 4,199 | 10,001 | 4.86 M |

"In view" counts the parts whose centre is inside the camera's view: what culling (#79) would leave to draw. Every
scene is far inside the High tier's budget on this PC, because nothing is shaded beyond one light yet; the numbers
are the baseline later work is measured against:

- **Culling (#79)** should cut `parts_100k`'s GPU time about in proportion to the 30% in view, and the 100k-part
  scenes are the ADR's scale target (100,000 parts, 10,000 in view, Mid budget at Medium).
- **`see_through_10k` is the transparent pass's worst case (#106):** 4,199 draws, one per run of same-shape
  parts in back-to-front order, cost 0.92 ms of render CPU (sorting 10,000 instances and recording the draws).
- **`overdraw`** draws its 64 walls back to front, each covering the screen but for a window (up to 64 layers per
  pixel); built the other way round the same scene takes 0.21 ms of GPU instead of 0.55 ms. A depth prepass or a
  front-to-back sort (ADR 0008 §7) is measured here.
- **`many_lights`** holds 1,024 `PointLight`s and 128 `SpotLight`s, which drew nothing until #83 (see "Point and
  spot lights" below).

The overlay costs about 0.02 ms of render CPU and 0.02 ms of GPU (its UI pass): `parts_100k` with `--overlay`
measured 1.052 / 1.189 / 1.207 ms per frame, 0.266 / 0.382 ms render CPU and 0.933 ms GPU.

### Culling (#79)

CPU culling (`metallic_render::cull`, ADR 0008 §3): a loose grid of every drawn instance's bounding sphere (32 m
cells, kept current from the scene's changes: moved parts are moved at the cull), culled per view by frustum, view distance and screen
size into the same draw lists, the opaque ones roughly front to back (cells and large items nearest first). The
renderer's defaults: the frustum, no view distance, nothing under a pixel (`CullSettings::default()`; the presets,
#84, set the thresholds). The stress example's `--no-cull` draws everything, `--view-distance` and
`--min-screen-size` set the thresholds and `--fov` narrows the camera. The overlay shows the cull time on its CPU
line and the instances culled.

**2026-11-03**, same machine and settings as the table above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed 1).
Before: `main` at `a947bfb`. After: branch `79-cpu-culling`. "Drawn" is the instances drawn (opaque and
see-through) against the parts whose centre is in view: a part is drawn when its bounding sphere reaches into the
view, so a few more. Cull CPU is part of render CPU, and includes moving the parts that moved in the grid. Times
are p50 in ms.

| Scene | In view | Drawn before → after | Render CPU before → after | Cull CPU | GPU before → after | Frame before → after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `parts_10k` | 3,010 | 10,000 → 3,077 | 0.154 → 0.180 | 0.019 | 0.200 → 0.122 | 0.283 → 0.257 |
| `parts_50k` | 15,179 | 50,000 → 15,345 | 0.197 → 0.282 | 0.076 | 0.528 → 0.228 | 0.610 → 0.388 |
| `parts_100k` | 30,206 | 100,000 → 30,448 | 0.259 → 0.419 | 0.151 | 0.913 → 0.364 | 1.042 → 0.565 |
| `parts_100k --fov 26` | 10,494 (10.5%) | 100,000 → 10,712 | 0.266 → 0.341 | 0.079 | 0.902 → 0.233 | 1.019 → 0.485 |
| `see_through_10k` | 10,001 | 10,001 → 10,001 | 0.922 → 0.905 | 0.016 | 0.639 → 0.641 | 1.000 → 0.983 |
| `overdraw` | 1,453 | 3,266 → 1,716 | 0.151 → 0.173 | 0.014 | 0.547 → 0.215 | 0.621 → 0.294 |

The "before" of `parts_100k --fov 26` is the branch with `--no-cull` (`main` has no `--fov`); for the other scenes
the branch with `--no-cull` measured the same as `main` within 3% (see_through_10k's render CPU: 8% lower, noise).

- **The ADR's scale target:** a camera seeing about 10% of `parts_100k` draws 10.7% of its instances (the engine
  test `a_camera_seeing_a_tenth_of_parts_100k_draws_about_a_tenth` checks it on every run), and the GPU time falls
  from 0.90 to 0.23 ms. It does not fall in proportion: the parts near the camera, which cover most of the screen,
  are all still drawn.
- **Render CPU** grows by the cull and by the upload of the visible lists, every frame here because 1% of the parts
  move: 0.42 ms p50 and 0.57 ms p99 on `parts_100k`, far inside the Mid tier's 2.5 ms (measured on the High-tier
  dev PC; there is no Mid-tier machine to run it on yet). A frame where nothing changed and the camera is still
  keeps the last cull and uploads no lists.
- **`overdraw`**: 0.547 → 0.215 ms of GPU from the front-to-back order (the walls are built so that unsorted they
  draw back to front; built the other way round the scene measured 0.21 ms). Ordering every instance by its own
  depth instead of by cell (a counting sort into depth buckets) was measured too: 0.206 ms on `overdraw` and
  0.379 ms on `parts_100k`, for a cull 2.1 times as long on `parts_100k` (0.274 ms against 0.129 ms, both with the
  grid then updated in the coalesce) and 2.7 times as long in
  `cull/100k_30pct` (230 µs). The cell order was kept.
- **`see_through_10k`** sees all of its parts, so there culling only costs its 0.016 ms; see-through parts are
  culled before they are sorted.

### Sun, ambient and sky (#81)

The `Lighting` service lights the scene (the sun or the moon, a hemisphere ambient) and the sky pass draws one
full-screen triangle at the far plane after the opaque parts, with a `GreaterEqual` depth test against the cleared
depth, so it shades only what no part covered (ADR 0008 §7). It reads the frame uniform the parts already upload
(now 336 bytes instead of 160), so it adds one draw of three vertices and no upload. `Lighting` is read again only in
a frame after it changed (the scene's change consumer reports it), and never allocates
(`crates/render/tests/no_alloc.rs`).

**2026-11-03**, same machine and settings as the culling table above (DX12, `AutoNoVsync`, 1,000 frames after 120,
seed 1). Before: `main` at `ca2ee18`, whose sky pass ran empty. After: branch `81-sun-sky`. Two interleaved runs of
each (main, branch, main, branch); the table has the first pair, and the second agreed within 3% (the sky pass
within 1%). Times are p50 in ms; "Sky" is the sky pass's GPU time.

| Scene | Size | Sky | GPU before → after | Render CPU before → after | Frame before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| `parts_10k` | 1920 × 1080 | 0.022 | 0.123 → 0.143 | 0.182 → 0.186 | 0.259 → 0.263 |
| `parts_50k` | 1920 × 1080 | 0.022 | 0.232 → 0.249 | 0.284 → 0.285 | 0.392 → 0.392 |
| `parts_100k` | 1920 × 1080 | 0.023 | 0.365 → 0.371 | 0.432 → 0.421 | 0.581 → 0.570 |
| `many_lights` | 1920 × 1080 | 0.022 | 0.226 → 0.248 | 0.176 → 0.180 | 0.311 → 0.329 |
| `overdraw` | 1920 × 1080 | 0.000 | 0.218 → 0.218 | 0.181 → 0.181 | 0.293 → 0.296 |
| `mobile` | 1280 × 720 | 0.011 | 0.092 → 0.093 | 0.169 → 0.173 | 0.241 → 0.246 |
| `see_through_10k` | 1920 × 1080 | 0.045 | 0.635 → 0.687 | 0.958 → 0.903 | 1.037 → 0.982 |

- **GPU:** the sky costs what it covers. With nothing opaque on screen (`see_through_10k`, whose parts are all
  see-through and drawn after it) it is 0.045 ms at 1080p, the whole price of a full-screen sky on this GPU; with
  about half the screen covered by parts it is 0.022 ms; inside `overdraw`'s closed room the depth test rejects
  every pixel and it costs nothing measurable. `parts_100k`'s opaque pass measured lower in this run, so its total
  grew by less than the sky's 0.023 ms.
- **Render CPU** does not change beyond noise: one more draw to record, and a larger uniform written when the
  camera or the lighting moves. `see_through_10k` measured lower on the branch in both runs (0.903 and 0.899
  against 0.958 and 0.956); nothing in this change touches the transparent sort, so that is run-to-run variation of
  this scene, which #79 also saw.
- A frame in which a script changes `Lighting` (the Player's demo moves `ClockTime` every frame) adds a drain entry
  for the service, six property reads (one of them the computed `SunDirection`) and the lighting maths: the read
  measured 0.16 µs, and a sync in which only `Lighting` changed 0.24 µs with it (release build, a throwaway timing
  loop of 200,000 changes, not committed); the scene's sync for drawn parts is unchanged (the watch is checked only for instances that
  are not drawn), so the perf gate's render workloads see no difference.

### Sun shadows (#82)

Cascaded shadow maps for the sun ([`metallic_render::shadow`](../crates/render/src/shadow/mod.rs), ADR 0008 §6):
by default 3 cascades of 2048² `Depth32Float` out to 256 m, texel-snapped, each culling its own casters with the
culling grid. The nearest two hold every caster and are drawn again in any frame in which the scene changed; the
far one holds only the static region's casters and is **cached**: drawn again only when the sun turns, its snapped
placement moves (it moves in steps of 1/32 of its width, and is centred on the camera when the view is wide) or a
static caster inside it changes. The parts sample them with a 3 × 3 tent of bilinear comparison taps. The stress
example reports each cascade's GPU time (`gpu_ms.shadow`, a kept cascade counting 0), how many of the measured
frames drew it (`shadow.redraws`) and its casters (`shadow.casters`); the overlay shows the same. `--no-shadows`,
`--shadow-cascades`, `--shadow-resolution`, `--shadow-distance` and `--dynamic-cascades` change the settings, and
`--moving-camera` walks the camera through the scene at 8 m/s, looking 20° left and right every 6 s.

**2026-11-03**, same machine and settings as the tables above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed
1), branch `82-sun-shadows`, `parts_100k` (1% of the parts moving every frame). "Off" is the same branch with
`--no-shadows`, which draws what `main` draws. Times are p50 in ms, except a cascade's GPU time: its mean over the
measured frames (the frames that kept its map count 0), with how many of them drew it.

| Camera | Shadows | Frame | Render CPU | Cull CPU | GPU | Opaque pass | Cascade 0 | Cascade 1 | Cascade 2 (cached) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| still | off | 0.618 | 0.460 | 0.167 | 0.375 | 0.352 | | | |
| still | on | 0.790 | 0.630 | 0.259 | 0.574 | 0.436 | 0.033 (1,000) | 0.083 (1,000) | 0.000 (0) |
| moving | off | 0.583 | 0.429 | 0.141 | 0.333 | 0.311 | | | |
| moving | on | 0.758 | 0.591 | 0.227 | 0.555 | 0.417 | 0.036 (1,000) | 0.081 (1,000) | 0.005 (8) |

Casters in the last frame (still camera): 385, 2,569 and 71,833 per cascade; the two near cascades' 2,954 make 8
draws and 0.38 M triangles a frame.

- **A still camera and sun redraw only the near cascades** (the done-when of #82): the cached far cascade, holding
  72% of the scene's parts, was drawn in none of the 1,000 frames although 1,000 parts moved in every one of them.
  Moving parts are promoted to the dynamic region in their first frames (which takes them out of the far cascade:
  one redraw), and after that only the static region's changes reach it. The render test
  `a_static_frame_does_not_redraw_the_far_cascade` (`crates/render/tests/shadows.rs`) checks it on every run.
- **A walking camera** redrew the far cascade 8 times in 1,000 frames (about once every 2 s at 8 m/s). One redraw
  costs 0.62 ms of GPU (about 70,000 casters, some 8.5 M triangles) and about half a millisecond of cull CPU (the
  run's slowest cull took 0.73 ms against a p99 of 0.36), which averages to 0.005 ms of GPU a frame. Drawn every frame, as without the cache, it is the frame's
  largest cost: with `--dynamic-cascades 3` (every cascade drawn again whenever anything moves), measured later on
  the same machine while it was busy with other builds (so slower overall), render CPU p50 rose from 2.15 to
  2.94 ms and from 2.24 to 3.06 ms in two interleaved pairs, and the far cascade alone took 1.35 and 1.48 ms of GPU.
- **GPU**: 0.12 ms for the two near cascades and 0.08 ms more in the opaque pass for sampling the maps (nine taps per
  lit pixel): the frame goes from 0.375 to 0.574 ms, about a third of a millisecond of the High tier's budget.
- **Render CPU**: +0.17 ms: the near cascades' culls (+0.09 ms, in the cull column), their list uploads, the
  fitting and the extra passes. Well within the Mid tier's 2.5 ms.
- **The perf gate's workloads** (the camera's cull and the sync) change only by a byte in each grid item (the caster
  flag, in the item's padding) and, in the sync, a region test and two flag comparisons per written slot, plus two
  noted bounding spheres for each static caster that moves or changes how it casts.

### Point and spot lights (#83)

Clustered forward lighting ([`metallic_render::lights`](../crates/render/src/lights/mod.rs), ADR 0008 §6): every
frame the CPU ranks the `PointLight`s and `SpotLight`s whose range reaches into the view by importance (brightness ×
how large their sphere looks), keeps the first `budget` of them (lights entering or leaving the budget fade over
0.3 s instead of popping), and bins them into a 16 × 9 × 24 froxel grid (depth slices exponential from 1 m to the
farthest light), at most `per_cluster` lights in a cluster. The lights reach the fragment shader in a 16 KiB
uniform array and the grid in an `R32Uint` texture, so both instance paths (and GLES) light the same way. Lights with
`Shadows` get tiles of a shadow atlas (a spot one, a point light six), drawn in one pass over the whole atlas and
**cached**: a tile is drawn again only when its light moves or changes, a static caster changes inside it, or a moving
caster is (or was) inside it. The part that holds a light casts no shadow for it.

The settings presets will use (#84), `LightSettings::{LOW, MEDIUM, HIGH}` (Medium is the default):

| Preset | Lights a frame | Per cluster | Shadowed lights | Atlas | Tile |
| --- | ---: | ---: | ---: | ---: | ---: |
| Low | 32 | 32 | 2 | 1024² (4 MiB) | 256² |
| Medium | 128 | 48 | 8 | 2048² (16 MiB) | 256² |
| High | 192 | 64 | 8 | 4096² (64 MiB) | 512² |

The stress example's `--lights low|medium|high|off`, `--light-budget`, `--lights-per-cluster`, `--shadowed-lights`
and `--no-light-shadows` set them; the JSON reports the light counts (`lights`: in the scene, in view, in the budget,
drawn, fading, shadowed, atlas tiles, cluster entries and the fullest cluster, the atlas's draws and how many frames
drew tiles), the lights' CPU time (`cpu_ms.lights`, part of render CPU: selection, clustering, packing and the atlas
tiles' caster culls) and the atlas pass's GPU time (`gpu_ms.light_shadows`, a frame that kept every tile counting 0).
The overlay shows the same on its two `LIGHTS` lines.

**2026-11-03**, same machine as the tables above (Ryzen 5 5500, RX 6650 XT, Windows 11, **DX12**, `AutoNoVsync`,
1,000 frames after 120, seed 1), branch `83-local-lights` at `4da09f8`, `many_lights` at 1920 × 1080 (the camera
still unless said otherwise; 1% of the lamps moving every frame). "Off" is the same branch with `--lights off`, which
draws what `main` draws: with no light drawn the parts use pipelines made without the light loop (below), and its
opaque pass measured the same as with the light code removed (0.398 ms). Times are p50 in ms; the atlas's GPU time
is its mean over the measured frames.

| Lights | Drawn (of 1,125 in view) | Shadowed | Frame | Render CPU (p99) | Lights CPU | GPU | Opaque pass | Atlas | Cluster entries (fullest) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| off | 0 | 0 | 0.679 | 0.300 (0.439) | | 0.579 | 0.397 | | |
| Low | 32 | 2 | 1.297 | 0.465 (0.649) | 0.100 | 1.176 | 0.999 | 0.001 | 5,012 (16) |
| **Medium** | 128 | 3 | 2.230 | 0.625 (0.814) | 0.219 | 2.078 | 1.901 | 0.001 | 13,548 (39) |
| High | 192 | 5 | 2.777 | 0.719 (1.019) | 0.275 | 2.483 | 2.294 | 0.006 | 16,713 (47) |
| Medium, `--no-light-shadows` | 128 | 0 | 1.810 | 0.566 (0.811) | 0.189 | 1.637 | 1.452 | | 13,548 (39) |
| Medium, `--moving-camera` | 124 (of 124) | 1 | 2.087 | 0.652 (0.964) | 0.248 | 1.849 | 1.643 | 0.000 | 13,742 (48) |

A second pair of runs (off, Medium) agreed within 3% (render CPU p99 within 0.2 ms).

- **Against ADR 0008's Mid budget at Medium**: render CPU 0.63 ms p50 and 0.81 ms p99 against the Mid tier's
  2.5 ms (and inside the High tier's 1.5 ms), on the High-tier dev PC. The GPU frame is 2.08 ms at 1080p on an
  RX 6650 XT; a GTX 1650 (the Mid reference) has a third or less of its shading throughput, which puts it near 6 to
  8 ms, inside the 16.7 ms of 60 fps with room for the post chain. There is no Mid-tier machine to measure on yet, as
  for #79 and #82; `many_lights` at Medium is the scene to run on one.
- **The lights' CPU time**: 0.22 ms at Medium for 1,152 lights in the scene: ranking all of them, clustering the 128
  drawn (about 0.15 ms of it: 13,500 cluster entries) and packing them. The atlas's caster culls are part of it
  (0.01 ms here: one tile a frame, below). It scales with the budget (0.10 ms at Low, 0.28 ms at High).
- **The atlas is cached**: with a still camera the 18 tiles of the 3 shadowed lights in the budget were drawn once,
  and after that one tile a frame on average (a moving lamp crosses one face of a shadowed light): 0.001 ms of GPU.
  With a walking camera lights come into the budget and leave it, and 136 of the 1,000 frames drew tiles (156 tiles).
  The render test `the_atlas_is_kept_until_a_caster_moves` checks a still frame draws none.
- **Shading cost**: the opaque pass grows from 0.40 to 1.90 ms at Medium: every pixel loops over its cluster's
  lights (13 on average in the lit clusters here, a dense town of lamps 8 to 24 m in range). Code that is in the
  shader costs registers, and so occupancy, even when it does not run, so the part pipelines are made in three
  variants (pipeline-overridable constants, all compiled when the renderer is made) and each frame uses the cheapest
  that draws it: **no local lights** (an unlit frame costs exactly what it did before), **lights without shadows**
  (1.45 ms here), or **lights with shadows** (1.90 ms; the shadow lookup in the loop costs 0.45 ms of occupancy even
  though only 3 of the 128 lights have shadows). Measured on the way: a second loop for the shadowed lights cost
  more (2.74 ms), not less, and a 3 × 3 loop of shadow taps instead of 4 fixed taps 0.06 ms more.
- **The cluster cap**: at 32 per cluster (the first Medium setting) the town overflowed 194 entries a frame (lights
  missing from a cluster, a hard edge); 48 never overflowed with a still camera and 15 entries in the walking
  camera's last frame, for 0.05 ms more GPU than 32.
- **The perf gate's workloads** do not draw lights. In the sync, a part's change now looks its class up once (as
  before) and, only when the scene has lights, a part's transform change stamps it if lights follow it (an index
  test); a non-part change checks the light table before the watch list.
