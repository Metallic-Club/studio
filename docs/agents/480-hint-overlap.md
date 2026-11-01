# #480: Parkour controls hints overlap

Branch `studio/480-hint-overlap`.

## Goal
One controls row in Parkour: the player's key-cap plate drew over the template's own
"WASD move   Space jump   Shift run   Right-drag camera" text, bottom left.

## Cause
Two hints in the same corner:
- `templates/parkour` HUD ScreenGui had a `Controls` TextLabel (from `af6e158`, the 1A template, written by
  `crates/metallic_runtime/examples/build_parkour/level.rs`).
- `metallic_player`'s `ControlsHint` plate (`crates/metallic_player/src/menu/hud.rs`, from #254 / PR #323) shows Jump,
  Run and Menu in the player's own bindings for 8 s at the start and after any glyph change.

## Done
- The plate now also shows Move (W A S D composite, or the left stick on a pad) and Camera (Look; mouse movement held
  behind a button shows that button, so "right-drag" reads as the right mouse button; the right stick on a pad).
  Glyphs are built once per glyph change instead of looked up per frame.
- The template's `Controls` TextLabel is gone (generator, `main.mscene`, `tests/smoke.mscene`, edited by hand: the
  generator now also emits a `ServerStorage` service the committed scenes lack, which is not this issue).
- Tests: `menu::hud::tests` (Parkour gives one plate Move/Jump/Run/Camera/Menu with the right glyphs; gamepad shows
  sticks) and `parkour_template_has_the_required_content` (the HUD's only TextLabel is `Banner.Message`).
- Screenshots (`metallic-player templates/parkour --no-overrides --capture 150:<png> --frames 160`, shrunk):
  `480-hint-overlap/before-player.png`, `before-corner.png`, `after-player.png`, `after-corner.png`.

## Next
- `templates/parkour/preview.png` was not regenerated here (branch `studio/478-studio-templates` just did); re-capture it
  once both land.
- Studio's Play (F5) never drew the player's plate; with the template text gone, Parkour shows no controls row in
  Studio Play. If wanted, share the plate with Studio (follow-up).
