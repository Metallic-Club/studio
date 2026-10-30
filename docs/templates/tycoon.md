# Template: Tycoon (collect, earn, protect)

Status: planned. Milestone: M3 (saving needs the platform data store from M2; Studio emulates it before then).
Default look: `toybox`. Players: 1 to 8. Difficulty: starter. Conventions and API group ids:
`docs/templates/README.md`.

Sample world: **Gizmo Yard**. A market belt carries little robot Gizmos past every base. Buy one, set it on a pedestal,
and it earns cash every second. Spend cash on more pedestals and better Gizmos. Turn on raids and other players can
carry your Gizmos away unless you lock your gate.

## Why this template

Collect-and-earn games led Roblox in 2025 and 2026: the biggest games by concurrent players were idle collection and
"steal" games, and tycoons fill Fortnite's charts too (research doc, section 2). Roblox ships no beginner tycoon
template. This one is the easiest way into persistence, economies and server-checked purchases. The loop is a classic
tycoon (things you buy produce money) with the current twist (a shared market and optional raids) as a setting, so
one template covers both.

## Game loop

1. Spawn at the plaza and walk into a free base (or get one after 10 s). The gate sign shows your name.
2. Gizmos ride the market belt through the middle of the map, each labelled with name, rarity, price and income.
3. Walk up and hold "Buy" on one you can afford. It hops off the belt, walks to your base and stands on a free
   pedestal.
4. Each Gizmo adds its income to your base's collector every second. Step on the collector pad to bank the cash.
5. Spend cash at your upgrade board: more pedestals, a gate lock, a bigger collector.
6. With `AllowStealing` on: hold "Grab" on another player's Gizmo (1.5 s), carry it home at reduced speed and set it on
   your free pedestal. Anyone who tags the carrier sends it home. A locked gate keeps raiders out for `LockSeconds`.
7. At `RebirthCost` banked cash, rebirth: reset Gizmos and upgrades for a permanent income multiplier.
8. Leave and come back: cash, Gizmos, upgrades and rebirths load; offline earnings are paid at half rate, capped.

## Economy (defaults, all scaled by settings)

| Rarity | Belt chance | Price | Income per second | Pay-back time |
|---|---|---|---|---|
| Common | 60% | 25 | 1 | 25 s |
| Uncommon | 25% | 250 | 6 | 42 s |
| Rare | 10% | 2,500 | 40 | 63 s |
| Epic | 4% | 25,000 | 250 | 100 s |
| Legendary | 0.9% | 250,000 | 1,600 | 156 s |
| Mythic | 0.1% | 2,500,000 | 10,000 | 250 s |

Two or three Gizmo designs per rarity, 14 in all, each a small Model of Parts (box body, ball eyes, cylinder wheels or
legs). Pedestals: 2 at start, up to 12; each new pedestal costs double the last, starting at 100. A Gizmo appears on
the belt every `BeltInterval` seconds and takes 60 s to cross. Cash is a whole number, shown with short suffixes
(12.4K, 3.1M).

Purchases use soft currency only. The template has no real-money purchases; the doc for creators says that selling
random-chance items for real money is subject to platform rules (assistant's call, to match the platform's policy when
it exists).

## Level layout

Map 300 x 220 studs at y 0. About 900 Parts before Gizmos.

| Area | Where | Contents |
|---|---|---|
| Plaza | (0, 0, -100) to the south edge | SpawnLocation, rebirth statue, leaderboard board |
| Market belt | along z = 0, x -140 to 140 | 8-stud-wide belt 1 stud up (dark Metal with Trim edges), entry and exit tunnels at both ends |
| Bases | 8 bases, 50 x 50; centres x in {-105, -35, 35, 105}, z in {65, -65} | Floor, low walls, gate with a lock (bars that slide down), 12 pedestals in a 4 x 3 grid (2 active), collector pad, upgrade board (a Part with a `SurfaceGui`), owner sign |

Each base is a copy of one Model, `BaseTemplate`, so editing one base and pressing "Apply to all bases" (a tutorial
step uses copy and paste) changes all eight.

## Instances

```
Workspace
  Plaza (Folder)
  Market (Folder): Belt (Part, Look:Trim), Path (Folder of invisible waypoint Parts)
  Bases (Folder)
    Base1 ... Base8 (Model, attribute BaseId, Owner): Floor, Walls, Gate (Model with ProximityPrompt), Pedestals
      (Folder of 12 Parts, attribute Unlocked), Collector (Part, tagged Collector), UpgradeBoard (Part with SurfaceGui),
      Sign (Part with SurfaceGui)
  Gizmos (Folder)               live Gizmos on the belt and on pedestals (created by the server)
  SpawnLocation
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  GizmoModels (Folder)          one Model per Gizmo type, the swap-in art
  Shared (Folder)
    GizmoCatalog (ModuleScript) names, rarities, prices, income
    Upgrades (ModuleScript)     upgrade list and price rules
    Format (ModuleScript)       12.4K style numbers
ServerScriptService
  BaseService, MarketService, GizmoService, EconomyService, UpgradeService, StealService, SaveService, RebirthService
  (Script)
StarterPlayer.StarterPlayerScripts
  HudController, MarketLabelController, UpgradeBoardController, NotifyController (LocalScript)
StarterGui
  Hud (ScreenGui): Cash, IncomePerSecond, Notifications, RebirthButton
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `BaseService` | server | Assign and release bases, sign text, gate lock state, new-player shield. |
| `MarketService` | server | Spawns a Gizmo every `BeltInterval` with weights from `GizmoCatalog` times `RarityLuck`, moves it along `Market.Path` (anchored, tweened), handles the Buy prompt. `-- TRY: change BELT_TRAVEL_SECONDS to 30 for a faster belt.` |
| `GizmoService` | server | Walks bought Gizmos to a free pedestal, every second adds income to each base's collector. |
| `EconomyService` | server | `leaderstats.Cash`, collector pad touch to bank, all spending goes through `TrySpend(player, amount)`. `-- TRY: make the collector pay double on weekends with DateTime.` |
| `UpgradeService` | server | Upgrade purchases from `Shared.Upgrades`. |
| `StealService` | server | Grab, carry, tag-to-return, place at home, carry timeout 30 s. Off unless `AllowStealing`. |
| `SaveService` | server | Loads on join, saves every 60 s, on leave and at shutdown (`BindToClose`); offline earnings. |
| `RebirthService` | server | Checks the cost, resets, raises the multiplier. |
| `HudController` | client | Cash with a short count-up tween, income per second, rebirth button and confirm. |
| `MarketLabelController` | client | A `BillboardGui` over each belt Gizmo: name, rarity (text and shape), price in red if you cannot afford it. |
| `UpgradeBoardController` | client | Draws the upgrade board buttons with prices; sends `BuyUpgrade`. |
| `NotifyController` | client | Short toasts: "Bought Bolt Bug", "Raider at your gate!", "Gizmo returned". |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `BuyUpgrade` | client to server | `{upgradeId}` | upgrade exists; player owns a base; prerequisites met; `TrySpend` succeeds; at most 5 per second |
| `LockGate` | client to server | none | owns a base; lock unlocked by upgrade; not on cooldown |
| `Rebirth` | client to server (RemoteFunction) | none | cash at least `RebirthCost`; returns the new multiplier |
| `CashChanged` | server to one client | `{cash, perSecond, gained}` | n/a |
| `Notify` | server to one client | `{text, kind}` | n/a |

Buy, Grab and place use `ProximityPrompt.Triggered`; the server checks distance, price, a free pedestal, that the
Gizmo is still on the belt (first buyer wins), raid rules and shields.

## UI

Top centre: cash and "+1.2K/s". Right: rebirth button (disabled with the missing amount shown). Toasts top right.
Belt labels above Gizmos. Upgrade board in the world. All in the `Look:UI` theme, rarity as text and shape.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `StartingCash` | 50 | 0 to 10,000 | Cash for a new player |
| `IncomeScale` | 1.0 | 0.25 to 10 | Multiplies all income |
| `PriceScale` | 1.0 | 0.25 to 10 | Multiplies all prices |
| `BeltInterval` | 3 | 1 to 15 | Seconds between belt Gizmos |
| `RarityLuck` | 1.0 | 0.5 to 5 | Raises rare chances |
| `AllowStealing` | false | bool | Raids on or off |
| `LockSeconds` | 60 | 10 to 300 | Gate lock duration |
| `NewPlayerShield` | 180 | 0 to 900 | Seconds a new base cannot be raided |
| `RebirthCost` | 1,000,000 | 10,000 to 100,000,000 | Cash needed to rebirth |
| `RebirthBonus` | 0.5 | 0.1 to 2 | Income multiplier added per rebirth |
| `SaveEnabled` | true | bool | Save progress |
| `OfflineEarnings` | true | bool | Pay half income while away, up to 2 hours |

## Multiplayer

Each base belongs to one player. The belt is shared, first buyer wins. Server-only money: no client ever sets cash.
A player who leaves is saved and their base frees up with its Gizmos removed. With raids on, a carried Gizmo that is
not home within 30 s returns to its owner, and the owner gets a toast. Late joiners see the belt and all bases as they
are.

## Tutorial

1. "Press Play. Buy a Gizmo from the belt and step on your collector." (`remote_fired` CashChanged)
2. "Open Game settings. Turn on Allow stealing, then Play again." (`setting_changed` AllowStealing)
3. "Pick the Meadow look." (`look_applied`)
4. "Make your own Gizmo: duplicate GizmoModels.BoltBug, recolour it and rename it." (`inserted` under GizmoModels)
5. "Open Shared.GizmoCatalog. Copy the BoltBug line and give your Gizmo a name and price." (`script_edited`
   GizmoCatalog)
6. "Play until your Gizmo rides the belt. Buy it." (`play_started`)
7. "Name your tycoon and share it." (`renamed_game`)

## Style variants

`toybox` (default): bright plastic robots and a toy factory. `meadow`: farm animals on a cart track (swap the Gizmo
Models, rename them in the catalog). `foundry`: industrial machines. `titanium`: a sleek gadget lab. Settings-only
variants: **Raid tycoon** (`AllowStealing` on, `LockSeconds` 30), **Idle tycoon** (`OfflineEarnings` on, `BeltInterval`
1, high `IncomeScale`).

## New API this template needs

A1 (catalog, upgrades, format), A2, A3, A4, A5 (`TweenService`, `ServerStorage`), A6 (CFrame for belt and pedestals),
**A8 (`leaderstats`)**, A10 (`PlayerRemoving`), A12 (belt and count-up tweens), A14, **A15 (`ProximityPrompt`)**, A16
(`RemoteFunction` for rebirth), **A17 (data store, `BindToClose`)**, A21 (`BillboardGui`, `SurfaceGui`, `TextButton`),
A24 (`Random.new`, `DateTime`).

## Acceptance checks

- [ ] `tests/smoke.mscene` with 2 bots, `BeltInterval` 1: A buys a Gizmo (cash drops by its price), it reaches a
      pedestal, A's collector grows by its income each second, A banks it; B tries to buy the same Gizmo in the same
      frame (refused); `BuyUpgrade` with too little cash (refused, cash unchanged); with `AllowStealing` on, B grabs
      A's Gizmo and A tags B (Gizmo returns); save, rejoin, cash and Gizmos match. Print `TYCOON_SMOKE_OK`.
- [ ] Offline earnings for a simulated 5-hour absence pay exactly 2 hours at half rate.
- [ ] `BuyUpgrade` 100 times in a second buys at most 5 times.
- [ ] 8 bases with 12 Gizmos each and a full belt: 60 fps on the reference PC; integrated graphics measured or
      "not verified".
- [ ] Numbers format correctly up to 10^15 (`Format` unit test).
- [ ] Screenshots in `toybox`, `meadow`, `foundry`, `titanium`.
