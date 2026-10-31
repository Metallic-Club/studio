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

Fixed observations are recorded after executing the same contracts. Requested
bytes/calls are allocator observations, not RSS, retained memory or elapsed time.
This report establishes no frame-time speedup or parent #99 wall-time acceptance.

## Allocations outside this scope

- First property insertion and BTreeMap growth still allocate. On this main base,
  warmed non-default assignments also replace their stored String key; PR #361
  addresses that separate path and is not included here.
- Owned String/other payload copies in `World::get`, `default_value`, caller-built
  values and storage `value.clone()` remain. Position in this probe is inline data.
- Unknown-property and unresolved-ID registrations retain owned names. First signal
  map/handler growth, error formatting and arbitrary user callbacks can allocate.
- Cold default-registry initialization and journal capacity growth are warmed out
  of this probe; they are not removed by the key change.

No public API, wire/scene representation, namespace, permission or cleanup
algorithm changes. This does not restructure the map for blocked issue #94.
