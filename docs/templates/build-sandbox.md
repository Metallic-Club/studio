# Template: Build Sandbox (the "Build example")

Status: planned. Milestone: M3 (saving needs the platform's data store from M2; Studio emulates it before then).
Default look: `meadow`. Players: 1 to 8. Difficulty: intermediate. Conventions and API group ids:
`docs/templates/README.md`.

Sample world: **Plot Island**. Every player gets a 64 x 64 stud plot and a build tool: place blocks on a grid, rotate,
paint, delete, undo, save. Friends can be invited to build together. An optional Challenge mode runs timed theme
rounds with a vote.

## Why this template

The owner named a "Build example". Building inside the game is the bridge between playing and creating: a player who
builds on a plot today is a creator in Studio tomorrow (Dreams and LittleBigPlanet used the same ladder, research doc
section 1). Technically it is the template that teaches mouse-to-world rays, grid snapping, server checks against
griefing and saving player-made data. Those pieces are reused by tycoons, tower defense placement and housing in
roleplay games.

## Game loop

1. Spawn at the plaza. Walk to a free plot and press "Claim" on its sign (or get one automatically after 10 s).
2. Press B (or the Build button). Pick a block, a material and a colour. Click to place. R rotates, T tips over,
   X deletes, Ctrl+Z undoes.
3. Your build saves every 60 s and when you leave. Next time you join, it loads on whichever plot you claim.
4. Visit other plots. Press "Like" on a plot sign. Likes show on the leaderboard.
5. Optional Challenge mode (Game setting): a theme appears ("A bridge", "A treehouse"), everyone builds for
   `ChallengeMinutes`, then a tour visits each plot for 15 s while players vote 1 to 5 stars. Top three stand on
   the podium. Plots clear back to each player's saved free build afterwards.

## Build tool

| Block | Size (studs) | Notes |
|---|---|---|
| Cube | 4 x 4 x 4 | |
| Plate | 4 x 1 x 4 | floors |
| Beam | 1 x 1 x 8 | frames, fences |
| Wall | 8 x 8 x 1 | |
| Wedge | 4 x 4 x 4 | roofs, ramps |
| Stairs | 4 x 4 x 8 | a wedge with steps on top |
| Cylinder | 4 x 4 x 4 | |
| Ball | 4 x 4 x 4 | |
| Window | 8 x 8 x 1 | Glass, `Transparency` 0.4 |
| Door | 4 x 7 x 1 | opens with a prompt for everyone |
| Lamp | 1 x 2 x 1 | carries a `PointLight` |
| Plant | 2 x 4 x 2 | a small Part model |

Materials: Plastic, SmoothPlastic, Wood, Brick, Concrete, Metal, Glass, Grass, Sand, Ice (the Task 2 list).
Colours: 24 swatches taken from the current look preset plus a custom picker.

Placement: the client casts a ray from the mouse into the world, ignoring the ghost. The block's position is the hit
point pushed out along the surface normal by half the block's size, then snapped to `GridSize` relative to the plot's
corner. Rotation snaps to 90 degrees (or 45 when `FineRotation` is on). A ghost block shows the result: a white
outline when allowed, a red outline and a cross icon when not (so colour is not the only signal).

## Level layout

Island 400 x 400 studs of Grass at y 0, sea around it (Water role). About 350 Parts before anyone builds.

| Area | Where | Contents |
|---|---|---|
| Plaza | centre, 48 x 48 | SpawnLocation, a board explaining the controls, the Challenge podium (3 steps), a theme sign |
| Plots | 8 plots, centres x in {-120, -40, 40, 120}, z in {-60, 60} | Each a 64 x 1 x 64 base Part tagged `Plot` with `PlotId`, a thin border (Trim role), a sign with a `SurfaceGui` (owner name, likes) and prompts Claim, Like, Invite |
| Avenue | between the two plot rows | 16-stud paths, lamp posts, benches |

Build volume per plot: the base area, up to 64 studs high.

## Instances

```
Workspace
  Plaza (Folder)
  Plots (Folder)
    Plot1 ... Plot8 (Model): Base (Part, tagged Plot, attributes PlotId, Owner), Border, Sign (Part with SurfaceGui
                             and ProximityPrompts), Blocks (Folder, empty: placed blocks go here)
  SpawnLocation
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  Blocks (Folder)                 one Model or Part per block type, the source the server clones; tagged by type
  Shared (Folder)
    BlockCatalog (ModuleScript)   sizes, allowed rotations, display names, icons
    Grid (ModuleScript)           the snapping maths used by both client ghost and server check
    Themes (ModuleScript)         Challenge themes
ServerScriptService
  PlotService, BuildService, SaveService, LikeService, ChallengeService (Script)
StarterPlayer.StarterPlayerScripts
  BuildController, BuildMenuController, HudController (LocalScript)
StarterGui
  BuildMenu (ScreenGui): block grid, material row, colour swatches, tool buttons (Place, Paint, Delete), block count
  Hud (ScreenGui): Build button, save indicator, Challenge timer and theme
  Vote (ScreenGui): five star buttons
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `PlotService` | server | Claim and release plots (release when the owner leaves, after saving), auto-assign after 10 s, sign text, invitations to co-build. |
| `BuildService` | server | Handles place, remove, paint and undo: runs every check below, clones from `ReplicatedStorage.Blocks`, keeps a per-player undo list of 50 actions, counts blocks. `-- TRY: set HEIGHT_LIMIT to 128 for taller builds.` |
| `SaveService` | server | Serialises a plot's blocks relative to the plot corner (type, position, rotation, colour, material), saves every 60 s and on leave, loads on claim. Keeps the save under the store's size limit by refusing builds past `MaxBlocksPerPlayer`. |
| `LikeService` | server | One like per visitor per plot per day, not your own plot; `leaderstats.Likes`. |
| `ChallengeService` | server | Only when `Mode` = Challenge. Theme pick, build phase, tour and vote, podium, restore free builds. |
| `BuildController` | client | Build mode on and off, mouse ray, ghost block, snapping with `Shared.Grid`, rotate and tilt keys, sends requests, plays place and delete sounds. `-- TRY: change GHOST_TRANSPARENCY to 0.2.` |
| `BuildMenuController` | client | The palette UI, keyboard shortcuts 1 to 9 for blocks, gamepad navigation. |
| `HudController` | client | Build button, "Saved" indicator, Challenge timer, vote buttons. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `PlaceBlock` | client to server | `{blockType, position, rotation, color, material}` | block type in the catalog; material allowed; colour is a Color3; position snapped to `GridSize` and inside a plot the player owns or was invited to, below the height limit; rotation is a multiple of 90 (or 45); no overlap with existing blocks (box query shrunk by 0.05 studs); player under `MaxBlocksPerPlayer`; at most 10 requests per second; not during a Challenge tour |
| `RemoveBlock` | client to server | block Instance | block is in a plot the player may build on; rate limit |
| `PaintBlock` | client to server | `{block, color, material}` | same as remove, plus the colour and material checks |
| `UndoBuild` | client to server | none | player has an undo entry; rate limit |
| `ClearPlot` | client to server (RemoteFunction, asks to confirm) | none | owner only; returns the number of blocks removed |
| `InviteBuilder` | client to server | target player | owner only; target is in the server; at most 4 invitees |
| `BuildRejected` | server to one client | `{reason}` | n/a; the ghost flashes the reason ("Outside your plot", "Block limit reached") |
| `ChallengeStateChanged` | server to all | `{phase, theme, timeLeft, plotOnTour}` | n/a |
| `CastVote` | client to server | `{plotId, stars}` | Challenge tour phase; plot is on tour; not your own plot; stars 1 to 5; one vote per plot |

## UI

Build menu docks at the bottom: block icons in a grid, a material row, colour swatches, three tool buttons with key
hints. A counter shows `312 / 1000 blocks`. Top right: "Saved" with the time of the last save. All panels follow the
`Look:UI` theme. Gamepad: shoulder buttons cycle blocks, triggers place and delete.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `GridSize` | 1 | 0.5, 1, 2, 4 | Snap step in studs |
| `FineRotation` | false | bool | Allow 45-degree steps |
| `MaxBlocksPerPlayer` | 1000 | 100 to 3000 | Block limit per player |
| `HeightLimit` | 64 | 16 to 128 | Highest build point above the plot |
| `PlotSize` | 64 | 32 to 96 | Plot edge length (applies on server start) |
| `SaveEnabled` | true | bool | Save and load builds |
| `AllowCoBuild` | true | bool | Owners can invite others to build |
| `AllowedMaterials` | all | multi-choice | Materials in the palette |
| `Mode` | `Free` | Free, Challenge | Free building or theme rounds |
| `ChallengeMinutes` | 5 | 2 to 15 | Build time per theme |
| `LightsInBlocks` | true | bool | Allow Lamp blocks (turn off on weak PCs) |

## Multiplayer

Only owners and invitees can change a plot; everyone else can walk on it and open its doors. Blocks are created by the
server, so every client sees them. A late joiner sees all plots as they are. Leaving saves and frees the plot. At the
default limits 8 players can place 8,000 blocks, so the renderer must hold 10,000 Parts at 60 fps (the M1 stress
target is 5,000; this template raises it).

## Tutorial

1. "Press Play, claim a plot and press B to open the build tool." (`remote_fired` PlaceBlock)
2. "Build a small hut: four walls and a roof." (`printed` "TUTORIAL_HUT", sent when a plot has 5 or more blocks)
3. "Pick the Toybox look. Your palette changes with it." (`look_applied`)
4. "In Game settings, set Grid size to 0.5 and turn on Fine rotation." (`setting_changed` GridSize)
5. "Add your own block: duplicate ReplicatedStorage.Blocks.Cube, rename it Pillar and make it 2 x 8 x 2." (`inserted`
   under ReplicatedStorage.Blocks)
6. "Open Shared.BlockCatalog and add a line for Pillar, using the Cube line as a pattern." (`script_edited`
   BlockCatalog)
7. "Play, build with your Pillar, and share the game." (`play_started`)

## Style variants

`meadow` (default): grass island, warm afternoon. `toybox`: bright plastic blocks, a kids' building game. `frost`:
snow island, ice blocks. `titanium`: graphite plots, a "design studio" feel. With `Mode` = Challenge it becomes a build
contest game; with `GridSize` 4 and only Cube and Wedge it becomes a voxel game.

## New API this template needs

A1 (`BlockCatalog`, `Grid`, `Themes`), A2, A3, A4, A5 (`UserInputService`, `TweenService`), A6 (CFrame for rotation and
plot-relative positions), **A7 (`Raycast` for the mouse ray, `GetPartBoundsInBox` for overlap)**, A8 (`Likes`), A10
(`CameraMaxZoomDistance`), A13 (`PointLight` in Lamp), A14, A15 (sign prompts), A16 (`RemoteFunction` for ClearPlot),
**A17 (data store)**, A19 (`ViewportPointToRay`), A20 (keys, mouse buttons), A21 (`TextButton`, `ImageButton`,
`UIGridLayout`, `SurfaceGui`).

## Acceptance checks

- [ ] `tests/smoke.mscene` with two bots: A claims Plot1, places 20 blocks, removes 5, undoes once (16 blocks); B tries
      to place on Plot1 (refused, `BuildRejected` "Not your plot"); A invites B, B places 1 block (accepted); A places a
      block overlapping an existing one (refused); A sends 50 `PlaceBlock` in one second (at most 10 accepted); save,
      clear, load: the same 17 blocks at the same positions. Print `BUILD_SMOKE_OK`.
- [ ] A save at `MaxBlocksPerPlayer` = 3000 loads back exactly and stays under the store's per-key size limit.
- [ ] Ghost block and the server agree on the snapped position in 1,000 random placements (shared `Grid` module).
- [ ] 8 players with 1,000 blocks each: 60 fps on the reference PC; integrated graphics measured or "not verified".
- [ ] Challenge mode runs three rounds in a row; plots restore to free builds after each.
- [ ] Screenshots in `meadow`, `toybox`, `frost`, `titanium`.
