# #166: pattern matcher, long `[...]` classes bypass step counting

Branch `studio/166-pattern-class-steps`, worktree `../metallic-studio-worktrees/166-pattern-class-steps`.

## Goal
In `crates/metallic_script/src/pattern.rs`, make matcher steps follow the real work so the 10M step cap
and the script deadline bound a long `[...]` class. Before the fix, reading a class cost 1 step whatever
its length, and `max_expand` scanned the subject before it counted any steps.

## Done
- `max_expand` ticks 1 step for each subject byte it scans, inside the loop, so the step cap and
  deadline can stop it mid-scan. Totals are unchanged: it used to tick `i` after the scan.
- Reading a class (`class_end`, `match_bracket_class`, which also covers `single_match` and `%f`)
  costs 1 more step per `CLASS_BYTES_PER_STEP` (16) pattern bytes read. Reads under 16 bytes cost
  nothing extra, so ordinary patterns count exactly as before. `single_match` and
  `match_bracket_class` now return `R<bool>`. The class scan itself moved into the free function
  `bracket_class`, with the same logic, which also returns where reading stopped.
- Error texts did not change: `pattern too complex` or the script timeout message.
- Two tests in `tests/vm_round2.rs` (debug build, time before -> after the fix):
  - `long_bracket_class_star_is_bounded`: 1 MB class `]*$` on 10k chars, 124.5 s -> 0.11 s.
  - `long_bracket_class_lazy_is_bounded`: 1 MB class `]-c` on 100k chars, 49.0 s -> 0.11 s.

## Next
- Nothing more for #166. The issue's other option, a pattern length cap (for example 64 KB), was not
  added because it would change behaviour. The worst gap left between two deadline checks is a single
  read of one class.

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p metallic_script -j 4 --test vm_round2 long_bracket
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all -- --check
```

## Interaction with other pattern.rs PRs
- #265 (#171) is merged, and this branch is rebased on it. #265 counts `steps` per starting position
  (`begin_attempt`) and polls the deadline every 4096 charged steps through `poll_steps`, which carries
  across attempts. Every charge here goes through `tick`, so it adds to both counters. A 1 MB class
  read charges about 62k steps: every read polls the deadline, and one attempt allows about 160 reads
  before `pattern too complex`. The rebase had no conflicts. `tests/linear_patterns.rs` and both #166
  tests pass. Without a deadline (host calls that are not budgeted), the step cap bounds each attempt,
  not the whole call. #265 chose that, and this branch does not change it.
- #235 (#165, plain find) and #233 (#167, gmatch) edit `find_plain` and `gmatch`, which this branch
  does not touch.

## Open questions
- None for #166.

## #274: table.sort without a comparator
- `STRING_LIMITS` in `vm.rs` also wraps `table.sort`: with no comparator and a raw length above
  262144 (2^18, about 35 ms of C sort) it raises `table.sort: table too large to sort without a
  comparator (limit 262144)`. With a comparator the sort runs Lua callbacks, so the interrupt still
  fires. Luau's own sort, ordering and error texts are unchanged for everything else.
- Why a cap and not a Rust sort: keeps Luau's exact semantics (`invalid order function`, error
  wording). No change to property reads and writes, so the `instructions` gate is not affected.
- Tests in `tests/vm_round2.rs`: `big_sort_without_comparator_is_refused_fast`,
  `big_sort_with_a_comparator_stays_interruptible`, `normal_sorts_behave_as_before`.
