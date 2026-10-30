# Issue 30: engine systems research

Date: 2026-10-30. Status: tested documentation; recommendations remain planned.
Branch: `studio/30-engine-systems-research`.

## Goal

Compare primary technical documentation from other engines and turn the findings into concrete Metallic priorities,
failure-path checks and reproducible benchmarks. Complement the existing platform/launcher study without changing
the M1 specification or active implementation issues.

## Done

- Read `AGENTS.md`, `CLAUDE.md`, `docs/STATUS.md`, architecture, M1 spec/plan and the existing platform study.
- Checked open work and created/claimed #30 as a separate documentation task.
- Read 12 primary sources covering Unreal, Unity, Godot, Roblox, Bevy, PlayCanvas, Babylon.js and Luau.
- Wrote `docs/research/2026-10-30-engine-systems.md`: comparison matrix, audited baseline, ten systems sections,
  milestone mapping and benchmark procedures. Source links and access date are included.
- Separated observed behavior from proposed Metallic work and proposed parameters from measured results.
- Documented the current baseline's limits: core exists, other baseline crates are scaffolds; no game-performance
  or feature-parity result is claimed. Console decisions under #29 remain open.
- Reviewed prose for unsupported claims and checked all 12 source references against fetched documentation.

## Validation

Windows, Rust toolchain `1.99`. Initial source audit baseline:
`25955d8c43d050c54d39ac027b4ae65ca2cc31bf`. Rebased and repeated all three checks against
`ca1314e704891af4d837b7ccf0d8fb039600cd16`, which includes physics PR #28.
Cargo was available through `%USERPROFILE%\.cargo\bin`; it was added to this process's PATH only.

- `cargo test --workspace -j 4`: pass, 33 tests (16 core integration, 12 physics, five scaffold), no failures.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: pass.
- `cargo deny check licenses`: pass, `licenses ok`; five existing unused-license-allowance warnings.
- `git diff --check`: pass.
- Manual: checked source attribution, proposal/status language and issue/milestone references. All 12 source
  definitions are used; a text check found no em or en dashes. Prose audit found no invented benchmark results or
  feature-parity claims; the technical tone and long comparison tables are intentional for maintainer review.
- No visual or runtime code change; no screenshot or competitor performance test was performed.

## Next

- Maintainers review the proposal and split accepted recommendations into scoped implementation issues.
- Keep active M1 workers on their assigned contracts; this document does not silently expand their scope.
- Fill benchmark reports with raw measurements once the related systems are implemented.

## Open questions

- Persistent scene IDs and collaboration merge policy need a versioned design before Team Create.
- Transport, cache caps, streaming cell sizes and aggregate script scheduling need workload evidence.
- Console architecture remains #29; this task does not choose a resolution.
