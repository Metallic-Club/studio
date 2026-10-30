# Issue 165: bounded plain string search

## Goal

Remove the quadratic repeated-prefix search behind explicit `plain = true` and implicit literal `string.find`,
preserving Lua byte positions and sandbox timeout reporting without a needle-sized off-heap allocation.

## Research and implementation

- Primary documentation: [memchr 2.8.3 memmem::find](https://docs.rs/memchr/2.8.3/memchr/memmem/fn.find.html)
  guarantees worst-case O(haystack + needle) time and constant space. Reviewed the installed crate's
  `src/memmem/mod.rs`: `find` uses its bounded small-haystack search or borrowed `Finder`, and the forward builder
  retains the needle through `CowBytes::new`; no owned finder or needle copy is used here.
- Added the existing locked `memchr` 2.8.3 as a direct dependency (MIT/Unlicense choice; MIT is allowed).
  The lockfile changes only the local script package dependency list.
- Replaced naive overlapping slice comparisons with `memmem::find` over existing Lua byte borrows. Before/after
  native search checks use the enclosing deadline and the existing `stop_error` script-location path.
- The native search is linear but is not preempted mid-scan. No KMP prefix table, input copy, new budget, range clamp
  or matcher-semantic change is introduced. `gmatch` is unchanged; issue 167 owns its separate memory fix.

## Tests-first evidence

- Two isolated children run the reported ten-million-byte haystack / hundred-thousand-byte repeated-prefix needle
  for explicit plain and implicit literal search. Original code exceeded each three-second outer limit; the parents
  killed and reaped both processes. The byte/index semantic test passed before the implementation.
- Each child now finishes or returns the standard timeout within the asserted 500 ms. One debug validation observed
  about 176 ms and 175 ms, respectively; these are regression observations on a shared host, not benchmark claims.
  Parent tests launch hidden on Windows and retain a three-second outer safety limit. The ignored worker is invoked
  by both normal tests, rather than executed unbounded by the ordinary harness.
- Tests cover empty matches, empty source, negative/zero/past-end starts, UTF-8 byte offsets, NUL/non-UTF-8 bytes,
  literal metacharacters and Lua truthiness. Private tests cover expired deadlines for hits/misses/empty needles and
  searches without an active deadline.
- A deterministic native-to-native test consumes the enclosing budget in a slow host callback, then calls plain find
  without a Luau bytecode interrupt between them. It verifies the original script/line timeout and another script's
  successful recovery.
- Focused script suite: 72 ordinary tests passed, one worker ignored by the parent harness but exercised twice in
  isolated children.
- Final guarded `cargo test --workspace -j 4 -- --test-threads=1`: 218 ordinary tests passed; the subprocess worker
  remains ignored by the outer harness and is exercised twice by its normal parent tests.
- Guarded `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed, including direct memchr; existing unused MPL-2.0 allowance warning only.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Independent read-only review of source, deadline/error routing and child-process tests found no actionable issue.
- Local temporary logs: `metallic-165-workspace-final.log` and `metallic-165-clippy-final.log`.

## Build provenance

Used the reviewed issue-209 `tools/cargo_lane.py` externally, with this worktree as `--workspace` and the agent-owned
`lane116` target. The guard exclusively cleaned only local workspace packages on the source-owner transition;
compile output confirms `165-linear-plain-find` source paths. Third-party artifacts remain reusable.

## Next / open questions

Local gates and source review are complete. [PR 235](https://github.com/Metallic-Club/studio/pull/235) is open and
issue 165 is marked for review. CI is pending. Do not merge.
No UI or public API signature changes. Native scans are linear but are not preempted mid-scan.
