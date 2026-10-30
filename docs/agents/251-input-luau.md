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

## Deferred / deviations
- `Game` does not yet feed `InputRuntime` events and states into `ScriptHost`, and the runtime does not yet use the
  `Sink`/`Pass` answers (`take_bound_results`) for arbitration. The script side and host API are ready for that wiring.
- `BindAction` with `UserInputType.Gamepad1` to `4` binds all gamepad buttons (not sticks); `Keyboard` binds every key.
  `InputBinding` has no device slot, so `Gamepad1` to `4` are the same.
- A bound handler that errors or yields before returning records no result.
- `GetButton` returns nil (no touch overlay).
- The `inputObject.KeyCode` of a bound handler is `Unknown`; `Position` carries the action value.
