# UI overhaul S6: Studio title bar

Goal: task S6 of `docs/research/2026-10-30-ui-overhaul.md`. Studio draws its own 40 px title bar
on Windows, and Windows 11 snap layouts, double-click maximise, drag, edge resize and the
unsaved-changes guard on close all keep working.

## Done
- `src/title_bar.rs` lays out and draws the bar: the emblem, the game's name, a centred search field
  (disabled), Settings, and minimise, maximise/restore and close. It publishes hit regions in
  client pixels. A subclassed window procedure answers `WM_NCHITTEST`:
  - `HTCAPTION` on the empty bar;
  - `HTMAXBUTTON` on maximise;
  - the edges for resizing;
  - `HTCLIENT` for egui's controls.

  The subclass handles maximise presses itself and tracks the hover with `TME_NONCLIENT`.
- `WM_NCCALCSIZE`: while the window is restored, the subclass returns 0. winit's undecorated window keeps
  a 1 px non-client strip at the top, and with that strip Windows never shows the snap flyout. This was
  reproduced with a plain WinForms window that answers `HTMAXBUTTON`. With a 1 px strip it gets no flyout;
  with none it does.
- The splash hooks the window on the first frame. `main.rs` and `splash::become_main_window` turn
  decorations off on Windows only. macOS and Linux keep the native frame (`CUSTOM` is false).
- Close sends `ViewportCommand::Close`, so it goes through the existing unsaved-changes guard.
- New token `caption-close` (`#c42b1c`, the same in every theme, with an `on-danger` glyph). The
  contrast test covers it, and the CSS has been regenerated.
- Screenshots: `docs/design/studio-title-bar/`.

## Checked live (Windows 11 26200, 96 DPI)
- Hit codes: caption 2, maximise 9, controls 1, edges 10/11/12/15/17.
- Double-click maximises and restores; clicking maximise maximises and restores.
- Dragging the right edge resizes; dragging the bar moves the window; close closes.
- Hovering maximise opens snap layouts (4 of 4 runs).
  - The test harness must not click the caption first. After a caption click, Windows often skips
    the flyout for the next hover, and the WinForms control window does the same.

## Open
- The search field is a disabled placeholder. When #402 (command palette) lands, the field should call
  `EditorUi::open_command_palette()`. Ctrl+K is `snap.grid` in the editor today, so the shortcut is only
  taken on the start screen.
- No Help button: there is nothing to open yet.
- Not checked at 125 % or 150 % scaling, or on a second monitor.
