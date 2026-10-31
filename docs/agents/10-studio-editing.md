# Issue #10: Studio editing (branch `studio/10-studio-editing`)

Goal: the editor itself: viewport, selection, gizmos, Explorer, Properties, Output, script tabs, undo/redo, save.
Plan: `docs/superpowers/plans/2026-10-30-engine-m1.md` Task 10; UI: `docs/research/2026-10-30-studio-ui-design.md` 5.3-5.5, 5.9, 5.15.

## Done
- `EditorState` (no UI) + `EditorUi` layout; the start screen opens a game into it (`app.rs`).
- `commands.rs`: `SetProperty`, `Batch`, `Create`, `Delete`, `Reparent`, `Duplicate`, `Group`; `History` with apply,
  record (for live-preview gestures), undo/redo, 200-step limit, save marker. Delete and undone creations *park*
  instances (detached roots) so undo restores the same ids; parked roots are destroyed when their command leaves the
  history, and skipped by Explorer, Save (`clean_world`) and the renderer.
- Viewport: `metallic_render` into a wgpu texture shared with egui (re-rendered only on world/camera/size change);
  fly camera (right-drag + WASD/QE, wheel speed), Alt-drag orbit, middle pan, wheel dolly, F / Home framing;
  click select (Model first, second click the part), Ctrl toggle, Alt-click cycle, box select; outlines.
- Gizmos (`gizmo.rs`, own code): move arrows and plane squares, rotate rings with swept arc, scale face handles
  (one face moves, 0.05 floor, Ball keeps equal axes); snapping relative to start, Ctrl inverts, Shift one tenth;
  Esc / right click cancel; one undo step per drag.
- Explorer (virtualised, filter, rename F2, drag to reparent, context menu with Insert), Properties (sections,
  Mixed, expressions, live preview + one step), Output (tags, levels, filters, repeat collapse), script tabs.
- Seams for A12 / #11: `actions.rs`, `keymap.rs`, `output.rs` (`OutputSink`), `play.rs` (`PlayMode`).
- Styling: panels use `tokens.rs` `ui_tokens()` only; `theme.rs` and fonts are untouched (owned by the lead, S1/S2).
- Screenshot: `docs/agents/10-shots/editor.png` (`--scene editor`, Parkour template, nothing saved).

## Tests
- `tests/commands.rs`: the five plan tests plus reparent, insert, history limit, batch failure.
- `tests/editor_ui.rs` (egui_kittest, no GPU): click / Model-first selection, gizmo drag snaps to whole studs as one
  step, Ctrl+Z, Esc cancels, tool and Delete keys, Alt-click cycle.
- Unit tests in transform, camera, picking, gizmo, expr, keymap, output, play, actions, properties, script_editor.

## Commands
```
export CARGO_TARGET_DIR=<shared target> CARGO_INCREMENTAL=0
cargo fmt --all -- --check
cargo clippy -p metallic_studio --all-targets -j 2 -- -D warnings
cargo test -p metallic_studio -j 2
cargo deny check licenses
METALLIC_TEMPLATES_DIR=templates target/debug/metallic-studio --screenshot out.png --scene editor
```

## Next / open
- Play (#11) registers a `PlayMode` with id `play.start`; the toolbar button enables itself.
- Not in this PR: docking (egui_tiles), command palette (Ctrl K, #128), Shift+A insert at cursor, keymap presets
  and Settings > Keyboard (#129), copy/paste, viewport grid, the renderer's outline pass (outlines are drawn as
  projected box edges with egui).
- Building tools #213-#215 build on `EditorState::record`, `Selection`, `GizmoDrag`.

## Merge with main 2804d761+ (re-review)
- Properties shows CFrame values as position plus orientation and skips `hidden` properties.
- Size writes from Properties and the scale handles stay inside core's 0.001-2048 limits
  (`metallic_core::transform::clamp_part_size` / `MAX_PART_SIZE`).
- Close game and the window close flush open script tabs before checking for unsaved changes.
