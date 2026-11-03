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
  At introduction (#99; ubuntu-24.04 runner, Rust 1.99.1, gungraun 0.20.0, release profile) it counted N `Ir`;
- [`crates/render/benches/perf_gate.rs`](../crates/render/benches/perf_gate.rs): the GPU scene's CPU side (#76), with
  no GPU. Each workload is one frame of `InstanceScene` over 10,000 parts with 1% changed in the `setup` (drain,
  repack, coalesce) plus the upload's CPU work written into a plain buffer: `render_sync_moving` (100 parts that
  move every frame, so they sit together in the dynamic region: one range, the copy path) and
  `render_sync_scattered` (100 parts spread over the scene recoloured: 100 ranges, the scatter path's layout).

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
