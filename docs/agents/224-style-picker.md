# #224 Style picker popover (L)

Goal: the building style picker of `docs/design/building-tools.md` 12.5 (mockup `docs/design/building-tools/style-picker.html`).

## Done
- `crates/metallic_studio/src/style_picker.rs`: `StylePicker` (480 x 360 popover, 4 x 2 tiles of 104 x 78, `inherit` first
  as "Place Look" over the place Look's name, the 12.3 note, Apply to Model / Each part, "Hover previews · Enter applies").
- Action `style.open` (L over the viewport, contextual toolbar Style button, Properties Style row); disabled while playing
  or with nothing selected.
- Hover 150 ms or Space previews through `state.preview`; leaving the tiles restores; click or Enter commits one undo step
  "Style: Retro / PS1 (1 model)"; Esc cancels and closes (the selection is unchanged). A banner in the viewport says what
  is previewed.
- Thumbnails: `ThumbnailSource` trait; `GpuThumbnails` renders an isolated deep copy of the selection with
  `Renderer::capture` at 208 x 156, one per frame, cached for the opening; placeholders read "Rendering". Pixel 3D and
  Retro / PS1 thumbnails are pixelated with 3-pixel blocks.
- The popover shows on the frame it opens (its egui area state is seeded, skipping the invisible sizing pass).
- Keyboard: arrows, Space, Enter, Esc. AccessKit: each tile a radio button named "Retro / PS1: wobbly vertices, ...".
- Properties: `LookOverride` is a "Style" row in Appearance ("Inherit · Realistic") that opens the picker.
- Explorer: a 12 px style glyph (with the name on hover) on rows with their own override.
- Tests: `tests/style_picker.rs` (5) and unit tests in the module (5). Screenshot: `docs/agents/224-shots/style-picker.png`
  (`cargo run -p metallic_studio --example style_picker_shot -- out.png hover`).

## Not done / next
- Per-part style drawing is #223 (open) on top of #398 (open): until then the viewport preview and the thumbnails use the
  place Look's shading (thumbnails differ only by the pixel blocks). The value written and the undo step are final.
- View > Shading > "Style overrides" debug view needs #223's style slots: follow-up.
- "Place Look..." link waits for the Look picker (#64); it is shown disabled.
- Ctrl K entry ("Style: Retro / PS1") waits for the command palette.
- When #380 lands: its gesture flag covers the preview (Save/Play/Undo wait), and its `register` play lock replaces the
  explicit `enabled` predicate here.
