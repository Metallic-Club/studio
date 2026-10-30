# #104 Full-capacity world change journal

Status: implemented and tested; PR CI pending.
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

## Next

Inspect PR CI. The ring is still bounded; a reader more than 16,384 mutations behind must rescan.
