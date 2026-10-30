# 253 Input Actions panel (Studio)

Branch `studio/253-input-actions-panel`. Closes #253. Spec: `docs/design/input-actions.md` sections 8, 10, 11.3, 7.

## Done
- `crates/metallic_studio/src/input_actions/`: self-contained panel (no editor shell on main yet; docks in after #10).
  - `commands`: `StepBuilder` -> `EditStep` (set, create, destroy with sibling order and id remapping), `CommandSink`,
    `History` (stand-in undo stack, merging for nudges). One click, drag or preset is one step.
  - `capture`: Listen state machine (8 s timeout, Esc cancels, Backspace clears, chords, lone modifiers, sticks by
    largest magnitude, mouse motion never binds), conflicts in the same rebind group, Swap / Replace / Keep both.
  - `checks`: the section 10.1 checks table (errors first), touch rules on every device preset.
  - `presets`: add an action from a role, default and one-handed keyboard/gamepad presets.
  - `touch`: device presets, automatic layout, custom `TouchPosition`/`TouchSize`, 4 dp snap, 48 dp / 8 dp / safe area rules.
  - `preview`: Preview as (keyboard, phone, tablet, gamepad), touch simulation, virtual pad, Studio key routing.
  - `view`, `view_touch`, `style`: the egui panel (Bindings tab with tree, matrix, properties, checks; Device preview tab).
- Tests: 46 unit tests in the module, `tests/input_actions.rs` (save and reopen keeps a moved button, one undo
  restores it; headless frames of every tab and mode; Tab walks 30+ widgets).
- Screenshots: `docs/agents/253-shots/`.

## Decisions
- Scenes without an `InputContext` show the built-in CharacterControls read only, with "Add to scene" (one step).
- Studio has no gamepad backend; the host feeds pad events with `InputActionsPanel::feed_raw`.
- The touch overlay in the Player is #252; the panel owns the layout model until then.

## Next
- Dock into the editor once #10 lands: replace `History` with the editor's command stack via `CommandSink`.
- Reuse #252's overlay drawing in the preview once it merges.

Commands: `cargo test -p metallic_studio`, `cargo run -p metallic_studio --example input_actions -- --tab preview --mode phone --edit`.
