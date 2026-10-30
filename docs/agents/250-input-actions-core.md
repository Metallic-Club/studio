# 250 input actions core and keyboard/gamepad on the Player

Goal: phase 1 of `docs/design/input-actions.md` (spec sections 3, 4.1, 4.2, 4.4 and 6): the headless input-action core in
`metallic_runtime::input`, fed by the Player's winit and gilrs sources, replacing `Action` and `InputState`.
`MoveInput` and `Game::frame` are unchanged (`FrameInput` stays the boundary).

## Done
- Class registry: `InputContext`, `InputAction`, `InputBinding` with the properties of spec 3.2 to 3.4
  (`crates/metallic_core/src/classes.rs`).
- `crates/metallic_runtime/src/input/`: `control.rs` (devices, `Control` names like Roblox `KeyCode`, `RawEvent`,
  `RawSource`, bounded `InputQueue`), `config.rs` (`InputConfig::from_world`, built-in `CharacterControls`),
  `runtime.rs` (`InputRuntime`: `push`, `evaluate`, `state`, `drain_events`, `sync`, `frame_input`, last-device rule),
  `script.rs` (`ScriptedSource`, the `--input-script` format), `tests.rs` (unit tests).
- Player: `devices.rs` (winit key/mouse mapping, gilrs `GamepadSource` with the spec 4.2 table), `window.rs` and `main.rs`
  use `InputRuntime`; `--input-script <file>` in windowed and headless runs (1/60 s of input time per frame).
- Parkour route test now sends raw events (gamepad stick plus keyboard buttons, and a gamepad-only variant), records them
  as a script and replays the script.
- `tests/input_alloc.rs` (counting allocator, zero allocations in push/evaluate/drain on 96 bindings), bench
  `benches/input_eval.rs`. `ARCHITECTURE.md` has an "Input flow" section; crate docs updated.

## Deviations from the spec (also listed in the PR)
- `evaluate(now, dt)` returns nothing; `frame_input()` (or `frame(now, dt)`) gives the `FrameInput`. Actions are found by
  `find_action("Context/Action")` once and used by `ActionId` index.
- `Vector2Scale` is stored as a Vector3 property (z ignored) because the `Vector2` value type arrives with phase 2 (#251).
- `Deadzone` property default is -1 meaning "the control's default" (0.18 sticks, 0.05 L2/R2, else 0); the spec lists the
  defaults but not how an instance property expresses them.
- `KeyCode` and the composite/modifier fields are plain strings checked by the runtime (unknown names become
  `InputRuntime::warnings()` and the field is ignored), not an enum in the registry.
- `ClampMagnitudeToOne` is applied to sticks and composites only, so `MouseDelta` look is not clamped (as today).
  Direction1D composites use `Up`/`Down`.
- Escape still quits the Player directly; the built-in `Metallic.System`/`Menu` context (spec 3.6) is not built (no Player
  menu yet).
- Not built from section 3: toggle (11.1, phase 5), `PreferredBinding`, touch layout behaviour (phase 3), `TouchButton(action)`
  virtual controls (only a generic `TouchButton` control exists), `GetGlyph*` (phase 5), `BindAction`/Luau (phase 2).
  The GUI-processed rule (3.7.1) exists as `InputRuntime::set_gui_capture(pointer, text_focus)` but nothing calls it yet
  (no focused `TextBox`/GUI button hit-testing in the Player).
- Spec 4.4 "mouse movement within 200 ms" is measured from `MouseDelta` or, when the cursor is free, `MousePosition`.
- Gamepad pad-family detection for glyphs (4.2) is not done (glyphs are phase 5).

## Next
- Phase 2 (#251) reads `InputAction` events through `drain_events` and `action_for_instance`.
- Phase 3 (#252) adds touch controls: `Device::Touch`, `TouchStick`, `TouchLook` controls already exist and count for the
  last-device rule.

## Commands that matter
```
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=<shared target> CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p metallic_core -p metallic_runtime -p metallic_player -j 2
cargo bench -p metallic_runtime --bench input_eval -j 2
cargo run -p metallic_player -- templates/parkour --headless --frames 300 --input-script <file>
```
Script lines: `30 keyboard Space down`, `36 keyboard Space up`, `40 gamepad0 Thumbstick1 axis 0.0 1.0`.

## Open questions
- Owner decisions of spec section 17 still apply; none blocked this phase.
