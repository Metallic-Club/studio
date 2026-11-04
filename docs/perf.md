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

Only the head builds on the cargo cache saved from `main`; the base always compiles from scratch. The cache also holds
what build scripts compiled with the runner's C and C++ compiler (Luau's VM, through `mlua-sys`), and cargo does not
rebuild those when the compiler changes, so the cache is keyed by the compilers' versions. Without that key, a head
built from a cache made by another compiler runs different machine code from the base: when `ubuntu-latest` moved
from Ubuntu 24.04 (GCC 13) to 26.04 (GCC 15), every PR's Luau workloads showed as 2.4–3.3% slower with no Luau
change, all of it in Luau's own C++ functions (#141).

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

The cost of an access does not depend on how many classes, members and enums the registry has: a member name's
string carries its atom, and the per-class cache is an array indexed by it, so after the first access `part.Position`
never goes back to the registry. `crates/luau/src/shared.rs`'s tests count it: the three workloads over the built-in
classes and over 50 and 500 more (each with four properties and an enum of its own) make the same 10,001 member
lookups and the same registry lookups (at most one, for `workspace.Part`). Measured with Callgrind for #141
(ubuntu-26.04 runner, GCC 15, Valgrind 3.26.0, `main` at `d71a457`, a fresh build):

| Extra classes | `luau_reads` | `luau_writes` | `luau_method_calls` |
| --- | ---: | ---: | ---: |
| 0 (the built-in classes, without `UserGameSettings`, `225d5e1`) | 10,836,525 | 14,269,035 | 18,747,126 |
| 0 (the built-in classes) | 10,836,696 | 14,269,293 | 18,747,302 |
| 50 | 10,836,487 | 14,268,993 | 18,746,927 |
| 500 | 10,847,234 | 14,279,754 | 18,757,670 |

The differences between rows (under 0.1%) do not follow the registry's size: on `225d5e1` the same workloads gave
10,840,527 at 50 extra classes and 10,836,350 at 500, and the accesses' lookups are the same in every row (the test
above).

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
statistics, but with the player's graphics settings (vsync on unless its settings file says `vsync = false`), so
its frame times show the display's rate, and the GPU, running at low clocks, takes longer per frame (about 3.5× on
`see_through_10k` here). Use the example for these tables; without `--preset` it draws with the renderer's
defaults, as every table before #84 did.

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
  variants (all compiled when the renderer is made) and each frame uses the cheapest that draws it: **no local
  lights** (a shader module without them, `lights_off.wgsl`, and a frame group without their resources, so an unlit
  frame binds and draws exactly what it did before #83), **lights without shadows** (a pipeline-overridable
  constant turns the shadow lookup off; 1.45 ms here), or **lights with shadows** (1.90 ms; the shadow lookup in the loop costs 0.45 ms of occupancy even
  though only 3 of the 128 lights have shadows). Measured on the way: a second loop for the shadowed lights cost
  more (2.74 ms), not less, and a 3 × 3 loop of shadow taps instead of 4 fixed taps 0.06 ms more.
- **The cluster cap**: at 32 per cluster (the first Medium setting) the town overflowed 194 entries a frame (lights
  missing from a cluster, a hard edge); 48 never overflowed with a still camera and 15 entries in the walking
  camera's last frame, for 0.05 ms more GPU than 32.
- **Memory**: a scene without local lights holds none of their GPU memory (`lights.gpu_bytes` in the JSON). The
  first frame that draws a light makes the lights' uniform (16 KiB), the shadow tiles' (6 KiB) and the cluster
  texture (232 KiB): 254 KiB. The first frame with a shadow adds the atlas at the size the settings say then (16 MiB
  at Medium, 4 MiB at Low, 64 MiB at High), its casters' uniform (16 KiB) and their lists (4 KiB at first). The
  pipelines are made with the renderer, so neither step compiles anything (ADR 0008 §9). The render test
  `light_memory_is_made_when_first_needed` checks it.
- **The perf gate's workloads** have no lights, and the sync must cost them nothing: a part's change pays one length
  test (are there lights at all?), and everything about lights is out of line (`#[inline(never)]`, the non-part
  path `#[cold]`). The first version of this change failed the gate with +6.6% and +6.7% on `render_sync_moving` and
  `render_sync_scattered`, though none of its light code ran there: the lights read their properties through the
  same typed getters (`DataModel::cframe`, `number`, `color3`, `bool`) as the parts' repacking, and with
  `codegen-units = 1` and LTO the extra callers changed how those were inlined into the parts' sync. The lights now
  read through one out-of-line call to `DataModel::get`. Timed on the dev PC with a throwaway copy of the two
  workloads (not committed; Callgrind needs Linux): `main` 6.92 / 7.58 µs a sync, the first version 7.58 / 8.33,
  with the light code compiled out 6.70 / 7.42, and now 6.81 / 7.37.

### Contact shadows (#126)

The shadow maps and the atlas now hold their casters' faces towards the light (back faces culled) with a
slope-scaled rasterizer bias, and the receivers' offsets are smaller, scaled by `1 − N·L` and capped (the reasoning is
in the docs of `LightSettings::depth_bias` and `ShadowSettings::depth_bias`). The caster passes draw the same
number of triangles (parts are closed, so culling the other half of them leaves as many), and the light loop gains a
few multiplies and two `min`s per shadowed light, no texture tap.

**2026-11-04**, same machine and settings as the tables above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed
1), `main` at `6fd85f4` against branch `126-contact-shadows`; `many_lights` at Medium and `parts_100k`, both still
cameras. Times are p50 in ms, except the shadow passes: their mean over the measured frames.

| Scene | Build | Frame | Render CPU (p99) | GPU | Opaque pass | Cascade 0 | Cascade 1 | Atlas |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `many_lights` | `main` | 2.201 | 0.513 (0.643) | 2.053 | 1.870 | 0.040 | 0.125 | 0.0009 |
| `many_lights` | #126 | 2.198 | 0.514 (0.638) | 2.049 | 1.866 | 0.041 | 0.124 | 0.0007 |
| `parts_100k` | `main` | 0.714 | 0.560 (0.744) | 0.582 | 0.441 | 0.034 | 0.083 | |
| `parts_100k` | #126 | 0.704 | 0.554 (0.754) | 0.586 | 0.444 | 0.034 | 0.084 | |

- **No cost**: every difference is within the run-to-run noise of these scenes (about 1%). The far cascade is
  cached in both (drawn in none of the frames), and the atlas draws one tile a frame (a moving lamp; 4 draws, 66
  casters, 16,180 triangles), as before.
- **What it buys**: under the image tests' cube resting on a plate, the floor 3 to 12 cm out from the cube's foot
  was 103% (spot light), 46% (point light), 107% (point light 12 m away) and 12% (sun at 17°) of the way from the
  shadow's brightness to the lit floor's; it is now 0% under all four (`tests/lights.rs`, `tests/shadows.rs`).

### Moving casters in the cached far cascade (#121)

Until #121 the cached far cascade held the static region's casters only, so with the defaults a moving part more
than about 57 m from the camera cast no shadow. Now its static casters are drawn into a **static map** of its own
(a 2048² `Depth32Float` texture, 16 MiB more), drawn again only on #82's triggers, and the cascade's map is
**built** from it: a pass that clears the map, copies the static map in with a full-screen draw writing each
texel's depth (`frag_depth`), and draws the moving casters inside the cascade on top. The map is built only when
the static map was drawn again or the moving casters changed; a frame that moves nothing keeps it, and so does a
frame with no moving caster in the cascade while the map still holds the static map alone. The moving casters are
the dynamic region's, tested one by one against the cascade's volume (no grid walk). The stress example reports the
build pass's GPU time per cascade (`gpu_ms.shadow_build`), the frames that built it (`shadow.builds`) and the moving
casters on it (`shadow.moving`); `gpu_ms.shadow` and `shadow.redraws` are the static map's, as before.

**The copy is a draw, not `copy_texture_to_texture`.** wgpu allows `Depth32Float` texture-to-texture copies of a
whole layer on Vulkan, Metal, DX12 and WebGPU, but its GLES backend copies through a read framebuffer with the
source on a colour attachment, which a depth texture cannot be, so GLES would need the draw anyway. The draw
reads the static map with `textureLoad` as an unfilterable float texture (allowed for depth formats), so the values
are copied exactly, works on every backend (the image test passes on `WGPU_BACKEND=gl`), and shares the pass that
draws the moving casters, with no copy command or barrier between them. It costs the same as the copy command: on
this PC (DX12, at full clocks, a 2048² map of blocky depth) the cleared pass with the draw took 0.066 ms and
`copy_texture_to_texture` 0.064 ms.

**2026-11-04**, same machine and settings as the tables above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed
1), `parts_100k` (1% of the parts moving every frame, about 720 of them inside the far cascade), `main` at
`6791e46` against branch `121-far-cascade-movers`; "all dynamic" is the branch with `--dynamic-cascades 3`, which
draws every caster into the far cascade whenever anything moves (`main` draws the same). Times are p50 in ms, except
the shadow passes: their mean over the measured frames, with how many frames ran them. A second round of all six
runs agreed within 1%.

| Camera | Build | Frame | Render CPU (p99) | Cull CPU | GPU | Opaque pass | Cascade 0 | Cascade 1 | Cascade 2 casters | Cascade 2 build |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| still | `main` | 0.709 | 0.559 (0.723) | 0.237 | 0.583 | 0.441 | 0.034 | 0.083 | 0.000 (0) | |
| still | #121 | 0.851 | 0.629 (0.789) | 0.260 | 0.741 | 0.446 | 0.036 | 0.085 | 0.000 (0) | 0.151 (1,000) |
| still | all dynamic | 1.178 | 0.887 (1.121) | 0.496 | 1.036 | 0.445 | 0.041 | 0.088 | 0.437 (1,000) | |
| moving | `main` | 0.696 | 0.547 (0.796) | 0.218 | 0.555 | 0.417 | 0.035 | 0.081 | 0.005 (8) | |
| moving | #121 | 0.828 | 0.617 (0.903) | 0.242 | 0.717 | 0.424 | 0.037 | 0.083 | 0.005 (8) | 0.151 (1,000) |
| moving | all dynamic | 1.120 | 0.876 (1.111) | 0.472 | 0.990 | 0.427 | 0.041 | 0.086 | 0.411 (1,000) | |

Last frame with the still camera: the static map holds 71,833 casters and 738 moving casters are drawn on its copy
(4 draws), against 46,368 casters in the far cascade drawn every frame when all are dynamic (it is fitted tighter
then: not centred on the camera, one-texel steps).

- **The look is back** (the done-when): moving parts cast shadows out to the shadow distance with the default
  settings. `a_moving_part_beyond_the_dynamic_cascades_casts_a_shadow` (`tests/shadows.rs`) checks a moving cube
  80 m away shadows the floor as darkly as with every cascade dynamic, on both instance paths.
- **The static map is still cached** (#82's done-when): it was drawn in none of the still camera's frames and in 8 of
  the walking camera's, as on `main`; `a_static_frame_does_not_redraw_the_far_cascade` keeps passing, and now also
  checks the far map is built with the moving ball.
- **Cost against `main`**: +0.16 ms of GPU, the build pass, every frame here because parts move in the far cascade in
  every frame (a scene whose far movers rest builds nothing). Of its 0.151 ms the copy is 0.144 ms (measured with
  the moving casters' draws switched off; the draws alone, in a cleared pass, took 0.022 ms): it reads and writes
  32 MiB, so it costs about what the card's bandwidth gives, whatever moves. Render CPU +0.07 ms: finding the moving
  casters (+0.02 ms, in the cull column: 1,000 bounding spheres against the cascade), uploading their list and
  recording the extra pass.
- **Cost against all dynamic**: the far cascade's GPU time drops from 0.437 to 0.151 ms and render CPU from 0.887 to
  0.629 ms (its cull of the whole static region every frame, 0.24 ms, is gone), with the same shadows. (#82 noted
  1.35 to 1.48 ms of GPU for the all-dynamic far cascade, measured while the machine was busy with builds; on a quiet
  machine it is the 0.44 ms here.)
- **What would cut the copy**: restoring only the texels last frame's moving casters covered, instead of the whole
  map, would make the build cost scale with the movers' footprint (#133).

### Levels of detail (#80)

Balls and cylinders far away are drawn with fewer triangles (`metallic_render::lod`, ADR 0008 §5). A Ball has four
levels (32 × 16 segments and rings, 960 triangles; 16 × 8, 224; 8 × 4, 48; 4 × 2, 8), a Cylinder three (32, 16 and
8 segments: 128, 64 and 32 triangles); a Block (12) and a Wedge (8) keep one. Each level halves the segments, so its
vertices are a subset of the finer level's. The cull chooses a part's level by its projected size, the measure
screen-size culling uses: a level is used while its outline strays from the true shape by less than half a pixel
divided by the LOD bias (`LodSettings`: Low 0.5, Medium 1, High 2; the renderer's default is High, a quarter of a
pixel). A part changes level only 10% past a threshold, and each grid item remembers the level it was drawn at, so a
part hovering at a threshold switches once. A kind draws one instanced draw per level in use: at most 9 opaque draws
for the built-in kinds. Shadow maps draw each caster at the coarsest level the camera could be drawing it wherever
the map is looked up (a cascade's slice's far corner; for a local light's tile, the light's distance from the camera
plus its range, rounded up to a power of two): never finer than the surface receiving the shadow, so levels add no
acne, and a cached cascade stays valid as the camera moves. The stress example's `--lod low|medium|high|off`,
`--lod-bias` and `--lod-tint` (the debug view colouring parts by level) set them; the JSON's `draws.levels` counts
the instances drawn at each level.

**2026-11-04**, same machine and settings as the tables above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed 1),
`main` at `441944f` against branch `80-primitive-lod` (default settings, so High). Two rounds each, which agreed
within 5%; the table is their mean (`see_through_10k`: a third round, after the last change to the see-through
sort). Times are p50 in ms, except the shadow pass: its mean over the measured frames.

| Scene | Build | Frame (p99) | Render CPU | Cull CPU | GPU | Opaque / transparent pass | Cascade 1 | Triangles drawn | Shadow triangles | Draws opaque / transparent / shadow |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `parts_100k` | `main` | 0.852 (1.061) | 0.635 | 0.265 | 0.742 | 0.447 / 0.000 | 0.085 | 3.67 M | 464,884 | 4 / 0 / 12 |
| `parts_100k` | #80 | 0.809 (0.915) | 0.657 | 0.314 | 0.529 | 0.261 / 0.000 | 0.063 | 0.66 M | 182,180 | 9 / 0 / 17 |
| `parts_100k --moving-camera` | `main` | 0.815 (1.419) | 0.633 | 0.257 | 0.718 | 0.425 / 0.000 | 0.083 | 2.31 M | 441,824 | 4 / 0 / 12 |
| `parts_100k --moving-camera` | #80 | 0.791 (1.256) | 0.638 | 0.287 | 0.543 | 0.276 / 0.000 | 0.061 | 0.46 M | 171,632 | 9 / 0 / 17 |
| `see_through_10k` | `main` | 1.265 (1.595) | 1.084 | 0.022 | 1.170 | 0.066 / 1.034 | 0.014 | 4.86 M | 24 | 1 / 4,199 / 2 |
| `see_through_10k` | #80 | 1.160 (1.235) | 1.077 | 0.047 | 0.758 | 0.065 / 0.626 | 0.013 | 1.23 M | 24 | 1 / 4,199 / 2 |
| `mobile` | `main` | 0.408 (0.546) | 0.264 | 0.033 | 0.324 | 0.118 / 0.002 | 0.080 | 183,800 | 319,808 | 4 / 1 / 12 |
| `mobile` | #80 | 0.378 (0.509) | 0.281 | 0.041 | 0.293 | 0.110 / 0.002 | 0.059 | 65,080 | 120,368 | 8 / 1 / 16 |

`parts_100k`'s last frame drew 24,277 instances at full detail (every block and wedge, and the near round parts),
1,051 at level 1, 4,801 at level 2 and 319 at level 3. At Medium it draws 0.51 M triangles for 0.508 ms of GPU, at
Low 0.43 M for 0.492 ms.

- **GPU**: the triangles at distance were most of the opaque pass's cost: 82% fewer of them on `parts_100k` take
  0.19 ms (42%) off it and 29% off the frame's GPU time; the redrawn cascades' casters lose 61% of theirs.
  `see_through_10k`'s 5,000 see-through balls take their levels too (half of them level 1), which takes 0.41 ms off
  its blended pass.
- **Draws**: one per kind and level in use, 9 at most for opaque parts, 5 more than `main` on `parts_100k`. The
  see-through runs stay as many (4,199): they were already broken by every change of shape, and same-shape
  neighbours in back-to-front order are mostly at the same level.
- **CPU**: the cull chooses each round part's level from its distance (a few multiplies, branch-free, the level
  written only when it changes): +0.05 ms of cull CPU on `parts_100k`, whose 30% round parts put levels in nearly
  every grid cell, and +0.02 ms of render CPU in all. A grid cell that has never held a round part takes the loop
  without levels, and cells with round parts go through an out-of-line one, so a scene of blocks chooses nothing;
  the perf gate's `render_cull` (10,000 blocks) still counts +4.8% instructions (334,647 → 350,854 `Ir`; a first
  version with the levels' loop inline counted +8.0%, of which a temporary walk-only workload put +8.1% in the walk
  itself, not in moving the 100 items), while its wall time on the dev PC is within 2% (a throwaway copy of the
  workload: 18.8 against 19.1 µs a walk). `cargo bench -p metallic_render --bench gpu_scene -- cull` on 100,000
  parts with the stress city's mix of shapes: 114.6 µs with levels, 109.7 µs without.
- **Cached shadows**: the far cascade's static map was drawn in 8 of the walking camera's frames, as on `main`: its
  casters' levels follow its slice and the camera's projection, not where the camera stands.
- **The look**: rendered offscreen at 1920 × 1080 with the default settings, `parts_100k`'s default view has 5 pixels
  of 2,073,600 more than 12 levels (of 255) away from the full-detail image, and a view from 45 m up over the whole
  city 2,842 (0.14%), lone pixels on silhouettes and shadow edges (`docs/screenshots/80-lod-*.png`). `tests/lod.rs`
  holds every preset to such a bound on rows of balls and cylinders, checks both instance paths draw the same, and
  that coarser shadows put no acne on their receivers: none at any preset, while with the casters drawn at full
  detail instead it finds 1.4% of the parts' pixels darkened.

### Graphics settings (#84)

A player's [`GraphicsSettings`](../crates/render/src/settings.rs) (ADR 0008 §8): a preset (Low, Medium, High, Ultra or
Auto) and any per-setting overrides (render scale, dynamic resolution with its target and range, frame cap and vsync,
view distance, shadow quality and distance, local-light budget, LOD bias), resolved into the settings of every system
above (`Quality`). Presets scale cost and never remove a game's look: none turns the sun's shadows or the local
lights off, and no override can. Anti-aliasing and effect quality (#85) are in the next section.

| Preset | Render scale | View distance | Culled under | Sun cascades | Local lights a frame (per cluster) | Shadowed lights, atlas / tile | LOD bias |
| --- | ---: | ---: | ---: | --- | ---: | --- | ---: |
| Low | 75% | 512 m | 2 px | 1 × 1024², 128 m (ADR 0008's Low) | 32 (32) | 2, 1024² / 256² | 0.5 |
| Medium | 100% | 1,024 m | 1.5 px | 2 × 2048², 192 m, the far one cached | 128 (48) | 8, 2048² / 256² | 1 |
| High | 100% | 2,048 m | 1 px | 3 × 2048², 256 m, the far one cached (the renderer's defaults) | 192 (64) | 8, 4096² / 512² | 2 |
| Ultra | 100% | none | 0.5 px | 3 × 3072², 256 m, the far one cached | 256 (64) | 10, 4096² / 512² | 4 |

**Auto** draws at the tier its calibration chose, with dynamic resolution between 50% and 100% holding 15 ms of
GPU time a frame (a 60 fps frame with 10% to spare). The fixed presets have dynamic resolution off unless the
settings turn it on.

**Render scale.** The scene is drawn into a scene colour texture at `scale ×` the window's size and the upscale pass
(`PassId::Upscale`, `src/upscale.rs`: one bilinear tap a pixel, the seam #85's better upscaler replaces) draws it over
the whole window; the UI is drawn after it at the window's resolution. At 100% with dynamic resolution off nothing
changes: no texture, no upscale pass, the same wgpu calls as before (`tests/no_alloc.rs`). **Dynamic resolution**
makes the scene's textures at its highest scale once and draws each frame into their top-left corner (a viewport
and scissor), so a new scale never makes a texture; the scale follows the GPU timestamps (one to three frames late)
with a damped step: from a frame drawn at scale `s` that took `t` ms the scale that takes the target `T` is
`s·√(T/t)`, each step goes a quarter of the way there, and none is taken within 3% of the target. Culling, levels of
detail and the lights' ranking keep the window's size, so the scale moves none of them; the light clusters follow
the size drawn (`tests/resolution.rs` checks the lights land in the same places at half and double scale).

**Auto's calibration** (`settings::Calibrator`) times the game's own first frames on the GPU: 10 frames at Ultra to
settle (shadow maps, atlas and first uploads), then the median of the next 30 timed frames; Ultra is kept if it takes
at most half a 60 fps frame (8.3 ms), otherwise High is measured the same way (at most 60%, 10 ms), then Medium (80%,
13.3 ms), and Low is what is left. It reads the timestamps the renderer already takes, so it waits for nothing; a
machine that takes Ultra is calibrated after about 45 frames (the Player on `many_lights` with vsync: at frame 50,
under a second at 60 Hz), the slowest after about 130. A device without timestamp queries gets a tier from its kind
(discrete: High, integrated or software: Low). The result is saved with the adapter's name and driver and measured
again when either changes. Measuring the game being played rather than a fixed workload keeps it short, but a
calibration made on a light game is kept for heavier ones, where dynamic resolution then holds the frame time (#139
tracks a fixed workload). Under vsync the GPU runs at lower clocks, so it measures longer times than the uncapped
numbers below (3.93 ms against 3.08 on `many_lights` at Ultra), which only makes it more careful.

**The file** is `graphics.toml` in the user's config folder (`metallic_platform::UserPaths::config_dir`; on Windows
`%APPDATA%\metallic\Player\config`): `key = value` lines, a small subset of TOML written and read by
`GraphicsSettings::to_text` and `from_text` (no dependency), every key not set listed as a comment with the preset's
value, and a `[calibration]` section. The Player makes it with the Auto preset the first time, falls back to Auto with
a warning when it cannot be read or understood (leaving the file as it is), applies `--preset` on top for one run, and
saves Auto's calibration back. **Scripts** read the settings in effect from `game:GetService("UserGameSettings")`
(`GraphicsPreset`, `QualityLevel` 1 to 4, `RenderScale`, `DynamicResolution`, `TargetFrameTime`, `ViewDistance`,
`ShadowQualityLevel`, `ShadowDistance`, `LocalLightBudget`, `LodBias`, `VSync`, `FrameRateCap`, and since #85
`AntiAliasing` and `EffectQualityLevel`), each read-only for
scripts and never saved with the game.

**2026-11-04**, same machine as the tables above (Ryzen 5 5500, RX 6650 XT, Windows 11, **DX12**, `AutoNoVsync`,
1,000 frames after 120 of warm-up, seed 1, the scenes' still cameras), branch `84-graphics-settings`; "defaults" is the
renderer's defaults (no `--preset`), what every table above measured. Times are p50 in ms, p99 in brackets; Auto
calibrated to Ultra on every scene here (in the warm-up) and then held its 15 ms target at 100%.

| Scene | Preset | Frame | Render CPU | GPU | Opaque pass | Upscale pass |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `parts_100k` (1920 × 1080) | defaults | 0.796 (0.890) | 0.647 (0.728) | 0.527 | 0.260 | |
| | Low | 0.723 (0.934) | 0.574 (0.775) | 0.330 | 0.187 | 0.053 |
| | Medium | 0.726 (0.888) | 0.577 (0.695) | 0.446 | 0.240 | |
| | High | 0.796 (0.921) | 0.646 (0.745) | 0.527 | 0.260 | |
| | Ultra | 1.019 (1.272) | 0.694 (0.913) | 0.906 | 0.301 | |
| | Auto (Ultra) | 1.080 (1.319) | 0.690 (0.894) | 0.959 | 0.285 | 0.045 |
| `many_lights` (1920 × 1080) | defaults | 2.261 (2.460) | 0.552 (0.714) | 2.100 | 1.831 | |
| | Low | 0.705 (0.861) | 0.344 (0.475) | 0.582 | 0.498 | 0.058 |
| | Medium | 2.135 (2.342) | 0.493 (0.612) | 1.980 | 1.798 | |
| | High | 2.599 (2.763) | 0.613 (0.742) | 2.457 | 2.218 | |
| | Ultra | 3.224 (3.449) | 0.652 (0.754) | 3.084 | 2.654 | |
| | Auto (Ultra) | 3.277 (3.484) | 0.676 (0.809) | 3.127 | 2.623 | 0.059 |
| `mobile` (1280 × 720) | defaults | 0.383 (0.509) | 0.283 (0.403) | 0.292 | 0.110 | |
| | Low | 0.310 (0.360) | 0.238 (0.280) | 0.121 | 0.069 | 0.024 |
| | Medium | 0.310 (0.450) | 0.235 (0.360) | 0.205 | 0.095 | |
| | High | 0.380 (0.518) | 0.281 (0.392) | 0.292 | 0.110 | |
| | Ultra | 0.673 (0.815) | 0.270 (0.387) | 0.587 | 0.131 | |
| | Auto (Ultra) | 0.697 (0.850) | 0.286 (0.404) | 0.605 | 0.117 | 0.024 |

- **High is the renderer's defaults** but for its 192-light budget: the same frame on `parts_100k` and `mobile`, and
  0.36 ms more GPU on `many_lights`, whose town of 1,152 lamps fills any budget.
- **Ultra's cost is its shadows**: 3072² cascades hold 2.25 times the texels of High's, and the cached far cascade's
  build pass copies them every frame (`parts_100k`: +0.38 ms of GPU); the opaque pass grows by the finer levels of
  detail and, on `many_lights`, the 256 lights.
- **Low** draws `many_lights` in under a third of Medium's GPU time: 32 lights a frame instead of 128 (those beyond
  the budget fade by importance, ADR 0008 §6) at 75% scale. On `parts_100k` its single cascade is redrawn whenever a
  part moves (one cascade cannot be cached), which is why its p99 is not much lower than Medium's.
- **The upscale pass** costs 0.04 to 0.06 ms at 1080p and 0.02 ms at 720p. Auto pays it at 100% too, because
  dynamic resolution draws into textures of its own.
- **Render CPU** stays within 0.1 ms of the defaults at every preset, inside the High tier's 1.5 ms everywhere; Ultra's
  256-light budget and finer levels of detail add the most.
- **At the renderer's defaults nothing changes** against `main` at `225d5e1` (two interleaved rounds, `main` then the
  branch): `parts_100k` frame 0.803 and 0.799 → 0.796 and 0.797, render CPU 0.653 and 0.649 → 0.645 and 0.647, GPU
  0.527 → 0.527 and 0.528; `many_lights` GPU 2.099 and 2.050 → 2.101 and 2.054; `mobile` render CPU 0.288 and 0.285 →
  0.281 and 0.281. All within the run-to-run noise: a frame at the window's resolution makes the same calls as before.

Render scale against GPU time on `parts_100k` at High (1920 × 1080; 600 measured frames, p50 in ms):

| Render scale | 50% | 75% | 100% (no upscale) | 150% | 200% |
| --- | ---: | ---: | ---: | ---: | ---: |
| GPU | 0.443 | 0.501 | 0.527 | 0.786 | 1.087 |
| Opaque pass | 0.132 | 0.182 | 0.260 | 0.430 | 0.684 |
| Upscale pass | 0.043 | 0.044 | | 0.042 | 0.048 |

**Dynamic resolution holds the target** (the done-when). On this GPU `parts_100k` costs 0.53 ms at 100% and 1.09 ms
at 200%, and about 0.38 ms of it (the shadow passes and the far cascade's build) does not depend on the resolution, so
a target of 2 ms, or Auto's 15, is met at every scale and nothing scales. The targets below are inside what the scales
can reach, so the controller has to hold them: 0.8 ms over 50 to 200% (about 150%, supersampling) and 0.48 ms over 50
to 100% (about 63%), at the High preset. 1,000 frames after 120 of warm-up; the deviation is from the target.

| Target | Range | Camera | GPU mean | GPU p50 | GPU p99 | Scale mean (max) |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 0.80 ms | 50–200% | still | 0.794 (−0.8%) | 0.796 (−0.5%) | 0.804 (+0.5%) | 1.488 (1.488) |
| 0.48 ms | 50–100% | still | 0.478 (−0.4%) | 0.478 (−0.4%) | 0.484 (+0.8%) | 0.630 (0.635) |
| 0.80 ms | 50–200% | walking | 0.802 (+0.3%) | 0.803 (+0.4%) | 0.887 (+10.9%) | 1.419 (1.513) |

With the walking camera the load changes under the controller: the mean and median stay within 0.4% of the target and
99% of the frames within 11% (the slowest 1% include the far cascade's static map drawn again, 0.6 ms of GPU in one
frame, which no resolution absorbs). `tests/no_alloc.rs` checks the moving scale allocates nothing, and that scaled
frames add exactly what the upscale pass and the viewports add by hand.

**Switching presets causes no hitch** (the done-when). Nothing waits for the GPU: a switch makes the new shadow maps,
atlas and scaled textures (wgpu frees the old ones once the GPU is done with them), every pipeline was made with the
renderer, and the first frame after it draws every shadow map again. `--switch-presets 30` switches between Low and
Ultra every 30 frames (34 switches in the 1,000 measured frames; frame times in ms):

| Scene | Steady Low p99 / max | Steady Ultra p99 / max | Switching p50 / p99 / max | Switching render CPU max |
| --- | ---: | ---: | ---: | ---: |
| `parts_100k` | 0.934 / 1.225 | 1.272 / 2.072 | 0.859 / 2.051 / 2.114 (2.425 in a second run) | 1.495 (2.094) |
| `many_lights` | 0.861 / 0.933 | 3.449 / 3.627 | 1.731 / 3.719 / 3.829 | 1.139 |
| `mobile` | 0.360 / 0.423 | 0.815 / 0.866 | 0.446 / 1.144 / 1.316 | 0.521 |

The worst frame of a switch sequence was 2.4 ms on `parts_100k` and 3.8 ms on `many_lights`, at most 0.4 ms over
the slowest steady Ultra frame: making the textures and culling every cascade's casters again, the far cascade's static
map included. That is a seventh of a 60 fps frame at worst.

### HDR post chain, anti-aliasing and upscaling (#85)

The scene is drawn into an HDR scene colour and finished into the window by one pass (`metallic_render::post`, ADR
0008 §7–8):

- **The scene colour** is `Rg11b10Ufloat` (4 bytes a pixel) where the adapter can render, blend and filter it, else
  `Rgba16Float`, else 8-bit sRGB (no headroom; the chain still runs). Every backend on the dev PC (DX12, Vulkan, GL)
  takes `Rg11b10Ufloat`; GLES without float colour buffers gets the fallbacks. The format holds green to 1/128 and blue
  to 1/64 of a value, so flat colours can land a level or two from where 8 bits put them.
- **One finish pass** (`PassId::Post`) reads it and writes the window: exposure, colour filter, bloom's composite, the
  tone curve, saturation and contrast, MLAA's neighbourhood blend, the render scale's bilinear upscale (so the
  upscale pass of #84 is no longer planned beside it), sRGB encoding where the window does not encode, and half a
  level of dither. **Exposure** is fixed (1, `PostSettings::exposure`); nothing is read back.
- **The tone curve** is the identity up to 0.85 and rolls off towards 1 above it, per channel. Before #85 the window
  clamped at 1, so everything below the knee looks as it did: 1.0 now shows as 0.945 (249 of 255 instead of 255),
  1.5 as 0.996, and highlights keep a little detail instead of clipping.
- **Bloom** (`PassId::Bloom`): downsampling passes from half (Low: a quarter) of the window's size to a thirty-second,
  then upsampling passes adding each level onto the next larger (a 3×3 tent), and 10% of the sum mixed into the image
  with no threshold, so light much brighter than its surroundings glows (`docs/screenshots/85-bloom-night.png`, a crop
  in `85-bloom-night-crop.png`). Its levels follow the window, not the render scale, so dynamic resolution never
  changes the glow. Every preset draws it as wide; lower ones skip the finest level and use the 5-tap dual filter
  instead of the 13-tap one.
- **MLAA** (`PassId::Mlaa`): one pass finds luma edges, walks them (8, 16 or 32 pixels each way) to their ends and
  their crossing edges, and writes the areas of MLAA's L, U and Z shapes as blend weights; the finish pass blends with
  one bilinear tap. **MSAA** resolves inside the last scene pass and discards the multisampled colour (the plan of #77).
- **An open chain**: the effects run in a `PostChain`'s order and only read the scene colour, so they can be added,
  removed and reordered; one not in the pass set begins no pass, makes no texture and binds nothing (the finish pass
  has a variant per combination). **Merged scene passes** (`PassId::Scene`: the Low preset, and every preset on
  tile-based GPUs) record opaque, sky and transparent as one render pass, depth and the MSAA colour cleared and
  discarded inside it, so on a tiler they never leave tile memory.
- Every pipeline is made with the renderer, the scene's for each MSAA count it can switch to, so changing the
  anti-aliasing or the effect quality makes textures and compiles nothing (`tests/post.rs` switches every frame).

| Preset | Anti-aliasing | Bloom | Passes after the shadows | Render passes | Full-screen passes |
| --- | --- | --- | --- | ---: | ---: |
| before #85 (every preset) | none | none | opaque, sky, transparent, (upscale at Low), ui | 4 (5) | 4 (5) |
| Low | MLAA, 8 px | 4 levels from ¼, 5-tap | scene, mlaa, bloom, post, ui | 11 | 4 |
| Medium | MLAA, 16 px | 5 levels from ½, 5-tap | opaque, sky, transparent, mlaa, bloom, post, ui | 15 | 6 |
| High | MSAA 4× | 5 levels from ½, 13-tap | opaque, sky, transparent, bloom, post, ui | 14 | 5 |
| Ultra | MSAA 8× | as High | as High | 14 | 5 |

"Full-screen" counts the passes that read or write a whole target at the scene's or the window's resolution; bloom's
passes work at half of it and below and are counted with the render passes. The overlay's `POST` line and the stress
JSON's `graphics.post` (`scene_format`, `anti_aliasing`, `bloom_levels`, `render_passes`, `full_screen_passes`, and
the planned `passes` in order) report them; the overlay's GPU line and the JSON's `gpu_ms.passes` time each, bloom's
chain as one. The stress example's `--aa`, `--effects`, `--merge-scene` and `--no-post` set them by hand.

**2026-11-04**, same machine as the tables above (Ryzen 5 5500, RX 6650 XT, Windows 11, **DX12**, `AutoNoVsync`,
1,000 frames after 120 of warm-up, seed 1, still cameras), branch `85-post-chain` at `3bee7fb` (measured before rebasing onto `77a0483`, whose changes are in `crates/luau`). Each preset is run as
it is now and with `--no-post`, which draws that preset's frame as before #85 (no HDR target, bloom, tone curve or
MLAA, the scene passes apart, the upscale pass at Low; High and Ultra keep their MSAA). Times are p50 in ms, the
frame's p99 in brackets; one round each.

| Scene | Preset | Frame before → after | Render CPU before → after | GPU before → after | MLAA / bloom / finish pass |
| --- | --- | ---: | ---: | ---: | ---: |
| `parts_100k` (1920 × 1080) | Low | 0.773 → 0.874 (1.458) | 0.613 → 0.712 | 0.348 → 0.399 | 0.058 / 0.048 / 0.098 |
| | Medium | 0.829 → 1.061 (1.825) | 0.660 → 0.883 | 0.445 → 0.720 | 0.094 / 0.109 / 0.089 |
| | High | 0.859 → 1.102 (3.074) | 0.701 → 0.927 | 0.661 → 0.792 | – / 0.117 / 0.074 |
| | Ultra | 1.288 → 1.404 (2.100) | 0.960 → 0.986 | 1.161 → 1.271 | – / 0.124 / 0.086 |
| `many_lights` (1920 × 1080) | Low | 0.695 → 0.842 (1.023) | 0.418 → 0.451 | 0.574 → 0.730 | 0.063 / 0.044 / 0.099 |
| | Medium | 2.155 → 2.398 (2.661) | 0.597 → 0.718 | 1.920 → 2.239 | 0.114 / 0.117 / 0.107 |
| | High | 2.864 → 3.004 (3.464) | 0.732 → 0.939 | 2.744 → 2.879 | – / 0.121 / 0.084 |
| | Ultra | 3.726 → 3.879 (4.423) | 0.800 → 1.012 | 3.598 → 3.724 | – / 0.122 / 0.085 |
| `mobile` (1280 × 720) | Low | 0.324 → 0.400 (0.716) | 0.248 → 0.325 | 0.123 → 0.191 | 0.030 / 0.039 / 0.042 |
| | Medium | 0.305 → 0.511 (0.844) | 0.232 → 0.428 | 0.203 → 0.387 | 0.051 / 0.088 / 0.042 |
| | High | 0.434 → 0.566 (1.083) | 0.285 → 0.442 | 0.352 → 0.471 | – / 0.100 / 0.038 |
| | Ultra | 0.786 → 0.893 (1.276) | 0.316 → 0.466 | 0.697 → 0.797 | – / 0.101 / 0.041 |

**`mobile` at Low, pass by pass** (the done-when: the bandwidth-heavy passes fused):

| | Passes (GPU p50, ms) | Full-screen passes | GPU |
| --- | --- | ---: | ---: |
| before | opaque 0.070, sky 0.005, transparent 0.001, upscale 0.023, ui 0.000 | 5 | 0.123 |
| after | scene 0.062, mlaa 0.030, bloom 0.039, post 0.042, ui 0.000 | 4 | 0.191 |

The three scene passes became one (0.076 → 0.062 ms: no depth or colour stored and loaded between them), and the
finish pass does what would otherwise be four full-screen passes over the window (tone mapping, bloom's composite,
MLAA's blend, the upscale) for 0.019 ms more than the bare upscale it replaces. The same fusion takes `parts_100k` at
Low from 0.273 ms of scene and upscale passes to a 0.145 ms scene pass.

- **What the chain costs**: bloom is 0.04 ms at Low (from a quarter) and 0.09 to 0.12 ms from half, about the same at
  720p as at 1080p, so most of it is its nine small passes, not texels; the finish pass is 0.04 ms at 720p and 0.07 to
  0.10 ms at 1080p; MLAA 0.03 to 0.06 ms at 75% scale and 0.09 to 0.11 ms at 1080p. MSAA (unchanged by #85; High's 4×
  against #84's High without it) costs about 0.13 ms on `parts_100k` and 0.06 ms on `mobile`, 8× about 0.25 ms.
- **Render CPU** grows by about 18 µs per render pass the chain adds (wgpu's recording and submit per pass): 0.1 ms at
  Low, 0.2 ms at Medium and High, inside the High tier's 1.5 ms everywhere (0.94 ms at most, `many_lights` at High).
  Bloom's chain is most of those passes; fewer, larger passes are the obvious next step.
- **Allocations**: `tests/no_alloc.rs` holds the default frame, MLAA's, the merged scene's, the scaled ones and every
  parts frame to the same wgpu calls written out by hand with the chain's passes: equal on Vulkan, DX12 and GL here.

**The look.** The reference images were made again through the chain (`crates/render/tests/reference/`; before,
after and their difference times eight in `docs/screenshots/85-references.png`, crops of the sun and the parts in
`85-references-crops.png`). Below the knee nothing moves but for bloom's soft edges and the scene colour's precision
(the rings in the eightfold difference of the night images are its steps, a level apart); the sun, the neon cube and
the brightest lit faces lose up to 6 levels to the shoulder and gain a glow. Against the old references the mean
difference is 1 to 3 levels. `docs/screenshots/85-aa-edges.png` shows the same edges of `parts_10k` with no
anti-aliasing, MLAA, 4× and 8× MSAA, four times enlarged.
