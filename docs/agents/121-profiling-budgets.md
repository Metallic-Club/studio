# Issue 121: CPU scopes and simulation budget reports

Status: implemented and tested for CPU recording, backend entrypoints and simulation intervals. Full displayed-frame/dev-mode integration remains planned; PR references #121 rather than closing it.
Branch: `studio/121-profiling-budgets`. Date: 2026-10-30.
Validated runtime base: `937dcd8` (includes #8 Parkour/runtime and #163 Studio shell).

## Goal

Provide bounded CPU timing and honest budget assessments, with opt-in real Game integration and optional external profilers. Preserve FrameOutput and gameplay. Missing render/GUI measurements must not certify a whole-frame policy. No UI or GPU timing changes.

## Implemented

- Independent `metallic_profiling` leaf: monotonic/test clocks, static lexical scopes, fixed trace/nesting capacity, owned interval reports, exclusive CPU accounting and explicit unavailable/incomplete values.
- Trace overflow drops samples without truncating stage totals. Bad clocks, bad nesting, excessive depth and forgotten guards remain visible; incomplete or absent required data cannot produce an overall budget pass.
- Proposed configurable CPU policy tables from performance research, labeled as proposals rather than measured/accepted targets.
- Default collector recording, overflow, iteration and budget assessment allocate zero heap objects in a counting-allocator test. External backends and existing game allocations are outside that guarantee.
- Optional mutually exclusive puffin/Tracy bridges. Puffin has a real in-memory nested-export test. Tracy uses manual lifetime and localhost-only access; inactive tests verify no client starts. Host initialization is explicit and handle drop does not shut down the upstream client.
- Static backend scopes in ScriptVm run/step, Scene/Renderer sync/render/prewarm, and runtime simulation/render/GUI entrypoints. No dynamic script names or source payloads.
- Game CPU recording defaults off. `set_cpu_profiling_enabled`, `latest_simulation_profile` and `latest_simulation_budget` provide bounded 128-sample simulation/camera reports with lifecycle clearing. FrameOutput is unchanged.
- Simulation intervals exclude separate rendering/GUI/app window processing and do not mark a full external frame boundary. `finish_backend_frame` remains an explicit host responsibility.
- Architecture/crate docs, integration guide and a synthetic CPU report example. No core, physics or platform files changed.

## Validation

- Tests first: collector tests failed on missing APIs before implementation; real-Game tests failed on missing additive methods before integration.
- `cargo test --workspace -j 4 -- --test-threads=1`: pass, including existing complete input-only Parkour route/player smoke tests and four new actual-Parkour profiling tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: pass.
- `cargo deny check licenses`: pass; one existing unused MPL allowance warning.
- Default profiling package: 11 integration tests and one doctest pass, including allocation accounting and missing-required-stage policy behavior.
- `cargo test -p metallic_profiling --features puffin -j 4`: 11 integration tests and one doctest pass; actual nested scopes retain depth two without UI/network transport.
- `cargo test -p metallic_profiling --features tracy -j 4`: 11 integration tests and one doctest pass; Client::is_running remains false before/after leaf scopes and frame marks.
- `cargo clippy --workspace --all-targets --features metallic_runtime/profiling-puffin -j 4 -- -D warnings`: pass.
- Same engine-wide clippy command with `metallic_runtime/profiling-tracy`: pass (compile/lint only; engine app initialization remains integration work).
- `cargo deny --all-features check licenses`: pass. This checks dependency metadata; mutually exclusive backends are not compiled together.
- `RUSTDOCFLAGS=-D warnings cargo doc -p metallic_profiling -p metallic_runtime --no-deps -j 4`: pass.
- Synthetic `frame_report` example: pass; displays missing stages and overall NotMeasured, not a game benchmark result.
- Targeted formatting and `git diff --check`: pass.
- Final gates use an isolated task-scoped local Cargo target (`lane121`) with four build jobs. This lane has only built this worktree; source ownership must be cleaned/guarded before any future worktree switch (infra #209).

## Remaining issue acceptance

- Physics-library backend entrypoints after the shared leaf lands, coordinated with physics work.
- Whole displayed-frame aggregation and explicit application backend startup/frame boundaries.
- Developer-mode budget consumer after #14; timeline/per-script diagnostics under #23 and its editor dependencies.
- A headless Parkour percentile benchmark using real reports; no benchmark target was measured or asserted here.

The collector is single-threaded and retains no history. No recorder-overhead benchmark, GPU measurement, Puffin UI or Tracy network capture was run. Actual simulation integration is tested; full application integration is not claimed.
