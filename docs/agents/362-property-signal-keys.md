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

## Fixed verification and publication

At production source `a4c167dc9168a6d6cd4aef3c4156efe59eb2b8c7`, all six focused
tests passed in debug and release. The three default-reset rows and warmed known
registration row all measured zero calls/zero requested bytes in each profile.
The baseline is debug-only; the direct comparison is debug-to-debug, with release
an additional contract check. Probe source bytes are identical across baseline
and fixed samples; full source/profile data are recorded in the research JSON.

- Release prebuild/checks completed in 52.15s; no elapsed-time benchmark was run.
- Guarded full workspace no-run passed in 3m23s; strict workspace/all-target Clippy
  passed in 1m27s. License, formatting and task-range whitespace checks passed.
- Locked serial workspace tests exited zero: 1,588 passed, zero failed, seven
  upstream ignores across 123 summaries including doc tests. All three complete
  Parkour routes passed. No retry, budget change or new ignore was introduced.
- Root worker-coordinator and studio_core source/fixture reviews found no
  actionable concern. The original implementation and typed registration behavior
  were reviewed without changing the frozen parent branch.

The seven upstream ignores are three window-opening tests, two manual getter
probes, one manual real-budget pattern probe and a Studio actions doc example.
Builds used guarded lane121 with four jobs. The actual full suite had the
coordinated sole managed Lua/GPU window. Local source remains pinned at `238c6fb`;
object-only merging with later main `c507b98` is clean (tree `4dcc5411`), which is
source compatibility evidence, not an execution of that composition.

Eight independent baseline/fixed/gate logs and fingerprints are retained outside
git at `integration-evidence/property-signal-keys`. The separately authorized
private composition of PR #361/`0ff86473` and child/`a4c167d` passed nine pure-core
tests at commit `0b02bedf877f660b9edc344728aa4dd2149b8385`, tree
`d6437dced0364bc6c4df11ad7b417fe0c2df4224`: all measured Position rows were zero
allocations/bytes. Its provenance/log/results remain separately under
`integration-evidence/property-key-composition`. No parent assertion, historical
measurement, public branch or repository acceptance changed. This is not a full
workspace test or wall-time comparison of that combined tree.

The final publication append changes only evidence/data after the frozen source
gates. Exact-head remote CI is inspected after publication. No own merge or
remote-machine action is performed.
