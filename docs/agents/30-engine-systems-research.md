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
- Wrote `docs/research/2026-10-30-engine-systems.md`: comparison matrix, audited baseline, ten numbered sections
  (five systems sections, 3-7), milestone mapping and benchmark procedures. Source links and access date are included.
- Separated observed behavior from proposed Metallic work and proposed parameters from measured results.
- Documented the current baseline's limits: core exists, other baseline crates are scaffolds; no game-performance
  or feature-parity result is claimed. Console decisions under #29 remain open.
- Reviewed prose for unsupported claims and checked all 12 source references against fetched documentation.
- Addressed all eight review points on PR #46: reference #31/#45 saver ownership, specify current 0.25 s clamps and
  #33 plus proposed lost-time accounting, distinguish no-device execution from a graphics-free server build graph,
  name affected crates and platform `metallic_net`/`metallic-server` proposal, give every benchmark a provisional gate
  or baseline owner/report path, separate stress-spec wording from proposed 1080p High settings, link closed #37
  lighting research and open #42 crate/API feasibility, and correct Addressables lifetime wording and section counts.
- Kept the original source audit pinned; checked review observations against `ebb94f3`. Normal-merged current main
  into this published branch without rewriting published history. No code/interface/dependency changes were added.

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

Review amendment (2026-10-30): freshly checked Unity Addressables 2.7.6 asset/bundle lifetime wording, Godot's
fixed-tick/interpolation explanation, Luau interruption/isolation and PlayCanvas timing definitions against their
primary documentation. Checked current physics/runtime/player source, dependency manifests, spec language and issue
states for #31/#45/#33/#37/#42. Documentation-only diff against main; no new Rust build or performance run is claimed.
The original three checks above are historical results against the named original base, not fresh main results.
Review validation passed: all 12 source definitions are used; all local links and both platform-section anchors
resolve; numbered sections are 1-10 with five systems sections; all nine benchmark rows have a provisional gate
or named baseline owner; `git diff --check` passes. The diff against current main contains only the two research
Markdown files. CI for the pushed review revision is tracked on PR #46.

## Next

- Maintainers review the proposal and split accepted recommendations into scoped implementation issues.
- Keep active M1 workers on their assigned contracts; this document does not silently expand their scope.
- Fill benchmark reports with raw measurements once the related systems are implemented.

## Open questions

- Persistent scene IDs and collaboration merge policy need a versioned design before Team Create.
- Transport, cache caps, streaming cell sizes and aggregate script scheduling need workload evidence.
- Console architecture remains #29; this task does not choose a resolution.
