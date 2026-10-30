# Template: Battle Royale

Status: planned. Milestone: M3, after FPS Arena (reuses its weapon kit; needs M2 multiplayer). Default look: `dusk`.
Players: 1 to 16 (24 once M2 networking proves it). Difficulty: advanced. Conventions and API group ids:
`docs/templates/README.md`.

Sample world: **Ring Island**. Drop from a carrier, loot chests, outlast a shrinking storm and the other players. Last
one standing wins. Matches take 8 to 12 minutes.

## Why this template

The owner named Battle Royale. Battle royale-style islands made up 5 of Fortnite's top 20 on 2026-10-29
(research doc, section 2). It is the largest template: a match state machine, a big map, loot tables, a storm, a
drop, spectating. It is advanced on purpose. A creator who has finished the FPS Arena tutorial meets the same weapon
code here, so the new ideas are only the match flow, loot and storm.

## Game loop

1. **Lobby** (a floating platform above the sea, in sight of the island): free weapons and targets for warm-up. The
   match starts `LobbyCountdown` seconds after `MinPlayers` are present. With one player, a **solo run** starts
   instead: loot and survive the storm to the end, scored by time and loot (so solo is never a dead lobby).
2. **Drop:** a carrier platform crosses the island on a random line through the middle third, 400 studs up, at 60
   studs/s. Press Space to jump. You glide: fall speed capped at `GlideFallSpeed` (35), steer at 40 studs/s. At the end
   of the line everyone left is ejected.
3. **Loot:** chests (hold the prompt 0.5 s) and floor items. Three weapon slots and two consumable stacks.
4. **Storm:** five phases. Outside the safe circle you take damage each second. The HUD shows the next circle and a
   timer.
5. **Elimination:** you drop your loot in a crate, see your placement and spectate. "Back to lobby" is always one
   click.
6. **Win:** last player alive. A banner and 10 s later everyone returns to the lobby.

## Items

| Item | Rarity | Use |
|---|---|---|
| Pistol | Common | 25 damage, semi-automatic, 12 rounds (from the FPS kit) |
| Rifle | Uncommon | 18 damage, automatic 600 per minute, 30 rounds (FPS kit) |
| Scattergun | Rare | 8 pellets of 9 damage, 1 shot per 0.9 s, falls off fast after 25 studs |
| Marksman | Epic | 70 damage, 1 shot per second, scope (field of view 30) |
| Bandage | Common | +25 health up to 75, takes 3 s, cancelled by sprinting or firing |
| Shield cell | Uncommon | +50 shield up to 100, takes 4 s |

Rarity is shown by name, border shape and colour, never colour alone. Health 100 plus shield 0 to 100. The camera is
third person over the shoulder; aiming moves it closer. `FirstPerson` = true switches to the FPS Arena camera and view
models.

## Storm phases (defaults)

| Phase | Wait (s) | Shrink (s) | End radius (studs) | Damage per second |
|---|---|---|---|---|
| 1 | 60 | 45 | 220 | 1 |
| 2 | 45 | 40 | 120 | 2 |
| 3 | 35 | 35 | 60 | 5 |
| 4 | 30 | 30 | 25 | 8 |
| 5 | 20 | 30 | 0 | 12 |

Start radius 450 covers the island. Each new centre is chosen at random so the new circle fits inside the old one.
The storm wall is one tall transparent Cylinder that each client sizes every frame from the phase data and server
time, so the server sends 1 message per phase, not 60 per second. `StormSpeed` scales wait and shrink times;
`StormDamageScale` scales damage.

## Level layout

Island 640 x 640 studs, sea at y 0, land mostly at y 2 to 40. About 3,000 Parts. Fog in `dusk` starts at 300 studs.

| Place | Where (x, z) | What |
|---|---|---|
| Old Town | (0, 200) | 8 two-storey houses (stairs, windows), 18 loot spots |
| Harbour | (0, -250) | Docks, boat sheds, cranes made of Parts, 10 loot spots |
| Watchtower | (0, 0) | Hill 40 high with a tower (best view, little cover), 6 loot spots, Epic chest at the top |
| Quarry | (220, 0) | Stepped pit 30 deep, ramps, 10 loot spots |
| Farmstead | (-220, 0) | Barn, silo, fields with hay bales (cover), 10 loot spots |
| Ruins | (180, 200) | Broken walls, arches, 8 loot spots |

Between places: hills built from large Wedges and Blocks, rocks, tree clusters (Cylinder trunk, Ball crown), so no
open field is wider than 80 studs without cover. Lobby platform: 80 x 80 at y 500 over the sea at (0, -450).
`LootSpot` Parts: 62. `LootDensity` (0.7) is the share filled each match.

## Instances

```
Workspace
  Island (Folder): OldTown, Harbour, Watchtower, Quarry, Farmstead, Ruins, Wilds (Folder)    Look role tags
  LootSpots (Folder)       invisible Parts, attribute Tier = Floor | Chest | EpicChest
  Lobby (Model)            platform, SpawnLocation, practice targets
  Carrier (Model)          the drop platform, anchored, moved by the server
  StormWall (Part)         Cylinder, CanCollide false, Transparency 0.6, tagged Look:Hazard
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  Items (Folder)           Tools and pickup Models (Pistol, Rifle, Scattergun, Marksman, Bandage, ShieldCell)
  ViewModels (Folder)      from the FPS kit, used when FirstPerson is on
  Shared (Folder)
    Weapons, Ballistics    (ModuleScript, the FPS Arena kit, unchanged)
    LootTable              (ModuleScript, rarity weights per tier)
    StormPhases            (ModuleScript, the table above, scaled by settings)
  MapImage                 (ImageLabel source: top-down Studio capture for the minimap)
ServerScriptService
  MatchService, DropService, LootService, StormService, InventoryService, EliminationService,
  WeaponService, HealthService (Script; the last two from the FPS kit, plus shields)
  LagCompensation (ModuleScript, FPS kit)
StarterPlayer.StarterPlayerScripts
  WeaponController, CameraController, GlideController, StormController, InventoryController, HudController,
  SpectateController (LocalScript)
StarterGui
  Hud (ScreenGui): AliveCount, StormTimer, Minimap, Health and Shield, Ammo, Killfeed, Inventory bar
  EndScreen (ScreenGui): placement, eliminations, Back to lobby (TextButton)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `MatchService` | server | Lobby, Countdown, Drop, Playing, Ended. Locks joining into the current match after Drop starts (late joiners wait in the lobby). Decides solo run or match. |
| `DropService` | server | Picks the carrier line, moves the carrier, puts players on it, handles `JumpFromCarrier`, ejects the rest, checks glide fall speed. |
| `LootService` | server | Fills loot spots by `LootDensity` and `Shared.LootTable` with `Random.new(seed)`; chest prompts; floor pickups; death crates. `-- TRY: give EpicChest a 50% chance of a Marksman.` |
| `StormService` | server | Runs phases from `Shared.StormPhases`, picks centres, damages players outside each second, sends `StormPhaseChanged`. |
| `InventoryService` | server | Slots, stacks, equip, consume (with interrupt rules), drop on death. |
| `EliminationService` | server | Placement, killfeed, spectate targets, win detection (also for the solo run). |
| `WeaponService`, `HealthService` | server | From FPS Arena, extended with shields. |
| `GlideController` | client | Jump input on the carrier, glide steering, landing; a simple back-mounted wing Model appears while gliding. |
| `StormController` | client | Sizes and places the storm wall each frame; tints the screen edge when outside; minimap circles. |
| `InventoryController` | client | Inventory bar, keys 1 to 3 and 4 to 5, scroll wheel, use and cancel. |
| `SpectateController` | client | Follows the eliminator, then the next living player; arrows to switch. |
| `HudController`, `WeaponController`, `CameraController` | client | HUD; FPS kit input; over-the-shoulder camera. |

## RemoteEvents

From the FPS kit, unchanged: `Fire`, `Reload`, `HitConfirm`, `Damaged`, `ShotEffect`, `Killfeed`. New:

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `JumpFromCarrier` | client to server | none | state is Drop; player is on the carrier; once per player |
| `UseItem` | client to server | `{slot}` | slot holds a consumable; not already using one; alive; health or shield below the item's cap |
| `CancelUse` | client to server | none | currently using |
| `DropItem` | client to server | `{slot}` | slot not empty; alive; at most 4 per second |
| `MatchStateChanged` | server to all | `{state, alive, countdown}` | n/a; sent to late joiners too |
| `StormPhaseChanged` | server to all | `{phase, center, radius, nextCenter, nextRadius, shrinkStart, shrinkEnd, damage}` | n/a |
| `Eliminated` | server to one client | `{by, placement, eliminations}` | n/a |
| `SpectateTarget` | client to server | `{player}` | requester is eliminated; target is alive |

Chests and pickups use `ProximityPrompt.Triggered`; the server checks distance, that the chest is still closed and
inventory space.

## UI

Top right: minimap (shipped top-down image, storm and safe circles, your arrow) with alive count and storm timer
under it. Bottom centre: health and shield bars, inventory bar with 5 slots. Bottom right: ammo. Killfeed top right
under the map. End screen: "#3 of 14", eliminations, Back to lobby.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `MinPlayers` | 2 | 2 to 16 | Players needed for a match; fewer starts a solo run |
| `MaxPlayers` | 16 | 2 to 24 | Server size |
| `LobbyCountdown` | 20 | 5 to 60 | Seconds from enough players to drop |
| `StormSpeed` | 1.0 | 0.5 to 3 | Higher is a faster match |
| `StormDamageScale` | 1.0 | 0.5 to 3 | Storm damage multiplier |
| `LootDensity` | 0.7 | 0.2 to 1 | Share of loot spots filled |
| `StartingWeapon` | None | None, Pistol, Rifle | What everyone lands with |
| `StartingShield` | 0 | 0 to 100 | Shield at landing |
| `GlideFallSpeed` | 35 | 15 to 80 | Glide descent speed |
| `FirstPerson` | false | bool | FPS camera and view models |
| `PracticeInLobby` | true | bool | Weapons and targets in the lobby |

## Multiplayer

Server-authoritative damage, loot and storm. Movement and gliding follow the engine's character model (client moves
its own character; the server checks speed and position each tick; milestone 2 decides the details). One match per
server; at most 16 x 10 `Fire` requests per second at full size. Players who leave mid-match drop their loot crate.

## Tutorial

1. "Press Play. Jump from the carrier, land and open a chest." (`remote_fired` UseItem or `printed` "CHEST_OPENED")
2. "Try the Frost look for a snow island." (`look_applied`)
3. "For quick matches set Storm speed to 2 and Starting weapon to Pistol." (`setting_changed` StormSpeed)
4. "Duplicate a house in Old Town, move it into an empty field, and copy a LootSpot inside it." (`inserted` under
   LootSpots)
5. "Open Shared.LootTable. Follow the TRY line to make epic chests richer." (`script_edited` LootTable)
6. "Play a solo run to the last circle." (`remote_fired` Eliminated or MatchStateChanged Ended)
7. "Name your island and share it." (`renamed_game`)

## Style variants

`dusk` (default): warm low sun, long shadows, fog at 300. `frost`: snow island. `meadow`: summer island. `toybox`
with `StartingWeapon` Pistol and `MaxHealth`-style settings low: a family paint-ball royale. Settings-only variants:
**Quick royale** (StormSpeed 2, LootDensity 1), **Snipers only** (edit `LootTable` to Marksman only).

## New API this template needs

Everything FPS Arena needs, plus: A7 (`GetPartBoundsInRadius` for loot crates), A10 (`CameraSubject` for spectating,
`LoadCharacter`), A15 (`ProximityPrompt` with `HoldDuration`), A16 (`GetServerTimeNow` for the storm), A18
(`AssemblyLinearVelocity` for the glide cap), A19, A21 (`ImageLabel` minimap, `UIListLayout`), A24 (`Random.new`).
Engine work: a draw distance of 640 studs with fog, 16 to 24 networked players (M2), and character movement checks.

## Acceptance checks

- [ ] `tests/smoke.mscene` with 3 bots and `StormSpeed` 3: match starts after the countdown; all bots jump; bot A loots
      a chest (item in inventory), bot B tries to open the same chest (refused); bot A eliminates bot B (loot crate
      appears, `Eliminated` placement 3); bot C stays outside the storm and is eliminated by it; A wins; everyone is
      back in the lobby within 15 s. Print `BR_SMOKE_OK`.
- [ ] A solo run with one bot reaches phase 5 and ends with a result screen.
- [ ] `UseItem` while already using, `JumpFromCarrier` twice, and `DropItem` 20 times in a second are refused or
      limited as listed.
- [ ] Storm wall position on two clients differs by less than 0.5 studs at the same server time.
- [ ] 16 clients in a local server test: 60 fps on the reference PC during the drop (the busiest moment).
- [ ] Screenshots in `dusk`, `frost`, `meadow`, `toybox`.
