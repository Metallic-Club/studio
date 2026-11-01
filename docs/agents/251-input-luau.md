# 251 Luau: InputContext, InputAction, ContextActionService and Vector2

Goal: spec `docs/design/input-actions.md` section 5 in the script crate (issue #251).

## Done
- `Vector2` (`new`, `X`, `Y`, `Magnitude`, `Unit`, `+ - * /`, unary minus, `==`), also allowed in RemoteEvent values.
- `Enum` global: `KeyCode`, `UserInputType`, `UserInputState`, `InputActionType`, `InputTrigger`, `TouchSlot`,
  `ContextActionResult`, `PreferredInput`, `ContextActionPriority`. #80 is still open and not on main, so the names are
  added here (Roblox names) for #80 to reuse.
- `InputAction:GetState()`, `GetHoldProgress()`, `Pressed`, `Released`, `Triggered`, `StateChanged`; the server errors with
  `InputAction can only be used from a LocalScript`.
- `ContextActionService`: `BindAction`, `BindActionAtPriority`, `UnbindAction`, `SetTitle`, `SetImage`, `SetPosition`,
  `GetButton` (nil). Hidden `InputContext`/`InputAction`/`InputBinding` instances under the local player.
- `UserInputService`: `GetLastInputType`, `LastInputTypeChanged`, `PreferredInput`, `TouchEnabled`, `KeyboardEnabled`,
  `GamepadEnabled`. Both services are Lua objects, not world instances (no core class change).
- Host API on `ScriptHost` (`input_action_event`, `set_action_state`, `set_input_devices`, `set_preferred_input`,
  `set_last_input_type`, `take_bound_results`). 16 events per step, queue of 1024.
- `crates/metallic_script/tests/input.rs` (21 tests, including the spec 5.5 dash example); a runtime test keeps
  `KEY_CODES` equal to the `Control` list. `docs/luau-api.md` updated.

## Verified
`cargo fmt`, clippy with `-D warnings` and tests for `metallic_script` and `metallic_runtime`.

## Wiring (added after review)
- `Game::input_frame(&mut InputRuntime, now, dt) -> FrameInput` replaces the Player's sync/frame/drain: it feeds events and
  action values of scene `InputAction`s to `ScriptHost`, runs the client handlers at once
  (`ScriptHost::dispatch_input_now`, max 16 events, the step then continues the same 0.25 s budget), applies the
  Sink/Pass answers, and returns the `FrameInput`.
- `InputRuntime::set_script_sink`: a Sink answer blocks the controls of the bound action for lower contexts while the
  action is down, and lower actions are re-evaluated the same frame (so the default jump is released before the character
  reads it). Pass, no answer yet (over budget, yielded) or an action already up: no sink.
- Tests: `crates/metallic_runtime/tests/input_scripts.rs` (Pass jumps, Sink does not, scene action reaches its script) and a
  zero-allocation check of `input_frame` in `tests/input_alloc.rs` (idle handlers; Luau values for a connected handler
  allocate, as before).

## Verified
fmt, clippy `-D warnings` and tests for `metallic_script`, `metallic_runtime`, `metallic_player`.

## Deferred / deviations
- A tap shorter than one frame cannot be sunk (the lower action already saw it).
- `BindAction` with `UserInputType.Gamepad1` to `4` binds all gamepad buttons (not sticks); `Keyboard` binds every key.
  `InputBinding` has no device slot, so `Gamepad1` to `4` are the same.
- A bound handler that errors or yields before returning records no result (the key passes).
- `GetButton` returns nil (no touch overlay).
- The `inputObject.KeyCode` of a bound handler is `Unknown`; `Position` carries the action value.
