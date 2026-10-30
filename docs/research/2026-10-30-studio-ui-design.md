# Metallic Studio UI: research and design

Date: 2026-10-30. Status: proposal for the owner. Docs only; no code changed.

Scope: how Metallic Studio should look and behave so a first-time creator finds it easier than Roblox Studio, and an
expert finds it faster than Unity, Unreal or Godot. It covers 2D, 3D and 2.5D games in any art style, and leaves room
for the modeller, animation, VFX and material editors, the Luau debugger and the profiler.

Mockups: `docs/design/studio-ui/` (five static HTML pages plus a PNG of each, section 6).

How to read the labels:
- **Sourced**: I fetched the page (or a research subagent did and I kept the URL). The URL follows.
- **Judgment**: my reasoning from the sourced facts. Treat it as a proposal.
- **Assistant's call**: a decision I made because the owner was away. Each one is listed again in section 9 so the
  owner can overturn it.

Related work in open pull requests, read for consistency: PR #60 (templates and onboarding) and PR #61 (rendering
styles, the 2D/2.5D/3D camera and layer model, animation and VFX systems). This document covers the editor around
them and does not repeat them.

---

## 1. Decisions in short

1. **One window, one shell, one viewport for 2D, 2.5D and 3D.** 2D is a view preset of the same viewport (orthographic
   camera, 2D gizmo, pixel grid), not a separate editor or project type. The Godot proposal to merge its 2D and 3D
   workspaces and Unity's switchable 2D mode both point here (3.6).
2. **Fixed default layout with docking, not floating-first.** Explorer left, Properties right, document tabs centre,
   Output below. Panels dock with egui_tiles; floating comes later as real OS windows. Figma reverted floating panels
   because they "slowed people down" (3.3).
3. **Ctrl K finds and runs everything**: objects in the level, assets, commands with their shortcuts, guides, and a
   last row that hands the query to Litha. Shift A opens the Insert palette at the cursor (3.3).
4. **Play is one colour.** Green marks every place that is a throwaway copy during Play: a 2 px band on the title bar,
   the viewport outline, the Explorer banner and the status bar. Stop lists what changed during Play so the creator
   can keep it. Unity's tinted play mode and Roblox's lost edits are the two failures this answers (3.1).
5. **Errors are links and questions.** Every error row names the script and line as a link, collapses repeats (×4),
   and has two buttons: Open line and Explain. Explain sends the error to Litha (3.1, 3.4).
6. **Litha lives in a dock tab and in place, never in a pop-up.** It explains first, edits only with a diff the
   creator applies as one undo step, and every edit lands in the saved world, not the play copy (3.5; matches PR #60
   section 4.3).
7. **Future editors are workspaces in the same shell**, built from shared widgets: Timeline, Curve editor, Node
   canvas, Inspector rows, undo and selection. Animate, Model, VFX and Materials appear in the workspace switcher only
   when they ship. Recording keys is an explicit toggle (K), never silent auto-key (3.7).
8. **The theme is graphite with a violet cast, one violet accent and silver icons.** The accent marks selection,
   focus, the primary action and progress only. Checkboxes and toggles use silver. Every colour pair is computed
   against WCAG (section 5.13).
9. **egui 0.37 everywhere, egui_tiles for docking, IBM Plex Sans and Mono for text, Phosphor for icons**, own gizmo
   code. IBM Plex is OFL-1.1, which the licence list does not allow yet: owner decision (section 8).

---

## 2. Method

- Five research subagents fetched sources in parallel: game engines; beginner tools and onboarding; creative tools;
  2D/3D/2.5D and sub-editors; egui. I checked crates.io versions and licences myself through the crates.io API, and
  measured fonts with fontTools on the files in the google/fonts repository.
- Reddit, the Blender manual (HTTP 403), Spline's docs, Construct's site and the Unreal "Play in Editor" page did not
  load. Findings that depend on them are marked or dropped.
- I rendered stand-in 3D scenes with three.js 0.160 (MIT) only to fill the mockup viewports. They stand in for
  `metallic_render` output and are not proposals for the renderer.
- I copied no code or assets from Roblox, Unity, Unreal or Godot. Mockup icons are Phosphor (MIT). Mockup fonts are
  IBM Plex (OFL-1.1), embedded as subsets in the HTML only.

---

## 3. Findings

### 3.1 Game editors: what people love and hate

**Roblox Studio**
- Sourced: the new "flexible" UI became the default in January with no opt-out. Complaints: lost vertical space,
  slow tab switching, blurry icons at high DPI, spaced-out padding, a ribbon that collapses on hover and costs time,
  colours too dark. https://devforum.roblox.com/t/an-update-on-the-new-flexible-studio-ui/4085392 ;
  https://devforum.roblox.com/t/beta-new-studio-ui-updates/3889612
- Sourced: after the redesign users could not find the View tab or Delete, Copy and Paste.
  https://devforum.roblox.com/t/this-new-studio-ui-design-is-a-tragedy-it-might-be-time-to-move-on/4220413
- Sourced: panels dock by dragging the header onto position selectors, pin to an edge or float. Camera: WASD and QE
  fly, F focuses, scroll speed. https://create.roblox.com/docs/studio/ui-overview
- Sourced: selection draws pixel outlines; Alt-click cycles through overlapping parts; Ctrl-click selects the parent.
  Users complain grouped and ungrouped selections look the same. https://devforum.roblox.com/t/selection-improvements/2125001
- Sourced: Test (F5), Test Here and Run (F8); client mode shows a blue border; Server and Clients runs up to 8
  clients; Stop resets the world. https://create.roblox.com/docs/studio/testing-modes
- Sourced: on Stop, Studio offers to keep script edits but drops instance edits made during Play.
  https://devforum.roblox.com/t/how-to-save-changes-when-im-in-client-mode/1775530
- Sourced: Output filters by message type, context and text, with timestamps and source.
  https://create.roblox.com/docs/studio/output

**Unity**
- Sourced: click-to-highlight delay, unreliable picking, slow hierarchy.
  https://discussions.unity.com/t/unity-top-5-annoyances-survey-discussion/768197 ;
  https://discussions.unity.com/t/enough-is-enough/945380 (the Create menu is three levels deep; Preferences and
  Project Settings get confused)
- Sourced: "everything is a cube" generic icons; "the UI is fine, make it faster".
  https://discussions.unity.com/t/unity-is-not-user-friendly-compared-to-godot/1720981
- Sourced: one global Inspector with a tiny lock and no back or forward; F frames too far out; fly speed annoys.
  https://discussions.unity.com/t/my-biggest-pain-points-714332/714332
- Sourced: Play mode tints the editor, and changes made in Play are temporary.
  https://docs.unity.com/en-us/engine/current/manual/unity-editor/editor-windows-views-reference/game-view
- Sourced: Console collapses duplicates, clears on Play, pauses on error, and stack lines are clickable.
  https://docs.unity.com/en-us/engine/current/manual/unity-editor/editor-windows-views-reference/console-window/console
- Sourced: Unity Search also runs menu commands.
  https://docs.unity.com/en-us/engine/current/manual/unity-editor/editor-navigating-managing/editor-searching/search-overview
- Sourced: the 2019.3 flat redesign drew "grey on grey" and "is it even faster? nope" replies, plus complaints about
  low contrast and blurry font hinting.
  https://discussions.unity.com/t/unity-why-have-you-made-your-entire-gui-look-like-garbage-now/750315 ;
  https://discussions.unity.com/t/2019-3-0a7-new-ui-accessibility-issue-font-rendering-colors-are-low-contrast/749152?page=5

**Unreal Engine 5**
- Sourced: the UE5 redesign maximised the viewport, added the Content Drawer (Ctrl+Space) and themes; the Output Log
  has a command prompt; Details panels can be locked. https://www.cgchannel.com/?p=126915 ;
  https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-editor-interface
- Sourced: row action buttons in Details hide the values they belong to.
  https://forums.unrealengine.com/t/ue5-editor-ui-ux-problem/2742994

**Godot**
- Sourced: users praise tooltips on almost everything.
  https://forum.godotengine.org/t/favorite-things-about-godot-coming-from-other-engines/132971
- Sourced: the Inspector has fuzzy property search, history and favourites, and edits sub-resources inline.
  https://docs.godotengine.org/en/latest/tutorials/editor/inspector_dock.html ;
  https://godotengine.org/article/godot-gets-new-inspector
- Sourced: the command palette lists each command's shortcut; editor layouts save and restore.
  https://docs.godotengine.org/en/stable/classes/class_editorcommandpalette.html ;
  https://docs.godotengine.org/en/stable/tutorials/editor/customizing_editor.html
- Sourced: Godot 4.4 embeds the running game in a Game tab while it stays a separate process, with pause, frame step
  and click-to-select at runtime. https://docs.godotengine.org/en/stable/tutorials/editor/game_embedding.html ;
  https://github.com/godotengine/godot-proposals/issues/7213

Judgment, what Metallic takes from this:
- Keep density. The Roblox and Unity redesigns both lost users by adding padding and lowering contrast. Our row
  heights stay at 24 px (tree) and 26 px (properties), and text contrast stays above 7:1 for primary text.
- Never hide a command in a redesign without a search path to it. Ctrl K covers every command, so a moved button is
  still one search away.
- Make Play state impossible to miss and edits during Play recoverable.
- Picking speed and selection feedback matter more than looks. Click-select answers on the same frame.

### 3.2 Beginner tools

- Sourced: Dreams gives the player one point of contact (the imp), separate modes for browsing and making, guided
  tutorials, remixing and community jams. https://www.playstation.com/en-za/editorial/dreams-beginners-guide
- Sourced: reviewers say Dreams' tutorials teach tools, not game design, and creators hit a wall at logic and
  animation. https://www.gameinformer.com/review/dreams/bringing-your-imagination-to-life ;
  https://epiloguegaming.com/the-creative-library-a-critical-review-of-dreams/
- Sourced: UEFN users love devices; they find Verse too complex, and one wrote that it "doesn't provide any feedback
  for what is causing errors". Another: "working undo is gold". https://forums.unrealengine.com/t/thoughts-on-uefn/805694
- Sourced: GDevelop's events show no warnings, and its maintainer calls the debugger and profiler complex.
  https://forum.gdevelop.io/t/i-love-gdevelop-but-this-is-why-i-cannot-bring-myself-to-use-the-engine-consistently-pros-and-cons-compared-to-construct-3/44457
- Sourced: Core described its approach as "opt-in complexity".
  https://80.lv/articles/making-game-develoment-easy-and-accessible-with-core
- Sourced: PlayCanvas shows collaborators' camera frustums and colours their selections; one teammate's syntax error
  could block everyone's launch. https://developer.playcanvas.com/user-manual/editor/realtime-collaboration ;
  https://forum.playcanvas.com/t/post-mortem-apple-watch-project-feedback-and-feature-requests/1062

Judgment: beginners need working undo, errors that say what broke and where, and complexity they opt into. A script
error must never block Play for the whole game: the failing script stops, the rest runs, and Output says why.

### 3.3 Creative tools outside games

- Sourced: Figma UI3 moved the toolbar to the bottom to free the canvas and added Minimize UI (Shift+\\). The team
  wanted to "focus the canvas less on our UI and more on your work". They tried floating panels and hover-only
  panels, then reverted: floating "slowed people down", and minimal unlabelled controls hurt accessibility. They kept
  W and H below X and Y for muscle memory. https://www.figma.com/blog/making-the-move-to-ui3-a-guide-to-figmas-next-chapter/ ;
  https://www.figma.com/blog/behind-our-redesign-ui3/ ; https://www.figma.com/blog/our-approach-to-designing-ui3/
- Sourced: Linear rebuilt its themes in LCH from three variables (base, accent, contrast) instead of 98, added a
  contrast variable for accessibility, and kept brand colour out of the neutrals. Its 2026 refresh made navigation
  recede with fewer dividers: "Structure should be felt not seen". https://linear.app/now/how-we-redesigned-the-linear-ui ;
  https://linear.app/blog/behind-the-latest-design-refresh ; Cmd+K runs any action:
  https://linear.app/enablement/guide/navigating-linear
- Sourced: Blender 2.80 made left-click select the default, added workspaces, gizmos tied to the active tool, and a
  dark theme to "focus on the artwork"; tool settings sit in a top bar; floating panels were rejected because they
  overlap. The first-launch splash offers an industry-compatible keymap. https://www.blender.org/download/releases/2-80/ ;
  https://code.blender.org/2018/04/tools-toolbar-and-tool-widgets/ ; https://www.blender.org/?p=57607
- Sourced: Ableton's Info View explains whatever the mouse is over; two views (Session and Arrangement) show the same
  tracks, switched with Tab. https://www.ableton.com/en/manual/first-steps/ ;
  https://www.ableton.com/en/manual/live-concepts/
- Sourced: DaVinci Resolve splits work into pages switched with one click.
  https://www.blackmagicdesign.com/products/davinciresolve
- Sourced: Houdini's Tab menu lists nodes for the current context and filters as you type.
  https://www.sidefx.com/docs/houdini/basics/tabmenu.html
- Sourced: Framer's right panel changes with the selection; the left holds Pages, Layers and Assets.
  https://www.framer.com/academy/lessons/framer-interface
- Sourced: VS Code's status bar puts workspace-wide items on the left and contextual items on the right, and uses an
  error background only as a last resort. Commands show their shortcuts and use no emoji.
  https://code.visualstudio.com/api/ux-guidelines/status-bar ;
  https://code.visualstudio.com/api/ux-guidelines/command-palette
- Sourced: good dark UIs avoid pure black, raise elevated surfaces by lightening them over 4 to 5 levels, and
  desaturate accents. https://www.toptal.com/designers/ui/dark-ui-design
- Sourced: Cloudscape sets density app-wide (comfortable by default, compact as an option), not per page.
  https://cloudscape.design/foundation/visual-foundation/content-density/

Judgment: workspaces (Blender, Resolve) are the right container for future editors. A keymap choice on first launch
(Blender) lowers the cost of switching from Roblox Studio or Unity. Hover help (Ableton) beats a manual. Elevation by
lightness, not shadows, keeps egui painting cheap.

### 3.4 Onboarding and help

- Sourced: Autodesk's learnability survey groups problems as task flow, awareness, locating, understanding and
  transitioning; help works best organised by goals and shown in context. Clippy failed.
  https://research.autodesk.com/app/uploads/2023/03/a-survey-of-software.pdf_recqWPTEYd6nsP0r8.pdf
- Sourced: ToolClips (short videos in tooltips) helped users complete unfamiliar tasks 7 times more often.
  https://research.autodesk.com/app/uploads/2023/03/toolclips-an-investigation-of.pdf_recbnW20cbxDanglq.pdf
- Sourced: CommunityCommands recommended commands from what similar users use, without interrupting.
  https://research.autodesk.com/app/uploads/2023/03/communitycommands-command-recommendations-for.pdf_recZiyrKCEIuGQLZh.pdf
- Sourced: Carroll's minimalism: give real tasks at once and support error recovery; hint cards halved learning time.
  https://www.instructionaldesign.org/theories/minimalism/
- Sourced: Stencils-based tutorials (which block everything except the next target) made users 26% faster.
  https://www.cs.cmu.edu/~caitlin/research.htm
- Sourced: NN/g found tutorials did not raise success (91% vs 94% without); skip onboarding when possible; limit
  progressive disclosure to two levels; empty states should teach the next action.
  https://www.nngroup.com/articles/mobile-tutorials/ ; https://www.nngroup.com/articles/progressive-disclosure/ ;
  https://www.nngroup.com/articles/empty-state-interface-design/
- Sourced: "low threshold, high ceiling, wide walls" for creative tools.
  https://web.media.mit.edu/~mres/papers/AERA-seymour-final.pdf

Judgment: teach inside the real UI with real tasks. The guide points at real controls and checks the real result, as
PR #60's tutorial runner proposes. No modal tours. Empty panels always say what to do next.

### 3.5 AI helpers in creative tools

- Sourced: Roblox Assistant docks as a panel, reads the selection, and its /plan mode waits for the user to press
  Build. https://create.roblox.com/docs/assistant/guide
- Sourced: GDevelop's AI asks before each change and keeps a restore point. https://wiki.gdevelop.io/gdevelop5/interface/ai/
- Sourced: VS Code inline chat shows a diff with Keep and Undo next to the code, separate from the chat side panel.
  https://code.visualstudio.com/docs/chat/inline-chat
- Sourced: Figma pulled "Make Designs" after it produced near-copies of an Apple app.
  https://techcrunch.com/2024/07/02/figma-disables-its-ai-design-feature-that-appeared-to-be-ripping-off-apples-weather-app
- Sourced: novices using code assistants struggled to understand and fix generated code.
  https://arxiv.org/abs/2304.02491

Judgment: Litha earns trust by being grounded in the creator's own error, selection and script, by showing diffs, and
by applying through the normal undo history.

### 3.6 2D, 3D and 2.5D in one editor

- Sourced: Unity's 2D mode is a project setting that changes defaults (sprite import, orthographic camera, 2D scene
  view, no realtime lighting) and can be switched at any time.
  https://docs.unity.com/en-us/engine/6000.0/manual/get-started/project-configuration/2dand3dmode-settings.md
- Sourced: Unity's scene view in 2D drops orbit and fly, and right-drag pans. Its view gizmo snaps on cone clicks,
  toggles perspective and orthographic from the centre, and hides in 2D.
  https://docs.unity.com/en-us/engine/current/manual/unity-editor/editor-windows-views-reference/using-the-scene-view/scene-view-navigation ;
  https://docs.unity.cn/Documentation/Manual/SceneViewNavigation.html
- Sourced: a Godot proposal asks to make 2D/3D a viewport setting and merge both into one Scene workspace.
  https://github.com/godotengine/godot-proposals/issues/15134 ; 2.5D gaps (separate physics, no tilemap in 3D, Sprite3D
  sorting): https://github.com/godotengine/godot-proposals/issues/14384 ;
  https://github.com/godotengine/godot-proposals/issues/3986
- Sourced: Unity's sorting settings hide in places users miss.
  https://discussions.unity.com/t/2d-sprite-renderer-isnt-sorting-sprites-from-the-same-layer-order-based-upon-transform-position/863728
- Sourced: pixel-perfect cameras use pixels per unit and a reference resolution.
  https://docs.unity.cn/Packages/com.unity.render-pipelines.universal@13.1/manual/2d-pixelperfect.html
- Sourced: GameMaker lists layers in draw order and reorders them by drag.
  https://manual.gamemaker.io/lts/en/The_Asset_Editors/Room_Properties/Layer_Properties.htm

Judgment: show sorting where people look. The Layers list shows each layer's sort rule ("sort by Y", "parallax 0.5")
in the row itself. PR #61 section 6 defines the camera and RenderLayer model this UI edits.

### 3.7 Animation, node graphs, curves and modelling

- Sourced: Rive separates Design and Animate modes; users fall into auto-keying traps.
  https://rive.app/docs/editor/fundamentals/design-vs-animate-mode
- Sourced: Spline's timeline has an Animate button, auto record and a graph editor.
  https://docs.spline.design/designing-in-3-d/timeline-animation (listed by the subagent; the page blocked my own fetch)
- Sourced: Roblox's Animation Editor uses media controls, one track per part, 30 fps and right-click easing.
  https://create.roblox.com/docs/animation/editor
- Sourced: Unreal's material graph tools for tidiness: comments (C), reroutes, named reroutes, collapse.
  https://dev.epicgames.com/documentation/en-us/unreal-engine/organizing-a-material-graph-in-unreal-engine
- Sourced: Shader Graph users call large graphs "ugly, curved spaghetti".
  https://discussions.unity.com/t/shadergraph-is-ugly-and-slow/789331
- Sourced: Niagara builds effects as a stack of modules from templates; VFX Graph runs contexts top to bottom.
  https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-niagara-effects-for-unreal-engine ;
  https://docs.unity3d.com/Packages/com.unity.visualeffectgraph%408.0/manual/GraphLogicAndPhilosophy.html
- Sourced: Plasticity runs one tool at a time; its extrude is hard to discover. https://blog.zsa.io/zsa-loves-plasticity/

Judgment: start each editor at its simplest honest form (properties, then a stack, then a graph), show the record
state loudly, and draw graph wires straight or with short corners, not long curves.

### 3.8 egui

- Sourced: egui 0.37.0 shipped 2026-10-15 under MIT OR Apache-2.0. It adds an experimental StyleProvider, roomier
  buttons, and drops `default_fonts` from default features. https://github.com/emilk/egui/releases/tag/0.37.0
- Sourced: 0.34 moved text to Skrifa and vello_cpu with hinting and made AccessKit always on (Windows and macOS);
  0.35 added harfrust shaping and panel slide animations, with a default animation time of 0.2 s.
  https://raw.githubusercontent.com/emilk/egui/refs/heads/main/CHANGELOG.md
- Sourced: the theme lives in `Visuals`, `Widgets` (five states: noninteractive, inactive, hovered, active, open) and
  `Spacing`. https://docs.rs/egui/latest/egui/style/struct.Visuals.html ;
  https://docs.rs/egui/latest/egui/style/struct.Widgets.html ;
  https://docs.rs/egui/latest/egui/style/struct.Spacing.html
- Sourced: `CornerRadius` uses u8 per corner; `Shadow` uses i8 offsets and u8 blur and spread.
  https://docs.rs/epaint/latest/epaint/struct.CornerRadius.html ; https://docs.rs/epaint/latest/epaint/struct.Shadow.html
- Sourced: egui draws text with grayscale anti-aliasing and subpixel positioning, not LCD subpixel rendering.
  https://docs.rs/epaint/latest/epaint/text/struct.TextOptions.html
- Sourced: egui has no backdrop blur (issue #1758); a frosted panel would need a post pass. (subagent; not re-fetched)
- Sourced: `egui_wgpu::Callback` paints with wgpu inside an egui rect, and `Renderer::register_native_texture` shows a
  wgpu texture as an image. https://docs.rs/egui-wgpu/latest/egui_wgpu/trait.CallbackTrait.html
- Sourced (crates.io API, checked by me): egui-wgpu 0.36.0 and 0.37.0 both require wgpu ^30.0, matching
  `metallic_render`'s wgpu 30.0.1. egui_tiles 0.18.0 (MIT OR Apache-2.0, Rerun) requires egui ^0.37. egui_dock
  0.21.1 (MIT), egui-snarl 0.12.0, transform-gizmo-egui 0.11.0 and egui-phosphor 0.14.0 still require egui ^0.36.
- Sourced (crates.io API, checked by me): `epaint_default_fonts` 0.37.0 declares
  "(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0". The current `deny.toml` would reject it.
- Sourced: Rerun's `re_ui` builds a design-token layer over egui and its licence includes OFL-1.1 for fonts.
  https://docs.rs/re_ui
- Sourced: Graphite uses web tech, and Bevy's editor uses bevy_ui, so neither is an egui reference.
  https://graphite.art/volunteer/guide/codebase-overview/ ; https://hackmd.io/@bevy/HkjcMkJFC

Judgment on what egui can and cannot do for a premium look:
- Can: flat fills, 1 px strokes, integer radii, one soft shadow on floating surfaces, gradients via meshes, hinted
  text, custom painters for gizmos, timelines and graphs, 60 fps animation of any value, per-frame repaint control.
- Cannot without extra work: backdrop blur, LCD text, CSS-like layout. The design uses none of them.
- Watch: large scroll areas are slow unless virtualised (issue #124 asks for a virtualised Explorer at 50k
  Instances); the first frame can jitter while sizes settle.

### 3.9 Fonts and icons (my measurements)

- IBM Plex Sans has tabular digits by default, so numbers in Properties and Output line up without OpenType feature
  selection, which egui does not expose per text run. x-height 0.516, cap height 0.698.
- Inter, Geist, Public Sans, Mona Sans, Instrument Sans, Red Hat Text and Atkinson Hyperlegible use proportional
  digits by default (tabular only through the `tnum` feature).
- Source Sans 3 has tabular digits but a small x-height (0.478), which reads small at 13 px.
- Every good UI face I found in google/fonts is OFL-1.1; none in the Apache folder fits a dense editor.
- Icons: Phosphor (MIT), Lucide (ISC), Tabler (MIT) and Material Symbols (Apache-2.0) are permissive. Remix Icon
  changed to a custom licence in January 2026; avoid it.

---

## 4. Principles

1. **The work is the brightest thing on screen.** Chrome stays dark and quiet; the viewport and the selection carry
   the colour.
2. **Density with air.** Rows stay compact; spacing between groups does the separating, not lines.
3. **One meaning per colour.** Violet means selected, focused or primary. Green means Play. Red means error. Axis
   colours always come with their letters.
4. **Everything is findable by typing.** If a control moves, Ctrl K still finds it.
5. **Undo covers everything,** including Litha's edits and edits kept from Play.
6. **No dead ends.** Empty panels say what to do next. Disabled controls say why in their tooltip.

---

## 5. Studio UI design

### 5.1 Window and layout

One OS window, drawn by egui with OS decorations off (custom title bar, Windows snap layouts kept through the
standard hit-test regions). Default layout at 1600×1000 (mockup `editor-3d.png`):

| Area | Size | Contents |
|---|---|---|
| Title bar | 40 px | Crow emblem; menus File, Edit, View, Insert, Test, Help; project name with unsaved dot; workspace switcher; Play cluster (centred); command field "Search or run a command  Ctrl K"; Litha button; window controls |
| Left dock | 280 px | Explorer and Assets as tabs. In a 2D level a Layers list splits in below |
| Centre | rest | Document tabs: scenes and scripts. A scene tab has a 36 px viewport header holding the tool settings |
| Right dock | 304 px | Properties and Litha as tabs |
| Bottom dock | 236 px | Output, Problems, then Profiler, Debugger and Timeline as they ship. In a 2D level, Tiles |
| Status bar | 24 px | Left: save state, problem count, Play state. Right: selection, cursor position, grid, workspace |

- Gutters between panels are 3 px of `bg.base`. No borders between panels; the gutter is the border. (Linear: "felt
  not seen".)
- Panels have square corners when docked. Only floating surfaces (menus, popovers, palette, tooltips) get radius and
  the one shadow.
- Workspaces: Build and Script ship in M1. Animate, Model, VFX, Materials and Debug appear only when they ship. A
  workspace stores its own dock layout; switching keeps the selection and the open scene.
- Minimise UI (Ctrl+Shift+\\): hides docks and leaves title bar, viewport header and status bar. Assistant's call,
  after Figma's Shift+\\.

### 5.2 Docking (assistant's call)

- **egui_tiles 0.18** with our own tab painting through its `Behavior` trait. It tracks egui 0.37 and supports tabs,
  rows, columns and grids nested freely.
- Drop targets: dragging a tab shows five zones on the panel under the cursor (centre = add as tab; edges = split),
  painted as `select-weak` rectangles with a 1 px accent edge. No floating panes in M1.
- Floating windows come later as egui viewports (real OS windows), so a second monitor works. Not in M1.
- Layout persists per workspace in the user's config folder, not in the project. View > Reset layout restores the
  default. Panel visibility toggles: Ctrl+B left dock, Ctrl+Alt+B right dock, Ctrl+J bottom dock (VS Code habits).
- Ctrl+Space maximises the panel under the cursor (Blender habit). Ctrl+Space again restores.

### 5.3 Panels

**Explorer**
- Filter field at the top ("Filter by name or class"); typing filters the tree and keeps ancestors visible.
- Rows 24 px; indent 18 px; class icon 16 px in silver; name in primary text; a right-aligned meta column in muted
  text (class for non-obvious types, child count for folders, "Character", "Script").
- Selected row: `select` fill. Ancestors of the selection get no fill; their caret opens.
- A script with an error shows a 6 px red dot at the row's right edge.
- Drag to reparent with a 2 px accent insert line; hold Alt to drop as sibling.
- Right-click: Insert, Cut, Copy, Paste, Paste into, Duplicate, Group, Rename, Delete, Select children, Focus in
  viewport. Every item shows its shortcut.
- Virtualised list so 50k Instances scroll at 60 fps (issue #124).

**Properties**
- Header: class icon, name (14 px semibold), class in secondary text, a help button that opens the class docs.
- Filter field ("Filter properties"), then collapsible sections in a fixed order: the class's own section first
  (Sprite, Part, Script), then Appearance, Transform, Physics, Behaviour, Attributes and tags. Rarely used properties
  collapse into "More (12)" at the end of each section. Progressive disclosure stays at two levels (NN/g).
- Row: 104 px label column, value column. Number fields scrub on horizontal drag with the label as handle (Blender
  and Figma habit) and accept expressions (`3*4`, `+2`).
- Vectors show X, Y, Z fields with their axis letters in axis colours; letters, not only colour.
- Mixed values across a multi-selection show "Mixed" in muted text. The plan shows a dash; a screen reader reads
  "Mixed" and says nothing useful for a dash.
- Changing a value previews live; dragging produces one undo step on release.
- Hover a label for one second: tooltip with one sentence, the API name (`Part.Anchored`) and a "Learn more" link.

**Assets**: grid or list of files in the project's asset folders with thumbnails; drag into the viewport or Explorer.
Full browser is M4 (#17).

**Output and Problems**: section 5.9.

**Script editor**: a document tab. Monospace 12 px Plex Mono, line height 20 px, line numbers in muted text,
breakpoint gutter (F9), error underline in `error` with the message on hover and in Problems.

### 5.4 Command palette (Ctrl K) and Insert palette (Shift A)

Ctrl K opens a 640 px palette under the title bar (mockup `editor-2d.png`). Results come in groups:

1. **In this level**: Instances whose name or class matches, with their path and a count when several match ("Coin ·
   Workspace.Pickups · 7 objects", Enter selects all seven).
2. **Assets**: files with type and size.
3. **Commands**: every menu command and setting, with its shortcut on the right. Names follow "Verb object"
   ("Insert coin pickup", "Reset layout").
4. **Learn**: guides and docs pages that match, with length ("Guide · 4 min, opens next to your level").
5. **Ask Litha about "…"** (Ctrl+Enter) as the last row, always present.

Prefixes narrow the search: `>` commands only, `@` objects, `#` assets. Tab jumps to the next group. Recently used
commands rank first for an empty query. Ctrl+Shift+P opens the palette with `>` typed (VS Code habit).

Shift A over the viewport opens Insert at the cursor: a short searchable list of classes and template parts, the
inserted item lands under the cursor on the surface below it. Over a node canvas the same key lists nodes for that
graph type (Houdini's Tab menu). One key, one meaning: "add something here".

### 5.5 Viewport

**Header (36 px, the tool settings bar):** Select (Q), Move (W), Rotate (E), Scale (R) as an icon segment, the
active tool's name, then that tool's settings: snap (1 stud, 15°), Space (World or Local), Pivot (Centre or
Origin). Right side: view (2D, Side, 3D), projection, shading (Lit, Unlit, Wireframe, Debug views), grid toggle. The
tool settings live in the header because Blender found floating tool panels overlap the work.

**Navigation (Metallic keymap):**
- Hold right mouse: look; while held, WASD moves, Q and E go down and up, Shift is faster, the wheel changes speed.
  The speed shows as a transient "Camera speed 1.0×" note. Fly keys work only while the right button is held, so
  Q, W, E and R stay free for tools.
- Alt+left drag orbits around the point under the cursor; middle drag pans; wheel dollies toward the cursor.
- F frames the selection with a margin that keeps the whole selection at about 60% of the view (fixes Unity's
  "frames too far out"). Home frames everything.
- View navigator (top right, 84 px): click an axis to look along it; click the centre to toggle orthographic. Numpad
  1, 3 and 7 for front, right and top; numpad 5 toggles orthographic.

**Selection:**
- Click selects the topmost Instance under the cursor; Ctrl-click adds or removes; Alt-click cycles through
  overlapping Instances at that point (Roblox habit); drag on empty space draws a box.
- Clicking a part inside a Model selects the Model first; a second click on the same part selects the part. The
  outline style tells them apart: a Model draws its bounding box corners, a part draws its silhouette (answers the
  Roblox complaint that grouped and ungrouped look the same).
- Selection outline: 2 px `accent` silhouette, drawn by the renderer as a post outline (PR #61 has the outline pass).
  Hover: 1 px `accent-hover` at 60% alpha.

**Gizmos (own code, painted with egui shapes over the wgpu image):**
- Move: three arrows plus three plane squares; the plane square takes the colour of the axis it is normal to (XZ
  plane = green Y), with letters at the arrow tips. Rotate: three rings plus a screen-space ring; the dragged ring
  thickens and fills its swept angle at 22% alpha, and a label shows the angle ("X −35°"). Scale: cubes on the axes
  and a centre handle for uniform scale.
- Hover thickens a handle from 2.5 px to 3.5 px; drag shows the value in a small label next to the cursor.
- Hold Ctrl to invert snapping while dragging; hold Shift for precision (one tenth speed).
- Gizmo size stays constant in screen space (about 90 px axis length at 100% scale).

**Overlays:** the Litha guide strip (bottom left), transient notes (bottom left, above the guide), selection info on
hover. Nothing in the top-left quarter, where the game's own HUD will sit during Play.

### 5.6 2D, 2.5D and 3D (one viewport)

- A scene has one world with 3D transforms (PR #61 section 6). The view switch in the header picks a preset:
  - **3D**: perspective fly camera, 3D gizmos, ground grid.
  - **Side**: orthographic camera looking down −Z, orbit off, right-drag pans, 2D gizmos on X and Y, depth kept in
    Properties (for 2.5D side-scrollers built from 3D parts).
  - **2D**: like Side, plus pixel grid ("16 px = 1 stud"), pixel snapping, the Layers list and the Tiles panel.
- New projects pick a default from the template (`metallic.toml` `view = "2d" | "3d"`); the user can switch at any
  time (Unity's switchable 2D mode, Godot proposal 15134). Assistant's call.
- **Layers list** (2D and 2.5D): one row per RenderLayer in draw order, top draws last. Each row shows a visibility
  eye, the name and the sort rule ("sort by Y", "order", "parallax 0.5", "on top"); drag to reorder; selecting a
  layer makes it the paint target. Sorting rules stay visible in the row (fixes Unity's hidden sort settings).
- **Sprites**: Properties shows Image, Pixels per stud, Filter (Pixel or Smooth), Flip, Tint, Layer, Order in layer.
- **Tiles panel** (bottom dock in 2D): tileset picker, Paint, Fill, Erase, Pick, auto-tile edges toggle, swatches.
  "Paints on layer Tiles" names the target so a stroke never lands on the wrong layer.
- **Camera bounds** show as a dashed rectangle in 2D (the game camera's view at the reference resolution).
- **Isometric**: the 2D preset with an Iso grid (2:1) and `Iso` sort (PR #61). Same tools, different grid.

### 5.7 Play loop

- **F5 Play** runs server and client in Studio, in the same window. The viewport becomes the player's view; the docks
  stay usable. Alt+F5 Play here (spawn at the camera). The caret next to Play lists Run (server only, no character)
  and, from M2, Players: 2 to 8.
- **During Play** (mockup `play.png`):
  - a 2 px `success` band across the top of the window, and the Play cluster turns into "● Playing 0:42", Stop
    (Shift+F5), Pause (F8), Step frame (Ctrl+F8);
  - a 2 px `success` outline around the viewport;
  - an Explorer banner on `success-bg`: "Play copy. Edits here reset when you stop. Stop lists them so you can keep
    the ones you want.";
  - the status bar shows "Playing · server and client in Studio" in green, the error count, players and fps;
  - the viewport header switches to Play controls: Player view or Free camera (F8 and detach), which side to show
    (Client or Server), shading, debug views.
- The band and outline fade in over 260 ms with ease-out. The editor does not tint the rest of the UI (Unity's tint
  makes text harder to read and still gets missed).
- **Stop** returns to the edit world. If anything changed in the play copy, a non-blocking strip above the Output
  lists the changes by Instance ("MovingPlatform: Position, Speed") with Keep all, Choose and Discard. Keep applies
  them to the edit world as one undo step. Script edits made during Play go to the saved file at once (they were
  never in the play copy). This fixes Roblox's lost instance edits. Assistant's call.
- **Hot reload**: saving a script during Play restarts that script only, keeping the world state (when the script
  has no state the restart can corrupt, the creator presses Ctrl+Shift+F5 to restart Play).
- A runtime error never stops Play. The failing script stops, the error goes to Output, and a red count appears on
  the Output tab and the status bar.

### 5.8 Start screen and first run

Mockup `start.png`. It is the "New" view of the Task 9 start screen and follows PR #60's flow.

- Title bar: crow emblem, "Metallic Studio" in silver semibold, Settings, Help, window controls.
- Left column (360 px): **Recent games**. First run shows the empty state "No games yet. Games you create or open show
  up here, newest first." with "Open a game folder (Ctrl+O)". Below it, **Before you start**: Shortcuts (Metallic,
  Roblox Studio, Unity-style), Interface size (90%, 100%, 125%, 150%), Contrast (Standard, High). These are the only
  three settings a new user needs, and all three stay in Settings later. Footer: version and channel, Third-party
  licences, Check for updates.
- Main: "What do you want to make?" and one sentence: "Every template opens ready to play. Press F5 to try it, then
  change anything you like." The selected template fills a large card: preview, name, kind ("3D · third-person"),
  the template's own description from `template.toml`, what's inside, Game name, Folder (pre-filled, Browse), the note
  "Studio creates this folder. It must be new or empty.", and **Create game (Enter)**.
- Below: all starting points as equal tiles with the selected one outlined in accent. M1 ships Parkour plus the empty
  starters; PR #60's catalog fills the row later. The mockup shows the M3 set (Platformer 2D, Side runner 2.5D) to
  prove the layout.
- Errors in place: a non-empty folder shows the plan's copy under the Folder field in `error` text: "This folder
  isn't empty. Choose an empty folder or a new one." Create stays enabled and re-checks.

**First run inside the editor** (after Create game):
- The level opens with the Play button highlighted (a 2 px accent ring that stops after the first Play).
- The guide strip sits at the bottom left of the viewport: "Your first level · step 2 of 5", one sentence, the next
  action as a button. It is the collapsed form of PR #60's tutorial checklist; clicking the title expands the
  checklist into a dock tab. Steps check the real result (a property changed, Play pressed) and point at the real
  control with a 2 px accent coach ring. The strip never blocks input (no stencil overlay). Assistant's call: Stencils
  measured faster learning, but blocking input fights the "try anything" goal of templates.
- Closing the strip asks nothing; Help > Guides reopens it.

### 5.9 Output, Problems and errors

Mockup `play.png`.

- **Output** rows: time (muted, mono), source (Server, Client, Studio), message (mono 12 px). Filter chips: All,
  Errors (n), Warnings (n), Messages (n) | Server, Client, Studio; a text filter on the right. Clear (Ctrl+L when
  Output has focus). Option: Clear on Play (on by default).
- **Error rows** sit on `error-bg`. The script path and line are a link (`Workspace.Course.LavaRun.Lava.Script:3`).
  Repeats of the same error and line collapse into one row with a ×4 badge and the latest time. Buttons on the row:
  **Open line 3** and **Explain**. A second line shows the stack trace in secondary text ("in function onTouched ·
  Lava.Script line 3 · touched by Workspace.Course.SpinningBar.Bar").
- **Explain** opens the Litha tab with the error attached as context. Without Litha (before M7, or switched off),
  Explain opens the docs page for that error class.
- **Problems** lists static issues that exist without running: Luau type errors, missing assets, broken references,
  template checks. Each row links to its source. The tab shows a count; zero shows "No problems" in the status bar.
- The status bar error count is the only red text outside Output. No red backgrounds in the status bar (VS Code's
  rule).
- No error pop-ups. A crash of the play session (not a script error) shows a strip at the top of the viewport: "The
  game stopped: <reason>. Your edits are safe." with Restart.

### 5.10 Litha

Litha is milestone 7 (#20). The shell reserves its places now so nothing moves later.

- **Places:** a dock tab next to Properties (Ctrl+Shift+L); the Litha button in the title bar opens it; the last row of
  Ctrl K; Explain on error rows; a "?" on a Properties row ("What does this do?"); Ctrl+I in the script editor for
  inline help on the current line.
- **Context chips** at the top of the tab show what Litha can see ("Error at 10:44:31", "Lava.Script", the selection).
  The creator can remove a chip.
- **Answers** are short, name the cause in the creator's terms ("The spinning bar sweeps over it"), and show the fix
  as a diff with `−` and `+` line markers and line numbers.
- **Apply to script** changes the saved script, not the play copy, as one undo step, and writes "Litha changed
  Lava.Script lines 2 to 5" to Output (PR #60 section 4.3). The button uses neutral `bg-active`, not the accent,
  because applying code is a decision, not the default path.
- Litha never interrupts: no pop-ups, no unprompted suggestions in M7. Ambient hints (CommunityCommands style) are a
  later, opt-in experiment.
- Litha has no face or avatar in the UI; its name and the chat icon are enough. The crow emblem belongs to Metallic.

### 5.11 Future editors in the shell

Each editor is a workspace with its own default layout. They share these widgets (build them once, in
`metallic_studio::widgets`, with the theme tokens):

| Widget | Used by | Notes |
|---|---|---|
| Timeline | Animate, VFX (spawn over time), Profiler (frames), cutscenes | Ruler with frame numbers every 5, playhead in accent with a frame label, track list on the left, keys as diamonds (silver, selected accent), events as flags |
| Curve editor | Animate, VFX curves, easing in Properties | Bezier handles in accent, keys in primary text, value grid lines in `line` |
| Node canvas | Materials, VFX graph, later visual scripting | Straight wires with short rounded corners, comment boxes, reroutes, Shift A node search, errors on the node that caused them |
| Inspector rows | Every editor | The same Properties rows and sections |
| Viewport | Build, Animate, Model, VFX preview | The same navigation and gizmos |
| History | All | One undo stack per document, shared commands |

**Animate** (mockup `animate.png`, for #40 and epic #19): Rig tree and Clips on the left; viewport with onion skin
(2 before, 2 after) and the rotate gizmo on the selected part; Keys and Curves tabs below; Properties shows the
selected keys: value, easing style and direction with a live curve preview, "Edit in Curves", Copy pose and Paste
mirrored. Transport: first frame, play (Space), last frame, loop, frame counter "12 / 30", fps. **Record keys (K)** is
off by default; while on, the viewport outline turns `error` red and the button fills. "Key selected (I)" adds keys
by hand. Assistant's call: explicit record only, after Rive's auto-key traps.

**VFX** (for #41): start as a **stack** of modules per emitter (Spawn, Init, Update, Render), each a collapsible
section of Inspector rows, with a live preview viewport and a Timeline for bursts. "Open as graph" switches the same
effect to the Node canvas for experts (one way, as PR #61 section 8 proposes).

**Materials** (M6): Properties sliders first (PR #61 section 5.3), node graph second, generated WGSL in a side tab,
naga errors on the node.

**Model** (M6): tool settings in the viewport header, one tool at a time with its name shown, edit modes as a segment
(Object, Vertex, Edge, Face), Shift A for primitives. The header names the active tool so Plasticity's "hard to
discover" problem does not repeat.

**Debug** (Task 15) and **Profiler** (Task 16): Debug adds Call stack, Variables and Breakpoints panels to the right
and bottom docks during a paused session; the script tab shows the current line with an accent bar in the gutter.
The Profiler is a bottom-dock tab with a frame Timeline (bars per frame, the 16.6 ms line in `warning`) and a
sortable scope table.

### 5.12 Accessibility

- AccessKit is always on in egui 0.37 on Windows and macOS. Every custom widget (tree rows, gizmo handles, timeline
  keys, palette rows) sets a role and a label. The viewport exposes the selection as a list, not the 3D scene.
- Contrast: primary text 13.5:1 on panels; secondary 7.5:1; muted 5.1:1. Muted text appears only on `bg.deep`,
  `bg.base`, `bg.panel` and `bg.raised` (it drops to 4.19:1 on hover fills). The `theme_text_contrast` test should
  check every text token against every surface it can appear on.
- High contrast (Settings and the start screen): borders on inputs and panels at `line.hc` #6b6776 (3.11:1 on
  panels), focus ring 3 px, muted text promoted to secondary.
- Keyboard: every control reachable by Tab; F6 and Shift+F6 cycle focus between panels; arrows move in trees and
  lists; Enter and Space activate; Esc closes the top popover or cancels a drag. A visible 2 px accent focus ring,
  offset 1 px, on every focused control.
- Interface size 90% to 150% (egui `pixels_per_point` multiplier) on the start screen and in Settings.
- Reduced motion: follows the OS setting and a Studio toggle; all durations become 0 except progress.
- Colour is never the only signal: axes carry letters, errors carry an icon and a count, Play carries text.
- Tooltips appear after 500 ms, stay while hovered, and every icon-only button has one with its shortcut.

### 5.13 Theme tokens

Values for `crates/metallic_studio/src/theme.rs`. Contrast ratios computed with the WCAG 2 formula (my script);
"on panel" means on `bg.panel`.

**Surfaces**

| Token | Hex | egui field | Use |
|---|---|---|---|
| `bg.deep` | #111015 | `extreme_bg_color` | Title bar, status bar, text inputs, segments, code blocks |
| `bg.base` | #16151b | `CentralPanel` fill | Window background, gutters between panels |
| `bg.panel` | #1c1b22 | `panel_fill`, `widgets.noninteractive.bg_fill` | Docked panels, tab bars |
| `bg.raised` | #24222b | `window_fill`, `faint_bg_color`, `widgets.inactive.weak_bg_fill` and `bg_fill` | Buttons, popovers, palette, menus, chips |
| `bg.hover` | #2c2a34 | `widgets.hovered.weak_bg_fill` and `bg_fill` | Hover fill |
| `bg.active` | #35323e | `widgets.active.*`, `widgets.open.*` fills | Pressed, open menus, the selected segment |
| `select` | #3a3354 | `selection.bg_fill` | Selected rows and text selection |
| `select.weak` | #2a2638 | (custom) | Rows related to the selection (sub-tracks), drop zones |
| `line` | #2b2933 | `widgets.noninteractive.bg_stroke` | Separators inside panels, grid lines (1.19:1, decoration only) |
| `line.strong` | #3b3845 | `window_stroke` | Popover borders, title bar separators |
| `line.hc` | #6b6776 | (high contrast only) | Input and panel borders in High contrast (3.11:1 on panel) |

**Text and icons** (contrast on deep / panel / raised / hover)

| Token | Hex | egui field | Ratios | Use |
|---|---|---|---|---|
| `text` | #e6e4ea | `widgets.*.fg_stroke`, `override_text_color` none | 15.0 / 13.5 / 12.4 / 11.2 | Primary text |
| `text.2` | #aeaab6 | (custom) | 8.3 / 7.5 / 6.9 / 6.2 | Labels, menus, inactive tabs |
| `text.3` | #8e8a97 | `weak_text_color` | 5.6 / 5.1 / 4.7 / 4.2 | Meta, hints, timestamps. Not on hover, active or select-weak |
| `icon` | #bdbac4 | (custom) | 9.9 / 8.9 / 8.2 / 7.4 | Icons (silver), checked checkboxes |

**Accent and states**

| Token | Hex | Ratios | Use |
|---|---|---|---|
| `accent` | #9d8be0 | 6.5 / 5.9 / 5.4 | Primary button fill, focus ring, selection outline, playhead, progress |
| `accent.hover` | #ab9be6 | | Primary button hover |
| `accent.text` | #b4a6ec | 8.7 / 7.8 / 7.2 | Links (`hyperlink_color`) |
| `on.accent` | #111015 | 6.50 on accent | Text on accent fills. White on accent is 2.92:1 and fails |
| `error` / `error.bg` | #ec7a72 / #3a2226 | 6.2 on panel; 5.29 on error.bg | `error_fg_color`; error rows |
| `warning` / `warning.bg` | #e0ad55 / #352c1d | 8.4 on panel; 6.72 on warning.bg | `warn_fg_color`; warning rows |
| `success` / `success.bg` | #62bf91 / #1d3329 | 7.6 on panel; 6.02 on success.bg | Play state only |
| `info` | #7fb0e6 | 7.5 on panel | Info messages, Studio source tag |
| `axis.x` / `axis.y` / `axis.z` | #e5675e / #79c46a / #5d95ea | 5.2 / 8.1 / 5.7 on panel | Gizmos and vector field letters, always with the letter |

Rules: the accent appears on selection, focus, the one primary button per view, the playhead and progress. Checkboxes
and toggles use `icon` silver with `on.accent` ticks. Hues of all neutrals sit at a slight violet cast (blue channel
highest, red above green) so the accent belongs to them without tinting them.

**Type** (IBM Plex Sans 400, 500 and 600 as three font families, since egui picks weight by family; Plex Mono 400 and
500)

| Token | Size / line | Weight | egui | Use |
|---|---|---|---|---|
| `caption` | 11 / 16 | 400 or 600 | `TextStyle::Small` | Badges, shortcut keys, axis letters (600) |
| `small` | 12 / 17 | 400 | `Name("Small2")` | Meta, status bar, hints, Output source |
| `body` | 13 / 18 | 400, 500 for emphasis | `Body`, `Button` | Default |
| `mono` | 12 / 20 (code), 22 (Output) | 400 | `Monospace` | Code, Output messages, numbers in code |
| `panel.title` | 14 / 20 | 600 | `Name("PanelTitle")` | Properties header name |
| `heading` | 15 / 20 | 600 | `Heading` | Section headings on the start screen and in dialogs |
| `title` | 20 / 26 | 600 | `Name("Title")` | Template name on the start screen |
| `display` | 26 / 32 | 600 | `Name("Display")` | The start screen question |

No letter-spacing changes; no uppercase labels. Numbers use Plex's default tabular digits.

**Spacing and size**

| Token | Value | egui |
|---|---|---|
| Space scale | 2, 4, 6, 8, 12, 16, 24, 32, 48 | constants |
| `item_spacing` | 8 × 4 | `Spacing::item_spacing` |
| `button_padding` | 10 × 4 | `Spacing::button_padding` (28 px buttons with 13/18 text, 26 px in rows) |
| `interact_size` | 40 × 26 | `Spacing::interact_size` |
| `indent` | 18 | `Spacing::indent` |
| Tree row | 24 | custom list item |
| Property row | 26, label column 104 | custom |
| Tab bar | 32 | custom tabs |
| Viewport header | 36 | custom |
| Title bar | 40 | custom |
| Status bar | 24 | custom |
| Icon | 16 (14 in tabs and buttons, 18 in headers) | Phosphor glyph size |
| Scroll bar | 8 wide, floating, appears on hover | `Spacing::scroll` floating |
| Menu margin | 6 | `menu_margin` |
| Window margin | 12 | `window_margin` |

**Radii and elevation**

| Token | Value | egui |
|---|---|---|
| `r.sm` | 3 | `widgets.*.corner_radius` for rows, fields, chips, keys |
| `r.md` | 5 | buttons, segments, `menu_corner_radius` |
| `r.lg` | 8 | `window_corner_radius`, palette, popovers, start-screen cards |
| Docked panels | 0 | `Frame` with no radius |
| `shadow.pop` | offset (0, 8), blur 24, spread 0, black at 45% (alpha 115) | `popup_shadow`, `window_shadow` |

Elevation comes from lightness (`bg.panel` to `bg.raised`), not shadows. Only floating surfaces cast `shadow.pop`.
No glow, no gradients on chrome, no blur.

**Motion**

| Token | Duration | Easing (`emath::easing`) | Use |
|---|---|---|---|
| `fast` | 90 ms | `cubic_out` | Hover fills, button press |
| `base` | 140 ms | `cubic_out` | Tab switch underline, collapse and expand, palette open |
| `slow` | 200 ms | `cubic_in_out` | Dock panel slide, maximise and restore (egui's default 0.2 s) |
| `play` | 260 ms | `cubic_out` | Play band and outline fade in and out |
| `progress` | linear | `linear` | Progress bars, the playhead during playback |

Set `Style::animation_time` to 0.14 and use `ctx.animate_bool_with_time_and_easing` for the rest. Reduced motion sets
every duration to 0 except progress. No looping animation on idle UI (it would also keep egui repainting; issue
#124 wants idle CPU near zero).

### 5.14 Fonts, icons and the emblem

- **Fonts (owner decision, section 8):** IBM Plex Sans (400, 500, 600 static instances) and IBM Plex Mono (400, 500),
  OFL-1.1, bundled in `crates/metallic_studio/assets/fonts/` with their licence file. Reason: tabular digits by
  default without OpenType feature selection, a grotesque with engineered, machined shapes that fit the "metal" name,
  and clear 1, l and I at 12 px. Fallback if the owner refuses OFL: load Segoe UI and Cascadia Mono from the system
  fonts folder at runtime on Windows (not redistributed), with system fonts on macOS and Linux.
- Enable no egui `default_fonts` (0.37 already makes it opt-in). Add a fallback chain for non-Latin scripts later
  through system fonts.
- **Icons:** Phosphor regular, MIT. Load the Phosphor TTF directly as an egui font family and keep our own constants
  module, instead of the egui-phosphor crate (it requires egui ^0.36). Icons draw in `icon` silver; never coloured,
  except error and warning glyphs and axis letters.
- **Emblem:** `branding/placeholder-icon.png` at 22 px in the title bar, rounded 5 px, no text next to it in the
  editor (the project name is there), "Metallic Studio" next to it on the start screen only. The window and taskbar
  icon come from the same file (Task 9).

### 5.15 Keyboard map (Metallic preset)

The first-run choice also offers a Roblox Studio preset and a Unity-style preset. Their exact bindings must be
checked against each tool's current docs when Task 10 builds them; I did not verify them.

| Action | Keys |
|---|---|
| Search or run a command | Ctrl+K (Ctrl+Shift+P opens with `>`) |
| Insert at cursor | Shift+A |
| Select, Move, Rotate, Scale | Q, W, E, R |
| Fly camera | Hold right mouse + WASD, Q down, E up, Shift faster, wheel speed |
| Orbit, pan, dolly | Alt+left drag, middle drag, wheel |
| Frame selection, frame all | F, Home |
| Front, right, top, orthographic | Numpad 1, 3, 7, 5 |
| Cycle selection under cursor | Alt+click |
| Toggle snapping while dragging, precision | Ctrl, Shift |
| Undo, redo | Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z |
| Cut, copy, paste, paste into | Ctrl+X, Ctrl+C, Ctrl+V, Ctrl+Shift+V |
| Duplicate, delete | Ctrl+D, Delete |
| Group, ungroup | Ctrl+G, Ctrl+Shift+G |
| Rename | F2 |
| Save, save as, open, new | Ctrl+S, Ctrl+Shift+S, Ctrl+O, Ctrl+N |
| Play, play here, stop, restart | F5, Alt+F5, Shift+F5, Ctrl+Shift+F5 |
| Pause, step one frame | F8, Ctrl+F8 |
| Debugger: continue, breakpoint, step over, into, out | F5, F9, F10, F11, Shift+F11 (Task 15) |
| Toggle left, right, bottom dock | Ctrl+B, Ctrl+Alt+B, Ctrl+J |
| Maximise panel under cursor | Ctrl+Space |
| Hide all docks | Ctrl+Shift+\\ |
| Next or previous panel | F6, Shift+F6 |
| Next or previous workspace | Ctrl+Page Down, Ctrl+Page Up |
| Litha panel, Litha on this line | Ctrl+Shift+L, Ctrl+I |
| Clear Output (Output focused) | Ctrl+L |
| Help for what is under the cursor | F1 |
| All shortcuts | Ctrl+/ |
| Animate: record keys, key selected, play clip | K, I, Space (timeline focused) |

Rules: one action per key per context; the palette and every menu show the current binding; Settings > Keyboard
edits bindings and warns on conflicts.

### 5.16 Performance targets for the UI

- Idle: repaint only on input, animation or a world change (issue #124). Viewport repaints at the display rate only
  while the camera moves, Play runs or an animation previews.
- Input to visible change under one frame at 60 Hz for selection, hover and gizmo drags.
- Explorer at 50k Instances: virtualised rows, filter results within 100 ms.
- Start screen visible within 1 s of launch on the reference PC; splash only if loading takes longer.

---

## 6. Mockups

`docs/design/studio-ui/`. Each HTML file is self-contained (fonts, icons and viewport images inlined) except the crow
emblem, which loads from `../../../branding/placeholder-icon.png` so the file is not copied. Fixed desktop size
1600×1000. Every effect is something egui draws: flat fills, 1 px lines, integer radii, one popover shadow, glyph
icons. Viewport pictures are three.js stand-ins rendered for the mockups.

| File | Shows |
|---|---|
| `start.html` / `.png` | First run: empty Recent games, the three first-run settings, the Parkour template selected, all starting points |
| `editor-3d.html` / `.png` | Parkour level in Build: Explorer, viewport with selection and move gizmo, guide step 2 of 5, Properties, Output |
| `editor-2d.html` / `.png` | A 2D platformer: Layers list, pixel grid, camera bounds, a sprite selected with its mover path, Tiles panel, the Ctrl K palette open |
| `play.html` / `.png` | Play state with a collapsed ×4 error, Explain answered by Litha with a diff, the green Play markers |
| `animate.html` / `.png` | The Animate workspace: rig, clips, onion skin, rotate gizmo, Keys timeline with events, key easing in Properties |

I looked at each screenshot and fixed what read as generic: violet checkboxes (now silver), an XZ plane handle in the
wrong colour, a palette that covered the selection, a diff that ran off the panel, gizmos too large for the part,
and a guide that disagreed with the Output.

---

## 7. What changes in current tasks

**Task 9 (#9): Studio shell, theme and start screen**
- Replace the plan's six colours with the token table in 5.13: deep #141217 becomes #111015, base #1c1922 becomes
  #16151b, panels #26222e become #1c1b22 (with #24222b for raised surfaces), accent #9b82d4 becomes #9d8be0, text
  #d7d5dc becomes #e6e4ea, dim #9a97a3 splits into #aeaab6 and #8e8a97. Some channels move more than the plan's ±8:
  the neutrals lose part of their violet tint so the accent stands out (Linear keeps brand colour out of neutrals,
  3.3). This is a deliberate change, listed in section 9.
- Extend `theme_text_contrast` to check each text token against each surface it may appear on, with the 4.5:1 limit,
  and muted text excluded from hover and active fills.
- Use egui 0.37 and egui-wgpu 0.37 (wgpu ^30 matches `metallic_render`). Do not enable `default_fonts`: its crate
  licence fails `deny.toml`. Bundle fonts per the owner's decision (section 8).
- Draw the custom title bar (OS decorations off) with the workspace switcher and Play cluster in place, even if only
  Build exists.
- Start screen per 5.8: the "What do you want to make?" view, the three first-run settings, empty states.
- Add egui_tiles 0.18 for docking with our own tab painting. Store layouts per workspace in the user config folder.

**Task 10 (#10): Studio editing**
- Keymap as data (a table of action id, default binding, context), so presets and Settings > Keyboard come free.
  Add Q/W/E/R tools, Shift+A Insert, Ctrl+K palette, F2, Alt-click cycle, Ctrl+Space maximise.
- Fly keys only while the right button is held.
- Mixed values show "Mixed" instead of a dash.
- Gizmos as own code painted with egui shapes; handles per 5.5; Ctrl inverts snapping while dragging.
- Model-first selection with a second click for the part.
- Every edit, including future Litha edits and kept Play edits, goes through `History`.

**Task 11 (#11): Play session**
- Play markers per 5.7 (band, outline, Explorer banner, status bar), the `success` token only.
- Stop collects a diff of the play copy against the edit world and offers Keep all, Choose, Discard as one undo step.
- Script errors never stop Play; Output collapses repeats and links source lines.
- Pause (F8) and step frame (Ctrl+F8) belong here even if the debugger comes in Task 15.

**Task 14 (#14): dev mode**
- Dev mode adds the Debug workspace to the switcher; it does not add a separate window. Debug views live in the
  viewport header's "Debug views" menu.

**Epic M4 (#17): creation workflow**
- The Assets tab grows into the asset browser; drag targets stay the viewport and Explorer.
- Plugins add panels as dock tabs and commands to Ctrl K only; no plugin toolbars.

**Epic M6 (#19): modeller, animation, VFX**
- Build Timeline, Curve editor and Node canvas once in `metallic_studio::widgets`, themed with these tokens. #40 and
  #41 are building early timeline and node editor demos now; they should take the tokens from 5.13 so the demos do
  not fork the look.
- Animate: explicit Record keys (K). VFX: stack first, graph second.

---

## 8. Owner decisions needed

1. **Allow OFL-1.1 for bundled font files?** Recommended: yes, for font files only (not crates), as Rerun does.
   Without it, Studio loads system fonts at runtime and looks different on each OS.
2. **egui version policy.** Recommended: one egui version across the workspace, 0.37 now. This rules out egui_dock,
   egui-snarl, transform-gizmo-egui and egui-phosphor until they update; we use egui_tiles, own gizmos and the
   Phosphor TTF. If the node editor work (#41) needs egui-snarl now, the alternative is pinning 0.36 everywhere and
   using egui_dock.
3. **Keep instance edits from Play (Stop diff)?** Recommended: yes. It is more work in Task 11 than Roblox's "reset
   everything", and it removes the most-cited lost-work complaint.

---

## 9. Assistant's calls (overturn any)

- Graphite tokens in 5.13 replace the plan's colours; silver checkboxes; accent reserved for four uses.
- IBM Plex Sans and Mono; Phosphor icons through the TTF.
- egui_tiles over egui_dock; no floating panels in M1.
- Fly keys only while the right mouse button is held; Q/W/E/R for tools.
- Model-first selection with second click for parts.
- One viewport with 2D, Side and 3D presets; view default from `metallic.toml`.
- Play markers in green only; no editor-wide tint.
- Stop offers to keep Play edits.
- Guide strip without input blocking.
- Litha's Apply button neutral, not accent.
- Explicit record keys in Animate.
- Minimise UI with Ctrl+Shift+\\.
- Interface size and contrast on the start screen.
- Layout stored per user, not per project.

---

## 10. Proposed new issues (not created)

1. **Studio: theme tokens module and contrast test.** Put the 5.13 tokens in `theme.rs` with egui mappings and a test
   that checks every text token on every allowed surface. Blocks every UI task.
2. **Studio: custom title bar with workspaces and Play cluster.** Draw the 40 px title bar with OS decorations off,
   snap-layout hit regions, menus, workspace switcher and Play controls.
3. **Studio: docking with egui_tiles and per-workspace layouts.** Own tab painting, drop zones, Reset layout, panel
   toggles (Ctrl+B, Ctrl+Alt+B, Ctrl+J) and maximise (Ctrl+Space).
4. **Studio: command palette (Ctrl K).** One index of Instances, assets, commands and guides with prefixes, groups,
   shortcuts shown and a Litha row reserved.
5. **Studio: keymap as data with three presets.** Action table, Metallic, Roblox Studio and Unity-style presets,
   Settings > Keyboard with conflict warnings, Ctrl+/ overlay.
6. **Studio: own transform gizmos (3D and 2D).** Move, rotate and scale handles painted with egui shapes, constant
   screen size, value labels, snapping modifiers, tested hit regions.
7. **Studio: Stop keeps chosen Play edits.** Diff the play copy against the edit world on Stop and apply the chosen
   changes as one undo step.
8. **Studio: Output collapse, links and Explain.** Collapse repeated errors with a count, link script:line, add Open
   line and Explain (docs page until Litha exists).
9. **Studio: 2D view preset, Layers list and Tiles panel.** Orthographic Side and 2D presets, pixel grid, sort rule
   in each layer row, tile painting with a named target layer (M3).
10. **Studio: shared Timeline, Curve and Node canvas widgets.** One themed implementation for Animate, VFX, Profiler
    and Materials, with AccessKit roles; align #40 and #41 with it.
11. **Studio: High contrast theme and interface size.** `line.hc` borders, 3 px focus ring, promoted muted text, and
    90% to 150% UI scale from the start screen.
12. **Studio: Light theme (later).** A light token set for daytime work, after M1, checked by the same contrast test.
13. **Owner decision: OFL-1.1 for bundled fonts.** Decide whether font files under OFL-1.1 may ship with Studio and
    the player; update the licence policy text.

---

## 11. What I could not verify

- Reddit threads (blocked), the Blender manual (403), Spline's docs and Construct's site (blocked), the Unreal
  "Play in Editor" page (empty). I relied on the other sources listed.
- egui issue #1758 (backdrop blur) came from a subagent; I did not re-fetch it.
- Exact shortcut lists for the Roblox Studio and Unity-style presets: not checked; Task 10 must check them.
- How the mockup layout feels at 125% and 150% interface size: the mockups are fixed at 100%.
- egui's real text rendering of IBM Plex at 13 px on Windows: the mockups use a browser's renderer. Task 9 should
  screenshot egui with the chosen fonts before committing to them.
- Performance numbers in 5.16 are targets, not measurements.
