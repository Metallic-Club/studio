# 319 wire editor panels

Goal: connect the merged Input Actions panel, colour popover and building-tool cores to the Studio editor.

Done:
- `editor_input_actions.rs`: View > Input actions (and the IA toolbar toggle) docks the panel on the right. Panel
  edits become `Command::Step(EditStep)` in the editor history; `History::take_remaps()` forwards rebuilt ids to
  `follow_remap`. Ctrl+Z over the panel undoes in the editor.
- `editor_colour.rs`: the Properties Color3 swatch (or C in the viewport) opens the popover. Preview writes the world,
  commit is one Batch step, Esc restores. Eyedropper reads the hovered part (Alt: screen pixel).
- `editor_tools/`: Align, Distribute, Mirror, Mirror copy, Array (linear, radial, grid, path, drag handle), Scatter
  brush (B), Measure (Shift+M), Select similar (Shift+G), lasso/paint modes (Q cycles). Each op commits one step.
- Tests: `tests/editor_wiring.rs` (one per tool family, Input Actions remap, colour popover).
- Picture: `cargo run -p metallic_studio --example editor_tools_shot -- out.png` (`319-shots/editor-tools.png`).

Next: tool previews could move to #337's preview-overlay path once it merges. Mouse-wheel array count is not wired.
Brush ownership means "inside `<parent>/<set>` folder" until tags (#78) exist.
