# #79 part 2: clickable UI

TextButton and ImageButton with Roblox's button events, GuiObject input events, hit-testing and input sinking. Part 1
(TweenService) is PR #389.

## Classes (`metallic_core/src/classes.rs`)
- `Frame` gains `ZIndex` (1), `Active` (false) and `ClipsDescendants` (false); `ScreenGui` gains `DisplayOrder` (0).
- `GuiButton : Frame` (`AutoButtonColor`), `TextButton : GuiButton` (`Text` "Button", `TextColor3`, `TextSize` 14),
  `ImageButton : GuiButton` (`Image`, `ImageColor3`, `ImageTransparency`). Roblox's GuiButton derives from GuiObject;
  `Frame` plays GuiObject here, so a button `IsA("Frame")`. A class cannot override an inherited default, so buttons
  always take clicks whatever their `Active`.

## Flow
1. The host (the Player window) gives the left mouse button, the cursor and fingers to `Game::gui_pointer` first, in
   GUI points. Fingers on the touch overlay's own buttons skip the GUI. The menu, when open, takes everything.
2. `gui_input::GuiInput` lays the GUI out without text (`gui::layout_into`, reused buffers), hit-tests it and keeps
   hover and press state. It returns true when a press is the GUI's; the host then keeps that press, its moves and
   its release from the `InputRuntime`.
3. Its `GuiEvent`s go to `ScriptHost::gui_event` at the start of `Game::frame` and fire in the client VM during the
   first tick's step (after inputs, before Heartbeat).
4. `gui::paint_shaded` draws buttons darker while hovered (0.85) or pressed (0.7) when `AutoButtonColor` is set, and
   clips with `ClipsDescendants`.

## Follow-ups
- Gamepad selection (`GuiService.SelectedObject`, D-pad navigation), `Modal`, `MouseButton2*` events, double-click
  counts, `ZIndexBehavior.Global`, drawing `ImageButton.Image` (needs GUI textures), re-hover when the GUI moves under
  a still mouse, Studio's Play mode routing (it hosts a `Game` too and should call `gui_pointer`).

## Tests
- `metallic_runtime/src/gui.rs`: ZIndex order and overlap, `Active`, `DisplayOrder`, `ClipsDescendants`, button text.
- `metallic_runtime/tests/gui_buttons.rs`: click order and once-only firing, ZIndex overlap, release off the button,
  touch tap and cancel, clicks off buttons, hidden buttons, and a click on a button not reaching a
  `ContextActionService` action bound to MouseButton1 (routed like the Player window).
