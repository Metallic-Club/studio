# Template: Collect and Upgrade Simulator

Status: planned. Milestone: M4 template wave (saving needs the platform data store from M2). Default look: `meadow`.
Players: 1 to 12. Difficulty: starter. Conventions and API group ids: `docs/templates/README.md`.

Sample world: **Shard Valley**. Tap crystals to break them into shards, fill your bag, sell, buy a better pick and a
bigger bag, open the gate to the next zone. Hatch helpers that float beside you and boost your haul. Rebirth for a
permanent multiplier.

## Why this template

Simulators (collect, sell, upgrade, unlock) and pet collection are long-running Roblox staples, and fishing and
gardening games, which use the same loop, are in the current top charts (research doc, section 2). The loop is easy
to understand, has instant feedback on every click, and teaches number tuning: a creator learns more about game
balance from this template's settings than from any other. The core (nodes, bag, sell, upgrade, zones) is also the
base for the Fishing and Garden catalog entries.

## Game loop

1. Spawn in the Meadow. Click a crystal (or hold to keep swinging). Each hit takes `ToolPower` from the crystal's
   health. A broken crystal drops shards into your bag and respawns after `NodeRespawnSeconds`.
2. When the bag is full, a prompt says "Bag full: sell at the market". Step on the sell pad: shards become coins.
3. Shop: picks (more power), bags (more capacity). Prices rise by tier.
4. Zone gates cost coins. Each new zone has crystals worth more and with more health.
5. Eggs (bought with coins) hatch a random helper. Equip up to `MaxHelpersEquipped`. Helpers float near you, multiply
   shards and tap the nearest crystal every 1.5 s.
6. Rebirth at `RebirthCost` coins: reset coins, tools, bags and zones; keep helpers; gain a permanent multiplier.
7. Progress saves.

## Numbers (defaults)

| Zone | Gate price | Crystal sizes (health, shards) | Zone multiplier |
|---|---|---|---|
| Meadow | free | small (10, 1), medium (40, 5), large (150, 25) | x1 |
| Canyon | 1,000 | same sizes | x8 |
| Glacier | 25,000 | same sizes | x60 |
| Crystal Cave | 500,000 | same sizes | x400 |

Crystal health and shards both scale with the zone multiplier. 30 crystals per zone (18 small, 9 medium, 3 large).

| Tier | Pick power | Pick price | Bag capacity | Bag price |
|---|---|---|---|---|
| 1 | 1 | free | 25 | free |
| 2 | 3 | 100 | 75 | 150 |
| 3 | 8 | 600 | 200 | 900 |
| 4 | 20 | 3,000 | 600 | 4,500 |
| 5 | 50 | 15,000 | 2,000 | 22,000 |
| 6 | 120 | 80,000 | 6,000 | 110,000 |
| 7 | 300 | 400,000 | 20,000 | 550,000 |
| 8 | 800 | 2,000,000 | 75,000 | 2,500,000 |

Swing rate: 3 per second. One shard sells for 1 coin times multipliers. Helpers: 4 eggs (one per zone, 250 / 5,000 /
100,000 / 2,000,000), each with 4 helper designs in rarities Common (x1.1, 60%), Rare (x1.3, 28%), Epic (x1.7, 10%),
Legendary (x3.0, 2%). Equipped helper multipliers add together.

Eggs cost soft currency only. Same platform note as the Tycoon: no real-money random items in the template.

## Level layout

Four zones in a line from south to north, each 120 x 120 at y 0, joined by 16-stud gate passages. Hub at the Meadow's
south edge. About 1,100 Parts.

| Area | Contents |
|---|---|
| Hub | SpawnLocation, sell pad (Goal role, 12 x 1 x 12), shop stall, egg stall, rebirth statue, top-10 board |
| Meadow | Grass, trees (Cylinder and Ball), 30 crystal spots, a small pond |
| Canyon | Sand and Concrete walls, a bridge, 30 crystal spots |
| Glacier | Ice and Glass, low fog, 30 crystal spots |
| Crystal Cave | Dark, lit by the crystals' own `PointLight`s, 30 crystal spots |

Gates are Parts with a `ProximityPrompt` ("Open for 1,000") and a `SurfaceGui` price label; an opened gate stays open
for that player only (it turns transparent and non-colliding on that client; the server allows passage by zone
ownership and moves anyone who skips a gate back to the hub).

## Instances

```
Workspace
  Hub (Folder), Zones (Folder): Meadow, Canyon, Glacier, Cave (Folder with Terrain parts and a Spots Folder)
  Gates (Folder)                Gate Parts, attribute Zone and Price
  Crystals (Folder)             live crystals, created by the server at spots
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  CrystalModels (Folder)        Small, Medium, Large (Model), recoloured per zone by attribute
  HelperModels (Folder)         16 helper Models
  Shared (Folder): Tiers, Zones, Eggs, Format (ModuleScript)
StarterPack
  Pick (Tool)
ServerScriptService
  CrystalService, BagService, ShopService, ZoneService, HelperService, SaveService, RebirthService (Script)
StarterPlayer.StarterPlayerScripts
  PickController, HelperController, HudController, ShopController, FloatingTextController (LocalScript)
StarterGui
  Hud (ScreenGui): Coins, Bag bar, Multiplier, Sell hint
  Shop (ScreenGui), Eggs (ScreenGui), Helpers (ScreenGui)
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `CrystalService` | server | Spawns crystals at spots, handles `HitCrystal`, applies damage, splits shards between hitters by damage dealt, respawns. `-- TRY: set LARGE_CHANCE to 0.5 for more big crystals.` |
| `BagService` | server | Bag contents and capacity, sell pad touch, sell multiplier. |
| `ShopService` | server | Picks, bags, eggs; every purchase through one `TrySpend`. |
| `ZoneService` | server | Gate purchases, which zones a player may enter, moving trespassers back. |
| `HelperService` | server | Hatching (weighted random), inventory, equip, auto-taps (server-side timers), multipliers. |
| `SaveService` | server | Load on join, save every 60 s, on leave and on shutdown. |
| `RebirthService` | server | Cost check, reset, multiplier. |
| `PickController` | client | Click or hold to swing; picks the crystal under the mouse or the nearest one in front within 10 studs; swing tween; sends `HitCrystal`. `-- TRY: change SWING_TILT to 60 for a bigger swing.` |
| `HelperController` | client | Floats equipped helpers around the player with a gentle bob (TweenService and Heartbeat). |
| `HudController` | client | Coins, bag bar with "full" state, multiplier. |
| `ShopController` | client | Shop, eggs and helpers panels. |
| `FloatingTextController` | client | "+25" pops at the crystal. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `HitCrystal` | client to server | crystal Instance | crystal alive; within 12 studs of the player's root; at least 0.3 s since the last hit (swing rate 3/s with 10% slack); bag not full; player allowed in that zone |
| `BuyItem` | client to server | `{kind = "Pick" | "Bag", tier}` | tier is the next tier; `TrySpend` |
| `HatchEgg` | client to server (RemoteFunction) | `{egg}` | egg zone unlocked; `TrySpend`; helper inventory below 50; returns the helper |
| `EquipHelper` | client to server | `{helperId, equip}` | owns it; equip count below `MaxHelpersEquipped` |
| `Rebirth` | client to server (RemoteFunction) | none | coins at least `RebirthCost` |
| `ShardsGained` | server to one client | `{amount, position}` | n/a |
| `StatsChanged` | server to one client | `{coins, bag, capacity, multiplier}` | n/a |

## UI

Top centre: coins. Bottom centre: bag bar `140 / 200` that pulses when full. Left: Shop, Eggs, Helpers buttons. Egg
hatch: a short reveal (egg wobbles, cracks, helper appears with its rarity name). Rarity shown as name and frame shape.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `NodeValueScale` | 1.0 | 0.25 to 10 | Shards per crystal |
| `NodeRespawnSeconds` | 10 | 2 to 60 | Crystal respawn time |
| `ToolPowerScale` | 1.0 | 0.25 to 10 | Pick power multiplier |
| `BagStart` | 25 | 5 to 500 | Starting bag size |
| `SellMultiplier` | 1.0 | 0.25 to 10 | Coins per shard |
| `PriceScale` | 1.0 | 0.25 to 10 | All shop and gate prices |
| `HelpersEnabled` | true | bool | Eggs and helpers on or off |
| `MaxHelpersEquipped` | 3 | 1 to 8 | Helpers at once |
| `EggLuck` | 1.0 | 0.5 to 5 | Raises rare helper chances |
| `RebirthCost` | 1,000,000 | 10,000 to 100,000,000 | Coins to rebirth |
| `RebirthBonus` | 0.5 | 0.1 to 2 | Multiplier added per rebirth |
| `AutoSwing` | false | bool | Swing automatically at the nearest crystal |
| `SaveEnabled` | true | bool | Save progress |

## Multiplayer

Crystals are shared. When several players hit one crystal, shards are split by damage dealt, so helping is never
worse than mining alone. Zones are per player. Everything that affects coins runs on the server. Late joiners load
their save and see crystals as they are.

## Tutorial

1. "Press Play. Break crystals until your bag is full, then sell." (`remote_fired` StatsChanged after a sell)
2. "Pick the Frost look." (`look_applied`)
3. "In Game settings, set Sell multiplier to 3. Play: progress is faster." (`setting_changed` SellMultiplier)
4. "Add crystal spots: duplicate a Part in Zones.Meadow.Spots and move it." (`inserted` under Zones.Meadow.Spots)
5. "Open Shared.Tiers and add a tier 9 pick, copying the tier 8 line." (`script_edited` Tiers)
6. "Play, buy your new pick." (`remote_fired` BuyItem)
7. "Name your simulator and share it." (`renamed_game`)

## Style variants

`meadow` (default). `frost` for an ice-mining game. `toybox` for a candy theme (swap crystal Models for sweets). `noir`
with the Cave lights for a mine at night. The same core with different art and words becomes the Fishing (spots
become ponds, picks become rods) or Garden (spots become plots that grow over time) catalog entries.

## New API this template needs

A1, A2, A3, A4, A5 (`StarterPack`, `TweenService`), A6, A8, A10 (`PlayerRemoving`), **A11 (`Tool.Activated`)**, A12,
A13 (`PointLight`), A14, A15 (gate prompts), A16 (`RemoteFunction`), **A17 (data store)**, A19 (`ViewportPointToRay`
for mouse picking), A20, A21 (`BillboardGui`, `SurfaceGui`, `TextButton`, `ScrollingFrame`, `UIGridLayout`), A24.

## Acceptance checks

- [ ] `tests/smoke.mscene` with 2 bots: both hit one medium crystal; shards split by damage; bot A fills its bag and
      further hits are refused; A sells (coins rise, bag empties); A buys pick tier 2 (power 3); A tries tier 4 directly
      (refused); A hits from 30 studs (refused) and 10 times in 0.5 s (at most 2 accepted); A hatches an egg; save and
      reload match. Print `SIMULATOR_SMOKE_OK`.
- [ ] A bot that walks past a locked gate is moved back to the hub.
- [ ] Hatch odds over 100,000 simulated hatches stay within 1% of the table.
- [ ] 12 players with 3 helpers each: 60 fps on the reference PC.
- [ ] Screenshots in `meadow`, `frost`, `toybox`, `noir`.
