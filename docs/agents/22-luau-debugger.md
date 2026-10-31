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
- ModuleScripts: `require` tells the debugger about each compiled module chunk (breakpoints, frame names), and
  its `pcall` does not count as handling an error, so break-on-error stops at the module's error line.
- A breakpoint hit during a tick stops the frame's tick loop at that tick; the frame's leftover time is dropped.
  The avatar pose holds while paused.
- Step Over and Out across a yield (`task.wait()`) wait for that thread and stop at its next line; other threads
  run meanwhile. Step Into across a yield stops in whatever script code runs next.
- Code resumed after a break (coroutines, `task.spawn` spawners) and watch expressions run as the paused frame's
  script, so their connections and threads count against that script's quotas and stop with it.
- Single-step mode is cleared after each debugger resume. Setting a local to a value of another type is allowed
  (Luau is dynamic) with a warning in Output.
- Tests: `crates/metallic_script/tests/debugger.rs` (23), `crates/metallic_runtime/tests/debugger_pause.rs` (2).

## PR 2: Studio UI (`metallic_studio`)

Done, in `crates/metallic_studio/src/debugger/`:

- Breakpoints live in `EditorState::debug` (`DebugSettings`), survive Stop and reach the next Play through
  `PlayStart::debug` before any script runs; during Play they also go to the game via `PlayView::debugger`.
- Breakpoints move with their lines: each edit in the script editor diffs the old and new text
  (`shift_lines`); lines below an insertion or deletion shift, a deleted line loses its breakpoint.
- Saved per place in Studio's settings, `breakpoints.toml` under `%APPDATA%\MetallicStudio` (`store.rs`), keyed
  by the scene file and each script's full name; loaded when a game is opened. Screenshot and test editors
  neither read nor write it.
- Gutter: click toggles, right-click adds a condition or log message; the paused line gets an arrow and a band.
  Wrapped lines: the gutter matches the text field's height and a line's click area covers all its rows.
- Keys: F9 toggle (cursor line), F5 continue when paused (Play otherwise), F10 over, F11 into, Shift+F11 out.
  The step keys are in the Play context, so when not paused they still reach the game. During Play, F9 is
  Studio's only while a script editor has focus (the editor is read-only then but takes a cursor); otherwise the
  game gets it. `debug.toggle_breakpoint` is a normal action (works from the command palette, #402).
- Debugger pane under the viewport during Play: status and step buttons, Break on errors, Call Stack (click a
  frame to select it and show its line), Variables (Locals/Upvalues/Globals tree, Set value on a local), Watch.
  The panes draw from shared (`Rc`) caches, so nothing is copied per frame.
- Hovering a name or field chain shows its value only when the shown script is the selected frame's script.
- Output gets one Studio line per pause and the editor jumps to it once; breakpoint edits and Set value while
  paused only refresh the panes. Log breakpoints print through the game's output.
- Screenshot: `metallic-studio --screenshot out.png --scene debugging` (Parkour, paused in the checkpoint
  script); see `22-luau-debugger-ui.png`.
