# 362: temporary property-signal name keys

Independent child of #99; frozen PR #361 changes only stored BTreeMap keys and
is not included. Claimed by the Codex engine-research worker on branch
`studio/362-property-signal-keys`, from exact main
`238c6fb8af7a05f031cb29930e59c8afe57cf0fa`.

## Design / scope

Change only the private property-signal key name to `Cow<'static, str>`. Emission
uses the already validated registry `PropDef.name`; known-property registration
uses the same static name. Unknown names and unresolved/stale IDs keep owned text
and remain accepted. String equality/hash, namespace/permission behavior, signal
cleanup, default removal, event ordering and the public API stay unchanged.
No signal-map restructuring (#94), stored-key update, borrowed getter or owned
Value/default optimization is bundled.

## Test-only baseline checkpoint

The 1,000-Part reset fixture warms non-default Position values and journal/default
state, then resets each to its default once. The no-signal row controls incidental
allocation; unrelated/matching listeners isolate temporary signal-key names.
Values, actual storage removal via the cached default reference, changed IDs and
callback counts/values are checked outside thread-local RAII counting. A separate
warmed known-registration loop isolates entry lookup. Requests/bytes are allocator
calls, not RSS or a wall-time speedup.

Behavior tests cover temporary caller strings, unknown/Unicode/case-sensitive
names, an unresolved ID becoming live, stale generations, destroy cleanup and
independent deep-clone subscriptions. The future-ID case is intentionally legacy
behavior; formerly accepted registrations must not be rejected.

No production edit or build yet. Report this source checkpoint before baseline
execution, then prove behavior/control validity before interpreting allocation
failures or changing production. GPU/Lua execution is not needed for these core
tests. Required broad gates and documentation remain next; no own-PR merge.
