# Metallic templates: shared conventions

Date: 2026-10-30. Status: proposal (planned). Author: Claude (research session).
Research and catalog: `docs/research/2026-10-30-templates-and-onboarding.md`. Checklist: `docs/templates/quality-bar.md`.

Every template in `docs/templates/<slug>.md` follows these rules. Anything marked "assistant's call" is a proposal the
owner may override. Nothing here is implemented yet.

## 1. What a template is

A template is a project folder under `templates/<slug>/`, the same shape as Parkour in plan Task 8, plus four files.

```
templates/<slug>/
  metallic.toml        name, main_scene, dimension = "3D" | "2D"          (Task 8 + dimension)
  template.toml        id, name, description, preview, order              (Task 8)
                       + dimension, genre, players_min, players_max, look,
                         difficulty, settings, tags                        (new, see 1.1)
  main.mscene          the level and all scripts
  tutorial.toml        the in-template tutorial steps (section 5)          (new)
  preview.png          Studio screenshot, 16:9                             (Task 8)
  preview.webm         4-6 s silent loop of real play for the start screen (new)
  tests/smoke.mscene   headless test scene that prints <SLUG>_SMOKE_OK      (Task 8 pattern)
```

### 1.1 New `template.toml` fields (assistant's call)

```toml
id = "horror"
name = "Horror"
description = "Find five fuses in the dark and get out. Something hunts by sound."
preview = "preview.png"
preview_video = "preview.webm"
order = 4
dimension = "3D"            # "3D", "2D", or "2.5D" (2.5D = 3D world with a locked camera)
genre = "horror"
players_min = 1
players_max = 4
look = "noir"               # default look preset id (section 3)
difficulty = "starter"      # starter | intermediate | advanced: how much code a creator meets
tags = ["first-person", "co-op", "npc"]

[[setting]]                 # one block per Game settings knob (section 2)
attribute = "FusesNeeded"
label = "Fuses to find"
group = "Rules"
kind = "int"                # int | number | bool | choice
min = 1
max = 10
step = 1
help = "How many fuses the players must bring to the fuse box."
```

## 2. Game settings: remix knobs without code

Each template has one `Configuration` Instance named `GameSettings` in `ReplicatedStorage`. Every knob is an Attribute on
it (`GameSettings:GetAttribute("FusesNeeded")`). Scripts read knobs only through this object. Studio shows the
`[[setting]]` blocks of `template.toml` as a **Game settings** panel: sliders, toggles and drop-downs, grouped, with the
help text. Changing a knob is an undoable edit like any property change.

Why attributes: Roblox creators already know `GetAttribute`/`SetAttribute`, they appear in the Properties panel, they
replicate to clients, and a script can listen with `GetAttributeChangedSignal`. The labels, ranges and help text live
in `template.toml` because Roblox attributes carry no metadata (assistant's call).

Rules:
- Knobs change rules a player feels (speed, timers, damage, counts). Look and art go in look presets, not knobs.
- Every knob has a safe range. The game must stay playable at both ends of it; the smoke test runs at the defaults and
  at both extremes of the three most important knobs.
- Scripts read a knob when they use it, not once at start, so changing a knob during Play in Studio takes effect live
  where that makes sense (speeds, damage). Knobs that only make sense at round start say so in their help text.
- Six to fifteen knobs per template. More than fifteen means the template does too much.

## 3. Look presets: restyle the whole game in one click

> **Superseded naming (#144).** `docs/design/look-presets.md` renames these presets **Themes**
> (`themes/<id>.theme.ron`, tags `Theme:<Role>` and `Theme:UI`, `template.toml` field `theme`), because `Lighting.Look`
> now names the render style. Each Theme picks a Look and a lighting preset; section 5.3 of that spec maps the nine
> below. The rest of this section stays as written until #83 updates it.

A look preset is a file in the Studio install, `looks/<id>.toml`. Applying one is a single undoable command. It sets:
- `Lighting`: `ClockTime`, `Brightness`, `Ambient`, `OutdoorAmbient`, `FogColor`, `FogStart`, `FogEnd`, plus sky
  gradient colours (needs API group A13).
- A palette of **roles**. Parts opt in with a CollectionService tag `Look:<Role>`; the preset sets `Color` and
  `Material` for every tagged part. Untagged parts keep what the creator gave them.
- A UI theme for the template's ScreenGuis: panel colour, text colour, accent, font, corner radius. UI objects opt in
  with the tag `Look:UI`.
- For 2D: pixel snapping on or off, outline width, palette for sprites drawn from Parts.

Roles (fixed list, so every template works with every preset): `Ground`, `Wall`, `Trim`, `Accent`, `Prop`, `Hazard`,
`Goal`, `Pickup`, `Water`, `Glass`, `Light`. `Hazard` and `Goal` keep a readable meaning in every preset: hazards stay
warm and high-contrast, goals stay cool and high-contrast. A preset that breaks this fails review.

Shipped presets (assistant's call; colours chosen by the M3 art pass, names fixed now):

| id | Name | Mood | Good for |
|---|---|---|---|
| `titanium` | Titanium | Graphite, silver and a little violet, clear daylight. The Metallic house look | Any template; the default when a template has no reason to differ |
| `toybox` | Toybox | Bright primaries, SmoothPlastic, noon sun, soft fog | Obby, party, tycoon, simulator |
| `meadow` | Meadow | Greens and warm wood, late afternoon, long shadows | Simulator, survival, build, tower defense |
| `dusk` | Dusk | Orange to violet sky, low sun, warm rim light | Racing, battle royale, parkour |
| `noir` | Noir | Near-black, cold moonlight, fog at 20 to 80 studs | Horror, stealth |
| `frost` | Frost | Pale blue, Ice and Glass, white fog | Platformer, parkour, racing |
| `foundry` | Foundry | Concrete, Metal, warm work lights, haze | FPS, arena, tycoon |
| `paper` | Paper (2D) | Off-white ground, ink outlines, muted colour | 2D platformer, puzzle, card games |
| `pixel` | Pixel (2D) | 16-colour palette, pixel snapping, no smoothing | 2D platformer, top-down, horde |

Brand rule check: the Metallic brand bans neon and glow in Metallic's own product look. Shipped presets follow it (no
emissive "neon" materials, no bloom-heavy looks). Creators can still make whatever they like in their own games.

## 4. Script conventions

Creator-facing scripts are the main teaching surface. Rules:

1. **Line 1 says what the script does**, in one plain sentence (Task 8 already asks this):
   `-- Moves the Keeper between rooms and chases players it hears or sees.`
2. **A short header block** under it: which RemoteEvents it uses, which Game settings it reads, which tags it looks for.
3. **`-- TRY:` comments** mark two or three places where a beginner can change one thing and see a result on the next
   Play. Example: `-- TRY: change 0.5 to 2 so the light flickers slower.` The tutorial and Litha point at these lines.
4. **No magic numbers that a creator would want to change.** Those are Game settings. Numbers left in code are
   physics or layout constants with a comment saying why.
5. **Server decides, client shows.** Anything that changes score, health, money, inventory, placement or round state
   runs in a server `Script`. Client `LocalScript`s handle input, camera, UI and effects, and ask the server through
   RemoteEvents. The server checks every request (range, rate, ownership, cooldown, type). Each template doc lists the
   checks per RemoteEvent. This holds even while milestone 1 runs server and client in one process.
6. **One job per script.** A script longer than about 200 lines splits into ModuleScripts (needs API group A1).
7. **Names:** services end in `Service` (server), controllers end in `Controller` (client), shared modules live in
   `ReplicatedStorage.Shared`. RemoteEvents live in `ReplicatedStorage.Remotes` and are named for what happened or what
   is asked (`PlaceTower`, `RoundStateChanged`), not `Remote1`.
8. **Plain words in comments.** No jargon the code does not need. If a comment needs a paragraph, the code is too clever
   for a template.
9. **Original code only.** Nothing copied from Roblox, Unity, Unreal, Godot or their samples.

## 5. In-template tutorial (`tutorial.toml`)

Five to seven steps that run in Studio the first time a creator opens a project made from the template. Each step has
short text (at most 25 words), an optional target to highlight, and a completion check Studio can observe. The creator
can skip any step or close the tutorial; it can be reopened from Help. Litha can read the same steps aloud, explain a
step in more depth, or answer a question about it (section 4 of the research doc).

```toml
[[step]]
id = "play"
text = "Press Play to try your game. Reach the first checkpoint."
highlight = "toolbar.play"
done_when = { event = "touched", path = "Workspace.Checkpoints.Checkpoint1" }

[[step]]
id = "look"
text = "Pick a look. The whole game restyles in one click."
highlight = "panel.looks"
done_when = { look_applied = true }
```

Completion checks Studio supports (assistant's call): `play_started`, `play_stopped`, `selected` (path),
`property_changed` (path, property), `setting_changed` (attribute), `look_applied`, `inserted` (class),
`script_edited` (path), `touched` (path, during Play), `remote_fired` (name, during Play), `printed` (text),
`renamed_game`, `published`.

Every tutorial follows the same arc: **play it, restyle it, tune it, build on it, change one line of code, play again**.
The last step offers publishing (from milestone 2, when the platform exists) or a "save and share" zip before then.

## 6. API groups not in the M1 plan

Plan Task 7 gives templates: `game`, `workspace`, `script`, `print`, `warn`, `task` (wait, spawn, delay),
`Instance.new`, `Vector3`, `Color3.fromRGB`, `UDim2.new`, Instance basics (`Name Parent ClassName FindFirstChild
WaitForChild GetChildren GetDescendants IsA Destroy Clone` and registry properties), `BasePart.Touched`, signals with
`Connect`/`Disconnect`, `GetService`, `Players` (`LocalPlayer`, `PlayerAdded`, `GetPlayers`,
`GetPlayerFromCharacter`), `Player.Character`/`CharacterAdded`, `Humanoid.Died`/`TakeDamage`, `RunService.Heartbeat`/
`IsServer`/`IsClient`, `RemoteEvent` (both directions). Classes: the Task 2 registry only. There is no client input
API, no camera API, no clickable UI, no sound and no `require`.

Template docs name the groups below. Names are Roblox names unless marked **Metallic addition**.

| id | Group | Members (Roblox names unless marked) | Templates needing it |
|---|---|---|---|
| A1 | Modules | `ModuleScript`, `require(moduleScript)` (Instance argument only, so the sandbox holds) | all except the smallest |
| A2 | Attributes and settings | `GetAttribute`, `SetAttribute`, `GetAttributes`, `GetAttributeChangedSignal`, `AttributeChanged`, `Configuration` class | all (Game settings) |
| A3 | Tags | `CollectionService:AddTag/RemoveTag/HasTag/GetTagged/GetInstanceAddedSignal/GetInstanceRemovedSignal` | all (look roles) |
| A4 | More Instance members | `Changed`, `GetPropertyChangedSignal`, `ChildAdded`, `ChildRemoved`, `Destroying`, `FindFirstChildOfClass`, `FindFirstChildWhichIsA`, `FindFirstAncestorOfClass`, `IsDescendantOf` | all |
| A5 | More services | `ServerStorage`, `StarterPack`, `Teams`, `SoundService`, `TweenService`, `UserInputService`, `ContextActionService`, `Debris` | most |
| A6 | CFrame | `CFrame.new`, `CFrame.lookAt`, `CFrame.Angles`, `CFrame.fromOrientation`, `*`, `:Inverse`, `:ToWorldSpace`, `:ToObjectSpace`, `Position`, `LookVector`, `RightVector`, `UpVector`; `BasePart.CFrame`; `Model:GetPivot/PivotTo` | all 3D beyond Parkour |
| A7 | Spatial queries | `workspace:Raycast(origin, direction, RaycastParams)`, `RaycastParams`, `RaycastResult`, `workspace:GetPartBoundsInBox/GetPartBoundsInRadius/GetPartsInPart`, `OverlapParams` (the Rust side exists in Task 4; this exposes it) | FPS, BR, build, horror, tower defense |
| A8 | Value objects and leaderboard | `IntValue`, `NumberValue`, `StringValue`, `BoolValue`, `ObjectValue`; a `leaderstats` Folder under a Player shows in a built-in player list | tycoon, simulator, FPS, BR, party, platformer |
| A9 | Teams | `Team` (`TeamColor`, `AutoAssignable`), `Player.Team`, `Player.Neutral`, `SpawnLocation.TeamColor`, `Neutral` | FPS, party |
| A10 | Character control | `Player:LoadCharacter()`, `Players.CharacterAutoLoads`, `Players.RespawnTime`, `Players.PlayerRemoving`, `Humanoid.HealthChanged`, `StateChanged`, `GetState`, `MoveTo`, `MoveToFinished`, `JumpHeight`, `UseJumpPower`, `AutoRotate`, `Sit`; `Player.CameraMode`, `CameraMaxZoomDistance`, `CameraMinZoomDistance` | all |
| A11 | Tools | `Tool` (`Equipped`, `Unequipped`, `Activated`, `Deactivated`, `RequiresHandle`), `Backpack`, `Humanoid:EquipTool/UnequipTools`, built-in hotbar | FPS, BR, simulator, build |
| A12 | Motion and effects | `TweenService:Create`, `TweenInfo.new`, `Tween:Play/Cancel`, `Tween.Completed`, `Enum.EasingStyle`, `Enum.EasingDirection`; `Debris:AddItem` | all |
| A13 | Lighting and lights | `Lighting.ClockTime/Brightness/Ambient/OutdoorAmbient/FogColor/FogStart/FogEnd`, `Atmosphere`, `Sky`; `PointLight`, `SpotLight`, `SurfaceLight` (`Brightness`, `Range`, `Color`, `Angle`, `Shadows`, `Enabled`). Renderer work: local lights, spot shadows | all (looks), horror (flashlight) |
| A14 | Audio | `Sound` (`SoundId`, `Volume`, `Looped`, `PlaybackSpeed`, `Playing`, `Play`, `Stop`, `Ended`, `RollOffMaxDistance`), `SoundGroup`, `SoundService`. Engine work: an audio subsystem (none is planned in M1) | all |
| A15 | Interaction | `ProximityPrompt` (`ActionText`, `ObjectText`, `HoldDuration`, `MaxActivationDistance`, `Triggered`, `PromptShown`), `ClickDetector` (`MouseClick`) | horror, tycoon, simulator, BR, build |
| A16 | More remotes | `RemoteFunction` (`InvokeServer`, `OnServerInvoke`), `BindableEvent`, `UnreliableRemoteEvent`, `workspace:GetServerTimeNow()` | FPS, BR, tower defense, party |
| A17 | Persistence | `DataStoreService:GetDataStore`, `GetAsync`, `SetAsync`, `UpdateAsync`, `game:BindToClose`; Studio keeps a local emulated store per project. Live store needs the platform (M2+) | tycoon, simulator, build |
| A18 | Physics control | `BasePart.AssemblyLinearVelocity`, `AssemblyAngularVelocity`, `ApplyImpulse`, `CollisionGroup`, `PhysicsService:RegisterCollisionGroup/CollisionGroupSetCollidable`, `LinearVelocity`, `AlignOrientation`, `PlaneConstraint`, `BasePart:SetNetworkOwner` (M2) | tycoon (conveyors), party, racing, 2.5D |
| A19 | Camera | `workspace.CurrentCamera`, `Camera.CFrame`, `FieldOfView`, `CameraType`, `CameraSubject`, `Focus`, `ViewportSize`, `ViewportPointToRay`, `ScreenPointToRay`, `WorldToViewportPoint`; `RunService.RenderStepped`, `RunService:BindToRenderStep` | FPS, horror, build, tower defense, 2.5D |
| A20 | Input | `UserInputService.InputBegan/InputEnded/InputChanged`, `IsKeyDown`, `GetMouseLocation`, `GetMouseDelta`, `MouseBehavior`, `MouseIconEnabled`, `TouchEnabled`; `ContextActionService:BindAction` (with on-screen touch buttons); `Enum.KeyCode`, `Enum.UserInputType` | all with custom controls |
| A21 | Clickable and richer UI | `TextButton`, `ImageButton` (`Activated`, `MouseButton1Click`), `ImageLabel`, `TextBox`, `ScrollingFrame`, `UIListLayout`, `UIGridLayout`, `UIPadding`, `UICorner`, `UIStroke`, `UIScale`; `GuiObject.AnchorPoint/ZIndex/LayoutOrder`; `TextLabel.TextScaled/Font/TextXAlignment`; `BillboardGui`, `SurfaceGui`; `StarterGui:SetCoreGuiEnabled` | all |
| A22 | View models | **Metallic addition**: `Camera.ViewModel` (ObjectValue-style reference to a Model drawn after the world in its own depth range, never clipping into walls) and `Camera.ViewModelFieldOfView`. Roblox has no equivalent; creators there fake it | FPS, BR, horror (hands holding a flashlight) |
| A23 | 2D | **Metallic addition** (proposal for the M3 2D design): `Vector2`, `Part2D`, `SpawnLocation2D`, `Checkpoint2D`, `TileMap2D`, `Camera2D`, a 2D Humanoid controller; Box2D under it. See section 7 | all 2D templates |
| A24 | Random and time | `Random.new(seed)` (`NextNumber`, `NextInteger`), `DateTime.now()` | most |
| A25 | Visual effects | `ParticleEmitter`, `Beam`, `Trail`, `Highlight` (milestone 6 VFX editor) | optional juice in all; never required by a template before M6 |
| A26 | Animation | `Animator`, `Animation`, `AnimationTrack` (milestone 6) | fighting, roleplay; M3 templates avoid it and animate with A12 and procedural code |
| A27 | Imported models | `MeshPart` from glTF (milestone 4) | racing, roleplay, survival art upgrades |
| A28 | Chat | `TextChatService` (milestone 2) | roleplay, hangout, party |

## 7. 2D proposal (for the M3 2D design session)

The spec picks Box2D for 2D in milestone 3 and leaves the 2D API open. Proposal (assistant's call, to be confirmed or
replaced when M3 is designed):

- A project is 2D when `metallic.toml` says `dimension = "2D"`. Workspace then holds 2D classes. 3D Parts in a 2D project
  are an error in Studio's Output, not a silent mix.
- `Vector2` (Roblox name: `Vector2.new(x, y)`, `X`, `Y`, `Magnitude`, `Unit`, `+ - *`).
- `Part2D`: `Position: Vector2`, `Rotation` (degrees), `Size: Vector2`, `Shape` = `Box | Circle | Capsule`, `Color`,
  `Image` (optional sprite path), `Anchored`, `CanCollide`, `Transparency`, `ZIndex`, `OneWay` (platform you can jump
  up through), `Touched`. Units are studs; `Camera2D.PixelsPerStud` (default 32) sets zoom.
- `SpawnLocation2D`, `Checkpoint2D` (same rules as the 3D ones).
- `TileMap2D`: a grid of tiles from one image, painted in Studio, one collider per merged run of solid tiles.
- `Camera2D`: `Position`, `PixelsPerStud`, `FollowSubject`, `FollowLerp`, `Bounds` (min and max Vector2).
- The 2D character is the same `Humanoid` (WalkSpeed, JumpHeight, Health, Died) inside a Model whose
  `HumanoidRootPart` is a `Part2D`. The 2D controller adds platformer feel: coyote time, jump buffering and
  variable jump height, exposed as Humanoid properties `CoyoteTime`, `JumpBufferTime`, `JumpCutMultiplier`
  (Metallic additions).
- 2.5D (side-on 3D and isometric) needs no 2D API. It is a 3D project with a scripted camera (A19) and, for side-on,
  a `PlaneConstraint` (A18) on the character so it stays on one plane.
