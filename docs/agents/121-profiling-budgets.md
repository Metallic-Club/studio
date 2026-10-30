# Issue 121: profiling scopes and frame budgets

Status: substrate implemented and tested; application integration remains planned.
Branch: `studio/121-profiling-budgets`.
Base: `2780922` (2026-10-30). No owner decision is required for this scoped foundation.

## Goal and refined acceptance

Implement bounded CPU scopes and explicit frame/stage budget reports as an independent leaf crate. Optional
profiling/puffin/Tracy exports are compile-time choices. No UI, GPU measurements, automatic game-stage budget integration
or claimed hardware performance. Preserve core and physics APIs owned by other work.

Acceptance: deterministic manual-clock tests for exclusive nested attribution, repeated scopes, measured-zero versus
unavailable values, equality/overrun checks, trace/nesting bounds, bad clocks, bad/forgotten guard lifetimes and
session reset. Default recording/reporting must make zero heap allocations in a counting-allocator test. Document
proposed budget values separately from achieved results. Run all repository checks before PR.

## Done

- Read AGENTS/CLAUDE, status, architecture, #121, #23 and the performance-and-scale research.
- Verified #121 was ready with no fresh claim or blocking dependency; posted refined scope, then claimed it.
- Added `metallic_profiling`, its test-clock API, bounded frame recorder, availability-aware budget checks, optional
  mutually exclusive external backends and a synthetic CPU-only report example.
- Added runtime re-export and feature forwarding. Runtime itself is still a scaffold; no synthetic game API added.
- Instrumented implemented ScriptVm run/step and Scene/Renderer sync/render/prewarm with static, opt-in backend
  scopes. This changes no method signatures. No dynamic script names/source are sent to profilers.
- Configured optional Tracy manual lifetime and localhost-only access. The lexical bridge keeps an optional span
  guard and safely skips uninitialized leaf scopes/frame marks; host startup is explicit and handle drop does not
  shut down the upstream client.
- Added integration documentation and architecture notes. No files in core, physics or platform were changed.

## Validation

- Before implementation: `cargo test -p metallic_profiling --test frame_reports -j 4` failed with missing API imports
  and macro, as expected for the tests-first step.
- Isolated targeted gate: `cargo test -p metallic_profiling -j 4` passed 10 integration tests and one doctest.
- `cargo clippy -p metallic_profiling --all-targets -j 4 -- -D warnings`: pass.
- The counting allocator observed zero allocations during recording, trace overflow, iteration and budget checks.
- `cargo test -p metallic_profiling --features tracy -j 4`: 10 integration tests and one doctest pass, including
  `Client::is_running() == false` before/after inactive scopes and frame marks. No Tracy client was started.
- `cargo test -p metallic_profiling --features puffin -j 4`: 10 integration tests and one doctest pass, including
  a real in-memory export asserting two nested scopes at depth two (no UI or network transport).
- Workspace test/clippy/license gates passed on the initial base, including after script/render instrumentation.
- Both engine-wide individual backend feature checks, all-features license metadata check, warning-free rustdoc,
  synthetic example and formatting check passed. Final rebased workspace results will be recorded before PR.
- Final commands use task-scoped `CARGO_TARGET_DIR` pointing to the local Metallic Cargo cache's isolated `lane121`
  subdirectory and `-j 4`, avoiding same-package artifacts built from other active branches.

## Next

Finish workspace checks after entrypoint instrumentation, backend compile/license checks and the synthetic example,
then open one PR with `Refs #121`. The issue remains open for physics instrumentation after the leaf lands, real
Game/frame benchmark integration after #8, host initialization/dev-mode budgets after #14 and timeline/per-script
diagnostics in #23. A source/proposal foundation is not full application integration.

## Limits

The collector is single-threaded and stores no history. Stage attribution errors are explicit; no GPU values or
unimplemented-stage zeros are invented. Backends can allocate. No recorder-overhead benchmark, game benchmark,
Puffin UI or Tracy network capture has been run.
