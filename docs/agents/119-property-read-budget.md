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
  timing samples without claiming a timing win. Methodology and exact results:
  `docs/research/2026-10-30-luau-property-reads.md`.

## Validation so far

- `cargo test --release -p metallic_script -j 4 --lib -- --nocapture --test-threads=1`:
  five passed, one manual diagnostic probe ignored.
- `cargo test --release -p metallic_script -j 4 property_getter_probe -- --ignored --nocapture --test-threads=1`:
  passed before and after; precompiled 1,000-read bounded batches, setup excluded.

## Next

- Rebase latest main and clean only the seven local workspace packages in the
  verified owned worker lane before final workspace tests, strict Clippy and licenses.
- Record final provenance/results, open and attach a PR, mark issue review. Do not merge.

## Open questions

None for this scope. Remaining binding/value allocations and the aspirational
getter-time budget need separately measured work.
