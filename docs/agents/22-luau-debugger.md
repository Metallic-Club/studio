# #22 Luau script debugger (Task 15)

Branch `studio/22-luau-debugger`. Two PRs: (1) VM hooks and the pause/step/inspect core, (2) the Studio UI.

## PR 1: core (`metallic_script`, `metallic_runtime`)

- `crates/metallic_script/src/debug/`: `mod.rs` (hooks, pause chain, resume, raw resume), `breakpoints.rs`
  (per-owner breakpoints, `lua_breakpoint` patching, conditions and logpoints), `inspect.rs` (frames, locals,
  upvalues, globals, expand, evaluate, set_local), `api.rs` (`Debugger` on `ScriptHost`).
- Pausing breaks the thread (`lua_break`): no blocking, so Studio's UI keeps running on the same thread.
  mlua's `Thread::resume` treats `LUA_BREAK` as an error, so the scheduler resumes through `debug::raw_resume`
  while the debugger is armed.
- A `task.spawn` thread that breaks makes its spawner break right after `task.spawn` returns (the Luau wrapper
  calls a C function that `lua_break`s). A coroutine that breaks breaks its resumer (Luau's corolib rule).
- Breaking needs every thread from the hook down to the scheduler-resumed one to be yieldable (tracked with the
  `preresume`/`postresume` callbacks); code running in a host `pcall` on the main state never pauses (its
  `task.spawn` threads can).
- Resuming from a hook break runs that instruction's hook again: skipped once (`Resumed`). Writes to the top
  frame's locals are applied in that hook, where Luau's pc is exact.
- Constant locals (`local x = 1`, never reassigned) are folded by the Luau compiler: `set_local` cannot change them.
- Scripts compile with debug level 2 (local and upvalue names).
- Zero overhead: callbacks are installed only while armed; unarmed resumes use mlua's path unchanged. Unit test
  `hooks_are_installed_only_while_needed`. The instructions gate (`perf_ci`) covers only `metallic_core`, and there are
  no script benches.
- Tests: `crates/metallic_script/tests/debugger.rs` (18), `crates/metallic_runtime/tests/debugger_pause.rs`.

## PR 2: Studio UI

Planned: gutter breakpoints in the script editor (click toggles, right-click condition/log), F5 continue, F9
toggle, F10 over, F11 into, Shift+F11 out, Call Stack / Variables / Watch panels, Output lines, Stop while paused.
