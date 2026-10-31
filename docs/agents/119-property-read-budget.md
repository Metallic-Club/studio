# #119 Instance member-key allocation

Goal: remove the owned Rust string allocation from valid UTF-8 member lookup while
preserving current Luau behavior. The historical 150 ns getter goal is aspirational.

## Done

- Rechecked closed dependency #7 and issue claims; refined acceptance and claimed
  #119 on `studio/119-property-read-budget`, starting at main `88f5512`.
- Added a guard-owning member-name wrapper used by both index and assignment.
  Invalid UTF-8 retains the existing owned lossy fallback; non-string keys still fail.
- Added five targeted regression tests plus an ignored release getter probe.
- Recorded the baseline allocation regression failing: 10,000 conversions allocate
  10,000 strings / 80,000 requested bytes. After the change, warmed conversions
  request zero allocations/bytes. Real scalar and vector batches each remove one
  8-byte key allocation per read; full getters still allocate.
- Updated API/architecture documentation and recorded measured slower busy-host
  timing samples. Added same-process AB/BA comparisons of both actual getters and
  key conversion: three runs showed lower borrowed medians, with variable p95.
  The test-only baseline switch is absent in production. Methodology and results:
  `docs/research/2026-10-30-luau-property-reads.md`.

## Validation so far

- `cargo test --release -p metallic_script -j 4 --lib -- --nocapture --test-threads=1`:
  five passed, one manual diagnostic probe ignored at that point.
- `cargo test --release -p metallic_script -j 4 property_getter_probe -- --ignored --nocapture --test-threads=1`:
  passed before and after; precompiled 1,000-read bounded batches, setup excluded.
- `cargo test --release -p metallic_script -j 4 --lib paired_property_getter_probe -- --ignored --nocapture --test-threads=1`:
  three runs passed; 40 AB/BA samples per run, shared warmed VM and precompiled functions.
- Rebased onto main `ebb94f3`; core/script baseline sources did not change.
- Verified the worker's target directory, rejected a reparse target, and ran
  `cargo clean --profile dev` for all seven Metallic workspace packages only
  before final gates (964.7 MiB removed). Third-party cached artifacts remained.
  Compile logs identify this #119 worktree for all seven local crates.
- `cargo test --workspace -j 4 -- --test-threads=1`: 217 passed, two ignored
  diagnostic probes (both manually run above). Includes the full Parkour route.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed; existing unused allowance warnings only.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Opened and attached PR #232; issue #119 is `status:review`. CI is running.

## Next

- Inspect CI and reviewer feedback on PR #232. Do not merge.

## Open questions

None for this scope. Remaining binding/value allocations and the aspirational
getter-time budget need separately measured work.
