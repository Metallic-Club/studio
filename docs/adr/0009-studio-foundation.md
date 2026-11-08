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
- **Windows of their own (#331, owner, 2026-11-07):** the shell runs egui's *immediate viewports*: Studio code shows a
  native window beside the main one with `Context::show_viewport_immediate`, and the shell opens it, draws it with the
  same device and `egui-wgpu` renderer (one set of textures for every window) while the main window draws, routes its
  events to it, and closes it once it is no longer shown. Settings is the first such window. The tests have no
  windowing system: there egui embeds such a viewport as a floating window over the main one.

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
- **Panels in this slice:** Viewport; Explorer (the DataModel tree, selection of several (#347), search; since #274 its rows touch and are striped like Properties'); Properties (the selected
  instance's properties from `PropMeta`, read-only until undo came, edited in place since #349); Assets (the
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
  **Since #402 the shipped icons are built into the executable** (the build lists `apps/studio/icons` with
  `include_bytes!`), because a Studio given to someone without the folder beside it drew none: startup writes them
  into the user's folder, and anything missing or unreadable there is read from them.
  **Since #403 edits to `icons.json` show at once**: Studio looks at the file's time about once a second and reads
  it again when it changed (colours at once, files on the next draw). On start the user's file is brought up to date
  (format 3): every icon and colour group Studio has is listed, the user's choices kept, and each icon's description
  line is gone (the ids say what they are; `docs/studio-icons.md` lists them).
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
  - **Amended in #331 (owner, 2026-11-07):** Settings opens in a **window of its own**, as Roblox Studio's does: the
    system's title bar and close button, centred over Studio's window when it opens. Keys typed in it reach it alone
    (a keybind being recorded reads them there). Where no window can open (the tests), it floats as #238 describes.

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

- The viewport panel asks the engine to render the open scene **at the panel's size** (physical pixels) on the
  **shared device**, into the top-left corner of a texture kept larger than the panel (#330, as Forge-Engine drew its
  view into part of a larger target): a resize is drawn at the new size in the same frame, never stretched and never
  late. The panel works out the texture's size for the frame as the engine does (`ViewportTarget::room_for`) and
  shows exactly the corner drawn. Studio registers that texture with `egui-wgpu` and draws it as an image, so the UI
  can draw overlays on top, which Forge's hole could not. The engine side is a new render API, filed for
  `area:render` (#213).
- The texture grows only when the panel outgrows it, by a fifth more than asked for, and then at once before the
  frame (made in the frame, not off it as #152's settings switches are, so the panel never waits); it never shrinks.
  Rendering presets, AA and the frame cap come from the Rendering settings.
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

### Editing (the owner's list of 2026-11-07, #346-#359)

Built after Forge-Engine's editor, which the owner wrote with these systems in it.

- **Selection (#347).** Several instances, in the order picked; the last is the active one, which Properties' header
  and the camera's orbit lead with. Explorer clicks: alone selects only that row, Ctrl (Cmd) adds or removes it, Shift
  selects from the anchor (the last row clicked without Shift), Ctrl+Shift adds that range; Edit, Select all
  (Primary+A) takes every row shown. A closed row holding something selected has a dot. The editor and a running game
  each have their own; what leaves the game drops out of it before each frame's panels.
- **Undo and redo (#348)** are Forge's command history, kept by Studio and not by the data model: in the editor only
  Studio changes the scene (no script runs there), so every change goes through the scene's edits (`Scene::set`,
  `Scene::move_to`), which note it with what it was before. The app ends the action under way once a frame, except
  while a mouse button is held, so a drag is one step; edits of the same properties under 0.6 s apart join one step,
  and a step holds the selection before and after when it changed (a click that only selects is a step, as in
  Forge). A delete only takes the instance out of the tree (the renderer draws Workspace's descendants only), so undo
  brings back the same instance where it was among its siblings (#346); instances out of the tree are destroyed once
  no step can bring them back. 1000 steps are kept. The title's `*` follows the history's revision against the one
  saved. A running game has its own history, dropped at Stop. Edit, Undo (Primary+Z) and Redo (Primary+Y,
  Primary+Shift+Z), and Undo and Redo tools for the toolbar.
- **Properties edits in place (#349)**, each change written through the scene (an undo step; a drag is one, a typed
  value is one, written when its field is left): a check box for booleans; fields to drag or type in for numbers and
  each component of a vector (within the property's range; a slider for `Widget::Slider`); a text field for strings
  (Enter or leaving it writes, Escape drops); a `CFrame` as two rows, its position and its orientation in degrees; a
  colour's swatch opens a picker beside its 0-255 channels; a drop-down of an enum's items. References (the Parent
  changes by dragging in the Explorer), read-only properties and computed ones without a setter are shown, not
  edited. Every control carries its property's label for screen readers (`Size X`).
- **Several instances in Properties (#350)**, as Forge's inspector: the properties they all have (same name and
  type), a value they disagree on shown mixed (a dash, "Mixed"); an edit reaches each of them as one undo step, a
  vector's component alone on each so they keep their others. The header names the count ("3 Parts", "4 objects").
- **Tree edits (#351)**, one undo step each, on the selection's top-level instances (services never move): Rename
  (F2, in place in the Explorer row: Enter or a click away keeps it, Escape drops it), Delete (Delete), Duplicate
  (Primary+D, the copies just after the originals, selected), Cut / Copy / Paste (Primary+X / C / V: beside the active
  instance, inside a selected service, in Workspace with nothing selected) and Paste into (Primary+Shift+V). The
  clipboard holds copies out of the tree, so a paste copies them again; a running game's clipboard does not paste into
  the editor's scene. The Edit menu has them, and a right-click on an Explorer row opens a menu of them.
- **Insert Object (#352)**: a searchable window of every class the registry can create (not services, not classes
  hidden from the Explorer), with icons and descriptions, opened from a row's `+` (shown on hover) or its menu; the
  class chosen is created inside that row, selected, as one undo step, a part in front of the editor camera.
- **Dragging Explorer rows (#353)** moves the selection (a dragged row not selected becomes it): onto a row's middle
  inside it, onto its top or bottom quarter before or after it among its siblings (a service's row only inside). An
  accent outline or line shows where it lands; a drop inside what moves, or onto the root, is refused. One undo step.
- **Selecting in the viewport (#354)**: `viewport::View` is the editor camera over the panel (Forge's `gizmo::View`:
  screen rays, projection, lines cut at the near plane), which picking, outlines and the gizmos share. A left click
  selects the nearest part the pointer's ray meets (each part as its box, every shape for now), or the outermost model
  it is in under Workspace, as Roblox Studio does; Ctrl or Shift adds or removes; a click on nothing clears; Alt is the
  orbit's. The selection is outlined in the accent colour, the part under the pointer faintly. Off while a game runs.
- **Transform tools (#355)**, after Forge's: `viewport::gizmo` is the maths (handles, picking on the screen, rays
  against lines and planes), `viewport::tools` the tools, drags and drawing (an egui overlay on the viewport, always in
  front). Select, Move, Scale and Rotate are toolbar tools (the default Model tab) and keybinds (1..4 since #374), with
  world or local axes (Primary+L). The gizmo sits on the active instance's pivot, or the middle of several. Move: six
  arrows and three plane handles facing the camera; a drag moves every selected instance by its pivot (a model with
  everything in it), measured each frame from where they started, shown with a line from where it started (#376) and
  the distance. The drag writes the data model through the engine's `pivot_to`, so the scene notes the `CFrame`s it
  changed (`Scene::watching`): one undo step per drag.
- **Scale (#356)**: six spheres off the faces of one part's own box, or the world-aligned box round several (shown
  faintly). A drag moves that face, the opposite one staying; Ctrl both about the centre; Shift all three sizes by one
  factor. Every part inside follows: resized along the axes its own line up with, moved with the box (a part turned
  between the box's axes only moves, as a stretch would shear it). Shown with a line and the new size or factor.
  `viewport::tools` keeps a file per gizmo (`moving`, `scale`), sharing the tool, the drag and the measurements.
- **Rotate (#357)**: three rings and a trackball inside them. A ring turns the targets about its axis through the
  gizmo's centre (several swing round their middle), its angle counted through half and whole turns, shown with marks
  round the ring (longest at the quarters, then the eighths), the angle turned filled in, and the angle; the
  trackball turns them as the pointer rolls it, adding up small arcs. Primary+R turns the selection a quarter about
  the up axis, Primary+T tilts it, as one group, one undo step each.
- **Carrying parts (#358)**, as Forge's Select tool: a drag that starts on a part, not on a handle, with any tool,
  picks the selection up (selecting that part first if it was not). Over another part the box round what is carried
  sits on its surface, centred on the pointer (snapped to the move increment along a surface lined up with an axis);
  over nothing it slides on a level plane through where it was grabbed (facing the camera when the view is level).
- **Snapping (#359)**: Settings, Editing has the move increment (1 stud) and the rotate increment (15 degrees), each
  with its switch, both on by default; moves, sizes and carried parts go in move steps, ring turns in rotate steps
  (the ring's marks follow the increment). The toolbar's Snap tool turns both on or off.

### Viewport and Properties (the owner's list of 2026-11-08, #374-#387)

- **Tools at hand (#374)**: the transform tools' keybinds are 1 (Select), 2 (Move), 3 (Scale) and 4 (Rotate), which
  never fire while typing. The Home tab starts with them, at the far left; `toolbar.json` version 2 puts them at the
  far left of a saved toolbar's first tab once, unless that tab already holds one of them.
- **Increments in the toolbar (#375)**: the Snap tool is drawn as the move and rotate increments, one above the
  other (`toolbar::increments`): a check box for each switch, the Move or Rotate icon, the increment as a field to type
  in or drag, and arrows stepping through common values (0.01 to 100 studs, 1 to 180 degrees). It edits Settings,
  Editing in place, and the Home tab has it after the transform tools.
- **Drag trails (#376)**: a Move or Scale drag shows a dot where it started (the pivot, or the face) and a line from
  there to where it is now, with its measurement; the ruler marked every 5 studs is gone.
- **Models turn properly (#377)**: a model without a `PrimaryPart` keeps no rotation, so the engine measures its pivot
  from its parts. A drag placed it each frame from where it started with `PivotTo` while it was still turned from the
  frame before, which turned it again, and a Rotate drag spun it away. Each frame of a drag now first puts back the
  properties it watches as they were when it began.
- **Check boxes alone (#378)**: a boolean is its check box, without "true" or "false" beside it ("Mixed" stays for
  a selection that disagrees), editable or not.
- **No sliders (#379)**: a `Widget::Slider` property gets a number field like any other, still kept in its range.
- **Plain text fields (#380)**: strings, vectors (a `CFrame`'s position and orientation too) and colours are one text
  field each with no box behind it, the row's stripe and lines framing it: a vector its numbers one after another
  (`-20, 40, 33`), a colour its 0-255 channels, a dash for a component the selection disagrees on. A dash or a blank
  typed leaves that component as each instance has it (`Edit::Components`). The colour's picker button is a square
  as tall as the row, flush with its lines. An enum's drop-down loses its box too.
- **Dragging numbers (#381)**: a single number (an `Int` too) is plain text like the others; dragged left it goes
  down and right it goes up, from where the drag began, at the property's speed (the resize pointer over it says
  so), and a click makes it a text field with the value selected. Kept in its range, whole for an `Int`.
- **One number fills all (#382)**: a single number typed into a vector or colour sets every component to it (`255`
  then Enter is `255, 255, 255`).
- **Play clears the Output (#383)**: starting a session empties the Output panel first, so it shows that session's
  lines alone.
- **Shapes stretch (#384)**: the renderer stretches a Ball and a Cylinder to the part's whole `Size` (an ellipsoid; a
  cylinder with elliptic ends) instead of keeping them round at the smallest side, as Block and Wedge already were;
  the shader turns normals by the inverse transpose, so they are lit right. `Part.Shape`'s description says so.
- **Camera controls from Forge (#385)**: a right drag looks; Alt with a right or left drag orbits the point under the
  pointer (the first part its ray meets, else the ground, else a point ahead), which stays put on the screen. During
  either, the shell holds the pointer where the drag began, shown, and the camera turns by the mouse's raw movement
  (`ViewportState::hold`, alongside a running game's `MouseBehavior`). Focus (F) glides to frame the selection, after
  which right drags orbit it and the wheel goes toward it, until the camera is flown or the selection changes. The
  fly keys go at the fly speed (40 studs/s), at the secondary speed (Shift, x3) with Shift held (`Command::with_shift`
  keeps them firing); Alt+wheel and Shift+wheel change the two, kept in the `[viewport]` settings (Settings, Editing)
  and shown for a moment in the viewport's corner. The wheel zooms toward what is ahead, a tenth of the way per notch,
  never through it, eased over a few frames.
- **Axis gizmo (#386)**: the viewport's top right corner shows which way the world's X, Y and Z point from the
  camera (`viewport::axes`), drawn as the Move gizmo's arrows in its colours, with their letters, the farthest first.
- **Panel menus (#387)**: a panel can have a menu (`Panel::has_menu`, `Panel::menu`): three dots at the right of its
  tab bar while its tab is in front open it (egui_tiles' `top_bar_right_ui`). The Viewport's holds its settings, for
  now whether the axis gizmo shows (`[viewport] show_axes`).

### Second viewport and Properties list (the owner's list of 2026-11-08, #402-#418)

- **Icons built in (#402)** and **icons.json applying edits (#403)**: see the icons above.
- **Lights (#404-#407)**: a spot light's cone is measured as one minus the cosine of the angle from its axis, which
  keeps a narrow cone's edge from flickering; `SpotLight.Angle` goes to 180; `Light.Brightness` has no upper limit
  (the API dump writes the open end as `null`); and `DirectionalLight` is gone, renderer variants and all:
  `Lighting`'s sun is the one directional light.
- **Settings (#408, #409)**: Keybinds draws lines between rows only, so a lone command shows no stray line. A window
  of its own opens hidden and shows once its first frame is drawn, and its first, unseen pass runs at the main
  window's scale: at egui's default one it laid text out at another size and rebuilt the font atlas every window
  shares, so Studio flickered and its text garbled as Settings opened.
- **No Editing page (#410)**: Settings loses its Editing page; the increments are edited in the toolbar and the camera
  speeds with the wheel, and both are still saved (`[editing]`, `[viewport]`).
- **Insert Object on a key (#411)**: Shift+Space (`edit.insert`) opens Insert Object for the selected instance, or
  Workspace with nothing selected. Studio draws the window over the editor (`panels::insert::frame`), not the Explorer,
  so it shows whichever panels are open.
- **Inserting where you look (#412)**: something inserted with a place in the world (a part, a light, a camera) goes
  where the camera looks (`viewport::aim`: the surface of the first part the middle of the view meets, a part resting
  on it; else the ground; else a point ahead), not the origin; inserted into something with a pivot, it goes at that
  pivot (a light in a part at its middle).
- **A model keeps its pivot (#413)**: `Model.WorldPivot` as in Roblox, where a model without a `PrimaryPart` pivots.
  Until first set (by `PivotTo`, a tool or a script) it is the centre of the model's parts, unturned, so older scenes
  pivot as before; `PivotTo` keeps where it moved the model to. It is kept in two hidden saved properties
  (`WorldPivotData`, `WorldPivotSet`), which the tools' drags watch for undo. So a turned model keeps its turn, and its
  outline and Scale box are the box round its parts along its pivot's axes, turning with it.

## Not in this slice

The script editor, Project Settings, real sign-in, thumbnails, and single-instance forwarding. (Gizmos, selection in
the viewport, undo and redo, editing properties and Insert Object came with the list of 2026-11-07, above.)

## Consequences

- One device for the engine and the UI: the viewport is a texture, not a copy, and overlays are free.
- Adding a panel, a setting or a command is local: one type and one registry line.
- The keymap is the only path from input to actions, so rebinding covers everything, and testing a shortcut means
  feeding input to the keymap.
- Studio depends on egui's release cadence matching wgpu's. The archive shows it does today.
