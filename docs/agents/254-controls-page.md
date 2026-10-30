# 254 Player Controls page: rebinding, glyphs, accessibility

Goal: spec `docs/design/input-actions.md` sections 8, 9 and 11 in the runtime and the Player (issue #254).

## Done
- Runtime `input/overrides.rs`: `PlayerOverrides` (bindings per action and device, per-action toggle, stick and mouse
  settings, assist options, glyph style) with a lenient JSON reader that keeps unknown keys, `apply(base)`, and
  `OverrideStore` (`<data>/input/<game>/local.json`, atomic write, empty overrides remove the file, a corrupt file is
  moved to `.json.corrupt`).
- Runtime `input/rebind.rs`: slots per device (2 to 4), conflict detection by `RebindGroup` (built-in `Metallic.System`
  context, group `System`, meets every group), Swap and Replace, required actions (Menu cannot be left without a key or a
  button), reset per action, tab and all.
- Runtime `input/glyph.rs`: our own glyph table for keyboard, mouse, Letters (A B X Y) and Positions (generic) face
  buttons; neutral words ("Bottom Button", "Left Bumper"); `metallic://glyph/...` image ids. Auto picks Positions for
  Sony and Nintendo vendor ids, Letters otherwise. No PlayStation shapes (owner question still open).
- `InputRuntime`: keeps the base config, `set_overrides`, toggles (press flips, clear on death), `presses`, cached
  `glyph`/`glyphs` (no allocation after the first lookup), `glyph_changes`.
- Player `menu/`: Escape or Start opens the menu (Resume, Controls, Leave game). Controls page with Keyboard and Mouse,
  Gamepad, and Hold and Toggle tabs; listening dialog with countdown, conflict dialog, reset with confirm and a 10 s Undo
  toast, sliders and switches for sensitivity, dead zone, invert X/Y, toggle every hold, hold to press, hold time.
  Fully usable with keyboard only, gamepad only or mouse. Glyphs drawn as vector shapes and text (`glyph_paint.rs`).
  A hint plate shows Jump, Run and Menu glyphs for 8 s at start and after any glyph change.
- Player data folder `data_dir.rs`: `%APPDATA%\MetallicPlayer` (never `%LOCALAPPDATA%\Metallic`), macOS Application
  Support, XDG elsewhere; `METALLIC_PLAYER_DATA_DIR` overrides. `--no-overrides`, `--menu main|keyboard|gamepad|assist`.
- Fixed a debug-build panic in `overlay.rs` (egui texture deltas dropped without `clear`).

## Not done
- Touch tab and touch glyphs (#252). Luau `GetGlyph`/`GetGlyphs`/`GlyphChanged` (follow-up). PlayStation shapes.
- Shared profiles across games (spec 8.4 open question); only the `local` profile exists.
