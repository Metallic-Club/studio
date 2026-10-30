# Building the visual language in egui 0.37

Date: 2026-10-30. Status: guide for the implementer. Companion to `visual-language.md` (the what); this file is the
how. API names below were checked in the local sources of `egui-0.37.0` and `epaint-0.37.0`; nothing here has been
compiled yet.

## 1. File plan (small on purpose)

Replace `crates/metallic_studio/src/theme.rs` with a folder, and add a `widgets` folder for the few widgets egui cannot
style through `Visuals`.

| File | Contents | Size guess |
|---|---|---|
| `src/theme/mod.rs` | `apply(ctx)`: installs fonts, `Visuals`, `Spacing`, `TextStyle`s. Keeps `contrast_ratio` and the tests | 120 lines |
| `src/theme/tokens.rs` | Every colour, space, radius, cut size and duration as `pub const` (the tables in `visual-language.md` section 3). No logic | 120 lines |
| `src/theme/fonts.rs` | `FontDefinitions` built from the bundled files; named families (section 3) | 80 lines |
| `src/icons.rs` | `pub const` Phosphor codepoints as `&str`, plus `icon(ui, ICON, size, colour)` helper | 70 lines |
| `src/widgets/cut.rs` | `cut_rect(rect, cut, radius) -> Vec<Pos2>` and `paint_cut(painter, rect, cut, fill, stroke)` | 50 lines |
| `src/widgets/button.rs` | `PrimaryButton`, `SecondaryButton`, `GhostButton`, `DangerButton` (one struct with a `Kind` enum) | 140 lines |
| `src/widgets/field.rs` | Labelled text field with help and error line | 90 lines |
| `src/widgets/card.rs` | Template card with Theme art fallback, selection ring, focus ring | 160 lines |
| `src/widgets/toast.rs` | Toast queue painted in a top `Area`, plus in-place banner | 120 lines |

`start_screen.rs` then calls these widgets; `paint_preview`'s "No preview" text goes away (card.rs paints Theme art).

## 2. Assets and licences

```
crates/metallic_studio/assets/fonts/
  Archivo[wdth,wght].ttf          OFL-1.1  google/fonts ofl/archivo (v2.001, 659 KB)
  JetBrainsMono[wght].ttf          OFL-1.1  JetBrains/JetBrainsMono fonts/variable (v2.211, 187 KB)
  Phosphor.ttf                     MIT      phosphor-icons/web src/regular (subset, see below)
  Phosphor-Fill.ttf                MIT      phosphor-icons/web src/fill (subset)
  Phosphor-Bold.ttf                MIT      phosphor-icons/web src/bold (subset)
  NotoSans[wdth,wght].ttf          OFL-1.1  google/fonts ofl/notosans (fallback for Cyrillic and Greek; optional in M1)
```

- Subset the three Phosphor files to the icons listed in `icons.rs` (about 30 KB each instead of 450-500 KB):
  `pyftsubset Phosphor.ttf --unicodes=U+E094,U+E0A4,... --output-file=Phosphor.ttf`. Keep the command in
  `assets/fonts/README.md` so adding an icon is one line. Do not subset Archivo or JetBrains Mono (names typed by
  players need every Latin glyph).
- `LICENSES/THIRD-PARTY-FONTS.md`: replace the table. Add Archivo and JetBrains Mono (OFL-1.1, with their copyright
  lines "Copyright 2020 The Archivo Project Authors" and "Copyright 2020 The JetBrains Mono Project Authors" and the
  full OFL text once), Phosphor (MIT, "Copyright (c) 2023 Phosphor Icons" plus the MIT text), Noto Sans if shipped.
  Remove Ubuntu Light, Hack, Noto Emoji and emoji-icon-font once `default_fonts` is off. Note: the Ubuntu Font Licence
  is not on the project's allowed list, so turning `default_fonts` off also fixes a licence problem.
- `Cargo.toml` for metallic_studio: `eframe = { version = "0.37.0", default-features = false, features = ["wgpu"] }`
  (drop `default_fonts`). egui still needs its own five special glyphs (checkbox tick, combo arrow, etc.); our
  widgets draw those with Phosphor, and egui's built-in ones (collapsing arrows, resize corner) are painted shapes, not
  font glyphs. Check after the switch that `ComboBox` and `CollapsingHeader` still render; if one does not, add
  `egui-icons` only (MIT) back to the Proportional family.

## 3. Fonts

egui 0.37 supports variable fonts per registration: `FontData::from_static(bytes).tweak(FontTweak { coords, .. })`,
where `coords` is `VariationCoords::new([(b"wght", 560.0), (b"wdth", 100.0)])`. The same `&'static` bytes can be
registered under several names at no copy cost. `TextStyle` maps to a `FontId { size, family }`, so each weight/width
pair we use becomes a named family.

```rust
// theme/fonts.rs (sketch)
use eframe::egui::{FontData, FontDefinitions, FontFamily, FontTweak};
use eframe::epaint::text::VariationCoords;
use std::sync::Arc;

static ARCHIVO: &[u8] = include_bytes!("../../assets/fonts/Archivo[wdth,wght].ttf");
static MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono[wght].ttf");
static PH: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
static PH_FILL: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Fill.ttf");
static PH_BOLD: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Bold.ttf");

/// (family name, wght, wdth). wdth 100 = UI voice, 112.5 = headings, 125 = display.
pub const FACES: &[(&str, f32, f32)] = &[
    ("ui", 400.0, 100.0),        // body, small
    ("ui-medium", 520.0, 100.0), // captions, labels
    ("ui-strong", 560.0, 100.0), // buttons, names
    ("heading", 620.0, 112.5),   // section and panel headings
    ("title", 680.0, 125.0),     // template name
    ("display", 700.0, 125.0),   // one per screen
    ("hud", 800.0, 125.0),       // HUD values, if the in-game HUD is drawn with egui
];

pub fn definitions() -> FontDefinitions {
    let mut d = FontDefinitions::empty();
    let reg = |d: &mut FontDefinitions, key: &str, bytes: &'static [u8], coords: VariationCoords| {
        d.font_data.insert(key.into(), Arc::new(FontData::from_static(bytes).tweak(FontTweak { coords, ..Default::default() })));
    };
    for &(name, wght, wdth) in FACES {
        reg(&mut d, name, ARCHIVO, VariationCoords::new([(b"wght", wght), (b"wdth", wdth)]));
        // Icons ride along in every family so a label can mix text and an icon.
        d.families.insert(FontFamily::Name(name.into()), vec![name.into(), "ph".into()]);
    }
    reg(&mut d, "mono", MONO, VariationCoords::new([(b"wght", 400.0)]));
    reg(&mut d, "ph", PH, VariationCoords::default());
    reg(&mut d, "ph-fill", PH_FILL, VariationCoords::default());
    reg(&mut d, "ph-bold", PH_BOLD, VariationCoords::default());
    d.families.insert(FontFamily::Proportional, vec!["ui".into(), "ph".into()]);
    d.families.insert(FontFamily::Monospace, vec!["mono".into(), "ph".into()]);
    d.families.insert(FontFamily::Name("ph-fill".into()), vec!["ph-fill".into()]);
    d.families.insert(FontFamily::Name("ph-bold".into()), vec!["ph-bold".into()]);
    d
}
```

Then in `apply`: `ctx.set_fonts(fonts::definitions())` and the text styles:

| TextStyle | FontId |
|---|---|
| `Small` | 12, `ui` |
| `Body` | 13, `ui` |
| `Button` | 13, `ui-strong` |
| `Heading` | 15, `heading` |
| `Monospace` | 12, `Monospace` |
| `Name("display")`, `Name("title")`, `Name("caption")`, `Name("body-lg")`, `Name("panel")` | 30 `display`, 24 `title`, 11 `ui-medium`, 14 `ui`, 12 `heading` |

Line heights: set `FontTweak::y_offset_factor` only if text sits visibly off-centre in 24 px rows after the switch
(measure; do not guess). Optional later: `TextFormat::coords` can vary weight inside one `LayoutJob` if a family per
weight becomes too many.

Tabular digits: egui has no OpenType feature switch, so anything that counts (times, coordinates, fps, versions) uses
`Monospace`.

## 4. Visuals and Spacing (what egui styles for us)

Start from `Visuals::dark()` and set:

| Field | Value |
|---|---|
| `panel_fill` | `BG_BASE` g850 (the start screen); editor docks use `G950` behind panels with 3 px gaps |
| `window_fill` | `BG_PANEL` g800 |
| `extreme_bg_color`, `text_edit_bg_color` | `BG_DEEP` g900 |
| `faint_bg_color` | `BG_RAISED` g750 |
| `code_bg_color` | `BG_DEEP` |
| `override_text_color` | None (let widgets pick `TEXT_1`/`TEXT_2`); set `widgets.*.fg_stroke` instead |
| `weak_text_color` | `Some(TEXT_3)` |
| `hyperlink_color` | `ACCENT_TEXT` v300 |
| `warn_fg_color`, `error_fg_color` | `WARNING`, `ERROR` |
| `selection.bg_fill`, `selection.stroke` | `SELECT` v800, `Stroke::new(1.0, ACCENT_TEXT)` |
| `window_stroke` | `Stroke::new(1.0, LINE)` |
| `window_corner_radius`, `menu_corner_radius` | 10, 6 |
| `window_shadow`, `popup_shadow` | `Shadow { offset: [0, 8], blur: 24, spread: 0, color: rgba(0,0,0,115) }` (0.45 alpha) |
| `window_highlight_topmost` | false |
| `text_cursor.stroke` | 2 px `ACCENT_TEXT` |
| `button_frame` | true |
| `collapsing_header_frame` | false |
| `indent_has_left_vline` | false (tree uses indent only) |
| `striped` | false |
| `slider_trailing_fill` | true |
| `handle_shape` | default circle; radius from `interact_size` |
| `disabled_alpha` | leave default for egui widgets; our own buttons paint `TEXT_OFF` explicitly |

`widgets` (every state sets `corner_radius = 4`, `expansion = 0`):

| State | `bg_fill` / `weak_bg_fill` | `bg_stroke` | `fg_stroke` |
|---|---|---|---|
| `noninteractive` | `BG_PANEL` | 1 px `LINE` (separators) | 1 px `TEXT_2` |
| `inactive` | `BG_RAISED` | none (`Stroke::NONE`) | 1 px `TEXT_1` |
| `hovered` | `BG_HOVER` | none | 1 px `TEXT_1` |
| `active` | `BG_ACTIVE` | none | 1 px `TEXT_1` |
| `open` | `BG_ACTIVE` | none | 1 px `TEXT_1` |

Text edits: egui draws `bg_stroke` around `TextEdit` from these states; since buttons have no stroke, wrap text
fields in `widgets/field.rs`, which paints the 1 px `LINE_CONTROL` border itself and the focus ring when
`response.has_focus()`.

`Spacing`: `item_spacing` (8, 8), `button_padding` (12, 7) for 32 px buttons, `interact_size` (40, 24),
`menu_margin` 6, `window_margin` 12, `indent` 16, `icon_width` 16, `icon_spacing` 6, `combo_height` 240,
`scroll.bar_width` 6 with floating bars. `Interaction::tooltip_delay` 0.4 s and `show_tooltips_only_when_still` true.

Focus ring: egui shows keyboard focus through `widgets.hovered`; that is not enough. Each custom widget checks
`response.has_focus() && ui.memory(|m| m.focused().is_some())` and paints a 2 px `FOCUS` stroke 2 px outside its
rect. For built-in widgets in panels, add a small `focus_ring(ui, &response)` helper and call it after them.

## 5. What needs custom painting

1. **The cut** (`widgets/cut.rs`). egui's `CornerRadius` is per-corner but always round, so the chamfer is a convex
   polygon: corners (left, top) to (right - cut, top) to (right, top + cut) to (right, bottom) to (left, bottom), with
   the three other corners rounded by inserting 3 to 4 arc points for `radius`. Paint with
   `Shape::convex_polygon(points, fill, stroke)`. For the selected-card ring, paint the outer polygon in `ACCENT` and
   the card on top (rect expanded by 3 px with cut + 1). The HUD tag mirrors the cut on both bottom corners.
2. **Top highlight** on raised surfaces: one `painter.line_segment([rect.left_top() + vec2(r, 0.5), rect.right_top() +
   vec2(-r, 0.5)], Stroke::new(1.0, HIGHLIGHT))` after the fill, where `r` is the corner radius.
3. **Theme art** (`widgets/card.rs`): a `Mesh` with two quads: sky (top colour to low colour, vertical gradient via
   per-vertex colours) and ground (ground far to ground near), split at 62 % height; then the genre glyph in
   `ph-bold` at 40 px (cards) or 104 px (stage), colour = the Theme's ink. No texture, so it is instant.
4. **Keycaps**: `painter.rect_filled(rect, 3, G650)` and the label in `Name("caption")`; on a primary button the cap is
   `rgba(17,16,21,46)` with `ON_ACCENT` text.
5. **Toasts**: an `egui::Area` anchored bottom right, `Frame` with fill `G700`, radius 6, `popup_shadow`, highlight
   line. Animate with `ctx.animate_value_with_time(id, target, 0.2)`.
6. **Tree rows and property rows**: `ui.allocate_exact_size(vec2(w, 24.0), Sense::click())`, paint fill by state, then
   caret, icon, name, class name. Selected uses `SELECT`; `TEXT_3` steps to `TEXT_2` on hover and selected fills.
7. **Viewport overlays**: selection box as a 2 px `V300` rect with 1 px `G900` rects inside and outside; gizmo axis
   letters on `G900` plates at 0.85 alpha.

## 6. Icons

`icons.rs` holds codepoints as `&str` constants (Phosphor's names, upper snake case) and one helper:

```rust
pub const PLAY: &str = "\u{E3D0}";
pub fn icon(ui: &mut egui::Ui, glyph: &str, size: f32, colour: Color32) -> egui::Response {
    ui.label(egui::RichText::new(glyph).font(FontId::new(size, FontFamily::Name("ph".into()))).color(colour))
}
```

Used by the mockups (regular unless noted): arrows-clockwise U+E094, arrows-out-cardinal U+E0A4, camera U+E10E,
caret-down U+E136, caret-right U+E13A, check U+E182, check-circle U+E184 (fill), code U+E1BC, crosshair-simple U+E1D8
(bold, FPS art), cube U+E1DA, cursor U+E1DC, dots-three-vertical U+E208, eye U+E220, flag U+E244 (bold, platformer
art), folder U+E24A, folder-open U+E256, folder-simple U+E25A, funnel-simple U+E268, gear-six U+E272,
globe-hemisphere-west U+E28C, info U+E2CE (fill), lock-simple U+E308, magnifying-glass U+E30C, minus U+E32A, person
U+E3A8, play U+E3D0 (fill), plus U+E3D4, question U+E3E8, square U+E45E, squares-four U+E464, stop U+E46C (fill), sun
U+E472, timer U+E492, trash U+E4A6, warning U+E4E0 (fill), warning-circle U+E4E2 (fill), x U+E4F6, x-circle U+E4F8
(fill), flag-banner U+E622, frame-corners U+E626, magnet U+E680, users-three U+E68E, person-simple-run U+E730 (bold,
parkour art), hammer U+E80E, file-code U+E914, ladder U+E9E4 (bold, obby art), flag-checkered U+EA38 (bold, racing
art), sidebar-simple U+EC24, grid-nine U+EC8C.

Rules: icon-only buttons get `.on_hover_text()` and an accessible label (`response.widget_info`); icons use `ICON`
colour except status (Fill weight, semantic colour) and axes.

## 7. Animation helpers

egui 0.37 has `ctx.animate_bool_with_time_and_easing(id, value, seconds, easing)` and
`ctx.animate_value_with_time(id, target, seconds)`; easing functions are in `emath::easing` (`cubic_out`,
`cubic_in_out`). Put the token durations and one switch in `theme/tokens.rs`:

```rust
pub const T_FAST: f32 = 0.09;  pub const T_BASE: f32 = 0.14;  pub const T_SLOW: f32 = 0.20;
pub fn t(ctx: &egui::Context, base: f32) -> f32 {
    // Studio setting "Reduce motion" (default: follow Windows "Show animations in Windows").
    if reduce_motion(ctx) { if base >= T_SLOW { 0.08 } else { 0.0 } } else { base }
}
pub fn hover_t(ui: &egui::Ui, id: egui::Id, hovered: bool) -> f32 {
    ui.ctx().animate_bool_with_time_and_easing(id, hovered, t(ui.ctx(), T_FAST), emath::easing::cubic_out)
}
```

Store the reduce-motion flag in `ctx.data` (set once from settings), and set `Style::animation_time` to `t(ctx, T_BASE)`
so egui's own collapsing headers follow it. Use `Color32::lerp_to_gamma` (or a small `lerp` helper) to blend fills by
the returned 0..1. Never animate on keyboard-opened UI (command bar, menus).

## 8. Tests to add or extend

- Extend `theme_text_contrast` to the table in `visual-language.md` section 5: `TEXT_1`, `TEXT_2`, `ICON`,
  `ACCENT_TEXT` at 4.5 on g900 to v900; `TEXT_3` at 4.5 on g900 to g750 only; `ON_ACCENT` on `ACCENT`,
  `ACCENT_HOVER`, `ACCENT_PRESS`; semantic text on panel and on its own background; `TEXT_1` on `DANGER` and
  `DANGER_HOVER`; `LINE_CONTROL` at 3.0 on panel, deep and g950; `FOCUS` at 3.0 on base and panel.
- Keep `deep_colour_stays_near_the_icon_background`: g900 #111015 is (17,16,21) against the icon's (20,18,25), within
  4 per channel.
- A font test: `fonts::definitions()` contains every `FACES` name and the Proportional family lists `ui` first.
- An icons test: every `icons.rs` constant has a glyph in the subset font (`ttf-parser` is already a transitive
  dependency through epaint's `skrifa`; or keep the codepoint list and the `pyftsubset` command in sync by a test
  that reads `assets/fonts/README.md`).
- Screenshot scenes in `screenshot.rs` (start, error) re-rendered and compared by eye against `start-1280.png`.

## 9. Order of work

1. Fonts and `default_fonts` off, licences updated (visible win on its own).
2. Tokens and `Visuals`/`Spacing` (the whole app changes colour and density at once).
3. `cut.rs` and `button.rs`, then the start screen form.
4. `card.rs` with Theme art; remove "No preview".
5. Recents rail with thumbnails and relative times; toasts and banners.
6. HUD tag: in this worktree no crate draws the in-game HUD yet (the Parkour runtime work, Task 8, is in progress, and
   `metallic_player` does not depend on egui). Whatever draws it should take its sizes and colours from section 6 of
   `visual-language.md`; share the few constants through a small crate later if Studio and the player both need them.
