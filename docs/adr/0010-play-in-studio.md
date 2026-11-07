# ADR 0010: Play in Studio

- **Status:** Accepted (2026-11-07, the owner's choices in #244)
- **Issue:** #244
- **Builds on:** [ADR 0004](0004-runtime-rendering-platform.md) (the engine's loop and render hook),
  [ADR 0006](0006-project-format.md) (scenes), [ADR 0009](0009-studio-foundation.md) (Studio's dock, viewport and
  keymap).

## Context

M6's goal (#22): press Play and the game runs inside Studio, through the same Player library that ships, while Studio
can still edit it. The owner opened it on 2026-11-07 with Forge-Engine's built-in playtesting as the reference
(`crates/forge_editor/src/play.rs` and `state/play.rs` in `ItsMattoid/Forge-Engine`).

**What Forge does**, and is kept here:

- Play opens a **Game** tab beside the viewport, in front, looking through `Workspace.CurrentCamera`. The viewport's
  own tab keeps the editor camera, and the game keeps running whichever tab is in front.
- The game gets the keyboard and mouse only while its view has the focus; **Esc** gives them back to the editor (the
  game still sees that Esc). `MouseBehavior.LockCenter` captures the cursor, kept inside the view.
- Edits made while playing (Explorer, Properties) change the running game at once. **Stop** throws the session away:
  the scene, the selection and the unsaved flag are as they were before Play. Saving during Play never writes the
  running game. Script output goes to the Output panel. A script can end the session.

**What Forge does that is not kept:** it ran the script host on the editor's own data model and, on Stop, rebuilt the
scene from a snapshot, then followed every old instance to its replacement for the undo history and the selection.
That is the error-prone part, and Metallic's engine allows something simpler.

**What Metallic has:** `metallic_player::Player` makes an `Engine` around a data model it loads itself (a project's
start scene or a built-in scene), and draws through a `SceneRenderer` that is the engine's render hook on a window.
`Engine::with_data_model` takes an existing data model. `SceneRenderer` can already draw into a `ViewportTarget` on a
caller's device (Studio's viewport does, #213), and the engine takes input from its host (`Engine::push_input`). A
`DataModel` cannot be cloned; a scene can be saved and loaded (`save_scene`, `load_scene`).

The owner chose (2026-11-07): **a Game tab**; **Play, Pause, Step and Stop** (no Run mode yet); **edits made while
playing are discarded on Stop**.

## Decision

### 1. The game is a Player, embedded

The game runs in a `metallic_player::Player`, never in Studio's own code, so Play in Studio runs exactly what ships.
The Player gains a fourth way to be driven, beside its binary, the Launcher and a host-owned window:

| Host | Owns | Drives the Player with |
|---|---|---|
| Studio's Game tab (M6) | the event loop, the window and the render device | a Player made from a data model and the project's script sources, [`attach_viewport`] on the host's device, then per frame `push_input`, `frame`, and `render_viewport`; the host shows the viewport's texture |

- **Made from a data model:** the host hands over the game to run and the script sources; the Player does not open
  the project itself. It is a local game (`RunContext::Local`) with `RunService:IsStudio()` true.
- **No window:** the engine is windowed (it has input and a renderer) but has no window attached, so its render hook
  never runs; the host asks the Player to draw into its viewport target after each frame. Rendering settings are the
  host's (Studio's Rendering settings), not `graphics.toml`.
- **Output** goes to the host's sink (`Player::with_output`): Studio sends it to the Output panel.

The API's exact shape is settled in its issue; the binary and the Launcher keep theirs.

### 2. Copy on Play, discard on Stop

Play **copies** the editor's scene into a new data model by saving it and loading the text (`save_scene` then
`load_scene`, the same round trip as a project file, so what plays is what would be saved), and gives the copy to the
Player. The editor's data model is **never touched** by a session:

- nothing the game or a live edit does can leak into the saved scene;
- Stop is dropping the Player: no rebuild, no remapping of instances. The selection, the Explorer's state and (once it
  exists) the undo history are simply still there;
- the scene's ids (`SceneIds`) say which instance of the copy is which instance of the editor's scene, for carrying
  the selection into the session (§5).

A session starts from the scene as it is in the editor, unsaved edits included. Scripts are read from the project's
members, as the Player reads them from a file.

### 3. Controls

| Control | Default binding | Does |
|---|---|---|
| **Play** | F5 | Copies the scene, starts the Player, opens the Game tab in front with the game's input |
| **Pause / Resume** | F7 | Freezes game time (`Engine::pause`); scripts and simulation stop, the view keeps drawing |
| **Step** | F6 | While paused, runs one frame of one fixed step (`Engine::step_frame`) |
| **Stop** | Shift+F5 | Ends the session (§2); the Game tab closes and the Viewport's tab comes back to the front |

They are keymap commands (rebindable, ADR 0009 Keybinds), in the **Test** menu, and in a group of play buttons in the
editor's top bar (Play in the theme's green "play" colour). Each is disabled when it cannot run (Pause without a
session, Step unless paused). **Save is disabled while playing**, so a save is never mistaken for saving the game's
state. Run (scripts without the game camera) and Play Here wait for characters; multiplayer waits for M10.

### 4. The Game tab and the input hand-off

- **Game** is a panel like the others (the dock's registry), opened by Play beside the Viewport and in front, and
  closed by Stop. It is not in the View menu, and the dock never saves it in `layout.json`, so a layout saved during a
  session does not bring back an empty Game tab.
- It shows the Player's viewport texture, drawn from `Workspace.CurrentCamera` at the tab's size.
- **Input:** while the Game tab has the focus (Play gives it, a click in it gives it back), Studio converts its
  keyboard, mouse and wheel input into the platform's events, positions relative to the tab, and pushes them to the
  Player; the keymap runs only commands marked as working during play (Stop, Pause, Step, Settings). **Esc** gives
  the focus back to Studio after the game sees it. Clicking any other panel does the same.
- **`MouseBehavior`:** while the game has the focus and asks for `LockCenter` or `LockCurrentPosition`, Studio's
  shell grabs the cursor in its own window (the Player has no window of its own) and feeds the game raw mouse
  motion; it lets go on Esc, on Stop and when the window loses the focus.

### 5. Live editing

While playing, the **Explorer and Properties show and edit the running game**, so a change applies at once, as in
Forge and Roblox. The selection is carried into the session by scene id and comes back on Stop as it was before Play.
The Viewport's tab keeps showing the editor's scene, unchanged, from the editor camera. Edits are discarded on Stop
(§2); keeping them is not planned.

### 6. Slices

1. #245: `metallic_player` embeddable in a host's viewport (§1).
2. #246: Studio: the Game tab with Play, Pause, Step and Stop (§2, §3), script output in Output.
3. #247: Studio: the input hand-off and `MouseBehavior` (§4).
4. #248: Studio: live editing (§5).

## Consequences

- One code path runs games everywhere; a bug in Play in Studio is a bug in the Player.
- Starting a session costs a scene save and load plus the Player's start (scripts' top-level code). For a large scene
  that is a visible pause; a faster copy can come later without changing this record.
- Two scenes are drawn while playing (the Viewport's and the Game's), each with its own GPU scene, on one device.
  A hidden tab is not drawn.
- Physics (M7) plugs into the Player's engine, so it plays in Studio with no Studio work.

[`attach_viewport`]: ../../crates/engine/src/scene_renderer.rs
