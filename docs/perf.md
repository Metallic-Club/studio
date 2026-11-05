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
of warm-up, seed 1, window at 100% scaling. Vulkan could not be measured: no Vulkan adapter accepted a window's
surface while the monitor was switched off (#110, "Vulkan in a window" below; offscreen Vulkan works, see
"Drawing parts").

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
  Since #106 the storage path draws them in one draw ("Transparent draws" below).
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
  map, would make the build cost scale with the movers' footprint (#133, below).

### Restoring the moving casters' footprints (#133)

The far cascade's build pass no longer copies the whole static map when the map already holds it. A map built
since the static map was last drawn holds the static map and the moving casters it was built with, so the next build
**loads** the map, copies the static map back only under those casters' **footprints** (one rectangle per caster,
drawn as one instanced draw from a vertex buffer of clip-space corners, with the copy's fragment shader), then draws
this frame's moving casters. The whole map is still copied, as before, when the static map was drawn again (or the map
is new), and when the footprints add up to more texels than the map holds.

A footprint is the square around the caster's bounding sphere as the cascade's orthographic projection sees it, and
a texel more on every side. It is conservative: a triangle covers a texel only when it holds the texel's centre (so
does the rectangle), and every built-in mesh at every level of detail lies inside the unit cube the bounding sphere
passes through the corners of, so whatever level the cascade draws a caster at (#80), it stays inside. The slope-scaled
depth bias of #126 moves only the depth a caster writes, never which texels, and the receivers' filter only reads
the map: a texel no moving caster wrote holds the static map's depth however far the taps reach. The unit test
`a_footprint_holds_its_caster_at_every_level_of_detail` (`shadow/maps.rs`) places every mesh and level, turned and
stretched, and checks every vertex lands a texel inside its footprint. The footprints are found with the moving
casters (one matrix product and a few additions each: about 9 µs a frame for 738 of them, timed on their own), kept
in lists that stop growing once they have held every moving caster, and uploaded with the casters' lists in the same
staging write (one more buffer copy).

The stress example reports, per cascade, the frames whose build restored footprints (`shadow.restores`) and the
texels the last frame's restore drew (`shadow.restored_texels`, overlaps counted as often as drawn); `--moving <n>`
keeps only the first `n` of the scene's moving parts moving.

**2026-11-04**, same machine and settings as the tables above (DX12, `AutoNoVsync`, 1,000 frames after 120, seed
1), `parts_100k`, branch `133-cascade-footprints`. "Copy" is the same build with the restore switched off (a
throwaway switch, not committed), which builds the map as #121 does. The far movers are the moving casters in the far
cascade in the last frame (`--moving 1`, `--moving 13` and all 1,000 moving parts); the footprint is the texels the
last frame's restore drew. Build times are the mean over the measured frames; GPU and frame are p50, in ms.

| Map | Camera | Far movers | Footprint (texels) | Build, copy | Build, restore | GPU, copy | GPU, restore | Frame, copy | Frame, restore |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2048² | still | 1 | 276 | 0.140 | 0.0001 | 0.718 | 0.569 | 0.840 | 0.714 |
| 2048² | still | 10 | 1,804 | 0.140 | 0.0001 | 0.719 | 0.571 | 0.843 | 0.711 |
| 2048² | still | 738 | 124,980 (3.0%) | 0.141 | 0.011 | 0.721 | 0.583 | 0.933 | 0.904 |
| 2048² | moving | 1 | 276 | 0.139 | 0.0006 | 0.733 | 0.584 | 0.860 | 0.727 |
| 2048² | moving | 9 | 1,660 | 0.139 | 0.0007 | 0.733 | 0.586 | 0.863 | 0.741 |
| 2048² | moving | 716 | 123,660 (2.9%) | 0.140 | 0.013 | 0.733 | 0.595 | 0.910 | 0.900 |
| 3072² (Ultra) | still | 1 | 573 | 0.356 | 0.0001 | 1.037 | 0.673 | 1.160 | 0.805 |
| 3072² (Ultra) | still | 10 | 3,680 | 0.357 | 0.0008 | 1.040 | 0.674 | 1.161 | 0.804 |
| 3072² (Ultra) | still | 738 | 253,850 (2.7%) | 0.367 | 0.038 | 1.049 | 0.705 | 1.183 | 0.921 |
| 3072² (Ultra) | moving | 1 | 573 | 0.355 | 0.0029 | 1.056 | 0.693 | 1.175 | 0.824 |
| 3072² (Ultra) | moving | 9 | 3,381 | 0.355 | 0.0034 | 1.056 | 0.694 | 1.176 | 0.827 |
| 3072² (Ultra) | moving | 716 | 251,375 (2.7%) | 0.365 | 0.040 | 1.065 | 0.722 | 1.192 | 0.896 |

A first round of all 24 runs, with the footprints rounded out to whole texels (about 14% more texels), agreed within
0.003 ms on every build time.

- **The build scales with the footprint, not the map** (the done-when): 0.140 ms whatever moves with the copy; with
  the restore, 0.0001 ms for one far mover, 0.011 ms for 738 of them covering 3% of the map. The walking camera's
  means include its 8 frames whose static map was drawn again, which copy the whole map as before (`shadow.restores`
  992 of 1,000 builds).
- **Against #121's numbers**: its build took 0.151 ms (0.144 of it the copy) and added 0.16 ms to the GPU frame over
  `main`; now the 738 far movers cost 0.011 ms, and the frame's GPU time is 0.583 ms against the copy's 0.721. (The
  opaque pass is about 0.2 ms cheaper than in #121's table since #80's levels of detail.)
- **Ultra** gains most: its 3072² copy took 0.36 ms, more than the 2.25 times 2048²'s that its texels would say, and
  the restore takes 0.04 ms for 738 movers, so the GPU frame drops by a third (1.049 to 0.705 ms).
- **Frame time** follows where the GPU bounds it (few movers: 0.84 to 0.71 ms); with every mover the frame is bound
  by the render CPU (0.75 ms), which the restore does not change: 0.776 and 0.752 ms p50 with the copy and the restore
  (still), 0.753 and 0.746 (walking), within run-to-run noise, and the cull column the same.
- **The fallback**: with the footprints made larger on purpose (a throwaway scale on their radius, every frame
  restoring), the 738 movers' restore took 0.026 ms at 11% of the map's texels, 0.042 at 23%, 0.071 at 39%, 0.092
  at 59%, 0.111 at 84% and 0.138 at 146%, against 0.141 for the copy: a texel restored costs about what a texel copied
  does, plus the rectangles' overlap. So the whole map is copied once the footprints add up to more than the map
  (`MAX_RESTORE_SHARE`, 1.0), short of the measured 1.4 for GPUs whose rectangles cost more. On a tiled GPU the
  restore's load reads the whole map into tile memory, as the copy's reads do; there is no Mobile-tier device to
  measure that on yet.
- **No stale shadow**: `a_moving_part_leaves_no_stale_shadow_behind` (`tests/shadows.rs`) moves an 80 m-away cube
  2 m a frame across 30 m of the far cascade, every frame restoring (about 2,600 texels), and compares the image with
  a fresh renderer's of the same scene, whose map is a whole copy of the static map with the cube on top: they are
  equal pixel for pixel, on both instance paths (and on DX12, Vulkan and GL here). With the restore draw left out the
  images differ by up to 37 levels, and with the footprints' radius halved by 35.
- **Allocations**: `a_restoring_cached_cascade_frame_allocates_nothing_beyond_wgpus_own` (`tests/no_alloc.rs`)
  compares frames that restore (a still sun, balls moving in every frame) with the same wgpu calls by hand, for 4 and
  24 moving balls.

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
  the perf gate's `render_cull` (10,000 blocks) still counted +4.8% instructions (334,647 → 350,854 `Ir`; a first
  version with the levels' loop inline counted +8.0%, of which a temporary walk-only workload put +8.1% in the walk
  itself, not in moving the 100 items), while its wall time on the dev PC was within 2% (a throwaway copy of the
  workload: 18.8 against 19.1 µs a walk). #138 found where and won it back (below). `cargo bench -p metallic_render --bench gpu_scene -- cull` on 100,000
  parts with the stress city's mix of shapes: 114.6 µs with levels, 109.7 µs without.
- **Cached shadows**: the far cascade's static map was drawn in 8 of the walking camera's frames, as on `main`: its
  casters' levels follow its slice and the camera's projection, not where the camera stands.
- **The look**: rendered offscreen at 1920 × 1080 with the default settings, `parts_100k`'s default view has 5 pixels
  of 2,073,600 more than 12 levels (of 255) away from the full-detail image, and a view from 45 m up over the whole
  city 2,842 (0.14%), lone pixels on silhouettes and shadow edges (`docs/screenshots/80-lod-*.png`). `tests/lod.rs`
  holds every preset to such a bound on rows of balls and cylinders, checks both instance paths draw the same, and
  that coarser shadows put no acne on their receivers: none at any preset, while with the casters drawn at full
  detail instead it finds 1.4% of the parts' pixels darkened.

**Where `render_cull`'s +4.8% was (#138).** Callgrind on the gate's runner (ubuntu-latest, the gate's release
build of the bench, then the same build with line tables to place each instruction), self `Ir` of `render_cull`:

| Function | before #80 (`a3363a2`) | #80 (`225d5e1`) | #138 |
| --- | ---: | ---: | ---: |
| `InstanceScene::cull` (the walk; the grid's cull is inlined into it) | 286,746 | 300,581 | 282,132 |
| `Grid::update` (moving the 100 queued items) | 26,704 | 29,000 | 29,000 |
| sorting the cells, `floorf`, `Grid::remove`, the rest | 21,251 | 21,285 | 21,297 |
| **Total** (the gate's count) | **334,701** | **350,866** | **332,429** |

- **The walk, +13,835**: none of it in the levels' code. The loop over a cell's items compiled to the same tests and
  the same push, but after every listed item it reloaded two values from the stack (the list table's address and the
  frustum's plane count): 2 instructions × 5,951 visible items. The loop had run out of callee-saved registers:
  #80 put a call on its likely path (the see-through push, moved out of line to keep `push` small) and kept the
  view's two level flags (`levels`, `remember`) live across the whole pass, and the register allocator spilled the
  item loop's values rather than those. Neither change alone gets them back: marking the see-through push `#[cold]`
  frees one (338,626, +1.2%), and deciding per cell while ordering whether it takes levels (a bit in the order
  entry, with the out-of-line helpers reading `remember` from the view) frees the other. With both, the push also
  falls through into the next item instead of jumping back: one jump fewer per listed item than before #80.
- **Moving the items, +2,296**: `Grid::update` works out each item's class of levels and keeps its level (about 23
  instructions a move). That is what levels cost and stays.
- `render_sync_moving` and `render_sync_scattered` count the same as `main` (132,488 and 143,697 on that runner).
- `main` at `cfe43ca` counted the same as #80 (350,878; `InstanceScene::cull` and `Grid::update` exactly so). The
  library's own `--emit asm` had shown the loop as before #80, probably because the spill depends on everything else
  live in the function the loop ends up in, which the gate's build (thin LTO, one codegen unit) decides: profile the
  gate's binary. The throwaway workflow that produced these (`diag-138-*` branches, deleted) ran `cargo bench -p
  metallic_render --bench perf_gate` per commit, again with `CARGO_PROFILE_BENCH_DEBUG=line-tables-only` and
  `GUNGRAUN_CALLGRIND_ARGS="--dump-instr=yes --compress-pos=no --compress-strings=no"`, and matched the counts per
  instruction against `objdump -d`.

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

**Auto's calibration** as #84 made it (#139 replaced it with a fixed workload: see
[Auto's calibration on a fixed workload](#autos-calibration-on-a-fixed-workload-139)) timed the game's own first
frames on the GPU: 10 frames at Ultra to settle (shadow maps, atlas and first uploads), then the median of the next
30 timed frames; Ultra is kept if it takes
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

### No hitches: pipelines off the frame (#86)

ADR 0008 §9 says no pipeline is compiled on the frame and nothing in a frame waits for the GPU. Before #86 every
pipeline was compiled up front, on the calling thread, inside `Renderer::new`: no frame hitched on a compile, but the
first frame waited for all of them (2 s on DX12), and the overlay compiled its pipeline the first time it was turned
on. Now (`metallic_render::pipelines`):

- **The first frame's pipelines are compiled at once on worker threads** (one per core but one, at most eight), and
  `Renderer::new` returns when they are done: the scene's pipelines without local lights for the format and MSAA
  count it draws with (5), and the post chain's passes it runs (the finish pass's variant and bloom's three; MLAA's
  or the upscale pass's when they run). The sun's and the lights' shadow pipelines (4, small) are made on the calling
  thread meanwhile, and the scene's lit pipelines (6) for the same target start right behind them.
- **A scene's first frame waits for what it draws with**: the first `Renderer::render_scene` after `Renderer::new`
  (or `Renderer::begin_scene`, which `SceneRenderer` calls for a newly loaded place) waits until the pipelines of the
  settings asked for (a change still compiling included, such as the preset's MSAA) are compiled and, when the scene
  has local lights, the lit ones too, moved to the front of the queue. The first image a player sees is the one every
  later frame shows: no unlit flash at a night start, no first frames without the preset's anti-aliasing. The Player
  presents its first frame that much later.
- **Every other permutation follows in the background**: the overlay's, every post
  effect and quality (the finish pass's other variants, bloom's other filters, MLAA) and the upscale pass, then the
  scene's 11 at every other MSAA count up to 8. A compile may wait for another whose shader stages it reuses (the
  depth prepass and the opaque pass after it wait for the opaque pass; another MSAA count for the first one's), so
  DX12, which caches each shader stage it compiles inside the device, compiles each stage once rather than on several
  workers at the same moment: that halved the CPU the background compiles took (4.3 s → 2.6 s summed over the workers
  on DX12) and shortened them (800 → 680 ms).
- **Later frames never wait.** They take the compiles that finished (one lock, nothing allocated; nothing at all once
  everything is compiled) and draw with what exists. A settings change whose pipelines are still compiling (an MSAA
  count, MLAA, an effect quality, the post chain off, a render scale without it) is kept waiting while frames draw
  with the settings before it, and takes effect in the first frame after its pipelines are; the overlay is drawn once
  its pipeline exists. A light made mid-game while the lit pipelines still compile (only possible in the first
  second or so of a scene that started without lights) moves them to the front of the queue, is not drawn until they
  exist, and then fades in over the lights' fade time (0.3 s) instead of popping on. `Renderer::wait_for_pipelines`
  waits for everything, for tests, tools and loading screens; `Renderer::pause_pipeline_compiles` holds the
  background compiles back (a compile a frame needs first still runs).
- **OpenGL compiles inline**, on its context's thread, all of it before `Renderer::new` returns, as before (so do
  WebAssembly and a machine with one core).
- **Stats:** `Renderer::pipeline_stats` (compiled, pending, failed, workers, the first frame's and the last compile's
  time, the compile CPU summed, a switch waiting, the pipeline cache's size at start), shown on the overlay's
  `PIPELINES` line and in the stress JSON's `graphics.pipelines`.
- **The pipelines:** 71 were made in `Renderer::new` before (the scene's 11 for the output format and for each of the
  four MSAA counts, though the output format's are only drawn with the post chain off; 11 post, the upscale pass, 4
  shadow), and the overlay's on first use. Now 61: 9 for the first frame, 4 shadow ones beside them, 48 in the
  background; the output format's scene pipelines only when the post chain is turned off, in the background then.

**2026-11-04**, same machine (Ryzen 5 5500, 6 cores / 12 threads, RX 6650 XT, driver 32.0.21045.16010, Windows 11),
branch `86-no-hitches` against `main` at `805b283`, `cargo run --release -p metallic_render --example prewarm`
(offscreen 1280 × 720, the renderer's defaults). "First frame" is from the start of `Renderer::new` until the first
frame is done on the GPU; "everything" until the last background compile is (from when the compiles start, a few
milliseconds into `Renderer::new`). Before #86 both are the same moment. (The example draws no scene, so its first
frame waits for no lights; the lit pipelines compiling beside the first frame's cost it 10 to 20 ms on DX12 and save
about 100 ms on everything.) A driver keeps its compiled shaders on disk
per executable, so the cold rows run a fresh copy of the executable; warm rows are its second and third runs. Times
in ms.

| Backend | Driver cache | Before: first frame (everything) | After: first frame | After: everything |
| --- | --- | ---: | ---: | ---: |
| DX12, FXC (default) | cold | 2,880 | 245 | 782 |
| | warm | 2,071 | 167 / 179 | 578 / 599 |
| DX12, DXC (`dxcompiler.dll`) | cold | 1,214 | 131 | 285 |
| | warm | 606 / 614 | 73 / 73 | 150 / 150 |
| Vulkan | cold | 691 | 86 / 88 | 178 / 179 |
| | warm | 58 | 22 | 15 |
| OpenGL (inline) | cold | 667 | 710 | 700 |
| | warm | 75 / 76 | 80 / 79 | 71 / 71 |

- **DX12 is where the time went**: FXC turns the lit parts' shader into DXBC in about half a second a stage, on one
  thread, every run (nothing caches it across runs). The renderer returns after the unlit scene's stages
  (≈ 150 ms on one worker); the lit ones (≈ 400 to 530 ms each, side by side) are what a scene with lights waits for
  in its first frame, below.

**A lit scene's first frame** (`stress --scene many_lights --warm-up 0`, 1920 × 1080 in a window, DX12): the
report's `first_frame_ms`, from making the device until the first frame is presented, the device's own setup
(about 360 ms on DX12) included. "Unlit first frame" is this branch before the scene's first frame waited for its
lights (the first frames of a night start then drew without them for about half a second); "waits" is the branch
now. The renderer's defaults, and the High preset (MSAA 4, which the first frame now also waits for). Times in ms.

| Settings | Driver cache | Unlit first frame | Waits (first frame lit) |
| --- | --- | ---: | ---: |
| defaults, FXC | cold | 811 | 1,286 |
| | warm | 612 / 605 | 999 / 1,027 |
| High preset, FXC | cold | 713 | 1,111 |
| | warm | 632 / 638 | 1,018 / 1,048 |
| defaults, DXC | cold / warm | | 587 / 529 |

The wait is the lit stages' FXC compile, about 0.4 s, on the critical path; the lit pipelines start compiling with
the first frame's, so it is shorter than their compile after `Renderer::new` returned would make it. With DXC the
lit first frame comes sooner than FXC's unlit one did.
- **Vulkan's pipeline cache** (`ContextOptions::pipeline_cache_dir`; the Player keeps it in the user's cache folder,
  `pipelines/wgpu_pipeline_cache_vulkan_<vendor>_<device>.bin`, 136 KB here) is loaded at start and written by a
  compile worker whenever its queue runs dry, never in a frame. The file is checked (its length and an FNV-1a hash in
  a header of its own) before wgpu sees it, and wgpu starts empty on another driver. On this PC it changes nothing:
  a cold driver cache with the file warm took 77 ms to the first frame and 179 ms for everything, as without it.
  AMD's Windows driver keeps its own cache per executable and returns about 2.4 KB a pipeline (keys, not binaries)
  into the application's; drivers that store the binaries there (Android's, Mesa's) are what it is for.
- **DXC on DX12 is an option, not the default** (`ContextOptions::dx12_compiler`, or `WGPU_DX12_COMPILER=dxc`). It
  compiles 3 to 4 times as fast as FXC (above), but needs `dxcompiler.dll` (14.3 MB, DXC 1.8.2502 or newer; the
  Windows SDK's works alone, without `dxil.dll`) shipped beside the executable. wgpu's `static-dxc` feature links it
  instead, but downloads a prebuilt library from a third-party GitHub release while building (which neither
  `cargo deny` nor the lockfile covers) and has none for Windows on ARM. The default build stays self-contained with
  FXC, and since the first frame no longer waits for the lit shaders, FXC's cost is mostly off the critical path. The
  default was wgpu's `Auto` before (DXC if a `dxcompiler.dll` is found on the DLL search path, FXC otherwise); it is
  FXC now, so the shaders a machine compiles no longer depend on what happens to be on its `PATH`.
- **OpenGL** compiles as before, inline (its context's thread); 61 pipelines instead of 71 cost the same.

**No GPU waits in the frame** (the audit): the frame path waits for nothing. `context.rs` blocks on the adapter and
device requests at setup; the timestamp readback (`timing.rs`) maps asynchronously and collects frames later with
`PollType::Poll`; the GPU scene's buffer growth submits a copy and moves on; acquiring a swapchain image is the
presentation's own pacing and is kept out of the render CPU time. `cargo xtask check-layout` now fails on
`PollType::Wait`, `wait_indefinitely`, `block_on(` or `.map_async(` in `crates/render/src` or `crates/engine/src`
outside `#[cfg(test)]` code, but for the two uses above (`GPU_WAIT_ALLOWED` in `xtask/src/layout.rs`, each with why).
Tests, benches and examples may wait (they read frames back).

**Allocations** (`tests/no_alloc.rs`): every renderer there waits for its background compiles before it is measured,
and the default frame drawn while a switch waits for pipelines still compiling (paused, so they stay pending) is held
to the same wgpu calls by hand: 200 / 200 allocations on DX12, 211 / 211 on Vulkan (on OpenGL, which compiles inline,
the plain frame is measured: 286 / 286). Over 100 such frames the heap changes by at most 8.6 KB on DX12 and Vulkan
(inside the tests' 16 KB for frames in flight). The compiles themselves allocate on their workers.

**Toggling every effect** (the done-when): `stress --scene many_lights --toggle-effects 30 --warm-up 0` toggles the
next of `stress::TOGGLES` every 30 frames from the first frame on, while the background compiles still run: every
preset, every anti-aliasing (MSAA falling back to what the device has), every effect quality, lights, shadows and the
overlay off and on. The spike counter (overlay and `spikes` in the JSON) counts the frames over twice the frame
target after the first 2 s; the target is a 60 fps frame unless the app sets a frame cap (`--frame-target`).

| Backend, present | Frames (toggles) | Pending at most | Frame p50 / p99 | Spikes over 33.3 ms | Worst after 2 s |
| --- | ---: | ---: | ---: | ---: | ---: |
| DX12, uncapped | 6,000 (199) | 35 | 1.19 / 4.03 | 0 | 16.6 |
| DX12, vsync (120 Hz) | 1,500 (49) | 25 | 8.33 / 8.95 | 0 | 23.5 |
| OpenGL, uncapped | 6,000 (199) | 0 | 4.42 / 10.09 | 9 | 38.0 |

No toggle compiles anything on a frame, and no frame waits for the GPU. What is left is the GPU work of a switch to
Ultra: its first frame makes the 3 × 3072² cascades and the 4096² atlas and draws all of them again (render CPU about
1 ms). That is 16 ms uncapped and 23 to 27 ms with vsync on DX12 (2 spikes against a 120 Hz target's 16.7 ms, none
against 60 fps), and 37 ms on OpenGL (all 9 of its spikes, at High → Ultra); #152 spreads that redraw over frames.
So the done-when of #86 holds on DX12; on OpenGL it depends on #152. (#152 found the cost was making the textures, not
drawing them: see the next section.)
`METALLIC_TRACE_SPIKES=1` prints each spike and the toggle before it. Vulkan had no adapter for a window while the
monitor was off (#110), so its numbers are the offscreen ones above. On OpenGL, `no_alloc`'s heap-growth tests fail on
this PC on `main` too (#153); CI does not run them there.

The perf gate's workloads (scene sync and cull, no GPU) are untouched: nothing was added to the sync or cull paths.

### Settings switches: new textures off the frame (#152)

The last spike of the toggle run above was the first frame after a switch from High to Ultra. **It was not the
redraw.** Timed pass by pass (`METALLIC_TRACE_SWITCHES=1`, below), that frame's GPU work on DX12 was 3.6 to 3.7 ms
against Ultra's steady 3.6: redrawing every cascade and the far one's static map took 0.4 ms (0.06 + 0.15 + 0.03, and
0.16 to build the far map), and the atlas 0.01 ms (Ultra keeps High's atlas size; 2 tiles of lights it adds). Render
CPU was 1.0 to 1.2 ms. The other 12 to 13 ms of the 16 to 17 ms frame (uncapped) were spent **making the new
textures**, on the CPU, in the settings call before the frame: the 3 × 3072² cascades (108 MiB) and their static map
(36 MiB), and 8x MSAA's colour and depth (63 MiB each). Overriding Ultra's shadows to High's size alone, or its MSAA
to 4x alone, left the spike as it was; both together removed it (3.8 ms).

What a new texture costs, from `cargo run --release -p metallic_render --example texture_cost` (dev PC, Ryzen 5 5500,
RX 6650 XT, Windows 11):

| | DX12 / Vulkan | OpenGL |
| --- | --- | --- |
| Making it, room in a memory block | 0.05 ms | 0.2 ms |
| Making it, a new block | about 0.12 ms a MiB, on the calling thread: 15 ms for 128 MiB, 25 ms for 256 | — |
| First clear (the first use) | 0.3 to 1.8 ms (108 MiB), then 0.1 to 0.4 | about 0.11 ms a MiB (12 ms for 108 MiB), then 0.2 to 0.6 |
| Freeing it | nothing in a block; 2.7 ms for a block of its own (144 MiB) | 0.05 ms |

The OS clears every new device memory block (on DX12 and Vulkan when it is made; on OpenGL the driver at its first
use). wgpu suballocates from blocks of 128 MiB growing to 256 (`MemoryHints::Performance`), so whichever texture
needs a new block pays for all of it. wgpu's own lazy zero-initialisation cost nothing here: every new texture is
first used by a pass that clears it (`LoadOp::Clear`), but for the atlas, whose size Ultra does not change.

**What changed** (`metallic_render::textures`, ADR 0008 §9):

- **One texture a frame, on a worker, while the frame is presented.** The large textures a settings change needs
  (16 MiB or more: the shadow maps, the lights' atlas, the scene's depth, MSAA colour and scene colour) are queued;
  right after each frame's submit a worker thread makes the next one, while the frame's present waits for the
  display. A frame that writes a buffer while the worker is still making its texture waits for it (wgpu holds its
  allocator's lock while the OS clears a block: the example's worker blocked such frames for the full 16 ms, frames
  without writes for none), so only one is made at a time, and the present's wait hides most of it. Making them
  inline at the same moment did not: a 13 ms texture made right before the present gave 24 and 52 ms frames; on the
  worker, 10 ms.
- **Each new texture is cleared in the frame it arrives**, before that frame's passes: its first use, which is where
  OpenGL pays, falls in a frame of its own too (OpenGL makes them inline: one context thread, and making them is
  cheap there).
- **Frames keep drawing with what the change replaces until all of a system's textures exist**, then that system
  switches at once: the sun's shadows (every cascade drawn again into the new maps, 0.4 ms), the atlas (every tile
  drawn again; lights keep their tiles), the scene's attachments (the anti-aliasing, scale or post chain of the
  switch, which waits for them as it already waited for its pipelines, #86). There is no seam: nothing is drawn half
  from old maps and half from new. A light's first shadow mid-game waits for the atlas the same way: drawn unshadowed
  for a frame or two, then its shadow fades in.
- **Memory blocks of 128 MiB** (`RenderContext` asks wgpu for them; its default grows to 256): a new block is made on
  the worker like any texture, and 15 ms of it hides behind the present, 25 to 30 ms does not. With wgpu's default
  blocks one switch to Ultra in three runs took 19.7 ms, and three vsync toggle runs had 0, 3 and 0 spikes (worst
  24.4 ms); with 128 MiB blocks 0, 0 and 0 (worst 13.6). Blocks of 8 to 32 MiB give each large texture a block of its
  own (in proportion to its size: the same switch's worst frame 11.3 to 12.3 ms), but freeing such a block costs the
  frame's thread 1 to 3 ms.
- **Not spread:** redrawing the cascades and the atlas tiles stays in one frame (0.4 ms at Ultra, 0.05 ms for 30
  tiles at High). A per-frame tile budget would have saved nothing measurable.
- **The first frame of a scene** still makes everything at once (before it is drawn, every texture is made inline),
  so it shows its shadows and lights; `Renderer::wait_for_pipelines` makes what is pending at once too.

**High → Ultra on `many_lights`** (the done-when; `stress --scene many_lights --toggle-effects 30 --warm-up 0` with
`METALLIC_TRACE_SWITCHES=1`, the switches to Ultra at frames 540 and 1140 (and every 600 frames after); the worst
of the ten frames after each, in ms). Before is `main` at `f0fb441`, run interleaved with the branch:

| Backend, present | Before | After |
| --- | ---: | ---: |
| DX12, vsync (120 Hz), 1,500 frames | 22.2 to 27.5 (7 runs, 14 switches) | 11.8 to 13.7 (9 runs, 18 switches) |
| DX12, uncapped, 6,000 frames | 16.8 to 18.1 (10 switches) | 7.4 to 9.0 (9 switches), 21.4 the first |
| OpenGL, uncapped, 6,000 frames | 40.9 to 41.9 (10 switches) | 27.4 to 29.7 (10 switches) |

Under vsync a frame that waits X ms on the CPU took about 9.2 + X ms here (measured with a busy wait at the switch: 6
ms gave 15.4, 8 gave 17.2), so twice the 120 Hz frame leaves about 7 ms for a switch: less than a new block for one
of Ultra's textures costs. Uncapped, the run's first switch to Ultra still took 21.4 ms (under 33.3); the later ones
under 9.

The switch takes **5 frames** on both backends: the cascades' maps (108 MiB), the static map (36 MiB), the MSAA depth
and the MSAA colour (63 MiB each) arrive one a frame (the scene colour, 8 MiB, is made at once; the atlas keeps its
size); the shadows switch in the third frame, the anti-aliasing in the fifth. At 120 Hz that is 42 ms. **Memory** (wgpu's
allocator report, DX12): High holds 222 MiB of textures and buffers and Ultra 368; during the switch the peak is 452
MiB, for 4 frames: Ultra's new textures beside High's, which wgpu frees once the GPU is done with them (before #152
both existed too, for the 2 to 3 frames the GPU took to finish with the old ones). The blocks reserved grow from 512 to
896 MiB.

The whole toggle run, against the run of #86 above (spikes are frames over twice the target after the first 2 s;
counted without `METALLIC_TRACE_SWITCHES`, whose own writes to stderr can cost a frame now and then):

| Backend, present | Frames (toggles) | Frame p50 / p99 before → after | Spikes before → after | Worst after 2 s before → after |
| --- | ---: | ---: | ---: | ---: |
| DX12, vsync (120 Hz, 16.7 ms) | 1,500 (49) | 8.33 / 16.8–17.8 → 8.33 / 9.6–10.6 | 2, 3 → 0, 0 | 24.4, 22.9 → 14.9, 10.7 |
| DX12, uncapped (33.3 ms) | 6,000 (199) | 1.48 / 4.09 → 1.48–1.50 / 4.38–4.43 | 0 → 0 (4 runs) | 16.8 → 8.6 to 14.5 |
| OpenGL, uncapped (33.3 ms) | 6,000 (199) | 4.97 / 13.28 → 4.94 / 13.74 | 10 → 0 | 39.3 → 24.5 |

No toggle spikes against a 120 Hz frame on DX12 any more, and none against 60 fps on OpenGL. The uncapped p99 grows by
0.3 ms: a switch now spends a few frames on its textures (each a millisecond or two of clearing and of the worker
holding wgpu's allocator) where one frame took it all before. On OpenGL the High switch's worst frame moved a little
(21.6 to 23.8 ms against 17.5 to 24.9): its frames clear the new textures one by one where one frame did it all,
under 33.3 ms either way.

- **Allocations** (`tests/no_alloc.rs`): a frame with nothing being made touches only an atomic flag (and a waiting
  switch's pipeline check), so every frame there makes the same wgpu calls as before, and the heap-growth tests pass
  as before; textures made off the frame allocate on the worker, and only while a switch makes them.
- **Tests**: `tests/shadows.rs` switches the map size after the first frame and checks the old maps are drawn until
  the new ones exist (two textures, at least two frames), every cascade is drawn into them, and the image is the one a
  renderer made with that size draws; `tests/pipelines.rs` does the same with an MSAA switch's attachments, and
  `tests/lights.rs` with the atlas a light's first shadow makes mid-game (drawn without the shadow until it exists).
  The scene's first frame still makes everything at once: the #121 and #86 tests and the reference images are
  unchanged.
- The perf gate's workloads (scene sync and cull) are untouched.

### Merged passes (#97)

Consecutive passes that draw into the same attachments, none sampling what another wrote, are recorded as one render
pass (`metallic_render::graph`). After #85 the frame merged the scene passes (opaque, sky, transparent) only at Low
and on tile-based GPUs, and lost their GPU timings when it did; everything else was one render pass per logical pass.
Now:

- **UI is drawn in the finish pass** (or the upscale pass without the post chain), at every setting: both draw over
  the whole window with no depth, so UI's own pass only stored the window and loaded it again. Without either (the
  scene straight into the window) UI keeps its pass: the scene's has depth or is multisampled.
- **Every preset merges the scene passes**, desktop included (`Quality::merge_scene_passes`, and `SceneRenderer`'s
  default). Merged or apart, the image is the same to the bit (below), merging never cost GPU time on DX12, Vulkan or
  GL here, and it saves two render passes' CPU.
- **The logical passes are still timed one by one** where the device writes timestamps inside a pass
  (`TIMESTAMP_QUERY_INSIDE_PASSES`: DX12, Vulkan and GL here): a timestamp between two passes ends one and starts the
  next, so the overlay and the stress JSON's `gpu_ms.passes` keep `opaque`, `sky`, `transparent` and `ui`. On a
  tile-based GPU (which draws a pass tile by tile, each tile running all of its draws, so a timestamp between draws
  means nothing) and without the feature, the merged pass is reported as `scene` (UI's time with `post`).
  `graphics.post.passes` lists the render passes as planned (`scene`, …, `post`).
- **The depth prepass is never merged**: its pipelines have no colour target, so they cannot draw in a pass with one.
  Merging a scene with a prepass failed wgpu's validation before (nothing had drawn that layout); now the prepass
  stays a depth-only pass and the merged pass after it only tests its depth. No preset draws a prepass.
- `stress --split-scene` (without `--preset`) draws the scene passes apart, to measure what merging saves;
  `--merge-scene` is the default.

**The `mobile` stress scene** (the done-when): 2026-11-04, same machine (Ryzen 5 5500, RX 6650 XT, driver
32.0.21045.16010, Windows 11), `AutoNoVsync`, 1,000 frames after 120 of warm-up, seed 1, `--preset <tier>`, `main` at
`e2b149a` (before) and this branch, run interleaved, two rounds each (shown as round 1 / round 2). Render passes per
frame and full-screen passes are the JSON's `graphics.post`; times are p50 in ms.

| Backend | Preset | Render passes before → after | Full-screen before → after | Render CPU before → after | GPU before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| DX12 | Low | 11 → 10 | 4 → 3 | 0.324 / 0.366 → 0.319 / 0.342 | 0.198 / 0.211 → 0.198 / 0.199 |
| | Medium | 15 → 12 | 6 → 3 | 0.457 / 0.470 → 0.423 / 0.447 | 0.397 / 0.397 → 0.398 / 0.398 |
| | High | 14 → 11 | 5 → 2 | 0.528 / 0.544 → 0.481 / 0.504 | 0.462 / 0.463 → 0.464 / 0.464 |
| | Ultra | 14 → 11 | 5 → 2 | 0.527 / 0.477 → 0.450 / 0.474 | 0.800 / 0.801 → 0.800 / 0.801 |
| GL | Low | 11 → 10 | 4 → 3 | 1.697 / 1.715 → 1.716 / 1.703 | 0.621 / 0.616 → 0.621 / 0.615 |
| | Medium | 15 → 12 | 6 → 3 | 2.498 / 2.512 → 2.452 / 2.504 | 1.266 / 1.266 → 1.266 / 1.267 |
| | High | 14 → 11 | 5 → 2 | 2.577 / 2.521 → 2.482 / 2.505 | 1.266 / 1.267 → 1.266 / 1.268 |
| | Ultra | 14 → 11 | 5 → 2 | 3.239 / 3.253 → 3.205 / 3.241 | 1.961 / 1.961 → 1.961 / 1.960 |

The planned passes go from `opaque sky transparent [mlaa] bloom post ui` to `scene [mlaa] bloom post` (Low:
`scene mlaa bloom post ui` to `scene mlaa bloom post`); bloom's chain is the rest of the render passes. The scene
passes' GPU time is the same merged as apart (DX12 High: opaque 0.105 / sky 0.008 / transparent 0.031 apart, 0.106 /
0.008 / 0.031 merged, timed inside the pass). The GPU frame does not move on this desktop GPU, whose pass boundaries
cost nothing measurable; on a tile-based GPU each removed boundary is a store and a load of the window (UI) or of the
scene's colour and depth (the scene passes at Medium and above), which is what #97 is for. Render CPU drops by 0.02 to
0.05 ms at Medium and above on DX12 (Ultra's two rounds 0.08 and 0.003: its own noise is that large), about 15 µs a
render pass; GL's CPU varies by more than that from run to run.
Vulkan had no adapter for a window while the monitor was off (#110), so it is measured offscreen below.

**Offscreen, every backend** (`cargo bench -p metallic_render --bench scene_passes`: a 10,000-part grid with 1%
see-through blocks under the sun, 1280 × 720, each preset's anti-aliasing, effects and scale, the scene passes apart
and merged; p50 over three rounds of 400 frames). This branch (UI in the finish pass either way); in ms:

| Backend | Preset | Render CPU apart → merged | GPU apart → merged | Scene passes' GPU apart → merged |
| --- | --- | ---: | ---: | ---: |
| DX12 | Low | 0.202 → 0.186 | 0.182 → 0.183 | 0.059 → 0.059 |
| | Medium | 0.220 → 0.204 | 0.270 → 0.270 | 0.073 → 0.073 |
| | High | 0.218 → 0.203 | 0.250 → 0.251 | 0.117 → 0.118 |
| | Ultra | 0.221 → 0.203 | 0.287 → 0.289 | 0.154 → 0.155 |
| Vulkan | Low | 0.159 → 0.147 | 0.179 → 0.173 | 0.065 → 0.059 |
| | Medium | 0.175 → 0.162 | 0.250 → 0.245 | 0.079 → 0.073 |
| | High | 0.177 → 0.162 | 0.242 → 0.235 | 0.135 → 0.128 |
| | Ultra | 0.178 → 0.162 | 0.270 → 0.260 | 0.162 → 0.153 |
| GL | Low | 1.367 → 1.311 | 0.594 → 0.595 | 0.199 → 0.198 |
| | Medium | 1.759 → 1.723 | 0.904 → 0.901 | 0.255 → 0.255 |
| | High | 1.579 → 1.518 | 0.735 → 0.734 | 0.355 → 0.355 |
| | Ultra | 1.668 → 1.598 | 0.816 → 0.815 | 0.437 → 0.436 |

Merging saves 12 to 18 µs of render CPU on DX12 and Vulkan and 36 to 70 µs on GL, and 5 to 10 µs of GPU on Vulkan
(its pass boundaries, MSAA's stores and loads among them, are not free there); nothing got slower. UI in the finish
pass saves another 2 to 8 µs of CPU on DX12 and Vulkan and 16 to 41 µs on GL (`main`'s bench against this branch's,
both apart: 12 render passes instead of 13 at Low, 14 instead of 15 at Medium, 13 instead of 14 at High and Ultra;
merged, two fewer). That decided it: merging is the default everywhere rather than only on tilers and at Low.

- **The same image** (`tests/post.rs`, `merged_scene_passes_draw_the_same_image`): the demo scene with the overlay on,
  at each preset's layout (no anti-aliasing, MLAA, MLAA at 75%, MSAA, MSAA at 75%, a depth prepass with and without
  MSAA, and without the post chain plain, with MSAA and at 75%), drawn apart and merged: no pixel differs, on Vulkan,
  DX12 and GL here; CI runs it on lavapipe, WARP and llvmpipe (GL).
- **Timings** (`tests/timings.rs`): a merged frame reports `depth_prepass, opaque, sky, transparent, bloom, post, ui`
  where timestamps can be written inside passes, and `depth_prepass, scene, bloom, post` otherwise.
- **Allocations** (`tests/no_alloc.rs`): the hand-written frames draw UI in the finish pass too, and every frame
  still makes exactly the wgpu calls written out by hand: the default frame 199 / 199 on Vulkan, 191 / 191 on DX12,
  272 / 272 on GL; the merged scene 178, 173 and 247. The timestamps written inside a pass
  allocate nothing of ours, and the heap-growth tests pass (they run with timing on).
- The perf gate's workloads (scene sync and cull) are untouched.

### Transparent draws (#106)

The transparent pass sorts its see-through instances back to front per instance (#77). It used to draw one
instanced draw per run of consecutive instances with the same mesh and level of detail, so the draw count grew with
how the shapes interleave in depth: 4,648 draws for `see_through_10k` at Medium (more than at #78, as #80's levels of
detail split the balls' runs further). Now, on the storage path, **every see-through instance is drawn in one draw**
(`parts/transparent.rs`), whatever their number and order:

- **Pulled vertices.** The pool keeps an unindexed copy of every mesh at every level (each index's vertex in turn,
  32 bytes a vertex, 142 KB for the built-in meshes). The sorted list holds a 12-byte record per instance (its slot,
  where its vertices start in the draw, and its mesh's place in the unindexed copy with its level) after a chunk
  table that names, for every 64 vertices of the draw, the record the chunk starts in. One non-indexed draw covers
  the sum of the instances' index counts; `vs_pulled` (instance_storage.wgsl) finds a vertex's record from its
  chunk's in a step or two (every mesh has at least a triangle), then reads the instance and the mesh vertex. A
  draw's triangles are blended in order, so the order stays exact, and the image is the same as the runs': the new
  `tests/parts.rs` test draws 194 interleaved see-through blocks and balls in 1 draw on the storage path and in 142
  on the fallback, and no pixel differs (Vulkan, DX12 and GL here).
- **Draw count: 1** on the storage path (0 without see-through parts). The vertex shader reads four storage buffers
  (the two regions, the records and the unindexed meshes); a storage path with fewer (none here, and WebGPU
  guarantees eight) draws runs instead.
- **The fallback** (the vertex-buffer path, GLES, whose vertex shaders read no storage buffers) keeps one draw per
  run: 4,648 at Medium and 4,647 at High on `see_through_10k` (DX12 with the stress example's new
  `--vertex-instances`, the vertex-buffer path on any backend), 1.52 and 1.69 ms of render CPU p50. Its vertex shader
  could pull from textures instead, which GLES 3.0 reads in vertex shaders; that is left for when a GLES device needs
  it.
- **The cost moves to the GPU's vertex work.** A non-indexed draw shades each vertex once per triangle it is in,
  with no post-transform cache: for these meshes about three times the vertex shader runs of the indexed draws
  (884,000 transparent triangles at Medium are 2.65 M vertex shader runs). That is the GPU time below; the fragment
  work is the same.

**Measured** 2026-11-04, same machine (Ryzen 5 5500, RX 6650 XT, driver 32.0.21045.16010, Windows 11), **DX12**,
`AutoNoVsync`, 1,000 frames after 120 of warm-up, seed 1, `--preset <tier>`, `main` at `bc58cbf` against this
branch, run interleaved, two rounds each (shown as round 1 / round 2). Times are p50 in ms; "transparent" is the
pass's GPU timestamp.

| Scene | Preset | Transparent draws | Render CPU before → after | GPU before → after | Transparent pass before → after | Frame before → after |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `see_through_10k` | Medium | 4,648 → 1 | 1.512 / 1.443 → 0.938 / 0.920 | 0.963 / 0.962 → 1.119 / 1.130 | 0.533 / 0.533 → 0.676 / 0.684 | 1.683 / 1.546 → 1.326 / 1.296 |
| | High | 4,647 → 1 | 1.547 / 1.468 → 1.072 / 1.020 | 1.273 / 1.277 → 1.474 / 1.483 | 0.893 / 0.902 → 1.095 / 1.107 | 1.665 / 1.579 → 1.751 / 1.682 |
| `parts_100k` | Medium | 0 → 0 | 0.859 / 0.859 → 0.836 / 0.868 | 0.771 / 0.766 → 0.772 / 0.766 | | 1.046 / 1.042 → 1.019 / 1.058 |
| | High | 0 → 0 | 0.958 / 1.003 → 0.956 / 1.004 | 0.825 / 0.823 → 0.825 / 0.825 | | 1.139 / 1.203 → 1.137 / 1.206 |

- **Render CPU** falls by 0.45 to 0.55 ms on `see_through_10k` (about a third): recording 4,648 draws cost more than
  writing the records and the chunk table (10,000 records and 41,000 chunk entries, about 286 KB written straight
  into wgpu's staging memory). The transparent list's update takes about 0.42 ms of what is left, 0.2 ms of it the
  sort of 10,000 instances, which is unchanged. p99 falls too (Medium 2.60 / 4.20 → 1.91 / 1.63).
- **GPU** grows by 0.15 ms at Medium and 0.2 ms at High on this worst case, all in the transparent pass: the vertex
  work above, on 0.9 and 1.2 M see-through triangles. At Medium the frame is 0.3 ms faster; at High, where this
  GPU-heavy scene's frame follows the GPU more, 0.05 to 0.1 ms slower. A scene with a few dozen see-through parts
  pulls a few thousand more vertices, which is nothing measurable.
- **`parts_100k`** has no see-through parts: the same within noise.
- **Vulkan** (one round, same settings) shows the same CPU drop on `see_through_10k` (Medium 1.313 → 1.002, High
  1.377 → 1.083 ms), but its GPU times on this PC moved by up to 20% between runs of the unchanged `parts_100k` (AMD's
  Vulkan driver under a window), so they are left out.

**Alternatives measured** (DX12, `see_through_10k` at Medium, the same session; p50 in ms, rounds as above):

| Approach | Draws (API calls / GPU draws) | Render CPU | Transparent pass GPU |
| --- | ---: | ---: | ---: |
| `main`: one draw per run | 4,648 / 4,648 | 1.318 / 1.296 | 0.539 / 0.544 |
| `multi_draw_indexed_indirect` of the runs (one indirect argument per run) | 1 / 4,648 | 0.979 / 0.964 | 0.659 / 0.663 |
| Pulled vertices, binary search over the records' starts | 1 / 1 | 0.835 | 0.978 |
| Pulled vertices, chunk table of 16 vertices | 1 / 1 | 1.227 | 0.667 |
| **Pulled vertices, chunk table of 64 vertices** (this branch) | 1 / 1 | 0.875 / 0.904 | 0.679 / 0.678 |
| Pulled vertices, chunk table of 128 vertices | 1 / 1 | 0.867 / 0.862 | 0.686 / 0.689 |

- **Multi-draw indirect** makes one API call, but the GPU still runs 4,648 draws, and wgpu's validation of the
  indirect arguments (on in release builds too) and DX12's `ExecuteIndirect` cost as much GPU time as the pulled
  vertices do, while saving less CPU (an argument is written per run, and the sort stays). It also needs
  `INDIRECT_FIRST_INSTANCE`, which some mobile Vulkan drivers lack. ADR 0008 §4 asks for a draw count that depends on
  the mesh kinds and shading models, never on the parts; read as the draws the GPU runs, not the calls the CPU makes,
  only the pulled draw meets it.
- **A binary search** for the record (14 dependent reads a vertex for 10,000 records) made the pass 0.44 ms slower;
  the chunk table brings that to a step or two. Smaller chunks barely help the GPU and cost CPU (four times the
  entries); larger ones save little CPU. 64 was kept.
- **Weighted blended OIT** was not needed: one sorted draw keeps the exact order.
- **Allocations** (`tests/no_alloc.rs`): the hand-written parts frames draw the see-through parts as the renderer does
  (the records written through `write_buffer_with`, one pulled draw), and every frame still makes exactly the wgpu
  calls written out by hand: the parts frame 233 / 233 on Vulkan, 225 / 225 on DX12 and 313 / 313 on GL, for both
  workloads (80 and 750 see-through parts); the shadowed frame 277 / 277 on DX12 and 387 / 387 on GL. The heap-growth
  tests pass.
- The perf gate's workloads (scene sync and cull) are untouched, and so is the sort.

### Vulkan in a window (#110)

From #78 to #97, `WGPU_BACKEND=vulkan` could not open a window on the dev PC, though offscreen Vulkan worked. The
renderer found "no graphics adapter (backends tried: Backends(VULKAN))" because no Vulkan adapter was compatible with
the window's surface. wgpu-hal logged why: `get_physical_device_surface_capabilities: An unknown error has occurred`
(`VK_ERROR_UNKNOWN`).

**The cause: no monitor was connected.** The dev PC's only monitor (3440 × 1440, `DISPLAY\MXM3400`) leaves the bus
when it is switched off at its button: Windows logs a surprise removal (`Kernel-PnP` event 1010 in
`Microsoft-Windows-Kernel-PnP/Device Management`) and the GPU has no output left. AMD's Vulkan driver here
(32.0.21045.16010, Adrenalin 26.10.1) then fails the surface-capabilities query for any window. DX12's adapter takes
the window either way. The evidence:

- **The failures fell while the monitor was gone.** It was removed at 06:22 on 2026-11-04 and arrived again at 16:11
  (EST; `DEVPKEY_Device_LastArrivalDate`). #86's branch, which found no Vulkan adapter for a window, was committed
  from 08:49 to 09:36, inside that gap. #106's windowed Vulkan round (merged at 18:11) and this section's (18:15 to
  18:30) came after it, with the same driver, boot, layers and code.
- **#78's failures fit too.** #78's branch was committed from 06:36 to 06:58 on 11-03, #110 was filed at 06:32 and
  the Player's log is from 06:48. The monitor had been removed at 02:24 and was not removed again until 10:24, so it
  came back somewhere in between; its arrival time then is not logged.
- **The display being off is not the cause.** Windows had turned the display off for idleness (`Kernel-Power` event
  566, `MonitorReason` 12) with the monitor still connected. A minute later, windowed Vulkan worked:
  `stress --scene parts_10k` drew 200 frames, and the probe described below found the surface supported.
- **Nothing else is wrong.** These were checked with the monitor connected:
  - The loader offers `VK_KHR_surface` and `VK_KHR_win32_surface`, and `vulkaninfo` presents to its own window (8
    formats, 3 present modes).
  - A throwaway winit + wgpu 30 probe sees the RX 6650 XT with `is_surface_supported = true` and full capabilities.
    That holds for every `InstanceFlags` tried (empty, the release default, debugging), for window sizes from 720p to
    4K, for a window placed off every monitor, and in debug and release builds.
  - `RenderContext::new_with_window` creates the surface on the instance it asks for adapters.
  - The implicit layers are Steam's overlay and Fossilize (`HKLM\SOFTWARE\Khronos\Vulkan\ImplicitLayers`) and AMD's
    switchable-graphics layer. The Steam layers load only under `ENABLE_VK_LAYER_VALVE_steam_*_1=1`, which Steam sets
    for its games. `vulkaninfo` loads the same set and works.

The failure itself has not been reproduced on purpose, since that needs the monitor switched off at the PC.

**The workaround: leave the monitor switched on (asleep is fine) for a windowed Vulkan run.** An agent measuring
while it is off should measure Vulkan offscreen (`cargo bench -p metallic_render`) or wait. The engine cannot work
around it, because the driver refuses every window until a monitor is back. `new_with_window` now says what happened:
when adapters exist but none can present to the window, it returns `RenderError::NoPresentableAdapter`, which names
them and the likely cause:

```text
stress: the renderer could not attach to the window: no graphics adapter can present to this window (backends tried:
Backends(VULKAN); adapters found: AMD Radeon RX 6650 XT (vulkan)): the driver rejected the window's surface. AMD's
Vulkan driver on Windows does this while no monitor is connected, for example one switched off at its button (#110,
docs/perf.md)
```

**Vulkan against DX12 in a window**, on the same build: `main` at `ea9ef58`, the renderer's defaults, `AutoNoVsync`,
1,000 frames after 120 of warm-up, seed 1, 100% scaling. Each figure is the median of 3 rounds, with each scene's
backends run back to back. Times are p50 in ms, p99 in brackets:

| Scene | Size | Backend | Frame | Render CPU | GPU | Opaque pass |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| `parts_100k` | 1920 × 1080 | DX12 | 0.947 (2.045) | 0.788 (1.683) | 0.719 | 0.242 |
| | | Vulkan | 3.509 (3.878) | 0.937 (1.161) | 2.074 | 0.824 |
| `many_lights` | 1920 × 1080 | DX12 | 2.342 (2.566) | 0.712 (0.876) | 2.197 | 1.778 |
| | | Vulkan | 3.073 (3.344) | 0.631 (0.792) | 1.901 | 1.516 |
| `mobile` | 1280 × 720 | DX12 | 0.567 (1.069) | 0.461 (0.829) | 0.413 | 0.093 |
| | | Vulkan | 1.705 (2.278) | 0.361 (0.512) | 0.807 | 0.191 |

The opaque pass is from the third round alone. The rounds of each scene and backend agree within 0.07 ms for the p50
frame (`mobile` on Vulkan within 0.13 ms).

- **On Vulkan the frame is 0.9 to 1.4 ms longer than its GPU time;** on DX12 it is 0.15 to 0.25 ms longer. Acquiring
  and presenting the image probably cost that much here.
- **The GPU time is 2.9 times DX12's on `parts_100k` and 2 times on `mobile`**, mostly in the opaque pass, yet 13%
  lower on `many_lights`. Render CPU is 0.08 to 0.1 ms lower on Vulkan except on `parts_100k`.
- Both gaps are #164.

### Directional lights as extra suns (#123)

`DirectionalLight`s are drawn beside the sun from `Lighting` as **extra suns**
([`metallic_render::lights`](../crates/render/src/lights/mod.rs), "Directional lights: extra suns"): each adds
Lambert diffuse and GGX specular from its forward direction, unshadowed.
The presets draw the brightest 1 (Low), 2 (Medium), 4 (High, Ultra), ranked by brightness × the colour's luminance;
which ones does not depend on the camera, so nothing fades. Only the sun casts directional shadows (a
`DirectionalLight`'s `Shadows` is ignored; cascades for them are #166). The stress example's `--directional-lights
<n>` adds that many to a scene (`StressScene::add_directional_lights`: different directions and tints, `Brightness`
0.6 to 1) and `--directional-budget <n>` sets how many are drawn; the JSON's `lights` reports
`directional_shining` and `directional_drawn`.

**No cost when unused.** The extra suns reach the shaders in the frame uniform (704 → 848 bytes, written only when it
changes, as before), and the part pipelines are made with and without them (`EXTRA_SUNS`, a pipeline-overridable
constant, as `LOCAL_SHADOWS` is for #83): a frame without an extra sun draws with exactly the pipelines it drew with
before. The other way was measured first and rejected: one set of pipelines looping over the frame's count of extra
suns, which is 0 without any. On Vulkan that loop of no iteration cost `parts_100k`'s opaque pass 8% (the pass over the
sky pass, which clocks the GPU: 10.5 to 10.7 against 9.8 on `main`, four interleaved rounds; with the call taken out
of the same build, 9.8) and `many_lights`' 1.5% (1.536 to 1.544 ms against 1.513 to 1.518): the registers it holds
cost occupancy even when it does not run, as #83 found for the local lights' loop. The variants with extra suns are
compiled only once a frame first draws one (then in the background, at every MSAA count; on OpenGL, whose compiles
run inline, up front with the rest), because on DX12 they would more than double the background compiles: 9 more
part pipelines per MSAA count, 57 → 93 pipelines, 3.2 → 7.5 s of compile CPU on the workers and 0.7 → 1.3 s until the
last is done (`parts_10k`, DX12, measured twice), which on `parts_100k` showed as a render CPU p99 of 3 ms instead of
1.4 for the first second while they compiled. A scene's first frame waits for the ones it draws with (#86): with a
`DirectionalLight` it took 850 ms instead of 660 on DX12.

**2026-11-04**, Ryzen 5 5500, RX 6650 XT (Adrenalin 26.10.1), Windows 11, **DX12**, `AutoNoVsync`, 1,000 frames,
seed 1, the renderer's defaults (Medium's lights: 2 extra suns; `--directional-budget 4` for 4). Before: `main` at
`668b3f9`. After: branch `123-directional-lights`, without (`none`) and with `--directional-lights 1` and `4`. Times
are p50 in ms; the pass is the opaque pass (`see_through_10k`: the transparent pass, which draws all of it).
`parts_100k` and `mobile` after 2,000 frames of warm-up (so the extra suns' background compiles are done), two
interleaved rounds agreeing within 0.03 ms (the passes within 0.002); `many_lights` and `see_through_10k` after 120,
two rounds agreeing within 0.06 ms (passes within 0.01).

| Scene | Extra suns | Frame | Render CPU | GPU | Pass |
| --- | --- | ---: | ---: | ---: | ---: |
| `parts_100k` | `main` | 0.939 | 0.783 | 0.723 | 0.243 |
| | none | 0.930 | 0.774 | 0.723 | 0.243 |
| | 1 | 0.934 | 0.773 | 0.759 | 0.279 |
| | 4 | 0.978 | 0.820 | 0.818 | 0.330 |
| `mobile` (1280 × 720) | `main` | 0.526 | 0.427 | 0.418 | 0.094 |
| | none | 0.533 | 0.434 | 0.418 | 0.094 |
| | 1 | 0.542 | 0.427 | 0.437 | 0.111 |
| | 4 | 0.559 | 0.418 | 0.461 | 0.135 |
| `many_lights` | `main` | 2.349 | 0.725 | 2.202 | 1.777 |
| | none | 2.411 | 0.762 | 2.207 | 1.767 |
| | 1 | 2.312 | 0.760 | 2.120 | 1.676 |
| | 4 | 2.427 | 0.756 | 2.231 | 1.785 |
| `see_through_10k` | `main` | 1.306 | 0.844 | 1.143 | 0.796 |
| | none | 1.307 | 0.848 | 1.144 | 0.797 |
| | 1 | 1.449 | 0.866 | 1.280 | 0.929 |
| | 4 | 1.698 | 0.911 | 1.516 | 1.151 |

- **Without a `DirectionalLight` nothing changed**: every pass, the GPU frame and render CPU measure as on `main`
  within run-to-run noise, on every scene.
- **One extra sun** costs 0.036 ms of opaque pass on `parts_100k` at 1080p (15% of the pass, about the sun's own
  shading) and 0.017 ms on `mobile` at 720p, and 0.13 ms in `see_through_10k`, whose see-through layers each shade it.
  **Four** cost 0.087 and 0.041 ms (0.36 ms in `see_through_10k`).
- **`many_lights`** measured faster with one extra sun than without (1.68 against 1.77 ms, in both rounds) and the same
  with four: FXC compiles each variant on its own, and the one with extra suns came out cheaper for the local lights'
  loop. On Vulkan (the opaque pass over the sky pass, which there varied with the GPU's clocks: `main` 91.0 to 91.7,
  none 91.1, one 94.6 to 94.8, four 100.1 to 100.3) it is the expected 4% and 10% more.
- **Render CPU** does not change: choosing the extra suns is a pass over the scene's directional lights (none, one or
  four here) into a fixed array, under a microsecond, inside the lights' CPU time. The scene keeps them as it keeps
  the local lights (a second set of the same records, following a part like them); the parts' sync pays the same
  one test as before for lights (a flag now covering both sets), and all the rest is out of line.

### Auto's calibration on a fixed workload (#139)

#84's calibration timed the game's own first frames, so the tier depended on the first game played (a light one's
calibration was kept for heavier ones), and on the GPU's clocks under vsync. Auto now times a fixed workload instead
(`metallic_engine::calibration`, `settings::Calibrator`); the game's frames never enter the measurement.

**The workload** is the `calibration` stress scene (seed 139, the same on every machine): 3,256 parts on a 120 m
square (the city's mix of blocks, balls, cylinders and wedges, 299 of them see-through glass), 256 lamps (224 point
lights and 32 spot lights, 16 with shadows) under the default `Lighting`'s sun, with 1% of the parts and an eighth of
the lamps (every shadowed one among them) moving, so the near cascades, the far one's build and the shadowed lights'
tiles are drawn every frame. 256 lights exceed Medium's budget (128) and High's (192), and 16 shadowed ones High's
(8) and Ultra's (10), so every tier draws more than the one below it. A renderer of its own draws it into an
offscreen 1920 × 1080 target (the resolution the tiers are sized for; a larger window is dynamic resolution's to
absorb) in the window's format, with the post chain. Uncapped on the dev PC (DX12, `stress --scene calibration
--preset <tier>`, 600 frames after 120, p50 in ms):

| Tier | Ultra | High | Medium | Low |
| --- | ---: | ---: | ---: | ---: |
| GPU | 2.50 | 1.95 | 1.53 | 0.62 |
| Opaque pass | 1.82 | 1.46 | 0.99 | 0.32 |
| Lights' shadow atlas | 0.13 | 0.09 | 0.07 | 0.02 |

**How it runs** (nothing on the frame path waits for the GPU or for a thread):

- **It starts after the game's first frame**, once the game's renderer has compiled its lit pipelines (at most 2 s
  later; `Renderer::lit_pipelines_ready`). The workload draws with the same shaders, which DX12 compiles once per
  device and keeps: started with the game, the workload's FXC compiles of the same lit stages competed with the game's
  first frame (`many_lights`' first frame 1,403 ms against 1,091 to 1,100 without) and its build took 0.8 to 0.9 s;
  started after them, it takes 25 to 50 ms.
- **It is built on a worker thread in three steps**, each started right after a game frame is presented: its game and
  GPU scene, its renderer (at Low), its first frame. A new device memory block holds wgpu's allocator while the OS
  clears it (#152), and a game frame writing a buffer then waits: built at once and at Ultra, the build gave the game
  21 to 69 ms frames. At Low its first frame's textures are small; Ultra's (3 × 3072² cascades and their static map,
  8x MSAA at 1080p) then arrive off the frame, one a frame, like any settings change's.
- **The workload is drawn after each game frame presented**, as many frames back to back as fill the GPU time the game
  leaves: the game's frame interval (the median of those seen while the workload was built, before it loads the GPU),
  less the game's GPU time and 1.5 ms, at most 8 (the first 4 are timed, `TIMING_RING_SIZE`; the rest keep the GPU
  busy). Their timestamps come back asynchronously like the game's (`Renderer::recent_gpu_timings` keeps the newest
  four, so a batch collected at once loses none).
- **Each tier** counts from 4 frames after its settings are drawn (its switch's pipelines and textures done), then 16
  frames, fewer when more than three quarters are over its share; its cost is their **lower quartile**. Ultra, High
  and Medium are kept at #84's shares of a 60 fps frame (8.3, 10 and 13.3 ms), Low is what is left.
- **The game** draws at Medium meanwhile, with Auto's dynamic resolution, and switches once, to the tier chosen, when
  it is done, through #152's off-the-frame path. The workload's renderer is dropped then.

**Clocks.** The first version drew batches of up to 8 ms and took each tier's median. In the Player at a 60 fps cap
(on this 120 Hz display) it chose High at 7.7 ms on the demo and `parts_100k`, and Ultra at 2.6 ms on `many_lights`:
the workload's High frames, 1.95 ms uncapped, took four times as long on a GPU a light game leaves idle most of the
time, whose clocks stay low when one 8 ms frame and the game's 0.5 ms fill half of each 16.7 ms. Filling the room the
game leaves keeps the GPU busy, so its clocks rise as for any game that needs them, and the lower quartile keeps the
frames timed while they rise from deciding. Now the tier and its time are the same capped, under vsync and uncapped.

**Measured** on the dev PC (Ryzen 5 5500, RX 6650 XT, DX12, driver 32.0.21045.16010, 2026-11-05). The Player with a
fresh settings file (`preset = "auto"`, `frame_cap = 60`, so 60 fps on this 120 Hz display), three runs a first game;
"after the first frame" is until the game switches, "waiting" the frames it waited for the game's lit pipelines, and
the worst game frame is the longest frame interval meanwhile, the game's first (its own pipeline wait) left out:

| First game | Tier (ms) | After the first frame | Game frames (waiting) | Workload frames (built in) | Worst game frame |
| --- | --- | ---: | ---: | ---: | ---: |
| demo | Ultra (2.50, 2.50, 2.51) | 261 to 273 ms | 17 to 18 (1) | 33 to 36 (32 to 40 ms) | 17.1 to 18.7 ms |
| `parts_100k` | Ultra (2.49, 2.51, 2.50) | 598 to 629 ms | 37 to 39 (20 to 22) | 34 (44 to 46 ms) | 17.1 to 17.2 ms |
| `many_lights` | Ultra (2.50, 2.50, 2.50) | 261 to 272 ms | 17 to 18 (1) | 28 to 32 (32 to 36 ms) | 17.1 to 17.3 ms |

- **Under a second at 60 Hz** from the first frame in every case. `parts_100k` has no lights, so its renderer compiles
  the lit pipelines in the background after its first frame; most of its 0.6 s is that wait. The workload itself
  takes 0.25 s: its build, Ultra's switch (6 frames, its textures one a frame) and 20 timed frames in batches of 6.
- **The game's worst frame** while calibrating is 17.1 to 18.7 ms against a 16.7 ms frame; at Medium without a
  calibration the same frames' worst was 17.3 to 21.8 ms (the early frames' own work). The game's single switch to
  Ultra afterwards costs what any switch to Ultra does (#152): frames of 18 to 22 ms while its textures arrive.
- **The stress bench** (`stress --scene <s> --preset auto`, two runs each) chose Ultra at 2.49 to 2.51 ms under vsync
  (120 Hz) and uncapped alike: `many_lights` in 188 to 209 ms (vsync) and 169 ms (uncapped), `parts_100k` in 510 ms
  and 470 to 490 ms, worst game frames 13.5 to 17.9 ms and 10.3 to 14.4 ms.
- **The game's first frame** does not move: `many_lights` 1,043 to 1,055 ms with Auto against 1,035 to 1,082 at
  Medium; `parts_100k` 759 to 811 against 746 to 761.
- **Memory** (wgpu's allocator report, DX12, `many_lights`): the game at Medium holds 100 MiB of textures and buffers
  (256 MiB of blocks); with the workload at Ultra beside it 476 MiB (896), so the workload adds about 376 MiB (the
  same peak in blocks as #152's High → Ultra switch); once it is dropped and the game is at Ultra, 369 MiB (640).
- **GPU time**: 28 to 36 workload frames, about 85 ms of GPU work in all at Ultra's 2.5 ms, most of it in the 10 or
  so game frames whose batches fill the room.

**Saved** with the calibration as before, plus the workload's version: the `[calibration]` section gains
`workload = 1` (`CALIBRATION_WORKLOAD`, raised whenever the scene, its size or how it is measured changes). A file
from before reads as it did, its calibration as workload 0, which is measured again; nothing else in the format
changed.

```toml
[calibration]
tier = "ultra"
gpu_ms = 2.50
adapter = "AMD Radeon RX 6650 XT"
driver = "32.0.21045.16010"
workload = 1
```

**Tests**: `metallic_engine::calibration::tests::the_tier_does_not_depend_on_the_game_played` runs the real
calibration on a device (its worker thread, its workload's renderer and switches) beside two games drawn every frame
at Medium, `mobile` and `many_lights`, with the workload's frame times injected (Ultra 9 ms, High 6), and both choose
High at 6 ms. `settings`' tests drive the calibrator as the engine does: the first tier that fits, frames before a
probe's switch not counting, a GPU whose clocks rise during the measurement (its first frames four times as slow:
Ultra, by the quartile, where the median would have failed it), batches filling the room, and a file from before the
workload measured again. The steady-state frames after a calibration take one branch more than before (no
calibration to advance) and allocate nothing (`tests/no_alloc.rs` unchanged); the perf gate's workloads (scene sync
and cull) are untouched.

### Bloom in one compute pass (#145)

Bloom's chain was 2N − 1 render passes (9 at Medium and above, 7 at Low): a downsample per level, then an upsample
adding each level onto the next larger. Its GPU time was the same at 720p and 1080p, so it was mostly the passes, and
each render pass cost about 18 µs of render CPU (#85). **Where the device has compute shaders, the chain is now one
compute pass** (`metallic_render::post`, `post/bloom.rs`), a dispatch per step:

- **The same filters at the same coordinates.** `bloom_compute.wgsl` is appended to `bloom.wgsl` and runs its taps
  (the 5-tap dual filter, the 13-tap one, the quarter's four, the upsample's 3 × 3 tent, `finite` on every tap), one
  invocation per destination texel at the texel's centre, as the fragment entry points do. A dispatch cannot blend
  onto what it reads, so each upsample writes the level's sum apart (its own level plus the tent over the sum
  below), which is what the blend added up to; the finish pass reads the largest sum. Every level and every sum is a
  texture of its own, not a mip of a shared one: OpenGL sets the mips a view samples on the texture, and Mesa's
  llvmpipe dropped the image stores into a mip outside them (CI's GL run drew no bloom at all with mips).
- **Its levels are the render passes' `Rg11b10Ufloat`** where the adapter writes it as storage (DX12, Vulkan; Metal
  on most GPUs), so the image is the one the render passes draw (`tests/post.rs`, below) and the memory is theirs plus
  the sums (2.8 MB more at 1080p). Elsewhere (OpenGL writes no `Rg11b10Ufloat` storage) they are `Rgba16Float`,
  which every device with compute shaders writes.
- **Large textures.** Bloom's textures are made with its settings, at once (#152 makes the scene's attachments, the
  shadow maps and the atlas off the frame; bloom's stay under its 16 MiB up to 4K). `Rg11b10Ufloat` levels are no larger
  than the render passes' were. `Rgba16Float` levels are twice as large, so where they would reach 16 MiB and the
  render passes' would not (an output above about 6 megapixels at Medium and above, on OpenGL: 4K), the chain draws
  the render passes, whose pipelines such a device compiles too.
- **The render passes stay** for devices without compute shaders (WebGL 2, GLES 3.0) and for
  `BloomSettings::compute = false`, which the stress example's `--bloom-render-passes` and
  `SceneRenderer::set_bloom_compute` set, to measure them and to test them anywhere.
- **Pipelines**: the compute chain's six (`PipelineKey::BloomCompute`: the first downsample's three filters, the later
  ones' two, the upsample) take the render passes' six places in the background compiles (#86): a device compiles
  one chain's, both where its compute levels are wider than the scene colour's. An effect-quality switch compiles
  nothing, as before. The compiler now carries compute pipelines as well as render pipelines
  (`pipelines::Pipeline`).
- **Reported**: the stress JSON's `graphics.post` gains `compute_passes` (1 with bloom's compute pass) and
  `render_passes` no longer counts bloom's chain there; the overlay's `POST` line reads `PASSES 2 + 1 COMPUTE`. The
  chain is still timed as one (`bloom`), its timestamps on the compute pass.

**Render passes per frame** (the JSON's `graphics.post`; DX12, the presets' own anti-aliasing and merged scene):

| Preset | Before | After: render + compute | Bloom's chain before → after |
| --- | ---: | ---: | --- |
| Low | 10 | 3 + 1 | 7 render passes → 1 compute pass (7 dispatches) |
| Medium | 12 | 3 + 1 | 9 → 1 (9) |
| High | 11 | 2 + 1 | 9 → 1 (9) |
| Ultra | 11 | 2 + 1 | 9 → 1 (9) |

Medium's three render passes are the merged scene, MLAA's and the finish pass; High's two the scene (MSAA resolved
in it) and the finish pass. Without compute shaders the frame keeps the counts before.

**The stress scenes** (the done-when): 2026-11-05, the dev PC (Ryzen 5 5500, RX 6650 XT, driver 32.0.21045.16010,
Windows 11), **DX12**, `AutoNoVsync`, 1,000 frames after 120 of warm-up, seed 1, `--preset <tier>`, `main` at
`2c47331` (before) and this branch, run interleaved, two rounds each; the mean of the rounds' p50s in ms. Bloom's
GPU time is the chain's (the JSON's `gpu_ms.passes.bloom`); in brackets, the same chain with `Rgba16Float` levels.

| Scene | Preset | Render passes before → after (+ compute) | Render CPU before → after | GPU before → after | Bloom GPU before → after (`Rgba16Float`) |
| --- | --- | ---: | ---: | ---: | ---: |
| `parts_100k` (1920 × 1080) | Low | 10 → 3 + 1 | 0.661 → 0.608 | 0.383 → 0.359 | 0.044 → 0.024 (0.025) |
| | Medium | 12 → 3 + 1 | 0.698 → 0.667 | 0.621 → 0.591 | 0.113 → 0.060 (0.058) |
| | High | 11 → 2 + 1 | 0.836 → 0.749 | 0.670 → 0.646 | 0.124 → 0.101 (0.092) |
| | Ultra | 11 → 2 + 1 | 0.897 → 0.809 | 0.929 → 0.909 | 0.125 → 0.103 (0.093) |
| `many_lights` (1920 × 1080) | Low | 10 → 3 + 1 | 0.444 → 0.379 | 0.752 → 0.737 | 0.046 → 0.025 (0.026) |
| | Medium | 12 → 3 + 1 | 0.665 → 0.579 | 2.171 → 2.161 | 0.119 → 0.069 (0.075) |
| | High | 11 → 2 + 1 | 0.795 → 0.697 | 2.814 → 2.820 | 0.133 → 0.127 (0.104) |
| | Ultra | 11 → 2 + 1 | 0.831 → 0.731 | 3.528 → 3.516 | 0.130 → 0.104 (0.104) |
| `mobile` (1280 × 720) | Low | 10 → 3 + 1 | 0.392 → 0.331 | 0.221 → 0.187 | 0.043 → 0.019 (0.022) |
| | Medium | 12 → 3 + 1 | 0.421 → 0.350 | 0.326 → 0.277 | 0.086 → 0.034 (0.035) |
| | High | 11 → 2 + 1 | 0.455 → 0.435 | 0.358 → 0.322 | 0.098 → 0.058 (0.056) |
| | Ultra | 11 → 2 + 1 | 0.484 → 0.359 | 0.518 → 0.479 | 0.099 → 0.060 (0.059) |

- **Render CPU** drops by 0.03 to 0.10 ms (0.02 to 0.12 on `mobile`, whose runs are noisier): 8 fewer render
  passes, about 10 µs each net of the compute pass and its dispatches.
- **Bloom's GPU time** halves at Low and Medium (0.044 → 0.024 ms, 0.113 → 0.060) and drops by a fifth to two fifths
  at High and Ultra, whose 13-tap downsamples are more of it. It no longer costs the same at 720p as at 1080p
  (Medium: 0.034 against 0.060), so what is left is texels. `many_lights` at High is the exception (0.133 → 0.127,
  three runs each: 0.126 against 0.110 with `Rgba16Float` levels): its chain starts while the heavy lit scene still
  drains, and its GPU frame does not move.
- **`Rgba16Float` levels** (measured with the format forced on DX12) are 0.01 to 0.02 ms faster at High and Ultra
  here and the same at Low and Medium, at twice the memory, a large texture at 4K and an image up to two levels off;
  `Rg11b10Ufloat` wins where the adapter has it.
- **A texture per level** (the GL fix above) measured as the mips the table was measured with: one more DX12 round
  of every row, bloom within 0.003 ms and render CPU within noise.
- **The render passes** in the new code (`--bloom-render-passes`, the GLES path) measured as `main` within noise at
  every row (bloom 0.044/0.113/0.124/0.125 ms on `parts_100k`, the same counts).

**OpenGL** (same machine and runs, `WGPU_BACKEND=gl`; `Rgba16Float` levels, OpenGL's only storage format of the two):

| Scene | Preset | Render CPU before → after | GPU before → after | Bloom GPU before → after |
| --- | --- | ---: | ---: | ---: |
| `parts_100k` | Low | 2.781 → 2.656 | 1.299 → 1.265 | 0.096 → 0.048 |
| | Medium | 3.910 → 3.641 | 2.162 → 2.025 | 0.316 → 0.148 |
| | High | 3.768 → 3.647 | 2.115 → 1.968 | 0.407 → 0.253 |
| | Ultra | 4.503 → 4.176 | 2.785 → 2.640 | 0.406 → 0.253 |
| `many_lights` | Low | 3.991 → 3.893 | 2.471 → 2.424 | 0.101 → 0.053 |
| | Medium | 5.797 → 5.465 | 3.903 → 3.698 | 0.180 → 0.078 |
| | High | 5.588 → 5.920 | 3.633 → 3.993 | 0.184 → 0.125 |
| | Ultra | 6.119 → 5.640 | 4.123 → 3.760 | 0.166 → 0.094 |
| `mobile` | Low | 1.756 → 1.627 | 0.626 → 0.590 | 0.077 → 0.036 |
| | Medium | 2.293 → 2.017 | 1.058 → 0.939 | 0.208 → 0.077 |
| | High | 2.329 → 2.063 | 1.056 → 0.934 | 0.260 → 0.134 |
| | Ultra | 2.771 → 2.494 | 1.473 → 1.351 | 0.260 → 0.133 |

OpenGL's render passes cost more (0.1 to 0.3 ms of render CPU saved), and its bloom GPU time halves.
`many_lights` on OpenGL varies by 0.4 ms between rounds (its own two rounds of `main` were 4.06 and 3.75 ms of GPU at
Medium), so its High row is noise.

**Offscreen, every backend** (the `scene_passes` bench's 10,000-part grid at 1280 × 720, each preset's anti-aliasing,
effects and scale, the scene passes merged, run with `BloomSettings::compute` off and on; p50 over three rounds of
400 frames; ms). Vulkan has no adapter for a window here while the monitor is off (#110), so this is its
measurement:

| Backend | Preset | Render passes | Render CPU passes → compute | GPU passes → compute | Bloom GPU passes → compute |
| --- | --- | ---: | ---: | ---: | ---: |
| Vulkan | Low | 10 → 3 + 1 | 0.146 → 0.109 | 0.177 → 0.150 | 0.037 → 0.011 |
| | Medium | 12 → 3 + 1 | 0.161 → 0.112 | 0.250 → 0.215 | 0.064 → 0.029 |
| | High | 11 → 2 + 1 | 0.161 → 0.112 | 0.236 → 0.198 | 0.076 → 0.038 |
| | Ultra | 11 → 2 + 1 | 0.161 → 0.113 | 0.261 → 0.224 | 0.076 → 0.038 |
| DX12 | Low | 10 → 3 + 1 | 0.189 → 0.136 | 0.186 → 0.164 | 0.040 → 0.019 |
| | Medium | 12 → 3 + 1 | 0.207 → 0.139 | 0.276 → 0.226 | 0.089 → 0.035 |
| | High | 11 → 2 + 1 | 0.204 → 0.135 | 0.251 → 0.207 | 0.097 → 0.054 |
| | Ultra | 11 → 2 + 1 | 0.205 → 0.134 | 0.290 → 0.250 | 0.097 → 0.057 |
| GL | Low | 10 → 3 + 1 | 1.336 → 1.141 | 0.600 → 0.555 | 0.075 → 0.032 |
| | Medium | 12 → 3 + 1 | 1.708 → 1.399 | 0.908 → 0.785 | 0.205 → 0.078 |
| | High | 11 → 2 + 1 | 1.517 → 1.196 | 0.740 → 0.613 | 0.259 → 0.134 |
| | Ultra | 11 → 2 + 1 | 1.601 → 1.288 | 0.826 → 0.703 | 0.259 → 0.134 |

On a frame with little else, the compute pass saves a third of the render CPU on DX12 (0.07 ms) and Vulkan (0.05 ms),
0.2 to 0.3 ms on GL, and half of bloom's GPU time everywhere.

**What was not done, and why** (by the numbers above):

- **A single-dispatch (SPD-style) downsample.** SPD reduces 2 × 2 boxes in a workgroup's tile and lets the last
  workgroup finish the small levels through a global atomic. Bloom's dual and 13-tap filters read texels beyond
  their 2 × 2 footprint at every level (the 13-tap one is why High flickers less), so a tile's level k needs a halo of
  about 2^(k+1) source texels: recomputed per workgroup, or read across workgroups, which WGSL cannot make coherent.
  A box filter would change the look. With the chain in one pass its CPU cost is one pass already; SPD could only
  save the GPU barriers between four or five small downsample dispatches, of a chain that now takes 0.02 to 0.06 ms
  at Low and Medium.
- **Fewer, wider upsample steps** (a larger tent, two levels a step) change the kernel (a tent of a tent is not a
  wider tent), so the look; and a step now costs a dispatch, not a pass.
- **Folding the last upsample into the finish pass** saves one dispatch at half resolution but moves the 9-tap tent
  into the finish pass at full resolution (four times the texels, in a pass that takes 0.04 to 0.10 ms), for every
  preset; not worth it once the upsample is not a pass.

**The GLES / WebGL path** (no compute shaders): the render passes as before, 7 for bloom at Low and 9 at Medium and
above (10, 12, 11 and 11 render passes a frame at Low, Medium, High and Ultra); measured above with
`--bloom-render-passes` as `main`'s. OpenGL 4.3 and GLES 3.1 have compute shaders and take the compute pass.

**The look.** The compute pass runs the render passes' filters at the same coordinates
(`tests/post.rs`, `bloom_draws_the_same_image_in_a_compute_pass_as_in_render_passes`: the lamp at night, where the glow
is the image, and the demo by day, at Low, Medium and High, compute pass against render passes):

| Backend (levels) | Max difference | Mean | Pixels over 2 levels |
| --- | ---: | ---: | ---: |
| Vulkan, DX12 (`Rg11b10Ufloat`) | 1 | 0.00003 to 0.00014 | 0 |
| GL (`Rgba16Float`) | 1 to 2 | 0.17 to 0.50 | 0 |

With `Rg11b10Ufloat` levels a handful of pixels differ by one level (the order the sums round in); `Rgba16Float` keeps
a few more bits of the glow, which `Rg11b10Ufloat`'s 5- and 6-bit mantissas round down, so the image is half a level
brighter on average where bloom covers it. Both are within the references' tolerance (a mean of 1, 1% of pixels over
12): the reference images are unchanged and pass on Vulkan, DX12 and GL here, so no before/after crops are needed. CI
runs the comparison on lavapipe, WARP and llvmpipe (GL).

- **Allocations** (`tests/no_alloc.rs`): the hand-written frames record the same compute pass, dispatches and bind
  groups (the render passes where a device has no compute shaders), and every frame makes exactly the wgpu calls
  written out by hand: the default frame 129 / 129 on Vulkan, 121 / 121 on DX12, 153 / 153 on GL (199, 191 and 272
  before: wgpu-core allocates for each render pass); the merged scene 105, 100 and 125 (178, 173 and 247).
- **Pipelines** (`tests/pipelines.rs`): every permutation is still compiled in the background before a switch needs
  it: 11 for the post chain where one bloom chain is compiled, 17 on OpenGL with a scene colour of `Rg11b10Ufloat`
  (both chains). `tests/post.rs` switches effects and anti-aliasing every frame as before.
- The perf gate's workloads (scene sync and cull) are untouched.
