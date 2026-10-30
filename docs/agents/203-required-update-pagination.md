# #203 Required-update pagination

Status: implemented and locally tested; independent code review passed; PR/CI pending.
Branch: `studio/203-required-update-pagination`, based on `ebb94f3`.

Note: after the Launcher L1 merge this logic lives in `crates/metallic_update/src/feed.rs` (`fetch_releases`, `version_from_tag`) and its tests in `crates/metallic_update/tests/updater.rs`; it applies to the Studio and Launcher feeds.

## Goal

Do not lose required-update flags from skipped releases on a later page. Keep the newest offered version while
collecting all relevant pages, and reject incomplete histories instead of presenting a partial optional result.

## Evidence and implementation

- Tests-first reproduction: page one has 50 dev releases 0.1.51 through 0.1.2, page two has required 0.1.1 and installed
  0.1.0. The original code returned 0.1.51 with required=false; the regression failed at the required assertion.
- Accumulate pages instead of overwriting the last page. A valid, published same-channel version at/below the
  installed version ends the scan, as does the end of the listing. Drafts and other/invalid tags do not end history.
- Keep the existing five-page/50-per-page cap. If it is reached before a boundary or list end, return BadResponse;
  later-page transport/HTTP/parse failures remain errors, never partial offers.
- This follows the existing release process's monotonically increasing version policy. A future backport or
  re-publication model requires a separately defined ordering contract; this change does not assume GitHub sorts
  arbitrary tags by semver.
- Seven deterministic fake-HTTP tests cover the reported case, full-page installed boundary, draft/other tags,
  empty final page, bounded truncation and later-page failure. No real release/network/credential operations.

## Verification

Original focused regression failed as expected. Reviewed issue-209 Cargo lane helper scoped-cleaned local packages
on ownership transition into this worktree; compile paths confirmed it.

Independent review found that the shared tag helper accepted semver prerelease/build suffixes outside the release
script's X.Y.Z grammar; an RC tag could impersonate the installed-version boundary. Added a second failing test and
invalid-boundary fixtures, then restricted the helper to plain versions. A second independent read-only review of
that fix and the complete pagination path found no remaining actionable findings under the documented publication policy.

- Final focused updater/auth suite: 25 passed.
- Final `cargo test --workspace -j 4 -- --test-threads=1`: 219 passed, including full Parkour route/player smoke,
  hostile-script safety tests and all seven new pagination/tag regressions.
- Final `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo deny check licenses`,
  `cargo fmt --check` and `git diff --check`: passed after the plain-tag refinement. No new dependencies.

## Next

Commit/push/PR, inspect CI. Do not merge.
PR #237 owns exact-tag download selection; #204 owns sign-in races; #205 owns release workflow isolation. #189 shared
crate extraction is temporarily coordinated behind #202/#204 so it does not move code while fixes are under review.
