# 498: Luau workloads in the instruction gate

Issue #498, part of #458. Design: `docs/design/class-system.md` section 4.4. Branch `studio/498-lua-perf-workloads`.

## Goal

Add `lua_reads`, `lua_writes` and `lua_method_calls` to the CI instruction gate in their own PR, so they exist in the
base when the generic binding (#462) opens. No script behaviour or binding code changes.

## Done

- `crates/metallic_script/examples/perf_ci_lua.rs`: the three workloads, using only the public `ScriptHost` API.
- `tools/perf/measure.py`: a `lua` suite (`--suite`) beside the unchanged core set, and `compare --new`.
- `tools/perf/test_measure.py`: new tests only; the existing tests are unedited.
- `.github/workflows/performance.yml`: builds, collects and gates the suite as a separate step and report.
- `docs/performance-ci.md`: lists the workloads and the new-workload rule.

## Decisions

- **Separate suite and report.** `test_measure.py` pins `WORKLOADS` to the five core names, and existing tests must not
  change. The core binary also stays built exactly as before, by its own `cargo build`.
- **The work runs on a resumed `task.wait` thread, not a Heartbeat handler.** `dispatch` defers a handler once the 0.25 s
  wall-clock step budget has passed, which Valgrind could reach. The first due thread is checked against a timer that has
  only just started; if a stall still skipped it, the result would never print and validation refuses the run.
- **Each collection function steps to a different time.** Identical bodies could be merged into one symbol, and
  `--toggle-collect` would then find nothing for the others. A test guards this, but only means something with `--release`.
- **`--new` is decided from the base checkout, before the copy.** The workflow already copies the head's workload source
  into the base, so a new workload gets real base numbers on its first PR. `--new` only marks the rows `NEW (not gated)`
  and waives the threshold; report validation still applies. It stops applying once the file is in the base.
- **The nightly wall-time job is unchanged.** The issue covers the gate only.

## Not run

No Valgrind and no Python on this machine, and WSL is not installed. So `measure.py`, `test_measure.py`, the workflow and
the `Ir` numbers have not been run. The PR's own `Performance` run is the first real test: check that all three Luau rows
show non-zero numbers and `NEW (not gated)`.

## Commands

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --locked --release -p metallic_script --example perf_ci_lua -j 4
cargo build --locked --release -p metallic_script --example perf_ci_lua -j 4
target/release/examples/perf_ci_lua lua_reads
```

`cargo test --workspace` builds this example but does not run its tests; ask for them with `--example perf_ci_lua`.

## Next

Open the PR. Confirm the gate output above, then #462 can compare against these workloads.

## Open questions

None.
