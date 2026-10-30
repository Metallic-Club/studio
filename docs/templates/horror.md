# Template: Horror (co-op escape)

Status: planned. Milestone: M3. Default look: `noir`. Players: 1 to 4. Difficulty: intermediate.
Conventions and API group ids: `docs/templates/README.md`.

Sample world: **The Archive**. You and up to three friends wake in a closed archive at night. Find the fuses, power the
service lift and get out. The Keeper walks the halls. It hears you run and sees your flashlight.

## Why this template

The owner named Horror. Horror is one of Roblox's top broad searches, and co-op
horror games sit in its most played list (research doc, section 2). It needs no weapons and teaches the parts every story game uses: an NPC
with states, objectives, interactable objects, scripted events, lights and sound. It also pushes the renderer where
Parkour does not: darkness that still reads, a shadowed flashlight per player, fog.

## Game loop

1. Spawn in the lobby. The HUD says "Find 5 fuses. Bring them to the fuse box."
2. Explore two floors with a flashlight. Its battery drains; battery packs refill it.
3. Each fuse lies at one of 12 hiding places, picked at random each round. Pick up with a prompt, carry one at a time,
   insert at the fuse box by the lift.
4. The Keeper patrols. Running, slamming doors and inserting fuses make noise. A lit flashlight makes you visible from
   farther away. Hide in a locker, crouch, or break line of sight.
5. Caught means downed. A teammate can revive you by holding the prompt for `ReviveSeconds` before
   `BleedOutSeconds` runs out. Solo players have `SoloLives`; each catch sends you back to the lobby.
6. When the last fuse goes in, the lights come on, the lift opens and the Keeper speeds up. Everyone still standing
   runs for the lift. The round is won when every standing player is inside, or 20 s after the first one gets in.
7. Results show time and who escaped. A new round starts with new fuse spots.

Session length: 8 to 15 minutes.

## Movement and the Keeper

Players: WalkSpeed 12 (a slower horror pace), sprint 20 for up to `StaminaSeconds`, crouch 6 and silent.

Noise is computed on the server from each character's real speed, never from what the client says: sprinting makes
a noise of radius 30 studs, walking 10, crouching 0. Events add noise: door slam 25, fuse insert 40.

The Keeper is a 7-stud Humanoid, dark grey, with no face detail (a general-audience design, no gore). States:

| State | Speed | Enters when | Does |
|---|---|---|---|
| Patrol | 10 | default | walks a loop of nodes |
| Investigate | 14 | hears noise within radius x `KeeperHearing` | walks to the noise point, looks around 4 s |
| Chase | `KeeperSpeed` (18) | sees a player: within `KeeperSightRange` (50, or 70 if the player's light is on), inside a 110-degree cone, clear ray to the head | runs at the player; catches within 3.5 studs |
| Search | 12 | loses sight for 3 s | checks the last seen point and the two nearest nodes |
| Leave | 10 | just downed someone | walks away for 15 s, so revives are possible |

The Keeper moves with `Humanoid:MoveTo` along a hand-placed node graph (`Workspace.KeeperNodes`, about 30 invisible
Parts, each with a `Links` attribute naming its neighbours). A small A* in `Shared.NodeGraph` finds routes. In Chase
with a clear line it runs straight at the player. No pathfinding service is needed. If the Keeper makes no progress
for 5 s, it moves to the nearest node out of every player's sight.

Hiding: 6 lockers with a "Hide" prompt. A hidden player is invisible to the Keeper, unless the Keeper saw them get in;
then it opens the locker.

## Level layout

Building footprint x -60 to 60, z -40 to 40. Floor 1 at y 0, floor 2 at y 14, ceilings 12 studs. About 900 Parts.

| Area | Where | Contents |
|---|---|---|
| Lobby | floor 1, x -60 to -30 | Spawn, fuse box and service lift (the exit) at (-50, 0, 30), two dim warm emergency lights |
| Stacks | floor 1, x -30 to 30, z -40 to 10 | Shelves 2 x 10 x 24 in rows every 8 studs (6-stud aisles), two cross aisles, 2 lockers |
| Reading room | floor 1, x -30 to 30, z 10 to 40 | Long tables, tall windows (moonlight), stairs up at x -40 (wedges, 14 up over 16) |
| Offices | floor 1, x 30 to 60 | Four offices 15 x 20 with doors, a corridor, 2 lockers |
| Mezzanine | floor 2 over the stacks | Walkway with railings, looking down into the aisles |
| Vault and boiler room | floor 2, x 0 to 60 | Tight rooms, 2 lockers, the loudest door |

12 `FuseSpot` Parts (on shelves, under desks, in the boiler room), 8 `BatterySpot` Parts, 10 doors, 6 scare triggers.

## Instances

```
Workspace
  Building (Folder)
    Floor1, Floor2, Stairs (Folder)     Parts tagged Look:Wall / Look:Ground / Look:Trim
    Doors (Folder)                      Door (Model with a hinge Part and ProximityPrompt), tagged Door
    Lockers (Folder)                    Locker (Model with ProximityPrompt), tagged HidingSpot
    Lights (Folder)                     Parts with PointLight, tagged Look:Light and Flicker
  Objective (Folder)
    FuseSpots (Folder)                  invisible Parts, tagged FuseSpot
    BatterySpots (Folder)               invisible Parts, tagged BatterySpot
    FuseBox (Model with ProximityPrompt), Lift (Model with door Parts and an inside zone Part)
  KeeperNodes (Folder)                  invisible Parts with attribute Links
  ScareTriggers (Folder)                invisible zone Parts, attribute Scare = Flicker | DoorSlam | Whisper | ShelfDrop
  SpawnLocation
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  Shared (Folder): NodeGraph, Noise (ModuleScript)
ServerStorage
  Keeper (Model with Humanoid)
  Pickups (Folder): Fuse, Battery (Model)
ServerScriptService
  RoundService, ObjectiveService, KeeperService, NoiseService, DownedService, DoorService, HidingService,
  ScareDirector (Script)
StarterPlayer.StarterPlayerScripts
  FlashlightController, MovementController, DreadController, HudController, ScareEffects (LocalScript)
StarterGui
  Hud (ScreenGui): Objective, Battery, Stamina, Prompt hints
  Results (ScreenGui)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `RoundService` | server | Lobby, Playing, Escape, Ended. Starts a round, ends it on win or when nobody is left standing, restarts with a new seed. |
| `ObjectiveService` | server | Places fuses and batteries at random spots (`Random.new(seed)`), handles pick-up and insert prompts, one fuse carried at a time, powers the lift, fires `ObjectiveChanged`. |
| `KeeperService` | server | The state machine above. Sight checks 5 times a second, not every frame. `-- TRY: change LOOK_AROUND_SECONDS to 1 for an impatient Keeper.` |
| `NoiseService` | server | Measures each character's speed and turns it into noise; receives event noises from other services; tells the Keeper. |
| `DownedService` | server | Downs a caught player (crawl speed 3), runs bleed-out, handles revive prompts, solo lives, spectating. |
| `DoorService` | server | Opens and closes doors with a tween; a fast close is a slam (noise 25). The Keeper opens doors at half speed. |
| `HidingService` | server | Enter and leave lockers; tells the Keeper who is hidden and whether it saw them enter. |
| `ScareDirector` | server | When a player enters a scare trigger, plays that scare once per round, at most one scare per `ScareCooldown` for all players. `-- TRY: change FLICKER_TIME from 1.5 to 4.` |
| `FlashlightController` | client | F toggles; a `SpotLight` on the camera for you, on the head for others (from the `FlashlightOn` attribute); shows battery. |
| `MovementController` | client | Sprint and crouch keys; asks the server; camera height drops when crouched; head bob. |
| `DreadController` | client | Heartbeat sound and a soft vignette that grow as the Keeper gets closer, stronger in Chase. Reads `ScareIntensity`. |
| `HudController` | client | Objective line `Fuses 2 / 5`, battery bar, stamina bar, short prompt hints. |
| `ScareEffects` | client | Plays the client side of a scare (sound, small camera shake), scaled by `ScareIntensity`. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `FlashlightToggled` | client to server | `{on}` | alive and not downed; battery above 0 to turn on; at most 5 per second |
| `MovementModeChanged` | client to server | `"walk" | "sprint" | "crouch"` | sprint only with stamina left; the server sets `WalkSpeed` and drains stamina itself |
| `ObjectiveChanged` | server to all | `{inserted, needed, exitOpen}` | n/a |
| `PlayerDowned` | server to all | `{player, bleedOutAt}` | n/a |
| `ScarePlayed` | server to clients near it | `{kind, position}` | n/a |
| `KeeperNear` | server to one client | `{distance, chasing}` 4 times a second while within 40 studs | n/a |
| `RoundEnded` | server to all | `{won, seconds, escaped}` | n/a |

Pick-ups, inserts, revives, doors and lockers go through `ProximityPrompt.Triggered`, which the server receives
directly. The server still checks distance and state (one fuse at a time, not downed, door not locked).

## UI

The HUD stays sparse. Top left: one objective line. Bottom: battery and stamina as thin bars that fade when full.
Prompts show their key. Results: escaped players, time, fuses found. Text in `Look:UI` style; minimum 18 px.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `FusesNeeded` | 5 | 1 to 10 | Fuses to find (12 spots exist) |
| `KeeperSpeed` | 18 | 10 to 26 | Chase speed; players sprint at 20 |
| `KeeperHearing` | 1.0 | 0 to 2 | Noise radius multiplier; 0 = deaf |
| `KeeperSightRange` | 50 | 10 to 100 | How far it sees |
| `FlashlightBattery` | 180 | 30 to 600 | Seconds of light per full battery |
| `StaminaSeconds` | 6 | 2 to 20 | Sprint time |
| `ReviveSeconds` | 4 | 1 to 10 | Hold time to revive |
| `BleedOutSeconds` | 20 | 5 to 60 | Time to be revived |
| `SoloLives` | 2 | 1 to 5 | Catches allowed when alone |
| `ScareCooldown` | 60 | 15 to 300 | Minimum gap between scares |
| `ScareIntensity` | `Full` | Off, Low, Full | Sound and shake strength of scares |
| `Darkness` | 0.8 | 0.2 to 1 | How dark the building is |
| `EnrageAtEnd` | true | bool | Keeper speeds up by 2 after the last fuse |

Presets in the panel: **Easy** (3 fuses, speed 14, hearing 0.6), **Normal** (defaults), **Hard** (7 fuses, speed 21,
hearing 1.4).

## Multiplayer

Up to 4 players, all server-authoritative. The Keeper targets the player it most recently sensed; it never camps a
downed player (Leave state). Flashlight state replicates as a character attribute, so every client draws every
light. Late joiners spawn in the lobby with no fuse and full battery, mid-round. Leaving while carrying a fuse drops it
where you stood.

## Tutorial

1. "Press Play. Find one fuse and put it in the fuse box." (`remote_fired` ObjectiveChanged)
2. "Want it less scary? Pick the Dusk look." (`look_applied`)
3. "In Game settings, choose Easy, or set Keeper speed to 14." (`setting_changed` KeeperSpeed)
4. "Select ScareTriggers.Flicker1, press Ctrl+D and drag the copy into a hallway." (`inserted` Part under ScareTriggers)
5. "Open ScareDirector. Make the flicker last longer on the TRY line." (`script_edited` ScareDirector)
6. "Play, walk through your new trigger, then escape." (`remote_fired` RoundEnded)
7. "Name your horror game and share it." (`renamed_game`)

## Style variants

`noir` (default): near-black, cold moonlight through windows, fog 20 to 80 studs. `dusk`: the same building at sunset,
for a mild "spooky" game for younger players. `frost`: a frozen research station feel (lights blue-white).
`toybox` with `ScareIntensity` Off: a hide-and-seek game for kids. Swap-in art: replace `ServerStorage.Keeper` with any
Model that has a Humanoid and the same name; replace `Pickups.Fuse` with any Model.

## New API this template needs

A1 (`NodeGraph`, `Noise`), A2, A3, A4, A5 (`ServerStorage`, `TweenService`, `UserInputService`, `SoundService`),
A6, A7 (`Raycast` for sight), A10 (`MoveTo`, `MoveToFinished`, `HealthChanged`, `PlayerRemoving`), A12 (doors,
flicker), **A13 (`SpotLight` with shadows, `PointLight`, `Lighting` fog and ambient)**, **A14 (positional `Sound`;
horror without sound fails the quality bar)**, A15 (`ProximityPrompt` with `HoldDuration`), A19 (camera height, shake),
A20 (sprint, crouch, flashlight keys), A21, A24 (`Random.new`). Optional A22 for flashlight hands.

Renderer work this template drives: at least 5 shadowed spot lights and 20 unshadowed point lights at 60 fps, fog,
and a dark exposure where silhouettes still read.

## Acceptance checks

- [ ] `tests/smoke.mscene` with two bots: bot A sprints 25 studs from the Keeper, Keeper enters Investigate; bot B
      crouches 10 studs away behind a wall, Keeper stays in Patrol; bot A teleports next to the Keeper, is downed once;
      bot B revives A, A stands with health restored; bots insert `FusesNeeded` fuses, `ObjectiveChanged` reports
      `exitOpen` once; both enter the lift, `RoundEnded{won = true}` fires once. Print `HORROR_SMOKE_OK`.
- [ ] A 20-minute soak with the Keeper patrolling and two bots wandering: the Keeper never stays stuck longer than 5 s.
- [ ] `FlashlightToggled` sent 50 times in a second toggles at most 5 times; turning on with an empty battery is
      refused.
- [ ] 4 players, all lights on: 60 fps at 1080p high on the reference PC; integrated graphics measured or
      "not verified".
- [ ] `ScareIntensity` Off removes every shake and stinger; no scene flashes more than 3 times per second.
- [ ] Screenshots in `noir`, `dusk`, `frost`, `toybox`; a first-time tester escapes on Normal within 3 attempts.
