# #118 Luau budget overhead

Status: implemented and tested locally; ready for review, not accepted or released.
Branch: `studio/118-budget-overhead`; started on `532c4f7`, rebased onto `937dcd8` for final checks.

## Goal and decisions

Reduce clock reads in hot Luau interrupt callbacks while preserving the 100 ms execution deadline, enclosing
budgets, sticky timeout handling, recovery and scheduler limits. Issue #6 is closed; the stale blocked label was
removed and acceptance refined before claiming this issue.

Blind every-1,024-interrupt polling is insufficient: a few native callbacks can each spend substantial time.
The installed Luau 740 source places interrupt points at CALL, RETURN and loop backedges. A passive host-only
worker therefore signals expiration independently of interrupt count. It owns no Lua, Function, Thread or
Scheduler handle and never changes a Lua callback pointer from another thread.

## Implemented

- One condition-variable worker per VM signals an atomic expired flag. The hot interrupt normally reads that
  flag and updates a small counter; a fallback reads the clock at the first interrupt and every 1,024 thereafter.
- The worker checks the current deadline under its mutex after every wake. An old timed wake cannot expire a
  newer budget. Arming/resetting expiration uses the same mutex. Nested calls do not reset the outer timer.
- Short calls can reuse an already scheduled earlier wake instead of waking the worker on every arm/disarm.
  The worker becomes idle after that pending wake; no busy-wait loop or per-resume thread creation.
- Scope guards restore/disarm deadlines on normal exit and Rust unwind. Return-boundary checks reject expired
  work even if no Lua interrupt occurred. Actual Lua timeout locations are retained; a host-only boundary does
  not invent a script location.
- VM drop/shutdown cancels pending tasks and stops/joins the worker. The state mutex is released before join.
  Budget handles reject work after VM drop even when trusted raw Lua access keeps the Lua state alive.
- Twelve new tests cover sparse clock checks, immediate expiration, old-wake isolation, join cancellation,
  host-only boundaries, unwind cleanup, slow native calls, inherited nested deadlines, repeated pcall timeouts,
  recovery and a retained raw Lua handle. All existing hostile-native/timeout/restart/lifetime tests pass.

## Resource and timing limits

Each VM adds a helper thread with a **requested** 128 KiB stack plus platform thread resources; actual virtual
stack commitment and resident memory were not measured. This has lifecycle cost, especially for short-lived VMs.
The worker sleeps when no deadline is active (after an already scheduled wake), and joins on drop/shutdown.
Native callbacks cannot be preempted by a Lua interrupt; they must return or enforce their own bound. Their time
counts against the active budget and the return boundary rejects an expired result. Host side effects are not
rolled back. OS scheduling can delay timer delivery, so these are not hard real-time guarantees.

A shared-process timer could reduce per-VM resources later, if measurements justify its additional lifecycle
and isolation complexity. This implementation does not claim an unconditional performance improvement.

## Measurements

Deterministic unit evidence: with a far-future deadline, 10,000 interrupt visits made **10** fallback clock
checks. Once expiration is signaled, the next visit aborts regardless of that counter. Worker/boundary clock
checks are separate and are not counted by that unit probe.

Reproducible probe: `cargo run --release -p metallic_script --example budget_bench -j 4 -- 100000`.
The example was added and run against the unmodified base VM before wiring the watchdog, then run again after.
Windows, Rust 1.99.1; 20 observed sample windows, setup batches of 10 and short-call batches of 1,000.
The desktop had concurrent build activity. These sample percentiles are observations, not confidence intervals.

| Probe | Clock-per-interrupt base p50 / p95 | Watchdog p50 / p95 |
| --- | --- | --- |
| VM create **and shutdown**, per VM | 0.739030 / 0.902840 ms | 3.321870 / 18.229280 ms |
| Budgeted short call | 1.4482 / 1.9416 microseconds | 1.4975 / 1.8913 microseconds |
| Budgeted 100,000-iteration arithmetic loop | 4.7629 / 6.1929 ms | 3.7885 / 5.5431 ms |
| Bare Luau arithmetic loop, same work | 2.0414 / 2.9792 ms | 1.8957 / 2.0160 ms |

Loop time decreased in this run; short-call medians were similar and VM create/shutdown became materially more
expensive and variable. The bare reference also changed between runs, demonstrating uncontrolled machine noise.
The combined lifecycle probe does not isolate construction from teardown or attribute all variation to the
watchdog. No FPS, quiet-machine speedup, or universal net win is asserted.

## Validation and next

- `cargo test -p metallic_script -j 4`: passed 78 tests, including all 12 new tests.
- `cargo test --workspace -j 4 -- --test-threads=1`: passed all 224 tests after rebasing, including full Parkour
  route/player smoke and all 78 script tests. Serial test execution limits contention in wall-time-budget tests.
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`: passed after rebasing.
- `cargo deny check licenses`: passed (one existing unused MPL-2.0 allowance warning).
- `cargo fmt --check` and `git diff --check`: passed.
- No dependencies or root Cargo configuration changed. No UI changes.
- The isolated worker target is `%LOCALAPPDATA%\Metallic\cargo-target-114`, reused sequentially after #114's
  local checks completed; no other worker uses it. Do not share compiled outputs with concurrent worktrees.
- Open/attach the scoped PR and inspect remote CI independently. Do not merge it here.

## Primary-source rationale

- [Luau sandbox interrupts](https://luau.org/sandbox/#interrupts): interruption is cooperative; long native C
  functions remain a limitation. Describes a watchdog approach used by the host.
- [mlua 0.12.2 Lua::set_interrupt](https://docs.rs/mlua/0.12.2/mlua/struct.Lua.html#method.set_interrupt): interrupt
  errors propagate through executing Luau; yield points have restrictions.
- [Luau performance](https://luau.org/performance/): separates interpreter and host integration overhead.
- Installed `luau0-src-0.22.0+luau740/luau/VM/src/lvmexecute.cpp`: interrupt sites inspected in CALL, RETURN and
  loop-backedge handlers. Installed `mlua-0.12.2/src/state.rs`: checked interrupt dispatch and its recursion guard;
  no third-party engine source copied.
- [Rust condition-variable waits](https://doc.rust-lang.org/std/sync/struct.Condvar.html#method.wait_timeout):
  state is rechecked under the same mutex after wake-up; no inference from a stale timeout result.
- [Rust thread stack requests](https://doc.rust-lang.org/std/thread/struct.Builder.html#method.stack_size) and
  [join](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html#method.join): request and lifetime semantics.
