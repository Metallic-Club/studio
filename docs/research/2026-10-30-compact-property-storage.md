# Compact property override storage

Status: measured research and component prototype for #187. Production World storage is unchanged.

The measured override containers can use substantially less requested heap after replacing repeated property-name ownership. A sorted numeric-key vector is a useful next production pilot for sparse properties. The data does **not** establish a whole-Part size of 256 bytes or a game/frame speedup. Dense-case CPU samples also do not justify choosing one representation for every class.

## Evidence and scope

The [captured tables](storage-187/windows-msvc/tables.md), [memory rows](storage-187/windows-msvc/memory.jsonl), [CPU samples](storage-187/windows-msvc/cpu.jsonl) and [metadata](storage-187/windows-msvc/metadata.json) retain the raw evidence. Metadata pins the probe commit, compiler, OS, binary and source-byte hashes. The engine code is the `937dcd8`/`88f5512` generation; their difference was visual-language documentation, not Rust/manifests/lockfiles. The later conductor documentation also changes no engine code.

Build: Rust 1.99.1, x86_64-pc-windows-msvc, Windows 11, release profile. Local core artifacts were explicitly cleaned in the release profile before the captured build. Development workspace gates used separately cleaned local packages. Third-party dependency artifacts were retained. This avoids the cross-worktree artifact mismatch tracked in #209.

The counter forwards System allocation operations and records successful requested layouts and released layout sizes on the current thread. Objects returned from a measured closure remain alive when its counts are read. Shared initialization is warmed outside the World intervals; the prototype's cached defaults are reported separately. Two complete memory runs must match exactly before the collector accepts them. They did.

These are observed allocation requests, not allocator headers, page commitment, RSS, GPU resources, Rapier storage, Lua memory or complete Game memory. Outer stack-object sizes are separate raw fields. Rust permits allocation optimizations, and type layout can change between builds; the retained objects pass through black_box, and results are pinned to this compiler/target. See [GlobalAlloc](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html) and [Rust type layout](https://doc.rust-lang.org/reference/type-layout.html).

## Whole World fixtures

Each fixture contains standard services, one Model, a preallocated ID vector and 300 or 5,000 Parts. Override densities are 0, 2 and 7. Non-default properties are added in this order: Position, Orientation, Size, Anchored, CanCollide, Material and Shape. There are no precision changes.

After construction, 20,000 temporary Folder create/destroy pairs saturate the journal without changing the Parts' override density. One reusable temporary slot/free-list allocation remains included. The rows therefore include slots, children, capacities, services, model, ID vector and populated history. They measure a specific long-lived World fixture, not an average across real games.

| Parts | Overrides | Requested live heap | Heap divided by Parts |
| ---: | ---: | ---: | ---: |
| 300 | 0 | 575,984 B | 1,919.947 B |
| 300 | 2 | 797,684 B | 2,658.947 B |
| 300 | 7 | 810,884 B | 2,702.947 B |
| 5,000 | 0 | 1,350,864 B | 270.173 B |
| 5,000 | 2 | 5,045,864 B | 1,009.173 B |
| 5,000 | 7 | 5,265,864 B | 1,053.173 B |

The per-Part average is much worse for a small fixture because bounded history and arena capacity are amortized across fewer Parts. The baseline still uses the old half-draining journal. #104 changes its retained capacity; it must be measured as a new baseline rather than mixed into these rows.

## Override components

All three prototypes use the same owned Value payloads and shared defaults. The numeric tokens are pre-resolved, class-local research keys. Containers implement reads, update, default removal and cloning. Tests preserve owned strings, Vector3 values including a large f64 coordinate, and instance-reference payloads. Class/type checking, reference liveness, events and World synchronization are outside these components.

The following numbers include each container's 24-byte header in a preallocated outer vector and its owned payloads. They exclude the owning Node/slot, World history and shared defaults. Do not add them blindly to the World totals, which already contain existing map headers.

| Overrides | Owned-name B-tree | Numeric-ID B-tree | Sorted ID vector |
| ---: | ---: | ---: | ---: |
| 0 | 24 B | 24 B | 24 B |
| 2 | 763 B | 504 B | 216 B |
| 7 | 807 B | 513 B | 417 B |

The component values repeat at both fixture sizes. Numeric keys remove repeated owned-name allocation; the vector also avoids B-tree node storage for these small maps. Its spare capacity still matters: the two-entry case retains capacity for four ID/Value pairs; the seven-entry case retains eight. Cached prototype defaults cost 656 requested bytes once, separately from these rows. This small schema is not the full all-class default cache from #114.

Rust's [BTreeMap documentation](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html) explains its ordered, multi-entry-node design. The measured difference is an observation of this workload and implementation, not a general statement that vectors outperform B-trees.

## What the CPU samples establish

The probe records six samples in rotated representation order, with 20 passes of seven borrowed reads and 20 Position-update passes per instance. Setup is excluded. The zero-override write case inserts Position on its first pass; subsequent passes update it. Explicit default removal is checked outside timing. Unknown-name handling is checked separately.

These timings exclude property-name resolution, type validation, World/reference lookup, events, journal writes and Lua conversion. The raw report gives median and range per operation. Concurrent engine builds caused substantial variation. Numeric-key representations are worth testing in the complete engine, but these samples do not certify a getter budget, a ranking for dense classes, or a frame-rate improvement. In particular, the ID B-tree's dense read median beat the sorted vector in this run; the ranges and end-to-end costs must remain visible.

## The 256-byte target needs a defined denominator

On this build, Value is 40 bytes and `(u16, Value)` is 48 bytes. Seven independent, unshared owned Value payloads alone require 280 bytes, before keys, container, instance identity, hierarchy or journal. Key compaction alone cannot make that case a complete 256-byte Part.

A journal entry is 16 bytes. A fully retained 16,384-entry history requires at least 262,144 payload bytes before capacity/allocator overhead. That fixed cost alone is about 874 bytes per Part in a 300-Part fixture. This is a calculated lower bound for full retention, not a measurement of the new ring's allocation.

Keep three budgets distinct: proportional instance/override storage, fixed bounded World caches/history, and actual resident game memory. Define density and included costs before accepting a per-Part target. Sharing immutable repeated values, class-specialized payloads, a different borrowed-value view or reducing a rarely used large variant's inline footprint are separate designs. None is implemented here, and none requires silently switching existing f64 values to f32.

## Production pilot and migration gates

1. Introduce a private override-storage abstraction and benchmark the sorted-ID vector with real template property distributions, including four/five-property cases and larger singleton classes. Evaluate importer reservation versus runtime spare capacity. Keep a dense-class alternative until evidence supports a cutoff.
2. Resolve names through the flattened class registry. Validate token bounds and inheritance; do not truncate indices to u16 unchecked. IDs remain internal. Scene/protocol formats continue to use their documented names and identities.
3. Preserve owned get and borrowed-read lifetime contracts, class-specific defaults, Name behavior, stale reference filtering and World:Send. Generic storage components alone do not prove these properties.
4. Preserve unknown-property/type/invalid-value error precedence. Rejected changes must alter neither value, journal nor signals. Equal/default writes retain existing removal and notification semantics.
5. Verify ordered child/signal behavior, generation reuse, subtree/world clone independence and reference remapping. Keep scene text deterministic and save/load/save byte-equal; numeric ID order must not reorder saved property names.
6. Run complete engine, hostile-script, character, Parkour route and player smoke tests. Add borrowed-read allocation checks and the instruction gate when available. Remeasure requested heap with the new journal and report real API costs separately from component results.

The implementation task should follow this evidence with a scoped issue and rollback plan. This research does not change production representation or mark the memory target achieved.

## Reproduce

Use an isolated worktree target, or the guarded lane workflow from #209 when available. Explicit profile selection matters for package-scoped cleanup; do not assume a default clean removed release artifacts.

```sh
cargo clean --profile release -p metallic_core
cargo build --release -p metallic_core --example compact_storage -j 4
python tools/perf/storage_probe.py --binary <target>/release/examples/compact_storage --output <fresh-evidence-directory>
cargo test -p metallic_core --example compact_storage -j 4
python -m unittest discover -s tools/perf -p 'test_*.py' -v
```

Use the `.exe` binary on Windows. The collector stores metadata, 24 memory rows and 108 CPU rows. It rejects incomplete, duplicated or arithmetically inconsistent data. Source-byte hashes describe the captured working copy; line-ending conversion changes those hashes even when code semantics are unchanged.
