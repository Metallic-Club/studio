# Template: Platformer (2D)

Status: planned. Milestone: M3 (first 2D template). Default look: `paper`. Players: 1 to 8. Difficulty: starter.
Conventions and API group ids (A1 ... A28): `docs/templates/README.md`.

Sample world: **Copper Hills**. Run right, jump gaps, stomp crawlers, collect gears, reach the flag. One level in three
zones, about 2 minutes for a first run.

## Why this template

It is the 2D flagship and the lowest floor in the catalog. Everything on screen is a coloured shape, so a creator can
redraw the whole level with the Part2D tool in minutes. It proves the 2D stack end to end: Box2D bodies, the 2D
character controller, Camera2D, sprites, one-way platforms and the 2D look presets.

## Game loop

1. Spawn at the left. The HUD shows hearts, gears and a timer.
2. Run and jump through Zone 1, Zone 2, Zone 3. Checkpoints save progress.
3. Hazards and crawlers cost one heart and knock you back. Falling into a pit or losing all hearts respawns you at the
   last checkpoint with full hearts.
4. Jump on a crawler from above to flatten it and bounce.
5. Touch the flag. A results panel shows time, gears and stars, with **Play again**.

Stars: 1 for finishing, 2 for collecting at least `CoinsForTwoStars` gears, 3 for also finding the 3 big gears.

## Feel numbers (defaults)

Studs, 2D gravity 60 studs/s², `Camera2D.PixelsPerStud` 40 (a 1920x1080 screen shows 48 x 27 studs).
Character root 1.2 x 2 studs. Walk 9 studs/s, run (hold Shift or gamepad X) 13.5. Jump height 4.2 studs, so the
take-off speed is about 22.4 studs/s and air time about 0.75 s. Horizontal reach: about 6.7 studs walking,
10 running. Coyote time 0.1 s, jump buffer 0.12 s, releasing jump early cuts upward speed by half.

Gap sizes follow from that: easy 3 studs, medium 5, hard 7 (needs a run-up). Walls taller than 4 studs need a
platform.

## Level layout

Ground top surface at y = 0. The level runs from x = 0 to x = 260.

| Zone | x range | What it teaches | Contents |
|---|---|---|---|
| 1 Meadow | 0 to 90 | Run, jump, collect, stomp | `SpawnLocation2D` at (4, 1). Ground 0-30, gap 3 (30-33), ground 33-55, steps 2 and 4 studs high at x 40 and 46, gap 4 (55-59) with a 5-gear arc over it, ground 59-90. One slow crawler patrolling x 70-85. 15 gears, 1 big gear on a ledge at (64, 6). `Checkpoint2D` (Order 1) at x 88 |
| 2 Gears | 90 to 180 | Timing and hazards | Pit x 100-112 crossed by a moving platform (4 x 1, travels 8 studs at 3 studs/s). Spike strip x 120-126 under two floating platforms at y 4. Two crawlers. Three one-way platforms. Bounce pad at x 150 launching 10 studs up to a gear cluster. 15 gears, 1 big gear. `Checkpoint2D` (Order 2) at x 178 |
| 3 Tower | 180 to 260 | Climbing and pressure | One-way platforms every 3.5 studs up to y 24, two crumbling platforms (fall 0.6 s after touch, return after 3 s), spikes at the bottom. Short run along y 24 to the `Goal` flag at (256, 24). 15 gears, 1 big gear behind a crumbling platform |

Totals: about 220 Part2D, 45 gears, 3 big gears, 3 crawlers, 2 checkpoints. `Camera2D.Bounds` from (-4, -12) to
(264, 40). Background: three Part2D layers with `ZIndex` -3 to -1 (hills, clouds) that scroll slower (parallax
attribute read by the camera).

## Instances

```
Workspace
  Level (Folder)
    Zone1, Zone2, Zone3 (Folder)          ground and platforms, Part2D tagged Look:Ground / Look:Prop
    Hazards (Folder)                      spikes, Part2D tagged Hazard + Look:Hazard
    MovingPlatforms (Folder)              Part2D tagged MovingPlatform; attributes Travel (Vector2), Speed
    CrumblePlatforms (Folder)             Part2D tagged Crumble; attributes Delay, ReturnAfter
    BouncePads (Folder)                   Part2D tagged Bouncer; attribute Power
    Background (Folder)                   Part2D with ZIndex < 0; attribute Parallax (0..1)
  Gears (Folder)                          Part2D Circle, CanCollide false, tagged Coin + Look:Pickup
  BigGears (Folder)                       same, tagged BigCoin
  Enemies (Folder)
    Crawler (Model)                       Part2D body; attributes PatrolMin, PatrolMax; tagged Enemy
  SpawnLocation2D
  Checkpoints (Folder)  Checkpoint2D x2
  Goal (Part2D)                           tagged Goal + Look:Goal
ReplicatedStorage
  GameSettings (Configuration)            attributes, see Game settings
  Remotes (Folder)                        RemoteEvents listed below
  Shared (Folder)
    Stars (ModuleScript)                  star rules, used by server and results UI
ServerScriptService
  CoinService, HazardService, EnemyService, PlatformService, GoalService (Script)
StarterPlayer.StarterPlayerScripts
  HudController, ResultsController, FeedbackController (LocalScript)
StarterGui
  Hud (ScreenGui): Hearts (Frame), GearCount (TextLabel), Timer (TextLabel)
  Results (ScreenGui, Enabled = false): Panel with Time, Gears, Stars, PlayAgain (TextButton)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `CoinService` | server | On a `Coin` or `BigCoin` touch by a character, if that player has not collected it this run, record it, add to `leaderstats.Gears`, fire `GearCollected` to that player only. `-- TRY: change 1 to 5 so each gear counts five.` |
| `HazardService` | server | On a `Hazard` touch: if the character is not invulnerable, take one heart, set attribute `InvulnerableUntil`, push the character away from the hazard, fire `PlayerHurt`. |
| `EnemyService` | server | Moves each `Enemy` between `PatrolMin` and `PatrolMax` at `EnemySpeed`. On touch with a character: if the character's root is above the enemy's centre and falling, flatten the enemy (hidden for 10 s), bounce the player with `StompBounce`; else treat as a hazard. `-- TRY: make crawlers hop by setting HOP_EVERY to 2.` |
| `PlatformService` | server | Moving platforms (ping-pong along `Travel`), crumbling platforms, bounce pads. Moving platforms move as kinematic movers so riders are carried. |
| `GoalService` | server | First touch of `Goal` per run: compute time from the player's run start, count gears, ask `Shared.Stars` for the star count, keep the best time in `leaderstats.Best`, fire `LevelComplete`. Handles `RequestRestart`. |
| `HudController` | client | Draws hearts, gear count and timer. Timer is cosmetic; the server's time is the one that counts. |
| `ResultsController` | client | Shows the results panel on `LevelComplete`; `PlayAgain` fires `RequestRestart`. |
| `FeedbackController` | client | Gear pop (scale up and fade with TweenService), squash on landing, short camera shake on hurt if `ScreenShake` is on, hides gears this player already collected. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `GearCollected` | server to one client | gear Instance | n/a |
| `PlayerHurt` | server to one client | hearts left | n/a |
| `LevelComplete` | server to one client | `{time, gears, bigGears, stars, best}` | n/a |
| `RequestRestart` | client to server | none | at most 1 per second per player; resets that player's collected set, timer and checkpoint, respawns at the spawn |

## UI

Top left: hearts (filled and empty shapes, not colour alone). Top right: gear icon and count, timer `1:42.3`.
Results panel: time, gears `38 / 45`, three star shapes, a **Play again** button focused for gamepad. All UI tagged
`Look:UI` so presets restyle it.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `JumpHeight` | 4.2 | 2 to 8 | Jump height in studs |
| `WalkSpeed` | 9 | 4 to 16 | Walk speed; run is 1.5x |
| `Gravity` | 60 | 30 to 120 | Lower is floatier |
| `CoyoteTime` | 0.1 | 0 to 0.25 | Grace time to jump after leaving a ledge |
| `DoubleJump` | false | bool | Allow one extra jump in the air |
| `Hearts` | 3 | 1 to 10 | Hits before respawning |
| `EnemySpeed` | 3 | 0 to 10 | Crawler patrol speed; 0 = standing still |
| `StompBounce` | 12 | 4 to 24 | Upward speed after a stomp |
| `CoinsForTwoStars` | 30 | 0 to 45 | Gears needed for the second star |
| `ShowOtherPlayers` | true | bool | Show other players as half-transparent runners |
| `ScreenShake` | true | bool | Camera shake on hurt |

## Multiplayer

Players never collide with each other (collision group). Gears are per player: each player sees and collects their
own set, so nobody steals progress. Crawlers are shared: a flattened crawler is gone for everyone for 10 s. A late
joiner starts at the spawn with a fresh run. The leaderboard shows `Gears` and `Best` time, so 8 friends can race
the same level.

## Tutorial

1. "Press Play. Reach the first flag post." (`touched` Checkpoint 1)
2. "Pick a look. Try Pixel." (`look_applied`)
3. "Open Game settings and turn on Double jump. Play and try it." (`setting_changed` DoubleJump, then `play_started`)
4. "Select a platform in Zone 1, press Ctrl+D, and drag the copy to make a new route." (`inserted` Part2D under Level)
5. "Open CoinService. Find the TRY line and make each gear worth five." (`script_edited` CoinService)
6. "Play to the flag and see your stars." (`remote_fired` LevelComplete)
7. "Name your game and share it." (`renamed_game`, then `published` from M2)

## Style variants

`paper` (default): flat shapes with ink outlines on off-white. `pixel`: 16-colour palette, pixel snapping, chunky
outlines. `frost`: ice platforms (lower friction is a Game setting, not part of the look). `toybox`: bright plastic
colours. Swap-in art: replace `Enemies.Crawler` with any Model of the same name; set `Image` on Part2D to use sprites.

## New API this template needs

A1 (`Shared.Stars`), A2, A3, A4 (`GetPropertyChangedSignal`, `Destroying`), A5 (`TweenService`,
`UserInputService`), A8 (`leaderstats`), A10 (`LoadCharacter`, `RespawnTime`, `JumpHeight`, `HealthChanged`),
A12, A18 (`CollisionGroup` for players), A20 (run key), A21 (`TextButton`,
`ImageLabel`), A23 (all of 2D), A14 (jump, gear and stomp sounds; the game is playable silent until A14 lands).

## Acceptance checks

- [ ] `tests/smoke.mscene` (headless, 1500 frames): teleport to Checkpoint 2, touch spikes `Hearts` times, assert
      respawn within 2 studs of Checkpoint 2; touch one gear twice, assert `Gears` = 1; touch the goal twice, assert
      one `LevelComplete`; fire `RequestRestart` 10 times in one second, assert one restart; print
      `PLATFORMER_SMOKE_OK`.
- [ ] With `EnemySpeed` = 0 and = 10, and `JumpHeight` = 2 and = 8, the smoke test still passes and every gap in
      Zones 1 and 2 stays clearable (a scripted bot run with fixed inputs reaches Checkpoint 2).
- [ ] A first-time tester finishes the level in under 4 minutes on the first try.
- [ ] 8 players in a local test: each sees their own gears; leaderboard correct; 60 fps on the reference PC; 30 fps
      on integrated graphics measured or "not verified".
- [ ] Screenshots in `paper`, `pixel`, `frost` and `toybox`; hazards and goal readable in each.
