# Metallic Studio visual language

Date: 2026-10-30. Status: proposal for the owner. Docs and mockups only; no Rust changed.
Implementation guide: `egui-implementation.md` in this folder.

Mockups in this folder (open the HTML in a browser; the PNGs are renders of it):

| File | What it shows |
|---|---|
| `start-1280.png`, `start-1920.png` | Start screen at 1280x800 and 1920x1080 (`start.html`) |
| `start-1280-firstrun.png` | First run: no recent games, folder error shown (`start.html?state=firstrun`) |
| `hud.png`, `hud-finish.png` | Parkour HUD: checkpoint banner, run timer, controls hint; finish state (`hud.html`, `?state=finish`) |
| `editor.png` | One editor frame at 1600x1000: Explorer, viewport, Properties, Output (`editor.html`) |
| `components.png` | Every component in every state (`components.html`) |

Labels used below: **Sourced** means I read the page this session and the URL follows. **Judgment** means my reasoning
from sourced facts, a proposal the owner can overturn.

---

## 1. Direction

**Machined graphite.** Studio is a tool made of one material: graphite with a faint violet cast, worked like metal. Its
one signature is *the cut*, a single 45 degree chamfer on the top-right corner of the most important thing on screen
(the primary button, the selected template, the template stage, the Play button), as if a machinist took one pass off
the corner. Raised surfaces catch a 1 px line of light along their top edge. Nothing glows, nothing floats without a
reason, and there are no gradients on the chrome. Colour belongs to the work: template art carries each template's own
lighting colours, the HUD sits over the game, and the editor viewport is the brightest thing in the window. Studio
itself stays graphite with one violet accent that marks only "this is the thing you will act on". Type does the rest:
Archivo at normal width is the working voice, the same family stretched to 125 % width is the display voice, and
JetBrains Mono carries every number that changes (timers, coordinates, versions).

**Design Read** (antislop core Part 3): desktop creative-tool launcher and editor for teens making their first game and
for pros who live in it all day. Dark because the viewport and game art must be the brightest thing on screen
(Judgment, the reason every tool in section 2 is dark by default). Dials: ENERGY 2 (calm chrome, the art supplies the
energy), RHYTHM 2 (tool layout, consistent panels, one large stage on the start screen), MOTION 1 (feedback only; no
idle motion).

Why not the default looks (Judgment):
- Not "purple gradient AI": the accent is one flat violet, used on at most one control per view plus selection.
- Not the SaaS-card kit: radii vary by role (4 widgets, 6 menus, 10 windows), only floating things have a shadow,
  docked panels are square, and the one memorable shape is the cut, not a radius.
- Not Inter: Inter is the default answer in this space (Linear, Blender 4, Rerun all use it, section 2). Archivo has a
  real width axis, so one licence and one file give us both a compact UI face and a wide, engineered display face that
  reads as "machined" without a second family.

---

## 2. Research

### 2.1 Pro and creative apps

| App | What they did | Sourced / Judgment | What Metallic takes |
|---|---|---|---|
| Linear (2024 redesign) | Theme built from 3 inputs (base, accent, contrast) instead of 98 variables, in LCH; limited the blue in the chrome "for a more neutral and timeless appearance"; Inter Display for headings, Inter for the rest; denser navigation, aligned labels and icons | Sourced: https://linear.app/now/how-we-redesigned-the-linear-ui | A small generated scale (graphite + violet), neutral chrome, one display treatment for headings |
| Figma UI3 | About 200 hand-drawn icons that "explain how to interact"; panels collapsible and resizable; sidebars that appeared and disappeared were dropped as "too unstable"; a minimal hover-only UI was dropped; x and y kept before width and height for muscle memory | Sourced: https://www.figma.com/blog/behind-our-redesign-ui3/ | Fixed docked panels, no auto-hiding chrome; keep property order stable (Position before Size) |
| Blender 4.0 | Default UI font changed to Inter; text output made "typographically correct"; old font picked in 2006 and unmaintained | Sourced: https://wiki.blender.org/release_notes/4.0/user_interface/ and https://www.blendernation.com/2023/10/03/blender-4-0-enters-beta-and-gets-a-new-font/ | A maintained open variable font is worth the switch; egui's bundled Ubuntu Light is the same kind of legacy choice |
| Unity 6 | New Light and Dark colours, icons redrawn to match Hub and web, new UI font for legibility; palette changed again in 6000.0.7f1 with deliberately higher contrast for readability | Sourced: https://blog.unity.com/technology/evolving-the-unity-editor-ux and https://issuetracker.unity3d.com/issues/editor-interface-has-low-contrast-when-any-theme-is-used | Contrast is checked per pair (section 5); secondary text stays at 5:1 or more on panels |
| Zed | Bundled UI font IBM Plex, mono Lilex in current docs (older default "Zed Plex"); chrome kept thin so the user's content owns the screen | Sourced: https://zed.dev/docs/visual-customization.html | Thin title bar (40 px), 24 px status bar, content first |
| Raycast | Three principles: fast, simple, delightful; a Compact Mode; native AppKit for speed | Sourced: https://www.raycast.com/blog/a-fresh-look-and-feel | Ctrl K command bar in the title bar; no open/close animation on things used hundreds of times a day |
| Rerun (re_ui, egui) | Inter variable with weight set through `wght` coords, JetBrains Mono variable; radii 10 window, 6 menus, 4 widgets; popup shadow offset (0,15) blur 50; 24 px list items, 36 px top bar, 8x8 item spacing; colours in a RON colour table with semantic aliases | Sourced: https://docs.rs/crate/re_ui/latest/source/src/design_tokens.rs | The proof that egui can look custom: same radius ladder, 24 px rows, variable fonts via coords, semantic tokens |
| Roblox Studio start page | Rebuilt because the old page's UI framework caused "known hangs and performance issues"; goal is to get into "your creation flow" quickly | Sourced: https://devforum.roblox.com/t/new-studio-start-page/3181233 | The start screen must be instant and one step from making: pick, name, create |
| Godot Project Manager | List of projects, tag filters, Create asks for a name and an empty folder; templates live in an online Asset Library, offline by default | Sourced: https://docs.godotengine.org/en/stable/tutorials/editor/project_manager.html | Same name + empty-folder form, but templates are local and shown first |
| Unity Hub | Templates in Core / Sample / Learning tabs; selecting one shows details; list depends on Editor version | Sourced: https://docs.unity3d.com/hub/manual/project-create.html | A selected-template panel with details next to the form |
| Unreal project browser | Five steps: category (Games, Film/TV, AEC, Automotive), template, project defaults (Blueprint or C++, platform, quality, starter content, ray tracing), location and name, Create | Sourced: https://dev.epicgames.com/documentation/en-us/unreal-engine/creating-a-new-project-in-unreal-engine | Judgment: one screen, no category step and no defaults step; the filter is enough for 6 to 30 templates, and defaults are changed later in Settings |
| GDevelop | Start page creates, opens and loads tutorials; users could not find a blank project among templates; a later release gave starters their own section | Sourced: https://wiki.gdevelop.io/gdevelop5/interface/ and https://newreleases.io/project/github/4ian/GDevelop/release/v5.5.245 | "Empty 3D world" is always a card in the same grid, and the Empty filter finds it |
| Rive, Spline, Framer, Arc | Not verified this session: searches returned marketing and third-party pages only | No claim made | Nothing taken |

### 2.2 How polished egui apps avoid the default look (Sourced from re_ui above and egui 0.37 source; Judgment where marked)

- Replace the fonts. egui's bundled Ubuntu Light and Hack are what make an egui app look like an egui app (Judgment).
  egui 0.37 supports variable fonts: `FontTweak::coords` and `TextFormat::coords` take `VariationCoords` such as
  `[(b"wght", 560.0), (b"wdth", 125.0)]` (Sourced: `epaint-0.37.0/src/text/font_tweak.rs`, `text_layout_types.rs`).
- Set every `WidgetVisuals` state and remove default strokes; re_ui sets button strokes to none and lets fill carry
  hover (Sourced: re_ui).
- Paint the signature shapes yourself (`Shape::convex_polygon` for the cut, a `Mesh` for the template art gradient).
- Own the row height (24) and spacing; egui's defaults are 18 to 20 px rows.
- Shadows only on popups (re_ui) and toasts.

### 2.3 Free resources and licences (checked through each project's licence file on GitHub)

| Resource | Licence | Ship? | Notes |
|---|---|---|---|
| Archivo (Omnibus-Type, via google/fonts `ofl/archivo/Archivo[wdth,wght].ttf`) | OFL-1.1, no Reserved Font Name | Yes | Chosen UI and display face. Axes wght 100-900, wdth 62-125. Latin, Latin Extended, Vietnamese; no Cyrillic |
| JetBrains Mono (`fonts/variable/JetBrainsMono[wght].ttf`) | OFL-1.1, no Reserved Font Name | Yes | Chosen mono. wght 100-800. Has Cyrillic and Greek |
| Phosphor Icons core (regular, bold, fill) | MIT, (c) 2023 Phosphor Icons | Yes | Chosen icon set |
| Noto Sans (google/fonts) | OFL-1.1 | Yes, as fallback | Proposed fallback for Cyrillic and Greek names (Archivo has none) |
| Lucide | ISC (some icons derived from Feather, MIT) | Allowed, not chosen | Thin 2 px rounded strokes are the "every AI site" icon look (antislop-ui) |
| Tabler Icons | MIT | Allowed, not chosen | Similar stroke look to Lucide |
| Inter, Geist, IBM Plex | OFL-1.1 (IBM Plex reserves the name "Plex") | Allowed, not chosen | Inter is the category default; Plex's RFN means a subset must be renamed |
| Radix Colors, Open Props | MIT | Allowed, used as reference only | Step structure of Radix scales informed the graphite and violet ramps |
| awesome-design-systems | Unlicense | Reference only | |
| egui bundled fonts (Ubuntu Light, Hack, Noto Emoji, emoji-icon-font) | Ubuntu Font Licence 1.0, MIT/Bitstream Vera, OFL, MIT | Ubuntu Font Licence is NOT on the allowed list | Another reason to turn off egui's `default_fonts` feature |
| anthropics/skills frontend-design | Secondhand sources say Apache-2.0 | Not verified | Used as guidance only, nothing copied |

---

## 3. Tokens

All values are also CSS variables in `assets/tokens.css` and map 1:1 to `theme/tokens.rs` (see the implementation guide).

### 3.1 Graphite (neutral, slight violet cast: blue highest, red above green)

| Token | Hex | Role |
|---|---|---|
| g950 | #0c0b0f | Title bar, status bar, dock gutters |
| g900 | #111015 | `bg.deep`: text fields, sunken wells, HUD plate (at 0.88 alpha), text on accent |
| g850 | #16151b | `bg.base`: window background |
| g800 | #1c1b22 | `bg.panel`: panels, rail |
| g750 | #24222b | `bg.raised`: cards, secondary buttons |
| g700 | #2c2a34 | `bg.hover` |
| g650 | #35323e | `bg.active` (pressed, segmented on), keycaps |
| g600 | #43404d | `line.strong`, empty HUD pips |
| g500 | #5f5b6b | `text.off` (disabled) |
| g400 | #817d8c | |
| g300 | #a3a0ad | |
| g200 | #c3c0ca | Checked checkbox fill ("silver") |
| g100 | #e6e4ea | = `text.1` |
| g50 | #f4f3f7 | HUD values |

Also: `line` #2b2933 (hairlines between areas, decorative only), `line.control` #6e6a79 (borders of inputs and
checkboxes, 3.25:1 on panel), `text.2` #aeaab6, `text.3` #8e8a97, `icon` #bdbac4.

### 3.2 Violet (the one accent)

| Token | Hex | Role |
|---|---|---|
| v50 | #f3f0fc | |
| v100 | #e4def8 | |
| v200 | #cdc2f2 | Focus ring on a selected card, selected-row icon |
| v300 | #b4a6ec | `accent.hover`, `accent.text`, `focus`, viewport selection box |
| v400 | #9d8be0 | `accent`: primary button, selected-card ring, HUD pips |
| v500 | #8673cf | `accent.press` |
| v600 | #6f5cb6 | |
| v700 | #584993 | |
| v800 | #3a3354 | `select`: selected row, active tool |
| v900 | #2a2638 | `select.weak` |
| v950 | #1f1c29 | |

Where violet may appear: the one primary button per view, selection (rows, card ring, active tool), focus rings, the
check icon on the selected card, HUD progress pips. Nowhere else. Text on violet is `g900`, never white (white on v400
is 2.92:1).

### 3.3 Semantic colours (text colour, then the tinted background used for banners and rows)

| Meaning | Text / icon | Background |
|---|---|---|
| Error | #ec7a72 | #3a2226 |
| Warning | #e0ad55 | #352c1d |
| Success | #62bf91 | #1d3329 |
| Info | #7fb0e6 | #1c2a3a |
| Danger button | fill #8f3a35, hover #a3443e, label `text.1` | |
| Axis X / Y / Z | #e5675e / #79c46a / #5d95ea (lighter on HUD plates: #ef8a82 / #8fd27f / #82aef0) | |

Every semantic colour is paired with an icon and words; colour is never the only signal. Axis colours always come with
the letter.

### 3.4 Colour in template art (colour belongs to the work)

A template card without a `preview.png` never shows a grey "No preview" box. It shows **Theme art**: a sky band, a
horizon and a ground band painted from the template's Theme, with one Phosphor Bold glyph that names the genre. The
Theme colours come from the lighting preset the template uses, so the card looks like the first second of the game.

| Theme (preset) | sky top | sky low | ground | ground far | glyph ink |
|---|---|---|---|---|---|
| titanium (Clear Day) | #aeb1c2 | #d9dae2 | #7a7686 | #5c5968 | #2e2c37 |
| toybox (Cartoon Bright) | #7fbcea | #cfe8f8 | #69b866 | #4f9a4d | #24467a |
| dusk (Golden Hour) | #463b66 | #e09a66 | #3a2f3d | #261f29 | #17131c |
| foundry (Overcast) | #57514b | #8d8376 | #433b35 | #2e2925 | #1c1916 |
| noir (Moonlit) | #151925 | #353c52 | #14161c | #0d0e12 | #9aa3b8 |
| paper (2D) | #f3eee2 | #ece2cc | #d6c9ac | #c7b894 | #4a4236 |

`meadow`, `frost` and `pixel` follow the same pattern when those templates exist. When a real preview loads later, it
fades in over the Theme art (140 ms); while loading, a "Loading preview" chip sits on the art. Template authors pick a
Theme in `template.toml` (`theme = "toybox"`, proposed field).

### 3.5 Type

| Family | File | Licence | Source |
|---|---|---|---|
| Archivo (variable: wght 100-900, wdth 62-125) | `Archivo[wdth,wght].ttf`, 659 KB, v2.001 | OFL-1.1 | https://github.com/google/fonts/tree/main/ofl/archivo (upstream https://github.com/Omnibus-Type/Archivo) |
| JetBrains Mono (variable: wght 100-800) | `JetBrainsMono[wght].ttf`, 187 KB, v2.211 | OFL-1.1 | https://github.com/JetBrains/JetBrainsMono |

Scale (size/line height in px, weight, width):

| Style | Size / line | Weight | Width | Use |
|---|---|---|---|---|
| display | 30/36 (36/42 at 1600 px and wider) | 700 | 125 % | One per screen: "What do you want to make?" |
| title | 24/30 | 680 | 125 % | Selected template name |
| heading | 15/20 | 620 | 112.5 % | Section headings ("Starting points", "Recent games") |
| panel heading | 12/16 | 620 | 112.5 % | Docked panel titles |
| body-lg | 14/20 | 400 | 100 % | Lede, template description |
| body | 13/18 | 400 | 100 % | Default UI text |
| strong | 13/18 | 560 | 100 % | Names in lists, button labels |
| small | 12/16 | 400 | 100 % | Meta lines, help text, property labels |
| caption | 11/14 | 520 | 100 % | Keycaps, chips; the minimum size |
| mono | 12/18 | 400 | | Coordinates, times, versions, log timestamps |
| HUD label / value / time | 17/22 600 112.5 %; 44/50 800 125 %; mono 52/58 700 | | | In-game, scaled by screen height / 1080 |

Rules: sentence case everywhere, no all-caps labels, no letter-spaced eyebrows. Numbers that change use JetBrains Mono
(egui cannot switch on `tnum`, and Archivo's default figures are proportional). Display tracking -0.01 em; body 0.

### 3.6 Space, radii, strokes, elevation

- Space (4 px grid with two half steps): 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48.
- Control heights: 36 primary, 32 buttons and fields, 28 compact (title bar, filters), 24 list rows and tree rows,
  22 property fields. Title bar 40, panel header 32, viewport toolbar 36, status bar 24.
- Radii: xs 3 (keycaps, chips), sm 4 (buttons, fields, rows), md 6 (menus, toasts, cards), lg 10 (dialogs, the start
  stage). Docked panels: 0.
- The cut: sm 6 px (primary button, Play), md 10 px (selected card ring), lg 14 px (start stage). HUD tag: both bottom
  corners, 16 px. Only one cut element per region.
- Strokes: 1 px `line` between areas; 1 px `line.control` on inputs; 2 px for focus and selection; 3 px card ring.
- Elevation: flat by default. Raised surfaces (`bg.raised` buttons and cards) get a 1 px top highlight
  rgba(255,255,255,0.06). Only floating surfaces (menus, popups, toasts, dialogs) get `shadow.pop`: 0 8 24 rgba(0,0,0,0.45).
  No glow anywhere.

### 3.7 Icons

Phosphor (MIT). Regular weight for UI at 16 px (15 in dense panels, 14 in the title bar and filters), Fill for status
glyphs (success, error, warning, info, Play, Stop) at 13-16 px, Bold only for the large genre glyph on Theme art (40 px
on cards and 104 px on the stage at 1280; 52 and 150 px at 1600 and wider; 18 px in recents thumbnails). Colour `icon` #bdbac4; coloured only for status and axes. Every icon-only button has a
text label for assistive tech and a tooltip. Codepoints used by the mockups are listed in `egui-implementation.md`.

### 3.8 Motion

| Token | Time | Curve | Reduced motion |
|---|---|---|---|
| fast | 90 ms | cubic out | 0 |
| base | 140 ms | cubic out | 0 |
| slow | 200 ms | cubic out / cubic in-out for movement | 80 ms fade only |

What moves: hover and press fills (fast), selection ring and stage swap (base: the art cross-fades), toast enter
(slow, 8 px rise plus fade; leaves with a 150 ms fade), HUD tag (slides down 220 ms cubic out, holds 2.5 s, fades out
150 ms). What never moves: anything opened by a keyboard shortcut (command bar, menus open instantly), panel resizes,
list scrolling. Press feedback: 1 px down shift on buttons, no scale. Reduced motion is a Studio setting (default
follows Windows "Show animations"): slides become fades, durations drop to the right-hand column.

### 3.9 States

| State | Treatment |
|---|---|
| Hover | Fill steps up one graphite level (panel to hover); ghost buttons gain a fill and `text.1`; `text.3` steps to `text.2` |
| Pressed | Fill steps up again (`bg.active`) or `accent.press`; 1 px down shift |
| Focus (keyboard only) | 2 px `focus` ring (v300) 2 px outside; fields: 1 px offset and the border turns v300. On a selected card the ring turns v200 so focus and selection differ |
| Selected | `select` (v800) fill for rows and tools; 3 px v400 cut ring for template cards; check icon on the card |
| Disabled | `bg.raised` fill, `text.off` label, no highlight, not focusable; a tooltip says why |
| Invalid | 1 px error border plus an error line under the field with icon and words |
| Loading | A labelled chip ("Loading preview"); no spinner without words |

---

## 4. Components

### Buttons
- **Primary**: 36 px, `accent` fill, `g900` label 560, cut sm on the top-right; optional keycap in a darker inset.
  One per view (Create game, Play).
- **Secondary**: 32 px, `bg.raised` + top highlight, `text.1`, icon in `icon` colour.
- **Ghost**: 32 px (28 in the title bar), no fill, `text.2`; hover fills `bg.hover` and turns `text.1`.
- **Danger**: 32 px, #8f3a35 fill, `text.1` label; only inside a confirmation, never the default button.
- Labels are verbs that say what happens ("Create game", "Turn on", "Choose folder"), and the result toast reuses the
  verb ("Created Rooftop Run").

### Text fields
32 px, `bg.deep` fill, 1 px `line.control` border, radius 4, label above (12/16 520 `text.2`), help line below
(12/16 `text.3`). Error: border `error`, the help line is replaced by an error line with a Fill icon. Focus: v300
border plus ring. Validation runs on submit and then live; the Game name field updates the target path in the help line
as you type ("Studio creates C:\Games\Rooftop Run. It must be new or empty.").

### Template cards
Card 6 columns wide at 1280, art 16:9 on top, then name (strong) and a meta line (small, `text.2`: "3D, third-person").
No preview image: Theme art (3.4). Hover: the card fill steps up to `bg.raised` and the meta line to `text.2`. Selected: 3 px v400 ring with the
cut md, plus a Fill check before the name. Keyboard: the grid is one tab stop; arrow keys move, Enter creates.

### Selected-template panel (the stage)
The largest surface on the start screen, cut lg. Left: the preview at 16:9 with chips for 2D/3D and the Theme. Right:
title, meta, description, "Inside:" line (at 1600 px and wider, a two-column list with icons), then the form (Game
name, Folder + Browse, help line, Create game). Template data comes from `template.toml`; "Inside" is a proposed field.

### Recents sidebar
Rail 272 px (320 at 1600 and wider). Top: "Open a game folder" secondary button with Ctrl O. Then "Recent games"
heading with a count, rows of 72x40 thumbnail + name + "Template, relative time" ("12 minutes ago", "Yesterday",
"Monday", "3 Oct"). Hover shows a remove button. A missing folder shows a folder icon instead of a thumbnail and a
warning line "Folder moved or deleted". Footer: version, Third-party licences, Check for updates.

### Empty states
Recents empty: folder icon, "No games yet", one sentence of what will appear, and the Open a game folder button. No
templates found: an info banner naming the folder searched and an action.

### Toasts and banners
Toasts float bottom right on `g700` with radius 6, the pop shadow and a highlight; a Fill status icon, one sentence,
optional action button (`g650`), close button. Success and info toasts leave after 5 s, errors stay until closed. Banners sit in place inside
a panel on the semantic background with `text.1` copy and a ghost action.

### Title bar
40 px on `g950`. Start screen: crow emblem 20 px, "Metallic Studio" (640, 112.5 %), Settings and Help ghost buttons,
window controls (close hover #a3443e). Editor: emblem, menus, project name and template, a centred Build/Script
segment and Play cluster, the command bar ("Search or run a command", Ctrl K), window controls.

### In-game HUD banner
See section 6.

### Editor panels (from `editor.png`)
Docked panels are square on `g950` with 3 px gutters. Panel header 32 px with a 12/16 heading and icon buttons.
Filter field under the header. Tree rows 24 px, 16 px indent, class name in `text.3` on the right, selected row `select`
fill, eye and lock icons on hover. Viewport toolbar 36 px: Q/W/E/R tools in a well (keys shown), World/Local, move and
rotate snap, then lighting preset, grid and camera on the right. Selection box 2 px v300 with dark 1 px edges so it
stays visible on a bright sky; gizmo arrows carry X/Y/Z letters on small plates. Properties: object name and class,
sections with carets, 28 px rows, X/Y/Z number fields with coloured axis letters, silver checkboxes, inline warning
hints under the property they explain. Output: mono timestamps, source, UI-font message, a tinted warning row with a
one-click fix ("Turn on").

---

## 5. Contrast (WCAG 2.x, computed with the formula; spot-checked with the antislop contrast script and tool)

Text on surfaces:

| Text | deep g900 | base g850 | panel g800 | raised g750 | hover g700 | active g650 | select v800 | select.weak v900 |
|---|---|---|---|---|---|---|---|---|
| text.1 #e6e4ea | 15.02 | 14.39 | 13.54 | 12.44 | 11.19 | 9.93 | 9.35 | 11.62 |
| text.2 #aeaab6 | 8.32 | 7.97 | 7.50 | 6.89 | 6.20 | 5.50 | 5.18 | 6.44 |
| text.3 #8e8a97 | 5.62 | 5.39 | 5.07 | 4.66 | 4.19 fail | 3.72 fail | 3.50 fail | 4.35 fail |
| icon #bdbac4 | 9.91 | 9.49 | 8.94 | 8.21 | 7.38 | 6.55 | 6.17 | 7.67 |
| accent.text #b4a6ec | 8.66 | 8.30 | 7.81 | 7.18 | 6.46 | 5.73 | 5.39 | 6.70 |

Rule from the failures: `text.3` is allowed only on g900 to g750. On hover, active, select and tinted fills it steps up
to `text.2` (implemented in the mockups: card meta on hover, tree class names on the selected row, Output timestamps on
the warning row, where text.3 would be 4.08).

Other pairs:

| Pair | Ratio | Needs |
|---|---|---|
| on.accent g900 on accent v400 | 6.50 | 4.5 |
| on.accent on accent.hover v300 | 8.66 | 4.5 |
| on.accent on accent.press v500 | 4.84 | 4.5 |
| white on accent (not used) | 2.92 | fail, the reason labels on violet are dark |
| accent v400 on panel (selected-card ring, non-text) | 5.86 | 3 |
| focus v300 on base | 8.30 | 3 |
| line.control vs panel / deep / g950 | 3.25 / 3.61 / 3.74 | 3 |
| line #2b2933 vs panel | 1.19 | decorative only, never a control edge |
| error on panel / on error.bg | 6.18 / 5.29 | 4.5 |
| warning on panel / on warning.bg | 8.36 / 6.72 | 4.5 |
| success on panel / on success.bg | 7.63 / 6.02 | 4.5 |
| info on panel / on info.bg | 7.53 / 6.42 | 4.5 |
| text.1 on error / warning / success / info bg | 11.59 / 10.89 / 10.69 / 11.55 | 4.5 |
| text.1 on danger / danger hover | 5.89 / 4.82 | 4.5 |
| text.1 on g650 (toast action, Turn on) | 9.93 | 4.5 |
| check mark g900 on silver g200 | 10.56 | 3 |
| checked box g200 vs panel | 9.53 | 3 |
| axis letters X / Y / Z on deep (property fields) | 5.80 / 8.94 / 6.27 | 4.5 |
| gizmo letters X / Y / Z on plate (g900 0.85 over white) | 5.08 / 6.88 / 5.45 | 4.5 |
| selection box v300 vs pillar #3d3947 / vs sky #a9aab9 | 5.12 / 1.05, so a 1 px g900 edge is added each side | 3 |
| HUD plate (g900 at 0.88) over worst-case white = #2e2d31: text.1 / text.2 / v300 | 10.84 / 6.01 / 6.25 | 4.5 |
| HUD caption on 0.78 scrim: text.1 / text.2 | 7.58 / 4.2 fail | so the plate is 0.88, not 0.78 |
| disabled label text.off on raised | 2.39 | exempt (WCAG), always with a tooltip |

---

## 6. In-game HUD

Shown in `hud.png` (checkpoint) and `hud-finish.png` (finish). Sizes are for a 1080 px tall screen; the game scales
them by height / 1080, clamped 0.75 to 1.5.

- **Tag** (checkpoint and finish): hangs from the top edge, centred, `g900` at 0.88 alpha, both bottom corners cut 16 px.
  Padding 26 x 56, min width 300. Label 17/22 600 112.5 % `text.2` ("Checkpoint", "Finish"); value 44/50 800 125 %
  `g50` ("2 of 3"), or the time in JetBrains Mono 52/58 700 ("0:07.04"); three 44x5 pips (filled v400, empty g600);
  finish adds "All 3 checkpoints". Motion: slides down 220 ms cubic out, holds 2.5 s, fades 150 ms (finish stays).
- **Run timer**: top right, plate with the cut on its top-right corner, timer icon, mono 26/30 600. Hidden at finish.
- **Controls hint**: bottom left, plate radius 4, keycaps (`g650` with a 2 px darker bottom edge) and words: "W A S D
  Move, Space Jump, Shift Run, Right mouse Turn camera". Fades after the player has used each control once (proposal).
- Never pure white on the plate's edge; never a glow behind the tag; the game stays visible around every plate.

---

## 7. Checks run on these mockups

Contrast: every pair above. Rendering: each PNG viewed after the last change. Interaction: `start.html` and
`editor.html` clicked through by script in a browser (template select by click and arrows, filter, name validation,
submit, every button gives feedback, keyboard focus order and the focus ring). The full antislop gate with each rule's
result is in the report:
the lead's local task notes (not in git).

Copy that is proposed, not real: the five non-Parkour templates' names, descriptions and Themes; the recent games
(made-up game names, no people); the editor's object tree, log lines and the Check system. Parkour's description is
the real `template.toml` text.
