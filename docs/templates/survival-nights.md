# Template: Survival Nights (co-op)

Status: planned. Milestone: M4 template wave. Default look: `meadow` by day; the night is part of the game, not a
preset. Players: 1 to 6 co-op. Difficulty: intermediate. Conventions and API group ids: `docs/templates/README.md`.

Sample world: **Campfire Nights**. A group is stranded in a forest. Gather wood and food by day, keep the campfire
burning, craft tools and walls, and hold out through five nights while shadow creatures close in on anyone outside
the firelight.

## Why this template

Co-op survival with a day and night cycle was one of Roblox's three breakout genres of 2025 and is still in the top
charts in October 2026 (research doc, section 2). It combines what the other templates teach one at a time: gathering
(Simulator), building (Build Sandbox's placement module), NPCs and reviving (Horror), round end states (Party Rounds).
It also exercises the lighting engine with a moving sun, a dark night and many small lights.

## Game loop

1. Spawn by the campfire at dawn. Everyone starts with a stone axe.
2. **Day** (`DayLength`, 240 s): chop trees for logs, pick berries, search scrap piles. Feed logs to the fire. Craft
   at the workbench.
3. **Night** (`NightLength`, 150 s): the sky goes dark. Shades spawn at the forest edge and from the cave. They
   avoid light and hunt players outside it. From night `BruteFromNight`, a Brute Shade walks straight for the fire and
   knocks fuel out of it.
4. Hunger drains all the time. At 0 hunger you lose health.
5. A player at 0 health is downed; teammates revive them (same rules as Horror). Solo: one revive per night by
   crawling back into the firelight.
6. **Lose** if the fire goes out at night or everyone is downed. **Win** at dawn after night `NightsToSurvive`.

## Numbers (defaults)

Fire fuel 0 to 100. It burns 1 per 3 s by day and 1 per 2 s by night; a log adds `LogFuel` (10). Light radius =
10 + 0.4 x fuel studs (up to 50). Shades will not enter a light radius; inside one they flee.

| Thing | Value |
|---|---|
| Player | Health 100, WalkSpeed 16, hunger 100, -1 every 4 s |
| Tree | 5 axe hits, gives 3 logs, regrows at the next dawn |
| Berry bush | 3 berries, +15 hunger each, regrows at dawn |
| Scrap pile | 2 to 4 scrap, sometimes a ration (+40 hunger) |
| Shade | Health 60, speed 14, 15 damage per second of contact; count per night 3 + 2 x night x sqrt(players) |
| Brute Shade | Health 300, speed 9, ignores players, -20 fuel per hit on the fire |

| Recipe (at the workbench) | Cost | Does |
|---|---|---|
| Iron axe | 3 scrap | chops in 3 hits |
| Spear | 2 logs, 2 scrap | 25 damage melee, reach 7 studs |
| Lantern | 4 scrap | your own light, radius 12, for 120 s per scrap refill |
| Wall | 3 logs | a 4 x 6 x 1 barricade on the 4-stud grid, 150 health |
| Snare | 2 logs, 2 scrap | stops the first Shade that touches it for 4 s |

## Level layout

Island 400 x 400 studs, the camp clearing in the middle. About 1,600 Parts.

| Area | Where | Contents |
|---|---|---|
| Camp | centre, radius 30 | Campfire (Look:Light, a `PointLight` whose `Range` follows fuel), workbench, log pile, SpawnLocation |
| Forest | rings around camp | 120 trees (Cylinder trunk 2 x 14 plus Ball crowns), 30 berry bushes |
| River | crosses the west half | a log bridge and a ford |
| Old cabin | (120, 0, -90) | scrap piles, a ration crate |
| Cave | (-140, 0, 130) | the richest scrap, and the night spawn point closest to camp |

Night spawn points: 10 invisible Parts at the forest edge and 2 at the cave mouth.

## Instances

```
Workspace
  Island (Folder): Ground, River, Cabin, Cave (Folder)    Look role tags
  Camp (Folder): Campfire (Model with PointLight and a prompt "Add log"), Workbench (Model with prompt), SpawnLocation
  Resources (Folder): Trees, Bushes, Scrap (Folder; Models tagged Tree, Bush, ScrapPile)
  NightSpawns (Folder)            invisible Parts
  Structures (Folder)             player-built walls and snares
  Creatures (Folder)              live Shades
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  Shared (Folder): Recipes, Grid (ModuleScript; Grid is the Build Sandbox module)
ServerStorage
  CreatureModels (Folder): Shade, BruteShade (Model with Humanoid)
  StructureModels (Folder): Wall, Snare (Model)
StarterPack
  StoneAxe (Tool)
ServerScriptService
  DayNightService, FireService, ResourceService, InventoryService, CraftService, CreatureService, CombatService,
  DownedService, StructureService (Script)
StarterPlayer.StarterPlayerScripts
  ToolController, PlacementController, HudController, CraftMenuController, AmbienceController (LocalScript)
StarterGui
  Hud (ScreenGui): Night counter and clock, Fire meter, Health, Hunger, Inventory
  CraftMenu (ScreenGui)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `DayNightService` | server | Moves `Lighting.ClockTime` through day and night, fires `PhaseChanged`, counts nights, win at the last dawn. `-- TRY: change DUSK_SECONDS to 30 for a slower sunset.` |
| `FireService` | server | Fuel, burn rate, light radius (sets the `PointLight.Range` and an attribute others read), "Add log" prompt, lose when out at night. |
| `ResourceService` | server | Chop, pick and search on server-checked hits; regrowth at dawn. |
| `InventoryService` | server | Logs, berries, scrap, rations, crafted items; eating. |
| `CraftService` | server | Recipes from `Shared.Recipes`; must stand at the workbench. |
| `CreatureService` | server | Spawns Shades each night, steers them (toward the nearest player outside any light radius, away from light, around walls with a short side-step probe), Brutes walk to the fire. `-- TRY: set FLEE_MARGIN to 0 so Shades stalk right at the light's edge.` |
| `CombatService` | server | Spear hits (cooldown, reach, target alive), Shade damage to players and walls. |
| `DownedService` | server | Shared with Horror: downed, bleed-out, revive prompt; the solo crawl-back rule. |
| `StructureService` | server | Places walls and snares on the grid near camp, health, removal. |
| `ToolController` | client | Swing for axe and spear, target under the mouse or in front. |
| `PlacementController` | client | Ghost wall or snare on the grid, using `Shared.Grid`. |
| `HudController` | client | Night `2 / 5` and a clock, fire meter (shape and number), health and hunger, inventory. |
| `CraftMenuController` | client | Recipes with costs; disabled entries say what is missing. |
| `AmbienceController` | client | Day birds, night wind, a warning sound when a Brute comes. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `Swing` | client to server | `{target}` (tree, bush, scrap pile, creature or nil) | alive, holding a tool; at least the tool's cooldown since the last swing; target within reach; target alive or not depleted |
| `Craft` | client to server | `{recipe}` | recipe exists; standing within 8 studs of the workbench; has the materials |
| `PlaceStructure` | client to server | `{kind, position, rotation}` | has the item; within 60 studs of the fire; on the grid; no overlap; under 40 structures per player; at most 4 per second |
| `Eat` | client to server | `{item}` | has it; hunger below 100 |
| `PhaseChanged` | server to all | `{phase, night, endsAt}` | n/a |
| `FireChanged` | server to all | `{fuel, radius}` | n/a; at most 4 per second |
| `Notify` | server to one client | `{text}` | n/a |

Adding logs and reviving use `ProximityPrompt.Triggered`; the server checks distance and that the player has a log.

## UI

Top centre: night counter and a small sun or moon clock. Top left: fire meter (a flame shape that shrinks, plus a
number). Bottom left: health and hunger. Bottom centre: inventory row. All text at least 18 px, `Look:UI` theme.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `NightsToSurvive` | 5 | 1 to 20 | Nights to win |
| `DayLength` | 240 | 60 to 900 | Seconds of daylight |
| `NightLength` | 150 | 60 to 600 | Seconds of night |
| `FireBurnRate` | 1.0 | 0.25 to 4 | How fast the fire burns |
| `LogFuel` | 10 | 2 to 50 | Fuel per log |
| `HungerRate` | 1.0 | 0 to 4 | Hunger drain; 0 turns hunger off |
| `CreatureCount` | 1.0 | 0 to 4 | Shades per night multiplier |
| `CreatureDamage` | 1.0 | 0.25 to 4 | Shade damage multiplier |
| `BruteFromNight` | 3 | 1 to 21 | First night with a Brute; 21 = never |
| `ResourceRegrow` | true | bool | Trees and bushes regrow at dawn |
| `SoloRevivesPerNight` | 1 | 0 to 3 | Crawl-back revives when alone |

## Multiplayer

All resources, the fire and creatures are shared; inventories are per player. Creature AI runs on the server and
updates its targets 5 times a second, not every frame. With 6 players on night 5 there are about 3 + 2 x 5 x 2.45 =
28 Shades; the server must hold 30 NPC Humanoids at 60 Hz. Late joiners spawn at camp with a stone axe at the current
time of day.

## Tutorial

1. "Press Play. Chop a tree and put a log on the fire." (`remote_fired` FireChanged after a log)
2. "Survive your first night." (`remote_fired` PhaseChanged to day 2)
3. "In Game settings, set Nights to survive to 3 and Creature count to 0.5." (`setting_changed` NightsToSurvive)
4. "Add a tree: duplicate a Model in Resources.Trees and move it near camp." (`inserted` under Resources.Trees)
5. "Open Shared.Recipes. Make walls cost 2 logs instead of 3." (`script_edited` Recipes)
6. "Play through a full day and night." (`remote_fired` PhaseChanged)
7. "Name your survival game and share it." (`renamed_game`)

## Style variants

`meadow` (default): summer forest. `frost`: snowy forest; pair with `HungerRate` 1.5 for a cold-survival feel. `dusk`:
autumn. `toybox` with `CreatureDamage` 0.25: a gentle "campout" for younger players. Swap-in art: replace
`CreatureModels.Shade` with any Model of the same name that has a Humanoid.

## New API this template needs

A1, A2, A3, A4, A5 (`StarterPack`, `ServerStorage`, `TweenService`), A6, A7 (`Raycast` for side-step probes,
`GetPartBoundsInBox` for placement), A10 (`MoveTo`, `HealthChanged`), **A11 (Tools)**, A12, **A13 (`ClockTime` day and
night, `PointLight` with changing `Range`)**, A14, A15, A19, A20, A21, A24. Engine work: a sun that moves with
`ClockTime` and keeps shadows stable, and 30 NPC Humanoids at 60 fps.

## Acceptance checks

- [ ] `tests/smoke.mscene` with 2 bots, `DayLength` 60, `NightLength` 60, `NightsToSurvive` 1: A chops a tree (3 logs),
      adds 2 logs (fuel +20), crafts a wall at the workbench and places it; B tries to craft away from the workbench
      (refused); at night Shades spawn and never enter the fire's light radius during 60 s (checked every frame); B
      walks into the dark and is downed; A revives B; dawn fires the win once. Print `SURVIVAL_SMOKE_OK`.
- [ ] With `FireBurnRate` 4 and no logs, the fire goes out on night 1 and the loss fires once.
- [ ] `Swing` 30 times in a second chops at most the tool's rate.
- [ ] 6 players, night 5, 28 Shades: 60 fps on the reference PC.
- [ ] Screenshots by day and night in `meadow`, `frost`, `dusk`, `toybox`.
