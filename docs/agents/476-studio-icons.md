# #476 Studio icons: real toolbar icons and per-class Explorer icons

Branch `studio/476-studio-icons`, worktree `../metallic-studio-worktrees/476-studio-icons`. Part of #454 (problems 1 and 5).

## Goal
Replace the `Icon::Text` letter placeholders (top bar `IA B M Al Mi Ar Si`, the scene toolbar's `T`, hidden
command actions) with icons drawn from egui shapes in `crates/metallic_studio/src/icons.rs` (no icon font), and give
Explorer and palette object rows an icon per class with a superclass fallback.

## Done
- `icons.rs`: 34 new `Icon` variants (Transform, Brush, Measure, Align, Distribute, Mirror, MirrorCopy, Array,
  SelectSimilar, SelectBox/Lasso/Paint, Gamepad, PivotMode, Pivot, Drop, Duplicate, Delete, Group, Rename, SelectAll,
  Save, Close, Deselect, Settings, FrameSelection, FrameAll, Search, Commands, Insert, Colour, ContextBar, Keyboard,
  More). `Icon::Text` is removed: nothing needs text any more. `Icon::ALL` lists every variant.
- `ClassIcon` + `class_icon(class, shape)`: exact class first, then the nearest of GuiButton, Frame, Script, BasePart
  (`metallic_core::classes::is_subclass`), else `Generic`. Parts follow `Shape` (Block cube, Ball, Cylinder, Wedge).
  `paint_class` keeps its signature (palette rows, Properties header); `paint_class_icon` paints a `ClassIcon`.
- Explorer (edit and play trees) uses `row_icon`, which reads a part's `Shape`.
- Call sites: `actions.rs`, `editor_tools/mod.rs`, `editor_colour.rs`, `editor_input_actions.rs`, `toolbar.rs` (the
  overflow chevron), `viewport_header.rs` (its "»" text button now draws `Icon::More`). The toolbar no longer hides a
  glyph next to an accessory label (that only existed for text placeholders), so Pivot mode shows its icon too.
- `palette.rs` untouched: its object and Insert rows already go through `paint_class`.
- Tests: every `Icon` and `ClassIcon` paints inside a 16 px box (+1 px), the `ALL` lists are in order; every registry
  class maps to a non-generic icon (only `Instance` is generic) and the issue's classes map to their own icon; parts
  follow shape and superclass; Explorer `row_icon` follows Shape.
- Screenshots in `docs/agents/476-studio-icons/`: `before-toolbar.png`, `after-toolbar.png`,
  `before-palette-explorer.png`, `after-palette-explorer.png`, `icon-sheet-2x.png` (row 1 every `Icon`, row 2 every
  `ClassIcon`, in `ALL` order, 2x nearest-neighbour; drawn by a temporary sheet in the components scene, not committed).

## Next
- PR (combined batch per the conductor), screenshots attached.
- Hands-on: hover/selected tints of the new glyphs in Titanium and Light themes (only Graphite was captured).

## Commands
```
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_TARGET_DIR=S:/Projects/Active/Metallic/cargo-target-1e
cargo test -p metallic_studio -j 4 --lib -- icons:: explorer::
cargo clippy -p metallic_studio --all-targets -j 4 -- -D warnings
cargo build -p metallic_studio -j 4
METALLIC_TEMPLATES_DIR=templates <target>/debug/metallic-studio.exe --screenshot out.png --scene editor|palette|components --size 1440x900
```
The target dir is shared with other worktrees that build the same crate under the same artifact hash: `touch` this
crate's sources before a build or test run, copy the exe away before capturing, and check `--list` if a test seems
missing.

## Open questions
- Sky uses three short rays over a half sun; if that reads too close to a starburst, drop the rays.
