# ADR 0007: Initial API surface (simple classes and services)

- **Status:** Accepted (2026-11-02, sign-off #9)
- **Issue:** #8

## Context

We start with simple classes and services only. Forge-Engine has about 200 classes, many of them Roblox compatibility or features that need physics, UI, audio or networking. This issue fixes the **starting set** so nobody adds classes ad hoc. A class outside this list needs its own issue, in the milestone where its subsystem exists.

## Decision

### Classes

| Class | Parent | Own members (first version) | Milestone |
|---|---|---|---|
| `Object` | none | `ClassName`, `IsA()`, `Changed` | M1 (#16) |
| `Instance` | Object | `Name`, `Parent`, `Archivable`, `Destroy()`, `Clone()`, `GetChildren()`, `GetDescendants()`, `FindFirstChild()`, `FindFirstChildOfClass()`, `FindFirstChildWhichIsA()`, `FindFirstAncestor()`, `IsDescendantOf()`, `IsAncestorOf()`, `GetFullName()`, `ClearAllChildren()`, `GetAttribute()`, `SetAttribute()`, `ChildAdded`, `ChildRemoved`, `DescendantAdded`, `DescendantRemoving`, `AncestryChanged`, `AttributeChanged`, `Destroying` | M1 (#16) |
| `WorldObject` | Instance | `PivotOffset`, `GetPivot()`, `PivotTo()` | M1 (#16) |
| `Folder` | Instance | none | M1 (#16) |
| `Model` | WorldObject | `PrimaryPart` | M1 (#16) |
| `BasePart` | WorldObject | `CFrame`, `Position`, `Orientation`, `Size`, `Color`, `Transparency`, `CastShadow`, `Anchored`, `CanCollide` | M1 (#17) |
| `Part` | BasePart | `Shape` (Block, Ball, Cylinder, Wedge) | M1 (#17) |
| `Light` | WorldObject | `Enabled`, `Brightness`, `Color`, `Range`, `Shadows` | M1 (#17) |
| `PointLight` | Light | none (uses `Range`) | M1 (#17) |
| `SpotLight` | Light | `Angle` | M1 (#17) |
| `DirectionalLight` | Light | hides `Range` | M1 (#17) |
| `Camera` | WorldObject | `CFrame`, `FieldOfView` | M1 (#17) |
| `ValueBase` + `BoolValue`, `IntValue`, `NumberValue`, `StringValue`, `ObjectValue` | Instance | `Value` | M1 (#17) |
| `SourceContainer`, `BaseScript` (`Enabled`, `Source`), `Script`, `LocalScript`, `ModuleScript` | Instance | see #4 | M2 |
| `BindableEvent`, `BindableFunction` | Instance | `Fire` / `Event`, `Invoke` / `OnInvoke` | M2 |

### Services

| Service | Purpose | Milestone |
|---|---|---|
| `DataModel` (`game`) | root, `GetService()` | M1 |
| `Workspace` | the 3D world; `CurrentCamera` | M1 |
| `Lighting` | ambient, sky colour, sun direction, `ClockTime` | M1 |
| `ReplicatedStorage`, `ServerStorage` | storage containers | M1 |
| `ServerScriptService` | runs server scripts | M1 |
| `RunService` | `Heartbeat`, `RenderStepped`, `PreSimulation`, `PostSimulation`, `IsStudio()`, `IsRunning()` | M3 |
| `UserInputService` | keyboard, mouse, gamepad, touch basics | M3 |

### Data types

`Vector2`, `Vector3`, `CFrame`, `Color3`, `EnumItem` (M1). Others (`UDim2`, sequences and so on) arrive with the feature that needs them.

### Enums

`PartShape`, `NormalId`, plus what the listed members need. Each enum is declared in the file of the class that owns it.

### Explicitly not now

- **Physics:** constraints, welds, movers. Forge's versions are not taken. Physics members beyond `Anchored` / `CanCollide` come with M7.
- **UI:** every GUI class (`ScreenGui`, `Frame`, `TextLabel`, …), which needs a game-UI decision first.
- **Forge-specific families:** effects (particles, beams, trails, fire, smoke, highlight, explosion), audio and sound effects, animation and tweens, characters.
- **Multiplayer and online services:** `Players`, `Player`, `RemoteEvent` (M10), `DataStoreService`, `MarketplaceService`, teleport, chat and voice.
- **Roblox compatibility shims:** `PVInstance`, deprecated aliases, lower-case method names, `wait()` / `spawn()` globals.
