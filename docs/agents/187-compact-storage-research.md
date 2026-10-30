# #187 Compact override storage research

Status: prototype implemented; initial accounting and component tests tested; final rebase/gates pending.
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

## Initial evidence

At base `532c4f7`, the first accounting matrix repeated exactly. On this MSVC build, Value is 40 B and (u16, Value) is 48 B.
For two overrides, component totals were 763/504/216 B per container (name-map/ID-map/sorted-vector), including each 24 B header.
These are not whole-Part sizes. Seven independent owned Value payloads alone use 280 B, before keys, owner and history storage.

The 5,000-Part full World cases with a saturated old journal used 270.173/1009.173/1053.173 requested B per Part at 0/2/7 overrides.
The new ring from #104 changes retained journal capacity and must not be silently mixed into that baseline. The prototype schema retained 656 B separately.
CPU samples showed substantial concurrent-build variation and are diagnostic, not an accepted speedup or quiet-machine baseline.

## Next

Rebase on current main, regenerate final raw evidence after the last source change, finish parser/semantic/full workspace gates,
write the feasibility/migration design and open a research PR. Keep production core source unchanged. Do not declare the 256-byte target achieved.
