# UI overhaul L1: Launcher pages

Goal: task L1 of `docs/research/2026-10-30-ui-overhaul.md`. The Launcher's bundled pages draw with
Archivo and JetBrains Mono with the network off, follow a theme switch, and match section 3.7.

## Done
- `tools/gen_web_fonts.py` writes WOFF2 copies of `crates/metallic_ui_theme/assets/fonts/{Archivo,JetBrainsMono}-Variable.ttf`
  next to them (deterministic; `--check` compares). `local_pages.rs` serves them at `/fonts/*.woff2` with their licences.
- `theme.js` (in `<head>`) sets `data-theme` before the first paint from page storage; `pages.js` copies the
  settings file's `theme` over it when they differ. The settings page has a Graphite / Titanium / Light / System
  segmented control that applies and saves at once, like Studio's.
- `LauncherSettings::theme` (`Theme` enum, default Graphite, unknown word reads as the default). Bundled pages may
  read and set it; site pages may not.
- `pages.css` follows 3.7: button sizes and states, inputs (hover, focus, disabled, error), label/help spacing,
  segmented control, status lines with Phosphor icons, no uppercase tracked labels, card with `cut-lg`.
- Screenshots: `docs/design/launcher-pages/*.png` (headless Edge, DNS blocked, a scratch server with the same CSP).

## Commands
- `python tools/gen_web_fonts.py [--check]` (needs `pip install fonttools brotli`).
- `cargo test -p metallic_launcher`.

## Open
- The native window background before the first paint is still Graphite in every theme (`shell.rs` `background()`).
- Greek and Cyrillic fall back to the system font on the pages (Noto Sans is not shipped as WOFF2).
