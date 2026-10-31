# #187 Compact override storage research

Status: prototype and measured research implemented and locally tested; review pending.
Branch: `studio/187-compact-storage-research`.

## Goal

Measure the remaining storage gap from #114 without changing production World storage. Compare owned-name B-tree,
numeric-key B-tree and sorted numeric-key vector components, preserve f64/owned String/Ref values, and define migration gates.

## Implemented

- Standalone `compact_storage` example; full World fixtures at 300/5,000 Parts with 0/2/7 overrides and saturated lifecycle journals.
- Thread-local System allocator accounting matching #114's requested/freed-byte method, with an unwind-safe recording guard.
- Shared-default component prototypes with reads, updates, explicit-default removal, cloning and unknown-name behavior.
- Semantic tests retain owned strings, exact Vector3 values and instance-reference payloads; they are storage tests, not complete World validation.
- Six rotated-order CPU component samples; pre-resolved tokens and no event/journal/world lookup costs in component timings.
- Standard-library Python evidence collector repeats all requested-heap rows exactly, retains raw JSONL/checksums/compiler metadata and checks sample completeness.

## Captured evidence

The final capture pins `8c5d52c`, rebased onto `ebb94f3`, with no production core source diff. Local core release artifacts were explicitly cleaned before building this capture. Both complete memory matrices repeated exactly. On this MSVC build, Value is 40 B and (u16, Value) is 48 B.
For two overrides, component totals were 763/504/216 B per container (name-map/ID-map/sorted-vector), including each 24 B header.
These are not whole-Part sizes. Seven independent owned Value payloads alone use 280 B, before keys, owner and history storage.

The 5,000-Part full World cases with a saturated old journal used 270.173/1009.173/1053.173 requested B per Part at 0/2/7 overrides.
The new ring from #104 changes retained journal capacity and must not be silently mixed into that baseline. The prototype schema retained 656 B separately.
CPU samples showed substantial concurrent-build variation and are diagnostic, not an accepted speedup or quiet-machine baseline.

## Verification

- `cargo test --workspace -j 4 -- --test-threads=1`: 212 tests passed after seven local workspace packages were cleaned and recompiled in an exclusively used external lane.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed.
- `cargo deny check licenses`: passed; no dependencies added.
- Final example-only accounting/layout additions: five `compact_storage` example tests and strict example Clippy passed; production source unchanged.
- `python -m unittest discover -s tools/perf -p test_storage_probe.py -v`: three tests passed.
- Dedicated Windows/Linux CI runs the example contracts and evidence-parser tests. It deliberately does not gate on timing or compiler-specific byte totals.
- Final raw evidence and feasibility/migration design are in `docs/research/2026-10-30-compact-property-storage.md` and its linked capture directory.

## Next

Review the measured scope and production-pilot gates. This research keeps production core storage unchanged and does not declare the 256-byte target achieved. A production pilot needs its own issue, semantic coverage and complete engine measurements.
