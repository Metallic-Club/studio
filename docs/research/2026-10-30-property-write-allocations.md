# Property-write allocation observations (#99)

Base: `238c6fb8af7a05f031cb29930e59c8afe57cf0fa`. PRs #112 and #185 already removed lookup work and added
borrowed property/default reads. This follow-up changes only existing-key updates in `World::set`:
`BTreeMap::get_mut` replaces the value while retaining its owned key. Validation, range handling, default
removal and event ordering remain on their original paths. There is no no-op early return or signal redesign.

## Requested allocation counts

The thread-local counting allocator in `tests/property_writes.rs` counts successful allocation/reallocation
requests and requested sizes, rather than RSS, peak heap or retained memory. A Drop guard disables counting
during unwind. Fixtures, shared defaults, journal capacity, listener construction and result assertions stay
outside the measurement. Callbacks only update preallocated atomics, isolating engine costs.

Test-only baseline `97d2b70` produced these rows, then failed its desired zero-key-allocation assertion. Other
value/cursor/callback checks passed. Fixed `b64bd23` passed all three tests in both debug and release, with the
same results. Each row is 1,000 accepted writes:

| Case | Baseline allocations / requested bytes | Fixed allocations / requested bytes |
| --- | ---: | ---: |
| Changed, stored Position | 1,000 / 8,000 | 0 / 0 |
| Unchanged, stored Position | 1,000 / 8,000 | 0 / 0 |
| Reset distinct Parts to default Position | 0 / 0 | 0 / 0 |
| Changed Position with unrelated Anchored listener | 2,000 / 16,000 | 1,000 / 8,000 |
| Changed Position with matching listener | 2,000 / 16,000 | 1,000 / 8,000 |

Remaining listener allocations come from the temporary `(InstanceId, String)` lookup key. First insertion,
owned strings/boxed Values, owned defaults and allocating user callbacks remain separate costs. Successful
borrowed reads retain #185's existing behavior; this is not a whole-engine zero-allocation claim.

The historical A1 journal fixture's 1,000 / 12,000 result used **Transparency**, whose name has 12 bytes. It was
not a Position measurement. The first new semantic fixture also incorrectly assumed Transparency was ranged;
that failure is retained as a fixture error. The corrected compatibility case uses the actual registry range
on Lighting.ExposureCompensation, and passed before the storage fix.

## Release get/set observations

The ignored `tests/property_write_timing.rs` test uses the default allocator, without a counting hook. Its
baseline is `56d4585`, a test-only child of `97d2b70`; production matches main. The fixed side is `b64bd23`.
Both use identical probe bytes, SHA256 `e643cc85bb2e21d5ad6f7891880f4a4894ace546b1cdb1379be95f02f674c47c`.
The original allocator-instrumented timing definition never executed. Separate retained release targets prevent
later workspace/profile changes from overwriting these executables; their hashes were checked before and after
every invocation. No compiler artifacts were copied.

After these observations, strict Clippy rejected the manual test's constant assertion for release mode.
The final probe expresses that same pre-loop guard as an explicit debug-mode panic, without changing the
measured loops. Timing results/fingerprints remain attributed to the original matching probe and source
checkpoints above; the frozen executables were not rebuilt or relabeled, and no timing repeats were run.

The bounded run used one recorded warm-up per side, then exactly eight alternating baseline/fixed and
fixed/baseline pairs, with 100,000 operations per loop. Setup and journal warmup precede each loop. Owned and
borrowed Position reads are unchanged controls. All 18 invocations passed; no pass-seeking repeats were run.
No Cargo/rustc process appeared in the entry snapshot, and the other team workers held builds/tests. Windows
CPU samples before/after were 38.8%/34.5% busy; these do not establish an otherwise idle host or explain variance.

| Loop | Baseline median ns/op (min–max) | Fixed median ns/op (min–max) | Median paired ratio (min–max) |
| --- | ---: | ---: | ---: |
| Owned get control | 19.197 (18.118–28.298) | 20.755 (18.459–36.530) | 1.0710 (0.9574–1.5726) |
| Borrowed get control | 12.228 (10.810–16.551) | 11.7445 (10.788–14.487) | 0.9800 (0.6686–1.3268) |
| Set | 117.8455 (102.933–201.722) | 69.4095 (52.991–89.815) | 0.5190 (0.3909–0.8613) |

The setter was lower in all eight pairs, but both unchanged controls vary substantially. These observations
do not establish a reliable speedup, universal five-percent wall-time result or cause of the variance. The
existing CI instruction threshold is unchanged. The deterministic improvement is the removed key allocation.

[All warm-ups, pairs, counts and fingerprints](data/2026-10-30-property-writes.json) are retained. Local raw logs,
the bounded runner and executable manifests live under `integration-evidence/99-reuse-property-keys`.

To reproduce the test definitions, use an owned guarded lane:

```text
cargo test --locked -p metallic_core --test property_writes -j 4 -- --test-threads=1 --nocapture
cargo test --release --locked -p metallic_core --test property_write_timing -j 4 -- --ignored --exact release_get_set_observations --nocapture --test-threads=1
```

Coordinate a timing window before executing the manual test. Run baseline and fixed versions with matching
probe source and normal system allocation; do not compare the counting target's wall time.
