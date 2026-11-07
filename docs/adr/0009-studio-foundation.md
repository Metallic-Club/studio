# ADR 0009: Studio foundation

- **Status:** Accepted (2026-11-06)
- **Issue:** #210
- **Builds on:** [ADR 0004](0004-runtime-rendering-platform.md) (own loop, `winit`, `wgpu`), [ADR 0006](0006-project-format.md)
  (`.mtproj` projects), [ADR 0008](0008-rendering-performance.md) (renderer).

## Context

The owner opened the first Studio slice on 2026-11-06:

1. a clean UI system with **dockable windows** and **settings** (themes, general, rendering, keybinds), improving on
   Forge-Engine's keybinds;
2. a **loading splash** like the archive's (`Metallic-Club/studio-achive-`);
3. a **sign-in or offline** screen (only offline works; there are no servers yet);
4. a **projects page** like Roblox's launcher, with thumbnails (blank for now);
5. selecting, creating or adding a project opens the **editor** with basic docked windows: Explorer, Properties,
   Assets, Output;
6. opening a **`.mtproj` file** goes straight to the project;
7. the **engine renders the viewport**.

"Start with the basics and work your way up after building a solid, scalable foundation." Only this slice is built
now. The rest of M5 (#21: gizmos, undo, Insert menu, script editor, Project Settings) waits for the owner's next list.

Two references were studied:

- **Forge-Engine** (reference only: none of its UI code, physics or unique classes is copied). It is built on Bevy 0.19
  with bevy_egui (egui 0.36, egui_dock 0.21, wgpu 29). Its viewport is a transparent hole in the UI over a Bevy camera
  that draws into the window, so it cannot carry over to our engine. Its keymap (one action table with stable ids,
  override-only storage, a key-capture flow) is a good start with known weaknesses, listed under "Keybinds".
- **The archive** (the owner's earlier Studio): egui 0.37 / eframe on **wgpu 30**, our version. It has a specified
  splash, design tokens and three themes (Graphite, Titanium, Light) with OFL fonts. Its splash layout is kept; its
  look is replaced by the website's (see Theme). It never built docking, sign-in,
  a launcher or file association.

## Decision

### Toolkit: egui on our own loop and the engine's device

- **egui 0.37** (immediate mode; proven on wgpu 30 by the archive) through **`egui-winit` + `egui-wgpu`**, driven by
  Studio's own `winit` loop. **Not eframe:** eframe owns the device and the loop, and Studio needs the engine's
  `RenderContext` to own the device so the viewport and the UI share it (no copies between devices).
- Exact crate versions are pinned when the shell lands (#211). If an egui release built for wgpu 30 isn't available,
  that issue stops and reports instead of downgrading wgpu.
- UI tests use `egui_kittest` (feed input, query widgets, take snapshots).

### Layout of the code

Everything lives in `apps/studio` for now (ADR 0001), as a library plus a thin binary, in modules: `app` (loop,
screens), `theme`, `settings`, `keymap`, `dock`, `panels/*`, `viewport`, `projects`. A module moves to its own crate
only when a second app needs it.

### Screens

One window, one state machine: **Splash → Sign in → Projects → Editor**. Opening a `.mtproj` from the command line goes
**Splash → Editor**.

- **Splash** (the archive's layout, recreated, not copied, and restyled with the website's tokens; see Theme). A
  720×420 borderless, centred window with 12 px rounded corners. A light neutral panel with the Metallic mark (the
  website's `favicon.svg`, also the window icon) on the left; a dark body with the two-line "Metallic / Studio"
  wordmark, the version line, the real step name, "n of N" and a 2 px accent bar. It fades in, and it is always the
  dark mode with the default accent (shown before settings load). Its
  steps are **real work only**: settings and fonts, the render device, the first-frame pipelines (#86), the recent
  projects list. It stays at least 700 ms so it never flashes. Then the same window becomes the main window, centred
  where the splash was.
- **Sign in:** "Sign in" is disabled with "Online services aren't available yet", and "Continue offline" goes on. A
  "Don't ask again" box is saved in settings. There is no account code yet, just the screen and the seam.
- **Projects** (Roblox-launcher-like): a left rail (Recent, Templates), a grid of cards (thumbnail area, blank for
  now; name; last opened; a warning if the file is missing), **New** (from `templates/`, #187), **Add** (pick an
  existing `.mtproj`), and open with a double-click or Enter. The list is `projects.toml` in the config folder. A
  corrupt file is renamed to `.bak` and the list starts empty.
- **Editor:** opening a project runs `Project::open` + `load_start_scene` (#187) and the asset loading (#189) behind a
  progress overlay, then shows the docked editor. Failures go back to Projects with the error.

### Docking

- **`egui_dock`** (tabs, splits, floating windows), in the release built for egui 0.37. If none exists, use
  `egui_tiles` and record it in #211.
  - **Decided in #216: `egui_tiles` 0.18.** egui_dock had no release for egui 0.37 (its newest, 0.21, is built for
    egui 0.36), and egui_tiles 0.18 is (the egui authors' own docking crate: tabs and splits, MIT or Apache-2.0). It
    has no floating windows, so a panel lives in a tab bar of the main window; Settings opens as a tab beside the
    viewport, and stays a floating window on the screens before the editor (since #238 it is a floating window in
    the editor too; see Settings). Tab cycling with Ctrl+Tab is turned off, because every shortcut goes through the
    keymap.
- **A panel registry:** every panel implements one trait (stable id, title, icon, default zone, `ui()`), and one list
  registers them. Adding a panel is one new type plus one line, not an enum, a match arm and zone logic (Forge's pain
  point).
- Panel state lives in the editor, not in the layout. A panel is open if and only if it is in the dock tree; the View
  menu, shortcuts and close buttons all use that one fact.
- The layout is saved per user as versioned JSON in the config folder. A layout with unknown or duplicate panels is
  repaired on load (they are dropped, and the default layout is used if nothing is left). **Reset layout** is in the
  View menu (since #239, in the Window menu of the Roblox-style menu bar: File, Edit, View, Plugins, Test, Window,
  Help, with items for features still to come shown disabled).
- **Panel borders** (#262, #273): docked groups sit flush, with no gap between them or around the dock. A line exactly one
  physical pixel wide, snapped to the pixel grid (#287), in the border colour parts neighbours (square: rounded corners looked wrong on docked panels); a side along the
  window's edge has none. Settings → Appearance → Panel borders turns the lines off (on by default).
- **Default layout:** Viewport in the centre with Output and Assets as tabs under it (Output first and in front,
  #323); on the right, the Explorer over the Properties, two fifths of the column to three (#256, #323).
- **Panels in this slice:** Viewport; Explorer (the DataModel tree, selection, search; since #274 its rows touch and are striped like Properties'); Properties (the selected
  instance's properties from `PropMeta`, **read-only in this slice**, since editing comes with undo); Assets (the
  `.mtproj`'s asset index, #188); Output (engine and script log, with filter and clear).

### Toolbar (#232)

- **Under the menu bar, with tabs.** By default Home, Model, UI, Scripts and Plugins; Home holds Studio's tools (Save, the
  game's controls, the panel toggles, Settings), the others start empty. Tabs can be added (+), renamed (double-click
  or right-click), deleted and moved (right-click or drag). Switching tab changes the toolbar only, unless the tab's
  **"Load a layout for this tab"** box is ticked (UI's is by default): then the dock shows that tab's own layout,
  started from the main one, kept up to date while it shows, and the main layout (the only one `layout.json` holds)
  comes back when another tab is in front.
- **Elements:** tools, spacers and separators. Right-click the toolbar to add one, or one of them to add another
  before it, move it or remove it; or drag one along the row.
- **A tool registry** (`toolbar::ToolRegistry`): every tool has an id, a label, an icon, a description and an action.
  Studio registers its own at start; **the plugin API will register plugins' tools the same way** (`Tool::plugin`,
  ids `plugin.<plugin>.<name>`), and they appear in the Add menu under Plugins. A click reports the tool's action and
  Studio carries it out, which is how a plugin's will reach its plugin; until then plugins' tools are disabled, and a
  tool no longer registered shows greyed out.
- Saved per user as `toolbar.json` beside the settings (versioned, repaired on load), each tab's own layout in it.

### Theme

The reference is the **Metallic website** (`Metallic-Club/website`: `src/design.css`, which overrides
`src/reference.css`, and `src/themes.js`), so Studio and the site look like one product. It replaces the archive's
Graphite look (decided by the owner on 2026-11-06, in #211).

- **Design tokens in one Rust module** (`apps/studio/src/theme/tokens.rs`). A look becomes a full `egui::Style`,
  re-applied only when it (or, in System mode, the system's mode) changes.
- **Dark** (the default): bg `#161616`, header `#1d1d1d`, panel `#1b1b1b`, surface `#262626`, surface-hover
  `#333333`, input `#1c1c1c`, border `#414141`; text `#f6f6f6`, muted `#c1c1c1`, subtle `#adadad`.
- **Light:** bg `#f5f5f5`, header and surface `#fff`, surface-hover `#efefef`, input `#f7f7f7`, border `#cfcfcf`;
  text `#262626`, muted `#565656`, subtle `#646464`.
- **Modes:** Dark, Light and System (follows the OS, dark when it says nothing). A **high-contrast** option makes
  muted and subtle text the body colour and the border `#898989` (dark) or `#6d6d6d` (light).
- **Accent, chosen by the user:** orange `#ff9b54` by default (hover `#ffb47f`, light mode `#a34506`), or blue
  `#82b5ff`, violet `#ba9aff`, mint `#72d9b1` or rose `#ff9bb5` (their hovers and light-mode values as in
  `themes.js`); ink on an accent fill `#24160d`. Used sparingly: the 3 px focus outline, the selected list item's 3×19 px
  left bar (since #288 the active dock tab has no accent underline: its raised fill marks it), text selection (accent fill, ink text), progress. **Never a
  button's fill.**
- **Buttons:** primary is neutral (`#ebebeb` with `#242424` text, white on hover; `#303030`/white in light mode);
  secondary is a surface with a 1 px border that goes to subtle on hover; quiet is transparent with muted text; "play"
  (run) is green `#39b878` (hover `#53d08f`, text `#0d281a`). Disabled is 48 % opacity. Since #302 the top bar's play
  controls have no coloured fill: Play is a filled green icon, Stop a filled red one while a game runs, Pause and Step
  the text colour, each greyed out while it cannot be used.
- **Shape:** **no chamfered corners anywhere.** Radii 8 (buttons, items), 9 (inputs), 12 (panels, cards, the
  splash), 15 (dialogs); 1 px borders. Shadows only on what floats: menus and popovers (`0 8 24 #0005`) and dialogs.
- **Type:** **Inter** (OFL 1.1) for the interface: heavy headings with tight negative tracking, body 14–15 px,
  controls 14 px (weight 650 once a variable Inter is bundled), uppercase 10–11 px section labels with +1.3 px
  tracking. JetBrains Mono (OFL 1.1) for code. Phosphor icons (MIT); since #257, Lucide icons (ISC,
  https://lucide.dev). All bundled with their licence files. egui's default fonts are replaced, because one of them
  is under a licence `deny.toml` rejects.
- **Icons** (#231): every icon is an entry in one table (`theme::icons::Icon`): a stable id (`explorer.part`), a
  group and a default Lucide glyph. **Groups are coloured** so sections are easy to tell apart. Since #290 they follow the owner's groups, moderately
  saturated: the workspace, server scripts and the server script service green; local scripts blue; module scripts
  purple; bindables pink; lighting and lights yellow; storage services orange; input and settings services cyan;
  parts, models, folders and cameras white (near-black on light); values and the rest grey. Theme tokens hold a dark
  and a darker light-mode colour for each; high contrast pulls them towards the text.
  Selection still wins: a selected row's icon is the accent. A **custom icon folder** (Settings → Appearance → Icon
  folder) replaces any icon with `<id>.svg` or `<id>.png`; it is read and rasterised (resvg, png) on a background
  thread when the setting changes or Reload is pressed, and cached as textures. A monochrome image takes its group's
  colour; a coloured one is drawn as it is; a bad file keeps the built-in icon with one warning in Output. The ids are
  listed in `docs/studio-icons.md`, generated from the table.
  **Since #291 the icons are files, not compiled in:** Lucide's SVGs ship in an `icons` folder beside the
  executable (the build copies `apps/studio/icons` there), with `icons.json` naming the SVG each icon id uses and
  what it marks; pointing an entry at another file changes that icon, and a missing or bad file falls back to the
  default with one warning. Each icon is rasterised (resvg) at exactly the pixels it is drawn at and cached per size,
  so it stays sharp at every scale. The Lucide font is gone from the binary.
  **Since #312** Studio reads them from `icons` in its own folder (`%APPDATA%\Metallic\Studio\icons`; Windows
  folders are now `Metallic`, capitalised, and a lowercase one is renamed): on start it copies any file missing there
  from the icons shipped beside the executable, so the user's edits are kept. Games go in `Documents\Metallic`,
  made on start, and the projects page lists every game in it.
  Since #278 egui's own glyphs are gone too: drop-downs draw Lucide's chevron, Studio's windows (Settings, About) have
  their own header with Lucide's close button, and checkboxes draw Lucide's check. Look-alike classes have their own
  icons (LocalScript, each value type, BindableFunction), still in their section's colour.
- **Motion:** 0.12–0.2 s, eased. **Reduced motion** (a setting, and the OS's switch where Studio can read it) turns
  off fades and eases.

### Settings

- **One typed `StudioSettings`** in categories **General**, **Appearance** (theme, UI scale, reduced motion),
  **Rendering** (preset, AA, frame cap, VSync and the other #84 overrides, applied to the viewport) and **Keybinds**.
  Stored as `settings.toml` in `metallic_platform`'s config folder for the app "Studio". Every field has a default and
  is clamped on load.
- **Applied live;** written atomically (temp file and rename) only when something changed. **A corrupt file is never
  overwritten:** it is renamed to `settings.toml.bak`, Studio starts on defaults and says so once.
- **The Settings window** is a dockable panel with a category list and a search box over every setting.
  - **Amended in #238 (owner, 2026-11-07):** Settings is a **floating window** on every screen, the editor's included,
    and never a docked panel. It is moved, resized and closed like a window. While the user's last click was in it,
    the panels under it get no shortcuts, but the editor's own commands (save, close, reset layout) still run.

### Keybinds (improving on Forge's)

- **Commands, not hard-coded keys:**
  - Every action is a command in one registry: a stable id (`file.save`, `view.reset_layout`,
    `viewport.fly_forward`), title, category, description, the contexts it applies in, its default bindings per
    platform, and a repeat policy.
  - Menus, the command palette (later), tooltips and the keymap all read the registry.
  - Lookup is a hash map built once, not a scan every frame.
- **Everything goes through the keymap.** No panel reads keys directly, except text editing inside a focused text
  field and Esc to cancel a capture. (Forge's Assets panel read F2 and Delete itself and could double-fire.)
- **Context from focus, not hover:**
  - The focused panel decides the context. Contexts nest: Global → Editor → a panel (Viewport, Explorer, …), and the
    most specific binding wins.
  - Hover only matters for starting viewport navigation (mouse-look and wheel) when the pointer is over the viewport.
  - A focused text field blocks every command not marked "while typing".
- **What a binding is:** keyboard keys, **mouse buttons** (including drag gestures such as right-drag to look) and the
  **wheel**, with Ctrl, Shift, Alt and Super. **"Primary"** means Cmd on macOS and Ctrl elsewhere, so defaults are
  per platform without duplicate tables. Two-step sequences (Ctrl+K then S) are in the data model; the UI for them
  can come later.
- **Physical or logical keys:** movement defaults (WASD, QE) bind **physical** keys so they work on AZERTY; command
  shortcuts (Primary+S) bind **logical** keys so the letter matches the label. Each binding records which it is, and
  the editor shows the user's layout.
- **Repeat policy:** commands ignore key auto-repeat by default (holding Primary+D duplicates once). Navigation and
  nudges opt in.
- **Conflicts:** detected across overlapping contexts. Instead of only refusing, the capture dialog offers
  **Replace** (unbind the other), **Swap**, or **Cancel**. (Since #266, the command's row offers them: there is no dialog.) Menus and tooltips show every binding of a command, not
  just the first.
- **Storage:** only overrides are saved (`[keybinds] "file.save" = ["Primary+S"]`; `[]` means unbound). A new default
  never takes a key the user already uses, and Studio tells the user when that happens. A keymap can be **exported and
  imported** as a file. Unknown ids are kept, not dropped, so a newer Studio's bindings survive a downgrade.
- **The Keybinds settings page:** search by name, key or description; group by category; click to capture; right-click
  to add or remove a binding; reset one, or reset all.
  - **+** (#265): a small + after each command's shortcuts adds another.
  - **Sections** (#264): commands are grouped in collapsible sections from their category path (`Viewport/Fly camera`
    is Fly camera inside Viewport), so sections nest. A top-level section is a bordered card; one inside it is
    indented along a line. A section with a changed command shows Reset, which resets everything in it.
  - **Recording in place** (#266): the shortcut being recorded shows "..." (then what is held) instead of a dialog.
    Everything held counts, in any order, and the binding is saved once every key and button is released; a click
    on it records a mouse button, the wheel over it the wheel. Esc or a click elsewhere cancels, and a recording
    that is no longer on screen stops. Right-click → "Record a two-step sequence" records Ctrl+K, then S.

### Viewport: the engine renders, the UI shows it

- The viewport panel asks the engine to render the open scene into a **texture of the panel's size** (physical pixels)
  (while a resize is under way, #311: the last frame is drawn at its own size, centred, never stretched)
  on the **shared device**. Studio registers that texture with `egui-wgpu` and draws it as an image, so the UI can draw
  overlays on top, which Forge's hole could not. The engine side is a new render API, filed for `area:render` (#213).
- When the panel is resized, the target is recreated through #152's off-the-frame path and the old one is kept until
  the new one exists, so there is no black frame. Rendering presets, AA and the frame cap come from the Rendering
  settings.
- **The editor camera** (orbit and fly) is driven by keymap commands. Input reaches the viewport only while it has
  focus, or is hovered for navigation.
- **Frame pacing:** continuous while the viewport is visible and the window focused; reactive (redraw on input or
  change) otherwise, to save power.

### Opening `.mtproj` files

- `metallic_studio <path.mtproj>` opens straight to the editor (Splash → Editor) and adds the project to Recent.
- **The file association** (Windows `HKCU\Software\Classes`; Linux an XDG MIME type and a `.desktop` file; macOS the
  bundle's `Info.plist` at packaging) is registered **per user** by an explicit action: the installer (M8), or
  `metallic_studio --register-file-types`. It is **never** registered automatically or by tests.
- A second launch with a file opens a second Studio for now. Single-instance forwarding is a later issue.

## Not in this slice

Gizmos, selection in the viewport, undo and redo, editing properties, the Insert menu, the script editor, Project
Settings, real sign-in, thumbnails, and single-instance forwarding. These wait for the owner's next list.

## Consequences

- One device for the engine and the UI: the viewport is a texture, not a copy, and overlays are free.
- Adding a panel, a setting or a command is local: one type and one registry line.
- The keymap is the only path from input to actions, so rebinding covers everything, and testing a shortcut means
  feeding input to the keymap.
- Studio depends on egui's release cadence matching wgpu's. The archive shows it does today.
