# #114 Borrowed property reads

Status: implemented and tested on main `9d0dc8e`; refreshed PR ready for review, not accepted or released.
Branch: `studio/114-borrowed-properties`.
Base: `2780922` (`origin/main` at start).
Rebased onto `532c4f7` before the final checks and PR.
Current-main integration: normal-merged `ebb94f3` at `e24b2e7` without rewriting the published branch. Fresh
semantic/allocation/workspace gates passed, as recorded below. Original Criterion timings remain pinned to
their original base and are not silently relabeled as fresh performance measurements.

## Goal and scope

Avoid cloning strings when engine systems inspect a property. Add `World::get_ref(id, property) ->
Result<&Value, CoreError>` and `classes::default_value_ref(class, property) -> Option<&'static Value>`.
Keep the original owned `get`/`default_value` paths and their performance characteristics; this issue does not
replace the world's property storage or require other crates to adopt the new getter.

The issue's former 256 B/Part target is aspirational. Its acceptance was refined before claiming: measure and
report the actual fixture footprint, without treating a getter change as a dense-storage redesign.

## Implemented

- Explicit values borrow from the world; default values live in a shared immutable registry initialized once.
  Concrete-class `Name` defaults remain correct for derived classes. Successful warm reads allocate nothing.
- Destroyed reference targets still read as `Ref(None)`; invalid instance/property errors match the owned API.
- `get` and `default_value` retain their original implementations. Sending numeric owned reads through the
  borrowed-default cache would add unnecessary lookup work for callers that have not migrated.
- `find_first_child` borrows names instead of cloning a String per candidate; `full_name` borrows its ancestor
  names before constructing the final path.
- Five semantic regressions cover inheritance, stable defaults, changes/default resets, independent owned
  values, destroyed/reused reference targets and error parity.
- A thread-local counting allocator tests successful reads independently of unrelated test-runner activity.
  Criterion now offers owned and borrowed property/frame-read benchmarks.

## Measured allocation evidence

Command: `cargo test -p metallic_core --test borrowed_allocations -j 4 -- --nocapture`.
Windows, Rust 1.99.1, system allocator. Numbers are requested heap bytes, not resident memory or allocator overhead.
The default cache is warmed before the level fixture is measured.

| Operation | Allocation calls | Requested allocated bytes | Requested freed bytes |
| --- | ---: | ---: | ---: |
| One-time shared default-cache initialization | 134 | 21,169 | 8,580 |
| 5,000 Parts, seven properties per Part, borrowed | 0 | 0 | 0 |
| Same reads through owned `get` | 10,000 | 60,000 | 60,000 |
| 10,000 stored Script.Source + 10,000 default Script.Name borrowed reads | 0 | 0 | 0 |

The cache retains 12,589 requested bytes for the process lifetime. The fixture with 5,000 Parts, four non-default
properties per Part, standard services, a Model, the world tree/journal capacities and an ID vector retains
5,075,848 requested bytes: **1,015.170 B/Part**. This is a precisely described fixture measurement rather than an
intrinsic `size_of::<Part>()` value. It does not meet the aspirational 256 B/Part target; typed/dense property
storage remains separate work. The earlier research's 894 B/Part is not asserted as the current footprint.

## Release-mode comparison

Command: `cargo bench -p metallic_core --bench world -j 4 -- frame_read --warm-up-time 0.2 --measurement-time 1
--sample-size 20`. Same fixture and seven properties for each API; both benchmark loops black-box the returned
result. This desktop had concurrent Cargo/Rust build processes. It was not an isolated or quiet benchmark run.

| Fixture | Owned get, Criterion interval | Borrowed get_ref, Criterion interval |
| --- | --- | --- |
| 300 Parts | 150.00–176.81 microseconds (estimate 162.96) | 127.02–139.21 microseconds (estimate 131.92) |
| 5,000 Parts | 3.4110–3.6602 milliseconds (estimate 3.5348) | 3.2895–3.4661 milliseconds (estimate 3.3781) |

These are observations under contention, not a guaranteed frame-time improvement. The 5,000-Part intervals
overlap; allocation counts are the deterministic acceptance evidence. This change does not establish a whole
engine frame-rate target, and existing physics/render readers need separate migration to benefit.

## Validation and next

- Five semantic borrowed-property tests: passed.
- Deterministic allocation test: passed, including owned baseline allocations and zero borrowed allocations.
- `cargo test --workspace -j 4`: passed all 196 tests after the rebase.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed after the rebase.
- `cargo deny check licenses`: passed after the rebase (one existing unused MPL-2.0 allowance warning).
- `cargo fmt --check` and `git diff --check`: passed.
- Final workspace gates used the isolated task target directory
  `%LOCALAPPDATA%\Metallic\cargo-target-114`. An earlier shared-target run picked up another
  worktree's incompatible core artifacts and was discarded; the isolated run resolved that environment failure.
- New dependencies: none. Public API and architecture documentation updated. No UI changes.

Current-main integration gates (2026-10-30): reviewed #209 Cargo guard scoped-cleaned local packages on ownership
transition and verified this worktree's source compilation. Five semantic tests and the allocation test passed
again. The complete allocation table and fixture totals above repeated exactly: borrowed reads allocate zero;
owned frame reads allocate 10,000 times / 60,000 requested bytes; the cache retains 12,589 requested bytes and the
fixture retains 5,075,848. No new Criterion timing run was performed.

`cargo test --workspace -j 4 -- --test-threads=1` passed 218 tests, including the current Parkour route/player smoke.
Strict all-targets Clippy, licenses, formatting and diff checks passed on this merged source. Inspect updated
PR #185 CI after the normal integration push. Do not merge it here.

## Latest-source refresh completed

Normal-merged `749316a` and then `9d0dc8e`, preserving published branch history. The latter includes avatar animation
and locomotion production changes. The diff against `9d0dc8e` contains only this issue's source, tests and documentation.
Earlier totals above remain evidence for their named bases.

- Reviewed Cargo lane guard cleaned only local workspace packages for this source/profile and verified worktree paths.
- Expanded workspace prebuild passed in 4m22s, then the actual serial workspace suite passed **646 tests, 0 failed,
  3 existing manual probes ignored**. This includes all five semantic regressions and the allocation regression.
- Strict workspace/all-targets Clippy passed in 2m06s; license, formatting and whitespace checks passed.
- A fresh optimized allocation run also passed:
  `cargo test --release -p metallic_core --test borrowed_allocations -j 4 -- --nocapture`.
  The counts repeat exactly: cache 134 allocations / 21,169 requested bytes allocated / 8,580 freed;
  retained cache 12,589 bytes; fixture retained 5,075,848 bytes, or 1,015.170 requested B/Part.
  Borrowed frame reads and 20,000 stored/default string reads allocate zero. Owned frame reads allocate
  10,000 times / 60,000 requested bytes. Release compilation took 15.59s; that duration is not a getter benchmark.
- Independent source review found no actionable issue in inherited defaults, owned API compatibility, reference
  filtering, allocation counting or current-registry assumptions.
- Original Criterion timing intervals above were not rerun or relabelled as current performance data. The
  256 B/Part aspiration remains unmet; this change establishes allocation behavior rather than a storage redesign.

Local raw logs: `<TEMP>/metallic-185-current-prebuild.log`, `metallic-185-current-tests.log`,
`metallic-185-current-clippy.log`, `metallic-185-current-licenses.log`, and
`metallic-185-current-release-allocations.log`. Inspect new PR CI after the normal push; no current CI result
is claimed before it runs. Only the lead/owner may merge.
