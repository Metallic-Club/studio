# 404: `RBXScriptSignal:Wait()` and `:Once()`

Branch `studio/404-signal-wait-once`, worktree `metallic-studio-worktrees/404-signal-wait-once`.

## Goal
`signal:Wait()` yields until the next fire and returns its arguments; `signal:Once(fn)` connects for one call.
Acceptance list: issue #404.

## Done (implemented, tested)
- `crates/metallic_script/src/api/signals.rs`
  - `Hub` keeps `waiters: SignalKey -> [SignalWaiter { thread, owner }]` next to the connections.
  - `Wait` is Luau (`=[engine]`): if `coroutine.isyieldable()` is false it calls `yield()` anyway, so the error is
    the one `task.wait` gives; otherwise a guarded Rust `register` adopts the thread in the scheduler (so
    `cancel_owner` blocks it) and parks it on the signal, then it yields.
  - `fire` snapshots the handlers and takes the waiters before dispatching. Handlers run first, then each waiter is
    resumed through `Budget::resume` with arguments built by the same `args` closure (fresh per waiter). Waits begun
    during the fire land in a new list, so they resume on the next fire.
  - Waiter resumes share the handler path (`dispatch_call` / `Deferred { call: Call::Handler | Call::Resume }`), so
    they are deferred to the next step when the step budget is spent; if the deferred queue is full the remaining
    waiters are put back on the signal.
  - Cleanup: `remove_owner` (via `stop_owner`) drops an owner's waiters; `prune` (via `prune_hub`) drops waiters of
    dead instances. Before resuming, `can_resume_parked` skips threads cancelled meanwhile.
  - `Once` = `Connect` with `once: true`; `fire` sets `alive = false` and removes the connection before the call
    (the call itself gets a fresh alive flag so a deferred call still runs).
- `task_lib.rs` / `vm.rs`: `adopt` and `can_resume_parked` on `Scheduler` and `Budget`.
- `host.rs`: `#[doc(hidden)] ScriptHost::signal_waiters()` counter for the leak tests.
- Tests: `crates/metallic_script/tests/signal_wait.rs` (9 named in the brief + a non-yieldable-context check).
- Docs: `docs/luau-api.md` (Signals), ARCHITECTURE.md (metallic_script bullet).

## Next
- Review, PR (lead). No tween `Completed` signal exists yet; it will get `Wait`/`Once` for free as a `LuaSignal`.

## Commands
```
cargo test -p metallic_script --test signal_wait -j 4
cargo test --workspace -j 4
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo deny check licenses
cargo fmt --all --check
```

## Open questions
- A user coroutine that waits and is then resumed by hand is not blocked from that manual resume (same as
  `WaitForChild` today); the signal fire then resumes it at whatever yield it is in.
