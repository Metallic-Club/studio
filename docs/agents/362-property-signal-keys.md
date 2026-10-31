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

Test-only checkpoint `964cd18422f715de82dfc469e444b40cdfa66ed2` has unchanged main
production source. All three registration/cleanup behavior cases passed. The
allocation target passed RAII recovery and all value/cursor/callback/storage-removal
controls, then failed its two allocation contracts as expected:

| 1,000 operations | Allocation calls | Requested bytes |
| --- | ---: | ---: |
| Default reset, no property signals | 0 | 0 |
| Default reset, unrelated listeners | 1,000 | 8,000 |
| Default reset, matching listeners | 1,000 | 8,000 |
| Warmed known-property registration | 1,000 | 8,000 |

The no-signal control proves that journal/default/storage removal did not add an
incidental allocation. No fixture correction or PR #361 code was required. Logs
are `metallic-362-behavior-baseline.log` and
`metallic-362-allocation-baseline.log` in the local temporary directory.

The production change is confined to the private key type, static validated
emission name and known-registration name selection. Unknown/unresolved fallback
still owns its name. Public docs clarify existing future-ID behavior proved by
the passing baseline, rather than rejecting it. Focused fixed checks and required
broad gates remain next. GPU/Lua execution is not needed for core probes; no
own-PR merge or parent PR change.
