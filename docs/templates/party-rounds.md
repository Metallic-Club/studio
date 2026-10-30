# Template: Party Rounds (minigame framework)

Status: planned. Milestone: M4 template wave (needs M2 multiplayer; can ship at the end of M3 if time allows).
Default look: `toybox`. Players: 1 to 16. Difficulty: intermediate. Conventions and API group ids:
`docs/templates/README.md`.

Sample world: **Party Rounds**. Players gather in a lobby, vote for the next minigame, play a 60 to 150 s round, score
points, and after five rounds the top three stand on the podium. Four minigames ship with it, including a hidden-role
round.

## Why this template

Round-based games are the backbone of Roblox and Fortnite multiplayer: party games, tag, deathrun, social deduction
(a hidden-role game was the second most played Roblox game in the October 2026 snapshot; research doc, section 2).
Each of those is the same machine: lobby, pick a mode, load a map, run a timer, eliminate, score, clean up. This
template makes that machine a reusable framework, and a new minigame is one ModuleScript and one map Model. It is the
template that teaches clean-up: every round must leave no stray connections, parts or attributes behind.

## Game loop

1. **Lobby:** players spawn on a floating plaza with a few toys (bounce pads, a slide) and a scoreboard.
2. **Vote:** three random enabled minigames appear; 10 s to vote; ties break at random. Without voting, a random pick.
3. **Load:** the framework clones the minigame's map from `ServerStorage.Maps` into `Workspace.Arena`, teleports
   players to its spawn points and shows the rules card for 5 s.
4. **Play:** the minigame runs until its own end rule or its time limit. Eliminated players spectate.
5. **Score:** 1st `PointsFirst`, 2nd `PointsSecond`, 3rd `PointsThird`, everyone else who took part 1.
6. **Clean up:** the map is destroyed and every connection the minigame made is disconnected.
7. After `RoundsPerMatch` rounds, a podium shows the top three for 10 s and points reset.

Solo: every shipped minigame has a solo rule (survive as long as you can, beat your streak), so one player can play
the whole match.

## Shipped minigames

| Minigame | Players | Length | Rule |
|---|---|---|---|
| **Floor Fall** | 1+ | until one is left (max 90 s) | Three layers of 4 x 4 tiles (10 x 10 grid) at heights 40, 25 and 10. A tile you touch turns to a warning colour and pattern, then drops 0.6 s later. Fall into the water below and you are out. Solo: time survived |
| **Colour Rush** | 1+ | until one is left (max 120 s) | An 8 x 8 grid of 6-stud pads in 6 colours, each colour with its own symbol. The announcer calls a colour and symbol. When the timer ends, every other pad drops for 1 s. The timer starts at 4 s and shrinks by 0.3 s each call to a minimum of 1.5 s. Solo: calls survived |
| **Bumper Ring** | 2+ (solo: bumper bots made of Parts) | until one is left (max 90 s) | A round platform of radius 40 on stilts over water. Q (or the touch button) dashes forward with a 2 s cooldown; dashing into someone knocks them back. The edge crumbles inward in rings every 15 s |
| **Hidden Hunter** | 4+ (hidden from the vote below 4) | 150 s | In a small manor, one secret Hunter freezes Runners by tagging them; one secret Guard has one freeze shot that stops the Hunter (if it hits a Runner, the Guard is frozen instead). Runners collect coins for points. Runners win if time runs out or the Guard freezes the Hunter. General-audience framing: "frozen", not harmed |

## The minigame contract

A minigame is a ModuleScript in `ServerStorage.Minigames` that returns a table. The framework passes a context `ctx`
that owns everything the round creates.

```lua
-- Drops floor tiles after players touch them. Last player standing wins.
local FloorFall = {
	Name = "Floor Fall",
	Description = "Keep moving. Tiles fall after you step on them.",
	MinPlayers = 1,
	TimeLimit = 90,
	Map = "FloorFall", -- a Model in ServerStorage.Maps
}

function FloorFall.Start(ctx)
	-- ctx.Map, ctx.Players, ctx.Settings, ctx:Track(connection), ctx:Eliminate(player),
	-- ctx:AddPoints(player, n), ctx:Announce(text), ctx:FireClients(kind, data), ctx:End()
end

function FloorFall.OnAction(ctx, player, kind, data) end -- optional: requests from clients
function FloorFall.OnPlayerLeft(ctx, player) end         -- optional

return FloorFall
```

The framework ends the round when `ctx:End()` is called, when one or zero players remain in a last-standing game, or
at `TimeLimit`. It then ranks players (still in > elimination order), disconnects everything passed to `ctx:Track`,
destroys the map and resets character attributes. A minigame never has to clean up after itself; that rule is what
makes adding minigames safe for beginners.

Optional client half: a ModuleScript with the same name in `ReplicatedStorage.MinigameClients` gets `Start(ctx)` and
`Stop()` on each client (Bumper Ring's dash button, Hidden Hunter's role card).

## Level layout

Lobby: 80 x 80 plaza at y 200 above the arena area, with a scoreboard wall, two bounce pads and a slide; about 150
Parts. Arena maps are placed at the origin. Floor Fall: 3 layers of 100 tiles over a 60 x 60 water plane. Colour Rush:
an 8 x 8 grid of 6-stud pads with 1-stud gaps, 20 studs over water. Bumper Ring: a 40-radius disc made of 24 wedge
segments in 3 rings. Hidden Hunter: a two-floor manor of 100 x 80 with 9 rooms, 30 coin spots and 4 hiding nooks.
Maps total about 1,400 Parts, but only one is loaded at a time.

## Instances

```
Workspace
  Lobby (Model), Arena (Folder, empty between rounds), SpawnLocation (in the lobby)
ReplicatedStorage
  GameSettings (Configuration)
  Remotes (Folder)
  MinigameClients (Folder): BumperRing, HiddenHunter (ModuleScript)
  Shared (Folder): RoundStates (ModuleScript)
ServerStorage
  Minigames (Folder): FloorFall, ColourRush, BumperRing, HiddenHunter (ModuleScript)
  Maps (Folder): FloorFall, ColourRush, BumperRing, HiddenHunter (Model, each with a Spawns Folder)
ServerScriptService
  RoundService, VoteService, ScoreService, PodiumService (Script)
  MinigameRunner (ModuleScript)    builds ctx, enforces the contract, cleans up
StarterPlayer.StarterPlayerScripts
  RoundHudController, VoteController, SpectateController, MinigameClientLoader (LocalScript)
StarterGui
  RoundHud (ScreenGui): Announcement, Timer, RoundCounter, Scoreboard, RulesCard
  Vote (ScreenGui): three cards
```

## Scripts

| Script | Side | Job |
|---|---|---|
| `RoundService` | server | Lobby, Voting, Loading, Playing, Scoring, Podium. Waits for `MinPlayers`. `-- TRY: change RULES_CARD_SECONDS to 8.` |
| `MinigameRunner` | server module | Loads a minigame module, builds `ctx`, runs Start, routes `MinigameAction` to `OnAction`, ends, ranks, cleans up. Catches errors in a minigame and ends that round with no points, so a broken minigame cannot stall the match. |
| `VoteService` | server | Picks three enabled minigames that fit the player count, counts votes. |
| `ScoreService` | server | `leaderstats.Points` and `Wins`; resets per match. |
| `PodiumService` | server | Places the top three on the podium and points the cameras at it. |
| `RoundHudController` | client | Announcements, timer, round counter, rules card, scoreboard. |
| `VoteController` | client | Vote cards with name, description and a preview colour band. |
| `SpectateController` | client | When eliminated, follows living players. |
| `MinigameClientLoader` | client | Starts and stops the optional client half of the current minigame. |

## RemoteEvents

| Name | Direction | Payload | Server checks |
|---|---|---|---|
| `RoundStateChanged` | server to all | `{state, minigame, timeLeft, round, rounds}` | n/a; also sent to late joiners |
| `Announce` | server to all | `{text, style}` | n/a |
| `VoteOptions` | server to all | `{options}` | n/a |
| `CastVote` | client to server | `{index}` | state is Voting; index 1 to 3; one counted vote per player (changing replaces it) |
| `MinigameEvent` | server to clients | `{kind, data}` | n/a |
| `MinigameAction` | client to server | `{kind, data}` | state is Playing; player is in the round and not eliminated; at most 10 per second; then the minigame's `OnAction` checks its own rules (Bumper Ring: dash cooldown; Hidden Hunter: role, range 6 studs, cooldown 1.2 s) |
| `RoleAssigned` | server to one client | `{role}` | n/a; sent privately, never to all |

## UI

Top centre: announcement line and timer. Left: round `3 / 5`. Hold Tab: scoreboard. Vote: three large cards. Rules
card: minigame name, one-line rule and controls. Colour Rush always shows the symbol with the colour.

## Game settings

| Attribute | Default | Range | Effect |
|---|---|---|---|
| `RoundsPerMatch` | 5 | 1 to 20 | Rounds before the podium |
| `IntermissionSeconds` | 15 | 5 to 60 | Lobby time between rounds |
| `MinPlayers` | 1 | 1 to 8 | Players needed to start |
| `VoteEnabled` | true | bool | Vote or random pick |
| `PointsFirst` | 5 | 1 to 20 | Points for first |
| `PointsSecond` | 3 | 0 to 20 | Points for second |
| `PointsThird` | 2 | 0 to 20 | Points for third |
| `EnableFloorFall` | true | bool | Include Floor Fall |
| `EnableColourRush` | true | bool | Include Colour Rush |
| `EnableBumperRing` | true | bool | Include Bumper Ring |
| `EnableHiddenHunter` | true | bool | Include Hidden Hunter |
| `Difficulty` | 1.0 | 0.5 to 2 | Minigames read it (tile drop delay, rush timer, ring speed) |

## Multiplayer

Late joiners wait in the lobby until the next round and see the current state. Leaving mid-round counts as
eliminated. Hidden roles stay on the server and the role holder's client only. Up to 16 players; Floor Fall at 16
players removes up to 16 tiles per second, which the server batches into one event per frame.

## Tutorial

1. "Press Play and finish one round." (`remote_fired` RoundStateChanged with Scoring)
2. "Try the Dusk look." (`look_applied`)
3. "In Game settings, set Rounds per match to 3 and turn off Bumper Ring." (`setting_changed` RoundsPerMatch)
4. "Open ServerStorage.Maps.FloorFall. Duplicate Layer3 and move the copy 15 studs lower." (`inserted` under
   Maps.FloorFall)
5. "Make your own minigame: duplicate Minigames.ColourRush and rename it. Follow its TRY line to make it faster."
   (`inserted` ModuleScript under Minigames, then `script_edited`)
6. "Play and vote for your new minigame." (`remote_fired` CastVote)
7. "Name your party game and share it." (`renamed_game`)

## Style variants

`toybox` (default). `dusk` for a sunset game show. `frost` for a winter party. `titanium` for a sleek "trials" show.
Settings-only variants: **Only Hidden Hunter** (a social deduction game), **Survival gauntlet** (Floor Fall and Colour
Rush only, `Difficulty` 1.5), **Quick party** (3 rounds, 5 s intermission).

## New API this template needs

**A1 (`require` of ModuleScripts is the whole design)**, A2, A3, A4 (`Destroying`, `ChildRemoved`), A5 (`ServerStorage`,
`TweenService`, `Debris`), A6 (`PivotTo` for maps and teleports), A7 (`Raycast` for the Guard's shot), A8, A10
(`LoadCharacter`, `CameraSubject`), A12, A14, A16 (`BindableEvent` for round signals), **A18 (`ApplyImpulse` or
`AssemblyLinearVelocity` on a character for knockback; the kinematic controller must accept it)**, A19, A20
(`ContextActionService` dash button), A21, A24, A28 (chat is useful for Hidden Hunter, not required).

## Acceptance checks

- [ ] `tests/smoke.mscene` with 4 bots and `RoundsPerMatch` 4: each shipped minigame runs once (forced order); after
      each round, `Workspace.Arena` is empty, the number of live connections returns to the pre-round count, and every
      bot's character attributes are reset; a minigame module that errors in `Start` ends its round with no points and
      the next round runs; podium shows after round 4. Print `PARTY_SMOKE_OK`.
- [ ] `RoleAssigned` is never received by a client other than the role holder (checked on all 4 clients).
- [ ] `MinigameAction` 100 times in a second reaches `OnAction` at most 10 times.
- [ ] Solo play completes a full match.
- [ ] 16 clients at 60 fps on the reference PC in Floor Fall.
- [ ] Screenshots in `toybox`, `dusk`, `frost`, `titanium`.
