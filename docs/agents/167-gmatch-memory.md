# #167 Count gmatch retained strings against the VM cap

Goal: iterators retain shared VM-managed string handles, so their subject/pattern
storage stays inside the existing 256 MiB allocator cap rather than uncounted Rust
byte vectors. No matcher algorithm, execution budget or public API changes.

## Done

- Claimed P1 #167 after finding #88 already claimed by another session.
- Read the exact mlua 0.12.2 string/value reference implementation: LuaString is a
  shared value handle with a weak VM reference; temporary borrowed bytes keep the
  VM alive during access without copying string storage.
- Added independent regressions for distinct retained subjects and patterns, plus
  shared storage, garbage collection, independent iterator progress and binary bytes.
  Lower test-only memory caps keep reproduction bounded; host-driven setup isolates
  allocator accounting from timing of the production execution quota.
- All three regressions failed against the original implementation: distinct
  subjects/patterns never exhausted the VM cap, and a shared subject disappeared
  from allocator accounting despite its retained iterators.
- Replaced the two Rust byte copies with LuaString handles captured by the callback;
  temporary bytes are borrowed inside each invocation. All three regressions pass.
- Updated public scripting limits, crate docs and architecture without changing APIs.

## Next

- Rebase, open/attach PR. Do not merge. Optional independent peer review requested.

## Validation

- New `gmatch_memory` integration tests: all 3 failed against original byte copies,
  all 3 pass after retaining handles. Distinct subjects and patterns hit the allocator
  cap; shared storage remains counted, then is released after iterator collection.
- Guarded `cargo test --workspace -j 4 -- --test-threads=1`: passed, including the
  full Parkour input route, player smoke, existing pattern compatibility and VM limits.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed (existing unused MPL-2.0 allowance warning).
- `cargo fmt --check`, `git diff --check`: passed.

## Commands

Used the reviewed #209 helper ([PR #230](https://github.com/Metallic-Club/studio/pull/230)) from its worktree with `--workspace` pointing here and
the existing worker lane. It cleans current local workspace packages on the owner
transition and preserves third-party dependency artifacts. No global environment edits.

## Open questions

The issue's literal acceptance of many iterators hitting the limit is refined:
iterators over one shared subject should share its allocation and succeed; distinct
retained subjects or patterns must exhaust the allocator. Both are tested.
