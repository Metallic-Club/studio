# #50 Repository status refresh

Status: implemented and tested; preparing review.
Branch: `studio/50-status-refresh`, originally based on main `ebb94f3`; rebased onto `12a6a6b` before review.

## Goal and evidence

Replace the obsolete bootstrap claim that only Task #1 is active and nothing is ready. At the verified
2026-10-30 14:39 UTC snapshot, issues #1-#9 are closed; #10-#14 remain open with blocked labels. Current main has
the engine/runtime/player/Parkour and Studio shell, while editing/Play/Stop, packaging, developer mode and owner
release acceptance remain distinct work. This document does not claim milestone release or change acceptance.

## Done

- Read live task states and main source/manifest/progress files. Verified all seven crates, Game/player/tests,
  Studio shell, auth/update code and startup Velopack hook; `tools/package.py` and Studio devmode modules are absent.
- Pinned a dated main snapshot and linked live ready/in-progress/PR/claim queries. Closed tasks with stale status
  labels are explicitly distinguished from open work.
- Distinguished design artifacts and pending PRs from merged implementations, and documented remaining owner
  update acceptance without treating old progress-note wiring TODOs as current source facts.
- Retained links to the working agreement, conductor/baton, spec/plan, architecture, API and later milestone epics.
- Kept scope to status/progress documentation. No code, UI, dependency, board priority or owner decision changes.
- Verified all 11 local Markdown links. Filed #246 for the separate README bootstrap-state wording; this branch
  keeps that follow-up distinct from the status snapshot.

## Next

Open the PR and inspect CI. Snapshot updates cannot replace checking live claims before work. Do not merge this PR here.

## Validation

- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: 212 passed. Prebuild and test logs verified this worktree
  supplied all local crates; the guard cleaned local workspace artifacts before rebuilding in lane122.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed, with the existing unmatched MPL-2.0 allowance warning.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- All 11 local Markdown links resolved; source facts and the command examples were checked against the pinned snapshot.
- Main advanced from `ebb94f3` to `12a6a6b` during validation. Its intervening change adds avatar art, tools and research/spec
  documents, with no workspace manifests, Rust source or test changes. The historical status snapshot remains explicitly pinned.
- No UI changes or screenshots. CI results will be recorded after publication.
