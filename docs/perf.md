# Performance checks

Two kinds of numbers: **instruction counts**, which gate every pull request that touches the engine, and
**wall-time measurements**, which are recorded by hand on a stated machine and are not a gate. Renderer frame budgets
(GPU and render CPU per tier) are set in [ADR 0008](adr/0008-rendering-performance.md); this page covers the CPU
hot paths of `metallic_core`.

## The perf gate

`.github/workflows/perf.yml` runs on every pull request that changes `crates/`, `Cargo.toml`, `Cargo.lock` or the
workflow itself. It counts the instructions (Callgrind's `Ir`) each workload in
[`crates/core/benches/perf_gate.rs`](../crates/core/benches/perf_gate.rs) executes, once for the PR's base and once
for its head, on the same runner. **The PR fails when any workload grows by more than 5%** (exactly 5% passes).

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

| Workload | Measured work | `Ir` at introduction |
| --- | --- | ---: |
| `reads` | Ten passes reading seven properties (`CFrame`, `Size`, `Color`, `Transparency`, `Anchored`, `CanCollide`, `Material`) of every part: 70,000 typed reads, three of the seven properties at their default | 8,185,762 |
| `writes` | Ten passes setting every part's `CFrame`; every write changes the value: 10,000 writes | 4,746,793 |
| `create` | 1,000 more parts, each created and parented into the folder | 629,340 |
| `destroy` | The 1,000 parts destroyed one by one, in creation order | 11,619,855 |
| `clone` | `clone_tree` of the folder with its 1,000 parts | 1,268,272 |
| `drain_changes` | One consumer draining 50 changed parts of 5,000 | 1,934 |

The counts are from the PR that added the gate (#29): ubuntu-24.04 runner, Rust 1.99.1, Valgrind 3.22.0,
gungraun 0.20.0, release profile. They are a reference point, not a target; the gate always compares a PR with its
own base. (`destroy` costing about 11,600 instructions per part, against about 630 to create one, is worth a look
when the tree's child lists are next tuned.)

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

The job takes three to five minutes, most of it compiling. Pushes to `main` that touch the same paths only build,
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

| Date | Machine | Bench | Result (criterion median) | Target |
| --- | --- | --- | ---: | ---: |
| 2026-11-02 | Ryzen 5 5500, 16 GB, Windows 11, Rust 1.99.1 | `set_cframe` | 38.3 ns | ≤ 100 ns |
| 2026-11-02 | same | `get_ref_stored` | 9.7 ns | ≤ 20 ns |
| 2026-11-02 | same | `get_ref_default` | 11.0 ns | ≤ 20 ns |
| 2026-11-02 | same | `cframe_typed` | 13.0 ns | |
| 2026-11-02 | same | `create_1000_parts_4_props` | 500 µs | ≤ 1 ms |
| 2026-11-02 | same | `large/create_100k_set_one` | 10.4 ms | < 50 ms |
| 2026-11-02 | same | `read_7_props_5000_parts` | 330 µs | ≤ 0.3 ms |
| 2026-11-02 | same | `drain_50_of_5000` | 236 ns | ≤ 20 µs |
| 2026-11-02 | same | `build_100_classes` | 472 µs | |
| 2026-11-02 | same | `prop_id_own` / `prop_id_inherited` / `prop_id_missing` | 25.3 / 21.6 / 17.5 ns | |
| 2026-11-02 | same | `is_a_true` / `is_a_false` | 2.7 / 2.7 ns | |

The 2026-11-02 run shared the machine with other builds, so its spread is wide (`set_cframe` 35–42 ns,
`read_7_props_5000_parts` 303–358 µs); `read_7_props_5000_parts` sits at its target and should be re-measured on an
idle machine before anyone tunes it.
