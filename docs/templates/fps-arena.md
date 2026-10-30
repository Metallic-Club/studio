# Template: FPS Arena (first person, view models)

Status: planned. Milestone: M3 (needs M2 multiplayer for real matches). Default look: `foundry`. Players: 1 to 12.
Difficulty: intermediate. Conventions and API group ids: `docs/templates/README.md`.

Sample world: **Foundry**. Two teams (or free-for-all) fight in a three-lane arena. First team to 30 eliminations or the
higher score when the 8-minute timer ends wins.

## Why this template

The owner named "First Person Shooter with view models". Shooters sit near the top of both Roblox and Fortnite play
charts (research doc, section 2). The template also builds the weapon kit that Battle Royale reuses, and it is the
template that forces the hard engine pieces: first-person camera, mouse lock, raycasts, view-model rendering and
server-checked hits.

## Game loop

1. Spawn in your team's base. While fewer than 2 players are present, a **practice range** in each base has 5 target
   dummies (Humanoids that stand still and respawn), so solo play is never an empty lobby.
2. When a second player joins, a 10 s warm-up starts, then the match.
3. Fight. Each elimination scores 1 for the team (or the player in free-for-all). Health regenerates after a pause.
4. Eliminated players watch their eliminator for 3 s, then respawn at the spawn farthest from enemies, protected for
   2 s or until they fire.
5. Match ends at `ScoreToWin` or when time runs out. A 10 s end screen shows the winner and the top three players,
   then a new match starts on the same server.

## Weapons and feel

| Weapon | Slot | Damage (body / head) | Fire | Magazine | Reload | Body time-to-kill |
|---|---|---|---|---|---|---|
| Rifle | 1 | 18 / 27 | automatic, 600 per minute | 30 | 2.0 s | 6 shots, 0.50 s |
| Pistol | 2 | 25 / 37.5 | semi, max 300 per minute | 12 | 1.4 s | 4 shots, 0.60 s |

Damage is full to 70 studs and falls linearly to 50% at 200 studs. Spread: the first shot is exact; automatic fire adds
0.4 degrees per shot up to 3 degrees and recovers in 0.3 s. Recoil follows a fixed, learnable upward pattern kept as a
separate camera offset that returns to zero; it never rewrites the player's aim. Right mouse aims down sights: field of
view 80 to 60 over 0.15 s, spread halved, walk speed 0.7x.

Movement: WalkSpeed 16, run 24 (Shift, Metallic default), JumpPower 50. Mouse look reads raw mouse delta, pitch clamped
to plus or minus 89 degrees. Player options (not Game settings, because they belong to each player): sensitivity, field
of view 70 to 100, invert Y, camera shake on or off.

**View models** are built from Parts, so they need no imported art: blocky arms in the avatar's titanium and violet and
a blocky rifle and pistol. They live in `ReplicatedStorage.ViewModels`. The client sets `Camera.ViewModel` (A22) so
the engine draws them after the world in their own depth range; they never poke through walls. Motion is procedural:
sway lags behind mouse movement, bob follows walk speed, recoil kicks back 0.15 studs and returns, reload drops the
magazine with a tween. Other players see the weapon as a `Tool` handle in the character's hand.

## Level layout

Floor at y = 0, arena 180 x 110 studs, walls 1 stud thick and 14 high. About 450 Parts.

| Area | Position | Contents |
|---|---|---|
| Red base | x -90 to -70 | 3 SpawnLocations (TeamColor red), practice range of 5 dummies behind a glass wall, exits to all three lanes |
| Blue base | x 70 to 90 | Mirror of the red base |
| Mid lane | z -15 to 15 | Open courtyard. Central platform 16 x 16 at height 6 with two wedge ramps. Longest sightline about 120 studs |
| North lane | z 30 to 50 | Indoor hall, ceiling 12. Crates 4 x 4 x 4 every 16 studs (jumpable, since the jump apex is 6.4). Two windows into mid |
| South lane | z -50 to -30 | Outdoor catwalk 8 studs up with railings, ramps at both ends, two drop-down gaps |
| Connectors | x = -40 and x = 40 | Short halls between lanes, so no lane is a dead end |

The map is mirror-symmetric, so neither team has an advantage. Every lane has cover at most 12 studs apart.

## Instances

```
Workspace
  Map (Folder)
    RedBase, BlueBase, Mid, North, South, Connectors (Folder)   Parts tagged Look:Wall / Look:Ground / Look:Prop
    Cover (Folder)                                              crates, tagged Look:Prop
  Spawns (Folder)  SpawnLocation x6 (TeamColor set)
  Dummies (Folder) Dummy (Model with Humanoid) x10, tagged Dummy
Teams
  Red (Team), Blue (Team)
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  ViewModels (Folder): Arms, Rifle, Pistol (Model)
  Shared (Folder)
    Weapons (ModuleScript)        weapon stats; reads damage from GameSettings
    Ballistics (ModuleScript)     spread, falloff and the shared ray rules
ServerStorage
  Weapons (Folder): Rifle, Pistol (Tool with Handle)
ServerScriptService
  MatchService, WeaponService, SpawnService, HealthService, DummyService (Script)
  LagCompensation (ModuleScript)
StarterPlayer.StarterPlayerScripts
  WeaponController, ViewModelController, CameraController, HudController, OptionsController (LocalScript)
StarterGui
  Hud (ScreenGui): Crosshair, HitMarker, Health, Ammo, Killfeed, MatchBar (team scores and timer), DamageArrows
  Scoreboard (ScreenGui, hold Tab), EndScreen (ScreenGui), Options (ScreenGui)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `MatchService` | server | States Practice, WarmUp, Playing, Ended. Assigns teams (balanced on join), keeps scores, runs the timer, ends and restarts the match, publishes `MatchStateChanged`. |
| `WeaponService` | server | Gives each spawned player the Tools, tracks ammo and reloads, handles `Fire`: checks, then raycasts with `Ballistics` from the shooter's head, rewinding targets with `LagCompensation`, applies damage through `HealthService`. |
| `LagCompensation` | server module | Keeps 1 s of character positions at 20 snapshots per second; rewinds targets to the shot time, at most 200 ms back. |
| `SpawnService` | server | Chooses the team spawn farthest from living enemies, sets the protection attribute, respawns after `RespawnSeconds`. |
| `HealthService` | server | One path for damage: friendly fire rule, headshot rule, spawn protection, regeneration after `RegenDelay`, elimination credit, `Killfeed`. |
| `DummyService` | server | Spawns practice dummies while the match is in Practice; respawns a dummy 2 s after it falls. |
| `WeaponController` | client | Input (fire, aim, reload, slot keys), fire-rate timing, local ray for an instant hit marker guess, sends `Fire`. `-- TRY: set AUTO_RELOAD to true.` |
| `ViewModelController` | client | Clones the equipped view model, sets `Camera.ViewModel`, adds sway, bob, recoil and reload motion each `RenderStepped`. `-- TRY: change SWAY to 0 for a steady gun.` |
| `CameraController` | client | Locks first person (`CameraMode`), hides your own body, applies the recoil offset and the aim-down-sights field of view. |
| `HudController` | client | Health, ammo, crosshair spread, hit marker (white for body, violet for head), damage direction arrows, killfeed, team scores, timer, scoreboard. |
| `OptionsController` | client | Player options panel (Esc): sensitivity, field of view, invert Y, camera shake. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `Fire` | client to server | `{weapon, origin, direction, shotTime}` | weapon equipped and alive; time since last shot at least 0.9 x the weapon's interval; ammo above 0; origin within 4 studs of the server's head position; direction is a unit vector; `shotTime` within 250 ms of server time |
| `Reload` | client to server | none | not already reloading; magazine not full |
| `HitConfirm` | server to shooter | `{damage, headshot, eliminated}` | n/a |
| `Damaged` | server to victim | `{fromPosition, damage}` | n/a |
| `ShotEffect` | server to other clients (unreliable) | `{origin, hitPosition, weapon}` | n/a; tracers and impact puffs only |
| `Killfeed` | server to all | `{eliminator, eliminated, weapon, headshot}` | n/a |
| `MatchStateChanged` | server to all | `{state, timeLeft, scores}` | n/a; also sent to each late joiner |

The client's own hit marker guess is only a guess. The server's `HitConfirm` decides. If they disagree the marker
fades instead of flashing, so players never see a false kill.

## UI

Centre: crosshair whose gap shows current spread; hit marker. Bottom left: health bar and number. Bottom right: ammo
`24 / 30` and weapon name. Top centre: team scores and timer. Top right: killfeed, last 5 lines. Hold Tab: scoreboard
with eliminations, deaths and ping. End screen: winner and top three. Team colours always come with a team name and
icon, never colour alone.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `TeamMode` | `Teams` | Teams, FreeForAll | Teams or every player for themselves |
| `ScoreToWin` | 30 | 5 to 100 | Eliminations to win |
| `MatchMinutes` | 8 | 2 to 20 | Match timer |
| `RespawnSeconds` | 3 | 0 to 10 | Time before respawn |
| `SpawnProtection` | 2 | 0 to 5 | Seconds of protection after spawning |
| `MaxHealth` | 100 | 25 to 300 | Health per life |
| `RegenDelay` | 5 | 1 to 30 | Seconds without damage before regeneration |
| `RegenPerSecond` | 15 | 0 to 50 | Regeneration speed; 0 turns it off |
| `RifleDamage` | 18 | 5 to 60 | Rifle body damage |
| `PistolDamage` | 25 | 5 to 80 | Pistol body damage |
| `HeadshotMultiplier` | 1.5 | 1 to 3 | Head damage multiplier |
| `FriendlyFire` | false | bool | Teammates can hurt each other |
| `InfiniteAmmo` | false | bool | No reloads |
| `PracticeRange` | true | bool | Dummies while waiting for players |

## Multiplayer

Server-authoritative hits with lag compensation up to 200 ms. Teams balance on join; a player who leaves frees a
slot and the next joiner fills the smaller team. Late joiners get `MatchStateChanged` with the current state. At 12
players the server handles at most 12 x 10 `Fire` requests per second; each costs one raycast.

## Tutorial

1. "Press Play. Hit three dummies in the practice range." (`remote_fired` HitConfirm, 3 times)
2. "Pick a look. Try Dusk." (`look_applied`)
3. "In Game settings, set Match minutes to 3 and Headshot multiplier to 2." (`setting_changed` MatchMinutes)
4. "Select a crate in the North hall, press Ctrl+D, and drag the copy into mid as new cover." (`inserted` Part)
5. "Open Shared.Weapons. Change the rifle's MagazineSize from 30 to 50." (`script_edited` Weapons)
6. "Play again and empty a full magazine." (`play_started`)
7. "Name your arena and share it. From milestone 2, test it with a friend." (`renamed_game`)

## Style variants

`foundry` (default): concrete, metal, warm work lights. `dusk`: low sun and long shadows. `titanium`: clean graphite and
silver. `toybox`: a family-friendly paint-ball look; pair it with the "tag" variant below. Variants a creator can make
with settings only: **Tag** (`MaxHealth` 25, `RegenPerSecond` 0, pistol only), **One shot** (`RifleDamage` 60,
`HeadshotMultiplier` 2), **Free-for-all**. Swap-in art: replace `ViewModels.Rifle` and `ServerStorage.Weapons.Rifle`
with Models of the same names.

## New API this template needs

A1, A2, A3, A4, A5 (`Teams`, `ServerStorage`, `UserInputService`, `Debris`), A6 (CFrame everywhere), A7 (`Raycast`),
A8 (`leaderstats`), A9 (teams), A10 (`CameraMode`, `HealthChanged`, `LoadCharacter`, `RespawnTime`, `PlayerRemoving`),
A11 (Tools), A12, A13 (muzzle `PointLight`), A14 (shots, hits), A16 (`UnreliableRemoteEvent`, `GetServerTimeNow`),
A19 (`CurrentCamera`, `RenderStepped`, `FieldOfView`), A20 (mouse lock, `GetMouseDelta`), A21, **A22 (view models)**.
Also `BasePart.LocalTransparencyModifier` (Roblox name) to hide your own body in first person.

## Acceptance checks

- [ ] `tests/smoke.mscene`: two scripted bots on opposite teams; bot A fires 10 `Fire` requests at bot B's head from
      30 studs; B is eliminated once, A's team score is 1, killfeed fired once. A `Fire` with origin 20 studs from the
      head, a `Fire` 50 ms after the previous one, and a `Fire` with zero ammo are all refused with no damage. Print
      `FPS_SMOKE_OK`.
- [ ] The match ends at `ScoreToWin` and at the timer, then restarts cleanly three times in a row (no doubled
      handlers, Instance count stable).
- [ ] View model never clips into a wall when standing pressed against one (screenshot).
- [ ] Mouse look feels the same at 30, 60 and 144 fps (look uses raw delta; checked with a fixed input recording).
- [ ] 12 clients in a local server test at 60 fps on the reference PC; hits register at 100 ms simulated latency.
- [ ] Screenshots in `foundry`, `dusk`, `titanium`, `toybox`.
