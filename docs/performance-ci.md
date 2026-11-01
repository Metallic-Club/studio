# Core performance checks

Implemented for #122, with mutator coverage added for #330. CPU instruction counts gate pull requests; native wall-time samples and Criterion output are trend artifacts. This does not measure rendered frames, GPU work, network latency, or player/Studio startup.

## Pull-request instruction gate

The `Performance` workflow checks out the exact PR base and head on the same Ubuntu 24.04 runner. It copies the head's fixed workload source and toolchain file into the base checkout, then builds both core examples with the same compiler and release/debug settings. This makes the workload identical even on the first PR that introduces it. A base API incompatibility is a build failure, not a silently skipped benchmark.

Callgrind collects inside five named, non-inlined functions. Fixture construction, program startup, timing calls, mutation-result validation and JSON output are outside collection. Each process first creates 1,000 Parts with deterministic properties under Workspace:

| Workload | Collected work |
| --- | --- |
| `reads` | Ten passes reading seven properties per Part, including stored values and string defaults |
| `writes` | Ten passes changing every Part's Position; every write changes the value |
| `clone` | Three full-world clones and their destruction |
| `create` | Create 1,000 additional Parts under the existing Workspace; includes normal allocation, linking and journal costs |
| `destroy` | Individually destroy the 1,000 precreated Parts; includes unlinking, property cleanup and journal costs |

Creation starts with the populated fixture and grows it to 2,000 Parts; destruction starts with that same 1,000-Part fixture and removes each Part in fixture order. Destruction is not a single subtree drop or repeated stale-ID call. Postconditions verify the mutation results before reporting completion, outside collection and native elapsed time. These fixed flat-tree workloads do not cover every subtree shape or journal-capacity boundary.

Three independent process samples are retained per workload. The gate compares median `Ir` (executed instructions), failing if **any** workload grows by more than 5%. Exactly 5% is allowed. Fixed input and a common environment reduce timing noise, but allocator behavior and randomized internal collections can still affect instruction counts. Inspect raw samples when diagnosing variation.

The collector and comparator refuse missing/zero counts, malformed profiles, missing workloads, insufficient samples, wall-time substitution, and mismatched compiler/platform/architecture/Valgrind/workload metadata. A report's supplied median cannot override its raw samples. Collection/build failures fail the job. The exact five-workload set is required, so historical three-workload reports cannot be compared as complete current reports; regenerate both sides from identical current workload source. Python unit tests exercise the failure policy, including 5%/6% boundaries for every workload, missing mutator results, collection symbols, incomplete mutation completion and invalid baselines.

Every run retains raw Callgrind profiles, base/head JSON reports and the Markdown comparison for 30 days. Reports identify revisions, compiler, platform, binary and workload checksums. The step summary contains the comparison or an explicit unavailable message.

Local Linux reproduction, after building the example at both revisions:

```sh
cargo build --locked --release -p metallic_core --example perf_ci -j 4
python3 tools/perf/measure.py collect --binary target/release/examples/perf_ci --source crates/metallic_core/examples/perf_ci.rs --output results/head/report.json --revision "$(git rev-parse HEAD)"
python3 tools/perf/measure.py compare --base results/base/report.json --head results/head/report.json --threshold 5 --summary results/comparison.md
```

Use a fresh output path per collection. Exit codes: 0 = collected/comparison passed, 1 = instruction regression, 2 = invalid or unavailable measurement. The instruction collector requires Linux and Valgrind; it does not pretend to collect instructions on Windows.

## Nightly and manual wall-time trends

The same workflow runs on a daily schedule and through `workflow_dispatch`, on a hosted Ubuntu runner. It records five native samples of the fixed workloads plus the existing core Criterion suite, retaining raw artifacts for 90 days. Only the workload body contributes to the native `elapsed_ns`, rather than subprocess startup. Criterion uses its own warmup and sampling.

Hosted wall time is noisy and **does not gate PRs or establish hardware budgets**. This implementation does not register the owner's PC as a self-hosted runner, run GPU workloads unattended, open regression issues automatically, or claim a quiet-machine baseline. Those require a separately configured controlled runner and a tested comparison/notification policy. GitHub only activates the schedule after this workflow reaches the default branch; a pre-merge CI pass does not demonstrate a nightly run.

Native workload collection can also run on Windows:

```sh
python tools/perf/measure.py collect --metric walltime --repeats 5 --binary target/release/examples/perf_ci.exe --source crates/metallic_core/examples/perf_ci.rs --output results/walltime.json --revision <commit>
```

Record a machine/environment description with any interpreted wall-time comparison. Do not compare the raw nanoseconds across different CPUs, operating systems, compilers or power settings as a code regression.

## Maintenance

- Add a workload only for an implemented hot path, with stable inputs and bounded execution. A new workload must compile against the PR base or be introduced after its required API has landed.
- Keep setup out of the collection symbol. Zero instructions indicate a broken selector; they are an error.
- Review changes to workload size, compiler configuration and thresholds as changes to the gate itself. Benchmark modifications in a PR are visible in its diff and still require review.
- Preserve raw evidence before changing a baseline or threshold. A missing measurement is not a pass.
- Run `python -m unittest discover -s tools/perf -p 'test_*.py' -v` when modifying the collector or comparator.

References: [Callgrind collection controls](https://valgrind.org/docs/manual/cl-manual.html), [Rust black_box](https://doc.rust-lang.org/std/hint/fn.black_box.html), and the existing [performance research](research/2026-10-30-performance-and-scale.md). Instruction counts are a CPU-work proxy, not elapsed time, energy consumption or GPU throughput. Adding creation/destruction coverage does not establish or replace PR #318's unresolved 5% wall-time target or reinterpret its retained local observations.
