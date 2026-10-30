# #209 Build-lane ownership guard

Goal: reuse third-party Cargo artifacts across worktrees without accepting stale
local workspace artifacts as validation of another source checkout.

## Done

- Claimed #209; branch `studio/209-build-lane-guard`, dedicated worktree from main.
- Added standard-library `tools/cargo_lane.py`: exclusive cross-process lane lock,
  metadata-derived local package cleanup on owner transitions, atomic local owner
  record, fixed target/intermediate directories and restricted Cargo argument arrays.
- Added isolated tests for ownership, process contention through test completion,
  root/source/symlink/junction rejection, overrides, failed metadata/cleanup/tests,
  and Ctrl-C child completion handling. Python 3.11+ uses stdlib TOML parsing to
  fingerprint Cargo configuration and recursive includes without recording contents.
- Profile/target switches clean the matching output selection. An isolated Cargo
  1.99.1 probe confirmed default package clean left a release artifact present.
- Peer review found and fixed fresh-lane initialization, Clippy compiler-output tail
  bypass, and Windows legacy-codec decoding of UTF-8 Cargo metadata. A real std-only
  probe covers fresh bootstrap, reuse and release in a Japanese-named workspace with
  Python UTF-8 mode disabled. Unknown nonempty unmarked targets are refused.
- Documented normal verification and explicit package-only recovery, with raw-Cargo
  and forcibly terminated process limitations in `docs/build-lanes.md`.

## Validation

- `python -I tools/test_cargo_lane.py`: 23 tests run, 22 passed, one ordinary symlink test
  skipped because Windows did not permit symlink creation. Actual Windows target,
  lock and owner junction tests passed; actual Cargo probe passed.
- `python -I tools/cargo_lane.py --target-dir <worker lane> -- test --workspace -j 4 -- --test-threads=1`:
  passed, including player smoke, complete input-only Parkour route and script suites.
- `python -I tools/cargo_lane.py --target-dir <worker lane> -- clippy --workspace --all-targets -j 4 -- -D warnings`:
  passed after review fixes; same-owner invocation skipped cleanup as expected.
- `cargo deny check licenses`: passed (existing unused MPL-2.0 allowance warning).
- `cargo fmt --check`, `git diff --check`, existing release Python tests: passed.
- Rebased onto `ebb94f3`; upstream changes were building-tools documentation only.
  Engine sources, Cargo manifests/lock, helper/tests and CI workflow were identical
  to the validated tip after rebase. Peer re-review independently passed fresh/reused
  real Cargo lanes, Clippy output rejection and the Windows Unicode-path case.
- Before switching from #121, verified the resolved lane and cleaned only local
  workspace packages through Cargo. Third-party artifacts were retained.

## Next

- Open and attach PR. Do not merge. Windows/Linux helper CI remains pending until the PR runs.

## Open questions

None. No engine API, runtime behavior or dependencies change.
