# Issue 173: task ownership, bookkeeping and duration coercion

Goal: queued task threads cannot be resumed with foreign functions/arguments, internal scheduler markers do
not escape through user coroutine calls, numeric string waits coerce, and nil delay means zero. Branch
`studio/173-task-thread-boundaries`, worktree `173-task-thread-boundaries`, fresh base `69a10d3`.

## Research and proposed design

Read the pinned Luau 0.740 `VM/src/lcorolib.cpp`, the
[Luau coroutine reference](https://luau.org/library/#coroutine-library) and
[Roblox task reference](https://create.roblox.com/docs/reference/engine/libraries/task). Native resume preserves
all yielded/returned values and error objects. Native wrap prefixes string-coercible errors, including numbers
(`lua_isstring`), while tables/booleans/nil/userdata retain object behavior. The task reference confirms
function-or-thread targets and default-zero wait; nil-delay compatibility follows this issue's explicit
acceptance rather than a claim that the sparse documentation establishes it.

Root approved a prototype with native pcall/xpcall unchanged and ordinary coroutine compatibility tests first.
Proposed: a private weak thread-key registry, fresh opaque queue lease identities, bound task-created function
and argument tuples, narrow resume/wrap admission adapters, and private recognition/consumption of exact
SLEEP/OK/ERR markers. Adopted user threads must be queued correctly when they wait, release queued ownership
for ordinary yields/returns, and retain arbitrary user error objects. Check task.spawn/delay(Thread) duplicate
ownership too; stale or externally closed queue entries must not produce later non-resumable errors.

Use typed Option<f64> for delay to retain existing numeric-string coercion and argument diagnostics while
defaulting nil to zero. Wait normalization must use the same numeric policy and retain useful positioned
errors. No new public task APIs, native pcall replacement, unsafe FFI or execution-budget relaxation.

## Integration prerequisite

PR #255 originally used nil task.delay duration as an invalid-argument formatter fixture. That conflicts with
this issue's acceptance. In its own existing worktree, replaced it with `pcall(string.match, {}, 'x')`, a truly
invalid typed string argument independent of task policy. All 16 formatter cases passed (0.14 s) after guarded
cleanup and verified source compilation; normal appended commit `95a787b` is pushed and its CI passed.
No pending PR #255 production code is imported here.

## Tests first / next

Eight original baseline tests cover delayed-function hijack, foreign wait elapsed values, user wait tag opacity
and adoption, native-wrap bypass on an adopted thread, wait numeric-string coercion, nil-delay default,
existing numeric-string delay compatibility, and an exact native coroutine result/error/frame snapshot.
The snapshot includes nil/multiple values, ordinary yields, dead threads, string error levels, arbitrary error
objects, numeric wrap errors and invalid arguments, and checks native pcall/xpcall remain C functions.

Baseline on the original `69a10d3` source: six expected behavioral failures, while native coroutine snapshots
and existing numeric-string delay coercion passed. Guarded cleanup removed 49.1 MiB of local package outputs;
both core/script paths were verified. Compile 7.68 s; eight tests 0.05 s.

The prototype is written and its first focused prebuild passed. Weak thread keys hold private Rust userdata state,
with exact u64 owners, cancellation flags and fresh Rc lease identities (no wrapping ticket counter). State
does not reference its thread or Lua. Normal coroutine values stay entirely in Lua so opaque error userdata
are not reconstructed while crossing Rust. Private callbacks only admit/finish threads and queue waits.
Binding task-created functions and full argument tuples happens at creation. Duration delay uses Option<f64>;
wait captures native tonumber. Native pcall/xpcall are unchanged.

The expanded matrix now includes ordinary later yields, guard-tag removal, opaque Rust error userdata identity,
duplicate and closed queues, stale leases, exact high-u64 cancellation ownership, weak-key GC and retained-handle
VM release. Rebased cleanly onto main `749316a`, preserving the new wave,
avatar/runtime features and host-configurable resume_budget field/default/use. Unpublished prototype head
`e67a11f`; focused lib/task_threads prebuild passed in 14.09 s after 91.8 MiB local-package cleanup, with both
source paths verified.

First actual run: the private cancellation/GC unit passed; fourteen of fifteen integration cases passed.
The remaining failure was a fixture-only print separator assumption (space versus tab), not a product red.
Changed that assertion to a single concatenated output argument. Corrected rerun: all fifteen integration
cases passed in 0.13 s after a 7.04 s incremental compile, including the exact native coroutine/error/frame
snapshot and opaque Rust error-userdata identity. No production change was needed for that correction.

Independent source review then found a real prototype ownership bug: cleanup on `status != Resumable` also
removed Running/Normal thread state after a failed self/parent resume. Separate regressions reproduced both
cancellation escapes (`armed`, `escaped`, `control`), with the unrelated delayed control working correctly.
Inspected the pinned mlua ThreadStatus enum and narrowed cleanup to terminal Finished/Error states only.
After the fix, all three private ownership tests passed (including both cancellation cases), and all fifteen
non-smoke integration cases passed again in 0.03 s. These two valid product reds are separate from the print
fixture correction above.

The precompiled 1,000-coroutine smoke then passed under the unchanged default 100 ms budget: 10.3084 ms for
500 create/resume and 500 wrap coroutines, each yielding and returning nil/multiple values. This is one host
observation with unmeasured background activity, not a speedup or universal latency claim.

Next: finish independent source review, refresh main before final gates, then complete full guarded
tests/Clippy/licenses/fmt. No public task APIs added; do not merge.
