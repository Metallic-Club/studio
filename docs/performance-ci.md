# Core performance checks

Implemented for #122, with mutator coverage added for #330. CPU instruction counts gate pull requests; native wall-time samples and Criterion output are trend artifacts. This does not measure rendered frames, GPU work, network latency, or player/Studio startup.

## Pull-request instruction gate

The `Performance` workflow checks out the exact PR base and head on the same Ubuntu 24.04 runner. It copies the head's fixed workload sources and toolchain file into the base checkout, then builds both revisions' examples with the same compiler and release/debug settings. This makes the workload identical even on the first PR that introduces it. A base API incompatibility is a build failure, not a silently skipped benchmark.

Callgrind collects inside named, non-inlined functions: five for the core workloads below and three for the [Luau workloads](#luau-workloads). Fixture construction, program startup, timing calls, mutation-result validation and JSON output are outside collection. Each process first creates 1,000 Parts with deterministic properties under Workspace:

| Workload | Collected work |
| --- | --- |
| `reads` | Ten passes reading seven properties per Part, including stored values and string defaults |
| `writes` | Ten passes changing every Part's Position; every write changes the value |
| `clone` | Three full-world clones and their destruction |
| `create` | Create 1,000 additional Parts under the existing Workspace; includes normal allocation, linking and journal costs |
| `destroy` | Individually destroy the 1,000 precreated Parts; includes unlinking, property cleanup and journal costs |

Creation starts with the populated fixture and grows it to 2,000 Parts; destruction starts with that same 1,000-Part fixture and removes each Part in fixture order. Destruction is not a single subtree drop or repeated stale-ID call. Postconditions verify the mutation results before reporting completion, outside collection and native elapsed time. These fixed flat-tree workloads do not cover every subtree shape or journal-capacity boundary.

### Luau workloads

Added for #498, so the generic Luau binding (#462) has a base to compare against. `metallic_core` cannot depend on `mlua`, so these live in their own example, `crates/metallic_script/examples/perf_ci_lua.rs`, measured as the separate `lua` suite with its own report and comparison. Each process creates 1,000 Parts under Workspace and runs one `Script` through a server `ScriptHost`, using only the public API.

| Workload | Collected work |
| --- | --- |
| `lua_reads` | Ten passes reading `Transparency`, `Anchored` and `Name` per Part (30,000 reads) |
| `lua_writes` | Ten passes writing `Transparency` and `CanCollide` per Part (20,000 writes); every write changes the value |
| `lua_method_calls` | Ten passes calling `part:IsA("BasePart")`, `part:FindFirstChild("Missing")` and `workspace:FindFirstChild("Part")` per Part (30,000 calls) |

Collection covers one `ScriptHost::step`, which resumes the script thread for the fixed work, so each count also includes one step's small fixed cost. Part creation, compiling and starting the script, a one-pass warm-up, and printing and validating the result are outside collection and native elapsed time. The script's result is checked against values computed from the World, and any script error or warning fails the run. The work runs on a resumed thread rather than a Heartbeat handler because a handler is deferred once the step's wall-clock budget has passed, and the resume budget is raised so Valgrind's slowdown cannot time the script out. These workloads cover scalar properties only; Vector3 and CFrame paths are not measured here.

The workflow copies this source into the base checkout like the core source, so the gate has real base numbers even on the PR that adds a workload. If the base revision has no `perf_ci_lua.rs` at all, the workflow passes `--new`: the rows still show base and head numbers but read `NEW (not gated)`, and the threshold is not applied. Every other check still applies, including missing workloads and mismatched metadata. Once the file is in the base, the suite is gated like the core workloads. A workload added later to the existing file is compared and gated from its first PR.

### Comparison

Three independent process samples are retained per workload. The gate compares median `Ir` (executed instructions), failing if **any** workload grows by more than 5%. Exactly 5% is allowed. Fixed input and a common environment reduce timing noise, but allocator behavior and randomized internal collections can still affect instruction counts. Inspect raw samples when diagnosing variation.

The collector and comparator refuse missing/zero counts, malformed profiles, missing workloads, insufficient samples, wall-time substitution, and mismatched compiler/platform/architecture/Valgrind/workload metadata. A report's supplied median cannot override its raw samples. Collection/build failures fail the job. Each suite's exact workload set is required (five core, three Luau), so historical three-workload reports cannot be compared as complete current reports; regenerate both sides from identical current workload source. Python unit tests exercise the failure policy, including 5%/6% boundaries for every workload, missing mutator results, collection symbols, incomplete mutation completion and invalid baselines.

Every run retains raw Callgrind profiles, base/head JSON reports and the Markdown comparisons for 30 days. Reports identify revisions, compiler, platform, binary and workload checksums. The step summary contains both comparisons, or an explicit unavailable message for each one missing.

Local Linux reproduction, after building the examples at both revisions:

```sh
cargo build --locked --release -p metallic_core --example perf_ci -j 4
python3 tools/perf/measure.py collect --binary target/release/examples/perf_ci --source crates/metallic_core/examples/perf_ci.rs --output results/head/report.json --revision "$(git rev-parse HEAD)"
python3 tools/perf/measure.py compare --base results/base/report.json --head results/head/report.json --threshold 5 --summary results/comparison.md

cargo build --locked --release -p metallic_script --example perf_ci_lua -j 4
python3 tools/perf/measure.py collect --suite lua --binary target/release/examples/perf_ci_lua --source crates/metallic_script/examples/perf_ci_lua.rs --output results/head/lua-report.json --revision "$(git rev-parse HEAD)"
python3 tools/perf/measure.py compare --suite lua --base results/base/lua-report.json --head results/head/lua-report.json --threshold 5 --summary results/lua-comparison.md
```

Use a fresh output path per collection. Exit codes: 0 = collected/comparison passed, 1 = instruction regression, 2 = invalid or unavailable measurement. The instruction collector requires Linux and Valgrind; it does not pretend to collect instructions on Windows.

## Nightly and manual wall-time trends

The same workflow runs on a daily schedule and through `workflow_dispatch`, on a hosted Ubuntu runner. It records five native samples of the fixed core workloads (not the Luau suite) plus the existing core Criterion suite, retaining raw artifacts for 90 days. Only the workload body contributes to the native `elapsed_ns`, rather than subprocess startup. Criterion uses its own warmup and sampling.

Hosted wall time is noisy and **does not gate PRs or establish hardware budgets**. This implementation does not register the owner's PC as a self-hosted runner, run GPU workloads unattended, open regression issues automatically, or claim a quiet-machine baseline. Those require a separately configured controlled runner and a tested comparison/notification policy. GitHub only activates the schedule after this workflow reaches the default branch; a pre-merge CI pass does not demonstrate a nightly run.

Native workload collection can also run on Windows:

```sh
python tools/perf/measure.py collect --metric walltime --repeats 5 --binary target/release/examples/perf_ci.exe --source crates/metallic_core/examples/perf_ci.rs --output results/walltime.json --revision <commit>
```

Record a machine/environment description with any interpreted wall-time comparison. Do not compare the raw nanoseconds across different CPUs, operating systems, compilers or power settings as a code regression.

## Maintenance

- Add a workload only for an implemented hot path, with stable inputs and bounded execution. A new workload must compile against the PR base or be introduced after its required API has landed.
- Keep setup out of the collection symbol. Zero instructions indicate a broken selector; they are an error.
- Give every collection function a different body. The compiler may merge identical functions into one symbol, which would leave the others collecting nothing.
- Review changes to workload size, compiler configuration and thresholds as changes to the gate itself. Benchmark modifications in a PR are visible in its diff and still require review.
- Preserve raw evidence before changing a baseline or threshold. A missing measurement is not a pass.
- Run `python -m unittest discover -s tools/perf -p 'test_*.py' -v` when modifying the collector or comparator.

References: [Callgrind collection controls](https://valgrind.org/docs/manual/cl-manual.html), [Rust black_box](https://doc.rust-lang.org/std/hint/fn.black_box.html), and the existing [performance research](research/2026-10-30-performance-and-scale.md). Instruction counts are a CPU-work proxy, not elapsed time, energy consumption or GPU throughput. Adding creation/destruction coverage does not establish or replace PR #318's unresolved 5% wall-time target or reinterpret its retained local observations.
