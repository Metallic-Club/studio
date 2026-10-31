# 319 wire editor panels

Goal: connect the merged Input Actions panel, colour popover and building-tool cores to the Studio editor.

Done:
- `editor_input_actions.rs`: View > Input actions (and the IA toolbar toggle) docks the panel on the right. Panel
  edits become `Command::Step(EditStep)` in the editor history; `History::take_remaps()` forwards rebuilt ids to
  `follow_remap`. Ctrl+Z over the panel undoes in the editor.
- `editor_colour.rs`: the Properties Color3 swatch (or C in the viewport) opens the popover. Preview goes through
  the preview overlay, commit is one step, Esc restores. Eyedropper reads the hovered part (Alt: screen pixel).
- `editor_tools/`: Align, Distribute, Mirror, Mirror copy, Array (linear, radial, grid, path, drag handle), Scatter
  brush (B), Measure (Shift+M), Select similar (Shift+G), lasso/paint modes (Q cycles). Each op commits one step.
- Tests: `tests/editor_wiring.rs` (one per tool family, Input Actions remap, colour popover).
- Picture: `cargo run -p metallic_studio --example editor_tools_shot -- out.png` (`319-shots/editor-tools.png`).

After merging main (#337): the colour popover previews through `EditorState::preview` and commits its
`SetProperties`; tool commits put their property changes in one `SetProperties` too.

Review fixes: the toolbar (`toolbar.rs`) collapses whole groups that do not fit into a "»" menu, lowest priority
first (Build, View, Snap, Settings, Edit; Tools and Play last), tested at 1280, 1600 and 2600 px. Ctrl+wheel over
the viewport changes the array count (linear, radial, path; clamped; the commit is still one step).
Second review (FIX-FIRST): Esc routing through keymap contexts (Popover, Tool) with the colour popover first; brush
strokes cancel on Esc, tool change, Play and focus loss; one gesture flag in `EditorState` (Save, Play, Undo, apply
and tool commits refuse during any gesture); tool commits check `edit_refusal()`; the viewport only yields the pointer
between gestures, so a release always ends its drag. Also: brush dab prefilter (`PickCache::near`/`first_in`),
paint/lasso trail cap and bounds prefilter, no per-frame plan/selection/toolbar allocations, overflow-menu
accessories disabled with their action. Tests in `tests/editor_wiring.rs`.

Brush ownership means "inside `<parent>/<set>` folder" until tags (#78) exist.
