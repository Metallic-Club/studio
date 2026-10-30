# #221 Colour popover: progress note

Branch `studio/221-colour-popover`. Spec: `docs/design/building-tools.md` section 11; mockup
`docs/design/building-tools/colour-popover.html`.

## Goal

The building colour popover as a self-contained egui widget, with its colour maths and persistence, ready for the
Properties panel and preview overlay to host. #213 (overlay, batched `SetProperties`) and #10 (editing, Properties)
are still open, so nothing is wired into a Properties panel yet; a demo screen stands in.

## Done (implemented and tested)

- `crates/metallic_studio/src/colour/maths.rs`: `Rgb8`, `Hsv` (keeps hue on greys), HSV/RGB, hex and CSS-name parsing
  (148 names, `css_names.rs`), OKLab/OKLCh, gamut fitting by chroma reduction, the five harmony rules, nearest CSS
  name by OKLab distance, undo names, typed expressions for number fields.
- `palette.rs`: `RecentColours` (12, newest first, user config `recent-colours.toml`) and `PaletteStore`
  (`<project>/palettes/<name>.palette.ron`, atomic writes, name validation incl. Windows device names, read-only Theme
  palettes).
- `popover.rs`: `ColourPopover` 320 x 476: ring 248/18, square 146, markers per 11.4, old/new chip (click restores),
  Hex/RGB/HSV fields (scrub on the letter, expressions, Enter commits), eyedropper toggle (I) with
  `EyedropperClick` events and `pick` / `pick_screen_pixel`, Recent row, palette row (select, new, rename, delete,
  add, remove, drag or Ctrl+arrows to reorder), harmony row (click applies, Shift+click saves). Tab order and keys as
  11.4, focus trap, 2 px `theme::FOCUS` rings, AccessKit nodes ("Hue" / "218 degrees, close to ...").
  Events: `Preview`, `PreviewHueShift` (Alt-drag on ring), `Commit` (one per gesture, 600 ms pause for keys),
  `CommitHueShift`, `Cancel` (Esc), `Closed`.
- `demo.rs` + `--scene colour`: a demo Part and a Properties-style Color row; shows the undo names.

## Next (after #10 and #213)

- Open from the contextual toolbar, Properties `Color` row and C; map events to the overlay and `SetProperties`.
- Eyedropper in the viewport: `Color` property on click, rendered pixel on Alt, Ctrl pick-all and paint mode, 24 px
  loupe. Today every click picks the Studio window pixel through a screenshot.
- BrickColor tab: needs a sourced, licence-checked list of the named palette (spec says the count is unverified).
- Theme role palettes: `PaletteStore::add_read_only` is ready; Themes are not in Studio yet.

## Commands

```
cargo test -p metallic_studio -j 2
cargo clippy -p metallic_studio --all-targets -j 2 -- -D warnings
cargo run -p metallic_studio -j 2 -- --scene colour
cargo run -p metallic_studio -j 2 -- --screenshot shot.png --scene colour
```

## Open questions

- The spec's example says #5280CC has hue 218; it is 217.4 (fields show 217). The AccessKit test steps the ring once
  to reach 218.
- Complementary row: chosen as darker base, base, complement, lighter complement (L +/- 0.15).
