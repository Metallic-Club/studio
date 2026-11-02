# #104 Full-capacity world change journal

Status: implemented and tested on main `9d0dc8e`; refreshed PR ready for review, not accepted or released.
Branch: `studio/104-journal-ring`.

## Goal

Keep the complete documented 16,384-mutation history so ordinary large-world frame updates do not cause periodic full rescans.

## Done

- Replaced half-buffer draining with a `VecDeque` ring. Each mutation evicts at most one oldest entry, without shifting the remaining history.
- Preserved revision ordering, deduplicated first-change ID order, foreign-world behavior, empty clone journals and the exact cursor eviction boundary.
- `changes_since` still uses binary partitioning, then iterates only the relevant suffix of the ring.
- Added three public-API regressions: full capacity at eviction, exact oldest-valid cursor, and ten frames moving 4,500 of 5,000 Parts with two mutations each.
- No new dependencies and no public signature changes.

## Verification

- Before implementation: all three new regressions failed with unexpected overflow; three existing journal tests passed.
- After implementation: all six journal tests passed on Windows in an isolated target directory.
- Full workspace tests: 193 passed, 0 failed on base `532c4f7`; strict all-targets clippy and licenses passed.
- Formatting and diff checks passed. No timing or renderer throughput claim.

Integration update: normal-merged current main `ebb94f3` into this published branch at `30a7e27`, without rewriting
published history. The diff against main remains the four journal source/test/doc files. Reviewed #209 guard
scoped-cleaned local workspace packages on lane transition; the six journal tests passed again after source
recompilation from this worktree. Full `cargo test --workspace -j 4 -- --test-threads=1` then passed all 215 tests,
including the current Parkour route/player smoke. Strict all-targets Clippy, licenses, formatting and diff checks
passed again. The source merge changed no journal behavior; the original red/green evidence remains above.

## Next

Inspect updated PR #186 CI after the normal integration push. The ring is still bounded;
a reader more than 16,384 mutations behind must rescan.

## Latest-source refresh completed

Normal-merged `749316a` and then `9d0dc8e`, preserving published branch history. The latter includes avatar animation
and locomotion production changes. The diff against `9d0dc8e` contains only this issue's journal source, tests and
documentation. Earlier test totals above remain pinned to their named bases.

The reviewed Cargo lane guard scoped-cleaned local workspace packages on ownership change and verified this
worktree's compile paths. The expanded workspace prebuild passed, then the actual serial workspace suite passed
**643 tests, 0 failed, 3 existing manual probes ignored**. All six journal cases passed, including full capacity,
exact oldest-valid cursor, deduplication and repeated 9,000-mutation frames. Strict workspace/all-targets Clippy
passed in 53.07s; license, formatting and whitespace checks passed. Independent source review found no actionable
concern in logical ordering across ring wraps, monotonic partitioning or cursor boundaries.

Raw local logs: `<TEMP>/metallic-186-current-prebuild.log`, `metallic-186-current-tests.log`,
`metallic-186-current-clippy.log`, and `metallic-186-current-licenses.log`. No new timing or renderer-throughput
claim. Inspect new PR CI after the normal push; previous-head results stay historical. Only the lead/owner may merge.
