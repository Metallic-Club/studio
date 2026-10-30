# #122 Performance CI

Status: implemented and tested locally and in Linux PR CI.
Branch: `studio/122-performance-ci`.

## Goal

Make core CPU regressions reviewable with an instruction-count PR gate, and retain native wall-time trends separately from that gate.

## Done

- Standalone core example with fixed read/write/clone workloads and non-inlined collection symbols; setup is excluded.
- Standard-library Python collector, strict Callgrind parsing, environment/workload compatibility checks, five-percent median regression gate and raw evidence retention.
- Seven Python failure-policy regressions, including malformed/zero profiles, a deliberate six-percent regression, incompatible metadata and wall-time substitution.
- Pull-request workflow builds the same harness with base/head engine revisions on one Linux runner.
- Scheduled/manual native and Criterion trend workflow, with explicit hosted-runner limitations and no unattended owner-PC setup or automated notifications.
- Documentation in `docs/performance-ci.md`. No new Rust/Python dependencies or library interfaces.

## Verification

- Python gate tests: 7 passed locally.
- Release workload example: built locally on Windows.
- Isolated full workspace tests: 190 passed, 0 failed on `532c4f7` base.
- Strict workspace/all-targets clippy and dependency license gate: passed. Existing unused-license-allowance warnings remain.
- Native Windows collector smoke: all three workloads completed and a three-sample report was written. Busy-host nanoseconds are not accepted performance baselines.
- Linux Callgrind collection passed in [run 41049035124](https://github.com/Metallic-Club/studio/actions/runs/41049035124) at `124dccc`. Base/head median Ir matched exactly: reads 26,382,538; writes 9,769,480; clone 4,588,247. This calibrates the harness on identical engine code, not a claimed speedup.
- The schedule does not activate until merged; no nightly run is claimed.

## Next

Keep inspecting raw profiles when changing the harness. The nightly schedule still requires merging; no nightly or controlled-hardware performance result is claimed. Do not infer measured frame performance from these infrastructure checks.
