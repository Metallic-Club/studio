# UI overhaul: research and the Metallic design system

Date: 2026-10-30. Asked by the owner ("make all of Metallic's UI look great, not cheap"), assigned by the lead session.
Status: research and design only. Nothing in Rust or in the website was changed; nothing is committed.
Mockups: `design/mockups/2026-10-30-studio-start/` (`index.html`, `splash.html` and the PNGs next to them).

## 0. The short answer

Studio looks cheap because the shipped app does not use the design the project already has. PR #199 (merged today)
defined a good visual language and `design/tokens/metallic.tokens.json` + `crates/metallic_tokens` hold the values,
but `crates/metallic_studio` still:

- renders egui's bundled **Ubuntu Light** (`eframe` feature `default_fonts`), the single biggest "default egui" tell;
- uses its own hard-coded colours in `src/theme.rs` (BASE `#1c1922`, ACCENT `#9b82d4`, ...) instead of the tokens;
- draws every card, button and field as the same flat tone with a 1 px border, with no imagery, no hierarchy and no
  title bar of its own (`start.png`: two text cards on an empty field; `splash.png`: icon, title, "Starting...").

So the overhaul is mostly **implementation**, not a new look. This document keeps #199's direction (machined graphite,
one violet accent, the 45 degree cut, Archivo + JetBrains Mono), fills the gaps it left (light and Titanium themes in
the shared tokens, elevation, exits, component specs for every surface, the splash), records what the linked projects
offer, and splits the work into agent-sized tasks (section 6).

## 1. What I looked at

- Studio today: `scratchpad/studio01/start.png`, `splash.png`; `origin/main` `crates/metallic_studio/src/{app,theme,start_screen,templates}.rs`.
- Design already in the repo (`origin/main`; the main checkout is on `studio/conductor-baton`, which lacks most of it):
  `docs/design/visual-language/*` (#199), `design/tokens/metallic.tokens.json`, `crates/metallic_tokens`,
  `crates/metallic_launcher/assets/pages/pages.css`, `branding/placeholder-icon.png` (the only brand file),
  `templates/parkour/` (the only real template).
- Brand rules: memory `metallic-platform.md`, repo `CLAUDE.md` section 6.
- Website: `C:\Users\alexi\Documents\plateform` does not exist. `C:\Users\alexi\Documents\metallic-plateform` holds only
  `docs/platform-feature-list.md`. **The web front end is Next.js 16 + React 19 with plain CSS modules (no Tailwind)
  in `metallic-plateform-worktrees/web-ui/web/`, branch `platform/web-ui` of `Metallic-Club/plateform`** (private;
  other branches: `platform/design-20261030`, `platform/p1-backend`, `codex/platform-foundation`). Its
  `web/docs/DESIGN.md` already follows #199 and adds Titanium and Light ("Aluminium") themes; 16 pages are
  screenshotted at 1440 and 390 in `web/docs/screenshots/`, plus Launcher before/after shots at 960 x 600.
- Skills run: `ui-ux-pro-max` (design-system generator, product/color/typography/style/ux/icons searches, font licence
  data), its `design-system` and `ui-styling` sub-skills, `antislop:antislop-ui`, `taste-skill`.
- egui facts checked in the local registry (`epaint-0.37.0`): variable fonts work through `FontTweak::coords`
  (`VariationCoords`); `Shadow { offset: [i8; 2], blur: u8, spread: u8, color }`. crates.io: `egui-phosphor` 0.14
  depends on egui **0.36**, not 0.37.

## 2. The linked projects

### 2.1 nextlevelbuilder/ui-ux-pro-max-skill (MIT, 143k stars, pushed today)

What it is: a Claude/agent skill plus a Python BM25 search over CSV data: 79 styles (50 active), 192 product types
with palettes and reasoning rules, 74 font pairings, 119 UX guidelines, 105 Phosphor/Lucide icon entries, 25 chart
types, 22 stack guides, and a `--design-system` generator that merges them. A paid tier adds brand/logo/banner work.

What it returned for us (run, not guessed):
- `--design-system "game creation platform developer tool editor dark"`: pattern "FAQ/Documentation Landing", style
  "Dark Mode (OLED)", slate palette with a **green** CTA `#22C55E`, JetBrains Mono headings over IBM Plex Sans, "minimal
  glow". Product "Gaming": "3D & Hyperrealism + Retro-Futurism ... vibrant + neon". Colour "gaming dark violet":
  `#7C3AED` "neon purple + rose action". All three contradict the brand (no neon, no glow, restrained violet,
  symbol-only emblem) and the antislop/taste rules (mono headings are a known tell).
- Useful and adopted: the font licence table (`google-font-licenses.json`, from google/fonts METADATA.pb rev
  `038b637`): Archivo, JetBrains Mono, Inter, IBM Plex Sans, Geist all **OFL**; the UX guidelines on motion
  ("use shared motion tokens; deceleration entering, acceleration leaving; never one duration for everything"),
  toasts (auto-dismiss), empty states (message plus action); the three-layer token architecture
  (primitive -> semantic role -> component) from the `design-system` sub-skill; Phosphor as a recommended icon library.

Verdict:
- **Adopt** the data as a checking tool (licences, UX guideline lookups, its pre-delivery checklist minus the
  Tailwind items) and the three-layer token structure (we have layers 1-2; layer 3, component tokens, is added below).
- **Skip** its palette, style and typography recommendations for Metallic. Its "gaming" category is neon esports
  marketing, the opposite of the brand. Treat its generator output as a list of defaults to avoid.
- **Skip** the `ui-styling` sub-skill's stack (shadcn/ui + Tailwind). The website is plain CSS modules on shared token
  variables; adding Tailwind would duplicate the token layer.

### 2.2 The nextlevelbuilder organisation's other repos

19 public repos. Apart from ui-ux-pro-max they are agent infrastructure and their own sites: GoClaw (Go agent
gateway, 3.7k stars) with CLI/MCP/docs/webchat, skillx (skills marketplace), dewee (chat widget, Chrome extension,
site), agentbrain-cli, f2u-cli (temporary file hosting), nlb-web-v2, directory toolkits, an event deck. None has a
licence field set on GitHub. **Nothing to adopt for Metallic's UI.** (3dviz-pro-max, advertised in the README, lives
under a personal account, not the org.)

### 2.3 maxbogo/awesome-ai-tools-for-ui (no licence file, 64 entries, last push 2026-09-25)

A link list in five groups. Per group, what is worth using:

| Entry | Verdict | Why |
|---|---|---|
| Taste Skill, Impeccable, Hallmark, Refactoring UI skill, Swiss Design System | Taste is installed and used here; skip the rest | They overlap; one anti-slop gate (antislop + taste) is enough. |
| Web Design Guidelines skill (Vercel) | **Adopt for website PRs** | Installed as `web-interface-guidelines`; run it on `plateform/web` diffs. |
| Emil Kowalski skills | **Adopt for motion** | Installed (`emil-design-eng`, `animate`); source of the "exit faster than enter" rule below. |
| Awesome DESIGN.md / Curio | **Adopt the pattern** | A `DESIGN.md` that agents read first. The website has one; Studio should get a one-page pointer to the tokens and this doc. |
| The Shape of AI | **Adopt for Litha** | Catalogue of AI UX patterns (citations, inline actions, undo of AI edits); fits Litha inside Studio. |
| Laws of UX, Realtime Colors | Use as references | Realtime Colors previews a palette on a live page, handy for owner reviews of the light theme. |
| Paper Shaders | **Evaluate later** for Launcher animated wallpapers (owner ask, 2026-10-26) | Licence not checked yet; never on chrome, only as user-chosen wallpaper. |
| 21st.dev, Magic MCP, AI Canvas, UI Layouts MCP, shadcn skills, daisyUI | Skip | Generate React/shadcn/Tailwind components; wrong stack and the generic look we are leaving. |
| Stitch, Variant, Superdesign, Open Design, v0-style apps | Skip | Hosted prompt-to-UI generators; output is the category default. |
| Lazyweb (MCP of real app screens) | Optional | Useful for research of real launcher/store screens; not needed for this wave. |
| SVG Loaders, SVG Backgrounds, Book of Shapes, Ditther, Pryzm | Skip | Decorative patterns and spinners that the brand rules out. |

### 2.4 onlook-dev/onlook (Apache-2.0, 27k stars, last push 2026-10-21)

What it is: an open-source visual editor for **Next.js + Tailwind** apps. The project runs in a web container; the
editor shows it in an iframe; the parser adds a `data-oid` attribute to every JSX element and keeps a map from oid to
file, location and component (`packages/parser/src/ids.ts`, `apps/web/client/src/components/store/editor/ast/README.md`
explains how instances are resolved from DOM plus AST). Clicking an element jumps to its code; style edits are written
back to the source as Tailwind classes; layers panel, brand/token panel, pages, checkpoints, branches, AI chat with the
selected element as context. The README now says this repo is "the open-source visual editor that started Onlook"; the
company's next product is hosted and waitlisted, so expect slower open-source work. Non-Next.js and non-Tailwind
projects are listed as unsupported.

Fit:
- **Website: skip the tool.** Our site uses CSS modules, not Tailwind, so Onlook cannot write styles back. Switching to
  Tailwind to suit an editor would duplicate the token layer. No reason to run it.
- **Studio: adapt four ideas, no code** (different stack, and Studio already owns the hard part, a scene tree with
  stable instance ids):
  1. *Select in the running game, land on the source.* Onlook's oid map is what Studio needs for in-game UI and
     scripted objects: click a `ScreenGui` element or a part during Play, and Studio selects the instance and offers
     "Open script that created it" (needs a creation-site record per instance, like `data-oid`).
  2. *Edit visually, write back to the one source of truth* (one gesture, one undo step). Studio's building work
     (#213 batched SetProperties) is already this; keep the rule for the coming UI editor and Look presets.
  3. *Checkpoints and branches* map onto the change journal (#294): named checkpoints in the History panel.
  4. *AI chat with the selection as context*: Litha should receive the current selection and show its changes as a
     reviewable diff with one-click undo (also a Shape of AI pattern).

## 3. The Metallic design system (exact tokens)

Source of truth stays `design/tokens/metallic.tokens.json` -> `crates/metallic_tokens` -> CSS for Launcher and web,
Rust constants for Studio. Values marked **new** are proposals from this document; everything else is already on
`main` (#199). Mockup file with all of it: `design/mockups/2026-10-30-studio-start/assets/tokens.css`.

### 3.1 Colour

Primitives (unchanged):

| Ramp | 950 | 900 | 850 | 800 | 750 | 700 | 650 | 600 | 500 | 400 | 300 | 200 | 100 | 50 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Graphite `g` | `#0c0b0f` | `#111015` | `#16151b` | `#1c1b22` | `#24222b` | `#2c2a34` | `#35323e` | `#43404d` | `#5f5b6b` | `#817d8c` | `#a3a0ad` | `#c3c0ca` | `#e6e4ea` | `#f4f3f7` |
| Violet `v` | `#1f1c29` | `#2a2638` | | `#3a3354` | | `#584993` | | `#6f5cb6` | `#8673cf` | `#9d8be0` | `#b4a6ec` | `#cdc2f2` | `#e4def8` | `#f3f0fc` |

Silver and titanium are the light end of the graphite ramp (`g-200` titanium plate, `g-100` silver ink); there is no
metallic gradient anywhere.

Roles in three themes (Graphite is the default; Titanium and Light values are the website's, proposed **new** for the
shared JSON's empty `$themes`):

| Role | Use | Graphite (dark) | Titanium (lighter dark) | Light ("Aluminium") |
|---|---|---|---|---|
| `bg-deep` | title bar, text fields, viewport surround | `#111015` | `#1c1b22` | `#ffffff` |
| `bg-base` | window ground | `#16151b` | `#24222b` | `#eceaf0` |
| `bg-panel` | docked panels, rail, cards at rest | `#1c1b22` | `#2c2a34` | `#f4f3f7` |
| `bg-raised` | secondary buttons, selected card, menus | `#24222b` | `#35323e` | `#ffffff` |
| `bg-hover` | hover fill, toasts | `#2c2a34` | `#3c3946` | `#e3e1e8` |
| `bg-active` | pressed, pressed segment, keycaps | `#35323e` | `#43404d` | `#d8d5de` |
| `select` / `select-weak` | selected row / multi-select | `#3a3354` / `#2a2638` | `#3a3354` / `#2f2b3b` | `#e4def8` / `#f3f0fc` |
| `line` / `line-strong` | dividers / panel edges | `#2b2933` / `#43404d` | `#3a3744` / `#5f5b6b` | `#d8d5de` / `#c3c0ca` |
| `line-control` | input borders (3:1 non-text) | `#6e6a79` (3.61) | `#8a8695` | `#7a7685` |
| `text-1` / `text-2` / `text-3` | body / secondary / hints | `#e6e4ea` / `#aeaab6` / `#8e8a97` | `#f4f3f7` / `#c3c0ca` / `#b3afbd` | `#16151b` / `#43404d` / `#57535f` |
| `text-off` | disabled | `#5f5b6b` | `#6c6878` | `#a3a0ad` |
| `icon` | default glyph | `#bdbac4` | `#cfccd6` | `#43404d` |
| `accent` / hover / press | the one action, selection ring | `#9d8be0` / `#b4a6ec` / `#8673cf` | same | `#6f5cb6` / `#584993` / `#4d4082` |
| `on-accent` | text on accent | `#111015` | `#111015` | `#ffffff` |
| `accent-text` / `focus` | links / focus ring | `#b4a6ec` / `#b4a6ec` | `#c2b6f0` / `#b4a6ec` | `#584993` / `#584993` |
| `highlight` | 1 px top edge of raised plates | `rgba(255,255,255,.06)` | `rgba(255,255,255,.07)` | `rgba(255,255,255,.7)` |
| `edge-low` **new** | 1 px bottom edge of raised plates | `rgba(0,0,0,.30)` | `rgba(0,0,0,.30)` | `rgba(22,21,27,.08)` |
| `error` / `error-bg` | | `#ec7a72` / `#3a2226` | `#f2948c` | `#b3261e` / `#fbe9e7` |
| `warning` / `warning-bg` | | `#e0ad55` / `#352c1d` | `#e8bb6a` | `#7a5200` / `#fbf0d9` |
| `success` / `success-bg` | | `#62bf91` / `#1d3329` | `#74cc9f` | `#1d7048` / `#e2f2e9` |
| `info` / `info-bg` | | `#7fb0e6` / `#1c2a3a` | `#95bfee` | `#1f5c9e` / `#e3edf8` |
| `danger` / hover | destructive buttons | `#8f3a35` / `#a3443e` | same | `#b3261e` / `#96201a` |
| `art-scrim` | plate behind text on game art | `rgb(18 17 22/.78)` | `rgb(18 17 22/.76)` | `rgb(12 11 15/.74)` |
| `backdrop` **new for Studio** | modal scrim | `rgb(12 11 15/.60)` | `rgb(12 11 15/.55)` | `rgb(22 21 27/.40)` |

Contrast (computed, WCAG 2): Graphite text-1/base 14.39, text-2/panel 7.50, text-3/panel 5.07, text-3/raised 4.66,
on-accent/accent 6.50, accent-text/base 8.30; Light text-1/base 15.20, text-2/panel 9.14, text-3/base 6.27, white on
accent 5.41, accent-text/base 6.31; Light semantic on white: error 5.95 (`#b4372f` sample), success 5.31, warning
5.93, info 6.16. All pass AA for body text.

Colour rules: violet only on the one primary action per view, the selection ring, the focus ring and links; never on
icons, headings or decoration. Semantic colours always come with an icon and words. Colour on screen belongs to the
work (game art, template previews); chrome stays graphite.

### 3.2 Type

| Family | Licence | Source | Use | egui |
|---|---|---|---|---|
| Archivo variable (wght 100-900, wdth 62-125), v2.001 | SIL OFL 1.1, no Reserved Font Name | github.com/google/fonts `ofl/archivo` (upstream Omnibus-Type/Archivo) | everything; wdth 100 = interface voice, wdth 125 = display voice | one `FontData` per instance with `FontTweak::coords` (`wght`, `wdth`), named families `ui`, `ui-strong`, `display` |
| JetBrains Mono variable (wght 100-800), v2.211 | SIL OFL 1.1 | github.com/JetBrains/JetBrainsMono | code, Luau, numbers that change (coordinates, timers, FPS) | `FontFamily::Monospace` |
| Noto Sans | SIL OFL 1.1 | google/fonts `ofl/notosans` | fallback for Cyrillic and Greek (Archivo has none) | appended to every family's fallback list |

`deny.toml` already allows `OFL-1.1` (for egui's own fonts), and font files are not crates, so no policy change is
needed; issue #135 can be closed with that note. Bundle Latin + Latin Extended subsets only (#281). Turning off
`default_fonts` also drops the Ubuntu Font Licence (not on the project's list).

Studio scale (desktop density; px = egui points at 100 % scale):

| Role | Size / line | Weight | Width | Tracking | Use |
|---|---|---|---|---|---|
| `display` | 32 / 38 | 700 | 125 % | -0.012 em | one per screen (start screen question) |
| `title` | 22 / 28 | 680 | 125 % | -0.006 em | stage title, dialog title |
| `heading` | 15 / 20 | 620 | 112.5 % | 0 | panel and section headings |
| `body-lg` | 14 / 20 | 400 | 100 % | 0 | descriptions |
| `body` | 13 / 18 | 400 | 100 % | 0 | default UI text |
| `strong` | 13 / 18 | 580 | 100 % | 0 | labels, button text, names |
| `small` | 12 / 16 | 400 | 100 % | 0 | metadata, help text |
| `caption` | 11 / 14 | 540 | 100 % | 0 | counts, keycaps; never below 11 |
| `mono` | 12 / 16 (code 13 / 20) | 400 | | 0 | numbers and code only |

Web scale (16 px base, the website's): body 16/24, lead 18/28, h3 20/28, h2 28/34 at wdth 125, h1 40/44 at wdth 125,
hero display `clamp(48px, 7vw, 96px)` / 0.95 at wdth 125, weight 700, tracking -0.03 em. No uppercase tracked labels,
no monospace headings, no styled accent word.

### 3.3 Space, radius, the cut

- Space (4 px grid): `s1 2, s2 4, s3 6, s4 8, s5 12, s6 16, s7 20, s8 24, s9 32, s10 40, s11 48`, **new** `s12 64,
  s13 96` (web section gaps). Studio rows: tree/property rows 24 px, list rows 56-64 px, toolbar 36 px, title bar 40 px,
  panel padding 12, gutters between panels 1 px line, not gaps.
- Radius: `r-xs 3` (keycaps, thumbnails), `r-sm 4` (buttons, fields), `r-md 6` (cards, toasts, menus), `r-lg 10`
  (stage, splash, dialogs). Docked panels 0. No pills except the presence dot.
- The cut (identity motif): one 45 degree chamfer on the **top-right** corner of the single most important thing in
  a view: primary button `cut-sm 6`, selected card `cut-md 10` (12 on the 2 px frame), stage / splash / dialog
  `cut-lg 14`. Never on more than two things per view. egui: a convex polygon (`epaint::PathShape`), see #199
  `egui-implementation.md` section 5.

### 3.4 Elevation

| Level | What | Fill | Edge | Shadow (CSS) | egui `Shadow` |
|---|---|---|---|---|---|
| 0 ground | window, viewport surround | `bg-base` / `bg-deep` | none | none | `Shadow::NONE` |
| 1 docked | panels, rail | `bg-panel` | 1 px `line` on the side facing content | none | none |
| 2 raised | cards, secondary buttons, segments | `bg-raised` | 1 px `highlight` top, 1 px `edge-low` bottom **new** | none | none (two `line_segment`s) |
| 3 floating | menus, popovers, toasts, tooltips | `bg-hover` | 1 px `highlight` top | `0 8px 24px rgba(0,0,0,.45)` (light `.16`) | `offset [0,8], blur 24, spread 0, color black a=115` |
| 4 modal **new** | dialogs, splash window | `bg-panel` | 1 px `highlight` | `0 16px 48px rgba(0,0,0,.55)` + `backdrop` scrim | `offset [0,16], blur 48, color black a=140` |

Only levels 3-4 cast shadows. No glow, no glass, no backdrop blur (egui cannot blur anyway).

### 3.5 Motion

| Token | Value | Use |
|---|---|---|
| `t-fast` | 90 ms | hover and press fills, segment switch |
| `t-base` | 140 ms | menus open, focus ring, toast exit |
| `t-slow` | 200 ms | toast enter, tab content cross-fade |
| `t-panel` **new** | 240 ms | stage swap, panel collapse, progress fill |
| `t-splash` **new** | 320 ms | splash in, splash to start screen cross-fade |
| `ease-out` | `cubic-bezier(.215,.61,.355,1)` = `emath::easing::cubic_out` | everything that enters or responds |
| `ease-in-out` | `cubic-bezier(.645,.045,.355,1)` = `cubic_in_out` | things that move between two places |
| `ease-in` **new** | `cubic-bezier(.55,.055,.675,.19)` = `cubic_in` | exits only |

Rules: exits take about 0.7x the entry time; travel at most 8 px (toasts) or 2 % scale (splash); nothing idles or
loops (the only exception is a real progress indicator); `prefers-reduced-motion` / Studio's "Reduce motion" setting
sets 90/140/200/240/320 to 0/0/80/80/80. egui: `ctx.animate_value_with_time(id, target, secs)` with the token
duration and an easing applied to the 0-1 value.

### 3.6 Icons

**Phosphor Icons 2.1, MIT** (c) 2023 Phosphor Icons, github.com/phosphor-icons. Regular weight at 16 px for chrome,
20 px in toolbars; Fill weight for status (check-circle, warning-circle) only; no Bold in chrome. Every icon sits next
to a word, or has a tooltip and an accessible name; no sparkle, magic or robot glyphs (Litha has her own art). In egui,
load `Phosphor.ttf` and `Phosphor-Fill.ttf` as fallback fonts and keep a small `icons.rs` of the codepoints used,
generated from Phosphor's `selection.json`; do **not** add `egui-phosphor` (0.14 pins egui 0.36). The website's own
square-capped icon set (`web/src/components/Icon.tsx`) should move to Phosphor too, so the platform has one icon voice.

### 3.7 Component tokens (layer 3) and specs

| Component | Size | Rest | Hover | Pressed | Focus | Disabled |
|---|---|---|---|---|---|---|
| Button primary | h 36, pad-x 16, `strong`, icon 16, gap 8, `cut-sm` | `accent` fill, `on-accent` text | `accent-hover` | `accent-press`, 1 px down | 2 px `on-accent` ring inset 5 px (the cut clips outside rings) | `bg-raised` fill, `text-off`; keycap dims |
| Button secondary | h 32, pad-x 12, `r-sm` | `bg-raised`, `highlight` top, `edge-low` bottom | `bg-hover` | `bg-active`, 1 px down | 2 px `focus`, offset 2 | as above |
| Button ghost | h 32 (28 in title bar), pad-x 8-12 | transparent, `text-2`, icon `text-2` | `bg-hover`, `text-1` | `bg-active` | as above | `text-off` |
| Button danger | as secondary | `danger`, `text-1` | `danger-hover` | 1 px down | as above | as above |
| Keycap | h 16, pad-x 5, min-w 18, `caption`, `r-xs` | `bg-active`, `text-2`; on primary: `rgb(17 16 21/.16)` + `on-accent` | | | | |
| Card (template) | 16:9 art + label 9/12/11 padding, `r-md` | `bg-panel`, `highlight` top | `bg-raised` | | 2 px `focus` offset 2 | |
| Card selected | same | 2 px `accent` frame following `cut-md`, `bg-raised`, check-circle-fill 14 in `accent` before the name | | | | |
| Tabs: segmented (filters) | track `bg-deep` + 1 px `line`, pad 2; segment h 26, pad-x 10, `caption` 12/540 | `text-2` | `text-1` | selected: `bg-active` + `highlight` | 2 px `focus` offset 0 | |
| Tabs: panel (Explorer, Output) | h 32, pad-x 12, `strong` | `text-2`, no fill | `text-1` | selected: `text-1` + 2 px `accent` bottom bar (the one place violet marks place, not action) | ring | |
| Input | h 32, pad-x 10, `body`, `r-sm` | `bg-deep`, 1 px `line-control` | border `text-3` | | 2 px `focus` offset 1 + border `focus` | `bg-panel`, `text-off` |
| Input error | | border `error`; message below: warning-circle-fill 14 + `small` in `error` | | | | |
| Label / help | `field-label` 12/16 540 `text-2`, 6 above; help `small` `text-3`, 6 below | | | | | |
| Panel (docked) | header h 32: `heading`-weight 13/580 title + ghost icon buttons 24; body pad 12 | `bg-panel`, 1 px `line` edge | header hover shows collapse caret | | | |
| Toast | min-w 300, max-w 420, pad 12/14, `r-md`, icon 16 + `body` + optional ghost action | `bg-hover`, level 3 shadow, `highlight` | | | | |
| Toast timing | enter `t-slow` `ease-out` from +8 px; stay 4 s (error: until dismissed); exit `t-base` `ease-in`; max 3 stacked, bottom-right 20 px | | | | | |
| Empty state | in place of the list, `bg-raised` plate `r-md`, pad 16 | `strong` title that states the fact ("No games yet"), `body` `text-2` with the cause and what fills it, one action if the area has none nearby | | | | |
| Chip on art | h 26, pad-x 9, `r-sm`, icon 14 | `art-scrim` fill, `#e6e4ea` text in every theme | | | | |

## 4. Per-surface notes

### 4.1 Studio splash (mock: `splash.html`, `splash.png`, `splash-presented.png`)

- A 720 x 420 borderless, centred window (`ViewportBuilder::with_decorations(false).with_transparent(true)`), `r-lg`
  corners with the top-right `cut-lg`, level 4 shadow. Always Graphite (it shows before settings load; no theme flash).
- Left 272 px: a flat **titanium plate** (`g-200`) holding the emblem `branding/placeholder-icon.png` at 160 px,
  unchanged. The dark emblem reads on the light metal; this is the brand's "silver/titanium" as one field, not a
  gradient. Right: wordmark "Metallic / Studio" in Archivo 700 at 44/46, wdth 125 (second line `text-2`, 520),
  "Version 0.1.0, dev channel", and at the bottom a real, determinate progress: step name left ("Starting the
  renderer"), "2 of 4" right, a 2 px track with the `accent` fill animating with `t-panel`.
- Steps are the real startup phases (fonts and settings, renderer, templates, start screen). If startup takes under
  the minimum splash time (`app.rs` keeps a minimum), keep it short; never fake progress.
- Replaces `app.rs` `fn splash` (icon + "Metallic Studio" + "Starting...").

### 4.2 Studio start screen (mock: `index.html`; `start-1440.png`, `start-1440-light.png`, `start-1280-firstrun.png`, `start-1440-toast.png`)

Keeps #199's structure (it tested well) and changes what made the mock still feel generic in places:
- **Own title bar** (40 px, `bg-deep`): emblem 20 px + "Metallic Studio", a centred search "Search templates and recent
  games" with `Ctrl K` (filters both lists live), ghost Settings and Help, Windows caption buttons (close hover
  `#c42b1c`, the system red). Needs `decorations(false)` plus drag and resize handling; if Windows snap behaviour is at
  risk, ship the native title bar first and add this later (task S6).
- **Rail** (288 px, `bg-panel`): "Open a game folder" secondary button with `Ctrl O`; "Recent games" with a count;
  rows of 80 x 45 thumbnails (last viewport capture of the game), name, "Template, relative time"; a missing folder
  keeps its row with a warning line; the first-run empty state replaces the list. Footer: version and channel, update
  state with an icon ("Up to date"), Release notes, Third-party licences.
- **Stage**: one plate (`bg-panel`, `r-lg`, `cut-lg` top-right) split into the template's real render (Parkour uses
  `templates/parkour` art; chips "3D" and the Look on `art-scrim`) and the form: title, facts, description, "Inside",
  Game name, Folder + Browse, a live help line ("Studio creates C:\Games\My Parkour. It must be new or empty."),
  inline validation, and the one violet action "Create game" with `Enter`. Height `clamp(380, 100vh - 400, 456)`;
  "Inside" hides under 860 px tall so the button never clips at 1280 x 800.
- **Starting points**: a 6-up strip with a segmented filter (All, 3D, 2D, Empty). The selected card carries the 2 px
  violet frame that follows the cut. Templates without a render get Theme art (sky, horizon, ground in the template's
  lighting colours plus a Phosphor glyph); this is a fallback for thumbnails only. Every shipped template should have
  a real 1280 x 720 render for the stage (task S10); the Racing stage tried in this mock with Theme art at full size
  looked thin and was dropped.
- Honest content: only Parkour exists. Obby, FPS arena, Racing, Platformer, Empty 3D world and the recent game names
  are proposals for the mock.

### 4.3 Studio editor shell and panels (guidance for the session building issue #10; no code here)

- Layout: title bar (40) with the game name, Build/Script mode switch and the one violet **Play** button (`cut-sm`,
  `F5`); a 36 px tool row; Explorer left (min 220), viewport centre, Properties right (min 260), Output/Problems bottom
  (collapsible, 160). Panels are level 1: `bg-panel`, separated by 1 px `line`, no gaps, no rounded corners. The
  viewport surround is `bg-deep` so the game is the brightest thing on screen.
- Panel headers 32 px with a `strong` title and ghost icon buttons; panel tabs per 3.7 (the 2 px violet bottom bar is
  the only violet in chrome besides Play and selection).
- Tree and property rows 24 px; selected row `select` fill with `text-1`; multi-select `select-weak`; hover
  `bg-hover`. Property names `text-2` left column (40 %), values right; numbers in JetBrains Mono, right-aligned,
  tabular. Inline editing uses the input spec at 24 px height.
- Viewport overlays: selection box 2 px `v-300` with 1 px `g-900` inside and out (readable on any game colour); gizmo
  axes use their own red/green/blue with letters (not brand colours); viewport chips on `art-scrim`.
- Status bar 24 px `bg-deep`: left the current tool and selection count, right FPS and memory in mono, `text-3`.
- Problems and Output: severity icon (fill) + word + message; click jumps to the script line.
- Keyboard: every panel reachable with F6 cycling; focus ring 2 px `focus` always visible on keyboard focus.

### 4.4 Metallic Play (Launcher, WebView2)

- The Launcher is a shell around the website; its look is the website's. Its own bundled pages
  (`crates/metallic_launcher/assets/pages/`: offline, settings, demo) already use the tokens and the cut. Gaps: fonts
  fall back to system fonts (#281, serve woff2 from `local_pages.rs`), no Light/Titanium theme switch, and the
  window frame should match Studio's title bar spec once Studio's is proven.
- Window: minimum 960 x 600 (the website already lays out for it). Native title bar until the custom one ships in
  Studio; then reuse the same 40 px spec (emblem, page title, status chip only when something needs attention).
- Customisation the owner asked for (widgets, animated wallpapers): wallpapers are user content behind the home page
  only, under an `art-scrim` where text sits; chrome never animates.

### 4.5 Website (plateform `web/`), home, game page, sign-in

The site is the strongest surface today (screenshots viewed: `home-1440-graphite`, `game-1440-graphite`,
`signin-1440-light`). Notes:
- **Emblem: owner decision needed.** `web/src/components/Emblem.tsx` is a redrawn, faceted SVG version of the crow
  ("after the engine repo's placeholder crow"). That is a simplification of the companion/mascot art, which the brand
  rule forbids, and it gives the platform two different emblems (Studio shows the painted crow). Until the owner
  approves a final symbol, use `branding/placeholder-icon.png` (served as an image) on the site too. Do not redraw it.
- **Screenshots are older than the CSS** (`docs/screenshots` 02:46 vs `globals.css` 07:35 on 2026-10-30); the Play
  buttons in them are a saturated indigo that matches no token. Re-shoot before any review.
- **Home**: keep the art plate hero, the numbered "Most played" chart and the genre word wall (they vary rhythm well).
  Tighten: one primary action in the hero ("Play", violet, `cut-sm`); "Servers and details" secondary; the footer
  wordmark at full width is good, but swap its emblem per the decision above. Use `s13` (96) between sections, `s9`
  inside them.
- **Game page**: the two-column top (art left, title + live count + Play right) is right. Make Play the only violet on
  the page; Favourite and rating are secondary. The server table is a real table (lines allowed): rows 48, friends
  first, fullness bars in `text-2` not violet, Join as ghost buttons with a visible label.
- **Sign-in**: inside the Launcher it is the device flow and never shows a password field (already true). In the
  browser: one column, the form left at max 400 px, "Sign in" primary with the cut, the "another device" explainer as
  a `bg-panel` plate. Errors inline under the field. No social-login logo row.
- **Tokens**: `web/src/styles/metallic-tokens.css` is a hand copy of studio's tokens and `globals.css` re-declares
  the themes. After task T1 the site should import the generated CSS (all three themes) and delete its copies.
- **Fonts**: `next/font/google` downloads at build time, so no runtime Google call, but #281 wants one set of files
  for all surfaces; switch to `next/font/local` with the same woff2 subsets.

## 5. Checks run on the mockups

- Rendered with headless Edge (`--headless=new --screenshot`) at 1440 x 900 (dark, light, toast) and 1280 x 800
  (first run); splash at 720 x 420 and presented on a 960 x 600 backdrop. Every PNG was viewed after the last edit.
- Iterations: (1) selection ring was cut off at the chamfer: rebuilt as a 2 px frame plus inner plate, both chamfered;
  (2) rail footer wrapped: split into two lines; (3) light theme first rendered dark buttons because the theme was set
  after first paint with transitions running: theme now set in `<head>`; (4) form clipped at 1280 x 800: stage height
  clamps and "Inside" hides under 860 px; (5) splash v1 (dark plate, dark emblem, licence line) read flat and boxed:
  v2 uses the titanium plate and a larger wordmark; (6) Theme-art stage (Racing) looked thin: dropped from the mock.
- Contrast pairs computed (section 3.1). No em dashes in the mock files. Interactions (card select, arrows, filter,
  search, validation, toasts, `Ctrl K`, `Ctrl O`, `F5`) are written but were checked only through renders driven by
  URL parameters (`?pick=`, `?toast=`, `?state=firstrun`, `?theme=`), not clicked through in a browser.
- Not verified: anything in egui. The mock uses only things egui can draw (flat fills, 1 px lines, convex polygons,
  epaint shadows, images, two-colour mesh gradients for Theme art), but nothing was compiled.

## 6. Prioritised tasks (each sized for one agent)

P0 makes Studio stop looking like default egui; do S1-S3 first, in order.

| # | Pri | Task | Files to touch | Done when |
|---|---|---|---|---|
| S1 | P0 | Bundle Archivo (variable), JetBrains Mono, Noto Sans and Phosphor regular+fill (Latin + Latin Ext subsets) with OFL/MIT texts; register families with `FontTweak::coords` (wdth 100/125, weights 400/580/620/700); turn off eframe `default_fonts`. Studio part of #281; close #135 with the deny.toml note | `crates/metallic_studio/Cargo.toml`, new `crates/metallic_studio/assets/fonts/*`, new `crates/metallic_studio/src/theme/fonts.rs`, `src/theme.rs` -> `src/theme/mod.rs`, licence notice file | `--screenshot --scene start` shows Archivo; `cargo deny check licenses` passes |
| S2 | P0 | Drive Studio's `Visuals`/`Spacing` from `metallic_tokens` (roles, radii, shadows, row heights, widget states per 3.7); delete the hard-coded constants | `crates/metallic_studio/src/theme/mod.rs`, `crates/metallic_tokens/src/lib.rs` (add a `const`/build-time Rust export if `color()` at runtime is awkward) | No hex literal left in Studio's theme; screenshot of start + splash attached |
| S3 | P0 | Widgets: `cut.rs` (chamfer polygon, frame variant), `button.rs` (primary/secondary/ghost/danger + keycap), `plate.rs` (highlight + edge-low) | new `crates/metallic_studio/src/widgets/{mod,cut,button,plate}.rs` | Unit tests for polygon points; components screenshot |
| S4 | P0 | Splash per 4.1 (borderless window, titanium plate, real step progress) | `crates/metallic_studio/src/app.rs`, new `src/splash.rs`, `src/main.rs` (viewport) | `--scene splash` PNG matches `splash.png` |
| S5 | P0 | Start screen per 4.2 without the custom title bar: rail with thumbnails and relative times, stage with art + form + validation, starting-points strip with filter, empty states, toasts | `crates/metallic_studio/src/start_screen.rs`, `src/recent.rs`, `src/templates.rs`, new `src/widgets/{card,toast,segmented}.rs` | `--scene start` PNG close to `start-1440.png`; keyboard path works |
| T1 | P1 | Fill `$themes` with Titanium and Light; add `edge-low`, `backdrop`, `shadow-modal`, `t-panel`, `t-splash`, `ease-in`, `s12`, `s13`; export one `[data-theme]` block per theme | `design/tokens/metallic.tokens.json`, `crates/metallic_tokens/src/lib.rs`, `examples/export_css.rs`, `tests/css.rs`, regenerated `design/tokens/metallic-tokens.css` | CSS test covers all three themes |
| T2 | P1 | Contrast test: every text/icon role on every background role in every theme meets 4.5 (text) / 3.0 (controls) | new `crates/metallic_tokens/tests/contrast.rs` | Test fails on a planted bad pair, passes on the real set |
| S6 | P1 | Custom title bar (decorations off, drag, resize borders, snap, caption buttons, `Ctrl K` search) | `crates/metallic_studio/src/app.rs`, new `src/title_bar.rs` | Win11 snap layouts and double-click maximise still work |
| S7 | P1 | Settings: theme (Graphite/Titanium/Light/System), interface size 90-150 %, reduce motion (#134) | `crates/metallic_studio/src/settings.rs` (new or existing), `src/theme/mod.rs` | Switching theme repaints without restart |
| S8 | P1 | Apply the same tokens to the anim and VFX editors' themes | `crates/metallic_anim_editor/src/theme.rs`, `crates/metallic_vfx_editor/src/theme.rs` | Both reuse `metallic_studio`/shared theme code |
| L1 | P1 | Launcher pages: serve the woff2 fonts, add theme switch support, align offline/settings pages to 3.7 | `crates/metallic_launcher/src/local_pages.rs`, `assets/pages/{pages.css,offline.html,settings.html}` | Pages render Archivo with the network off |
| W1 | P1 (owner decision first) | Website emblem: replace the redrawn `Emblem.tsx` with the real branding art until a final symbol is approved | plateform `web/src/components/Emblem.tsx`, `web/src/app/icon.png` | Same emblem on site, Launcher and Studio |
| W2 | P1 | Website imports generated token CSS (after T1); remove hand copies; self-host fonts with `next/font/local` | plateform `web/src/styles/metallic-tokens.css`, `web/src/app/globals.css`, `web/src/app/layout.tsx`, `web/public/fonts/*` | One token source; build passes |
| W3 | P1 | Re-shoot web screenshots and review home, game, sign-in against 4.5 (one violet action per page, emblem) | plateform `web/docs/screenshots/*`, small CSS fixes in `web/src/app/**/*.module.css` | New screenshots attached to the PR |
| W4 | P2 | Website icons to Phosphor (one icon voice across surfaces) | plateform `web/src/components/Icon.tsx` and users | No custom icon paths left |
| S9 | P2 | Editor shell polish pass after #10 lands: panel headers, tabs, rows, status bar per 4.3 | files #10 creates under `crates/metallic_studio/src/` | Editor screenshot reviewed against 4.3 |
| S10 | P2 | Every template ships a real 1280 x 720 stage render and a 640 x 360 thumbnail, rendered by the engine | `templates/*/preview*.png`, `crates/metallic_studio/src/templates.rs` | No Theme-art fallback on the stage for shipped templates |
| S11 | P2 | Studio `DESIGN.md` pointer for agents (tokens, this doc, #199, the mock) | new `docs/design/DESIGN.md` | Linked from `AGENTS.md` |
| L2 | P3 | Research Launcher animated wallpapers (Paper Shaders licence and cost, WebView2 GPU use, reduce-motion) | new `docs/research/<date>-launcher-wallpapers.md` | Recommendation with licence verified |

## 7. Open questions for the owner

1. The emblem: the website's faceted crow is a redraw of the painted crow. Which one is Metallic's symbol, or is a
   final symbol still coming? Until then this document uses the painted `branding/placeholder-icon.png` everywhere.
2. Splash: the titanium plate (light metal behind the emblem) is new. Keep it, or stay all-graphite like #199?
3. Default Studio theme stays Graphite; Light is offered in Settings. Is that right for a tool used by kids in daytime?
