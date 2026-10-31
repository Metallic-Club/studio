# Property-signal lookup names

Issue #362 is independent of #99's stored-property-key PR #361. On main `238c6fb`,
`World::set` builds a temporary owned signal name on every changed write whenever
the signal map is nonempty. Known-property registration also builds an owned name
on each lookup. This child changes the private key to `Cow<'static, str>`: validated
registry names can be borrowed; unknown/unresolved registration names remain owned.
Text equality, case, World-local IDs, event ordering and cleanup are unchanged.

The test-only baseline `964cd18` keeps production main intact. It warms 1,000
non-default Position values, the default registry and journal capacity before
resetting each Position to its default once. Default removal does not insert a
stored BTreeMap key, even before PR #361. Thread-local RAII counting excludes
fixture creation, name creation, handler registration, logging and assertions.
Values, cached-default references, changed IDs and callback values/counts are
verified outside the measured interval. A no-listener control checks incidental
allocation. `Signal::fire` itself walks handlers without allocating; the fixture
handlers only update atomics.

Debug baseline observations per 1,000 operations:

| Operation | Allocation calls | Requested bytes |
| --- | ---: | ---: |
| Default reset without property signals | 0 | 0 |
| Default reset with unrelated listeners | 1,000 | 8,000 |
| Default reset with matching listeners | 1,000 | 8,000 |
| Warmed known-property signal registration | 1,000 | 8,000 |

All behavioral controls passed before allocation assertions failed. Separate
baseline cases prove temporary string lifetime, unknown/Unicode/case-sensitive
names, an unresolved ID becoming live before cleanup, stale generations, destroy
cleanup and independent deep-clone subscriptions. Owned and borrowed name keys
must compare by exact text; no new registration rejection is allowed.

The same probe bytes at fixed source `a4c167d` passed all six allocation/behavior
cases in both debug and release. All four measured rows requested zero allocations
and zero bytes. The direct before/after comparison is debug against debug; release
is additional fixed-contract verification, not an unmeasured release baseline.
Source/profile identities, probe blob and complete row data are in
[`data/2026-10-30-property-signal-allocations.json`](data/2026-10-30-property-signal-allocations.json).
Requested bytes/calls are allocator observations, not RSS, retained memory or
elapsed time. This establishes no frame-time speedup or parent #99 wall-time
acceptance. The independent branch still retains the stored-key path below.

## Separate private combined-source check

An independent worker composed frozen PR #361 at `0ff86473f7caabd1f00636a5afa54676ac5dc89e`
with this child's `a4c167dc9168a6d6cd4aef3c4156efe59eb2b8c7`, base `238c6fb`.
The normal merges were conflict-free: combined commit
`0b02bedf877f660b9edc344728aa4dd2149b8385`, tree
`d6437dced0364bc6c4df11ad7b417fe0c2df4224`. Three unchanged pure-core targets
passed all nine tests. Combined-source 1,000-write rows (changed/no-op/default
reset/unrelated/matching Position) and known-registration rows were all zero
allocation calls/bytes. PR #361's existing upper bounds required no adjustment.

This is separate evidence for both pending optimizations, not a relabeling of this
branch's independent reset results, a full-workspace composition, new timing data
or repository acceptance. Neither public branch nor its historical observations
changed. Complete private provenance/log/results remain under workspace
`integration-evidence/property-key-composition`; the independent logs remain in
`integration-evidence/property-signal-keys`.

## Allocations outside this scope

- First property insertion and BTreeMap growth still allocate. On this main base,
  warmed non-default assignments also replace their stored String key; PR #361
  addresses that separate path and is not included here.
- Owned String/boxed-CFrame payload copies in `World::get`, `default_value`, caller-built
  values and storage `value.clone()` remain. Position in this probe is inline data.
- Unknown-property and unresolved-ID registrations retain owned names. First signal
  map/handler growth, error formatting and arbitrary user callbacks can allocate.
- Cold default-registry initialization and journal capacity growth are warmed out
  of this probe; they are not removed by the key change.

No public API, wire/scene representation, namespace, permission or cleanup
algorithm changes. This does not restructure the map for blocked issue #94.
