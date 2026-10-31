# 333 Touch follow-ups

Goal: issue #333, the follow-ups from the #252 review: role changes, a doc comment, the HUD banner under the menu
button, and a Touch tab on the Controls page (spec `docs/design/input-actions.md` 7.2 and 8.1).

## Done
- **Role change cancels a held press.** `TouchButtonDef` carries the action's `role`; `TouchSet::follow(old)` finds the
  same action only when path and role both match (it replaces `slot_of`). `InputRuntime::remap_touch_buttons` never
  moves a held value onto the menu slot (`TouchButton7` only pulses), so a held press whose action became `Menu` is
  released as a cancel (Released, no Triggered). `TouchControls::remap` drops the finger the same way.
- **`TOUCH_BUTTON_SLOTS` doc** now says 1 to 6 are the action buttons and 7 is the menu button.
- **GUI inset.** `Game::set_gui_inset(points)` moves the `ScreenGui` root down (as Roblox's `IgnoreGuiInset = false`
  does for its top bar). The Player sets it each frame from `touch_overlay::gui_inset`: below the menu button plus
  8 dp while touch controls show, 0 otherwise, eased with the controls' 140 ms fade. Only a menu button whose top edge
  is in the top quarter of the screen counts (a custom layout may put it at the bottom). The Parkour banner no longer
  covers the pause button; no script change.
- **Touch tab** (`Tab::Touch`, fourth tab, Q/E and LB/RB like the others; `--menu touch`):
  - Stick: Floating / Fixed (segmented; Left/Right or Enter; a click selects the clicked segment, as does the glyph
    style's).
  - Opacity: 25% to 100%, 5% steps.
  - Button size: 80% to 150%, 10% steps (`DeviceProfile::scale`, `CONTROL_SCALE_RANGE`); never below 48 dp, and the
    ring radii grow to keep the 8 dp gaps.
  - Large controls: the spec's preset (every control at least 15 mm / 24 mm).
  - "Reset this tab" resets only the touch settings. Saved as `"touch": {"stick", "opacity", "size", "largeControls"}`.

## Tests
- `input::touch::tests::a_role_change_cancels_a_held_press`
- `gui::tests::a_top_inset_moves_the_screen_down`
- `touch_overlay::tests::the_gui_starts_below_the_menu_button_while_touch_shows`,
  `the_players_touch_settings_drive_the_overlay` (size and Large reach the profile)
- `menu::controls::tests::the_touch_tab_sets_stick_opacity_and_size`, `tabs_cycle_and_the_hold_switch_toggles`
- `layout_keeps_every_rule_on_every_preset` now also runs at 0.8x and 1.5x.
- Review fixes: `a_menu_button_at_the_bottom_leaves_the_gui_alone`, the inset easing in
  `the_gui_starts_below_the_menu_button_while_touch_shows`, `the_touch_tab_works_with_the_mouse`,
  `a_click_selects_the_clicked_glyph_style`, `a_nan_button_size_loads_as_the_default`.

## Screenshots
- `docs/agents/333-shots/touch-tab.png`: the Touch tab (Fixed, 60%, 120%), cropped and small.
- The phone emulation and banner shots are in the PR body (commit 71656e60), not in the tree.

## Open questions
- Swap sides, "Edit layout" and preset layouts (rest of spec 8.1's Touch tab) are not built.
