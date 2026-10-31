# Luau API (Metallic Engine, milestone 1)

Scripts are sandboxed Luau with Roblox-style names. `Script`s run in the **server VM**, `LocalScript`s in the **client VM**
(one local player, `Player1`, in the default combined host). Errors read `<Instance full name>:<line>: <message>` and go to the output sink; one failing
script or handler never stops the others. Every call into a script (start, handler, Heartbeat, remote) has a 0.1 s budget.

Code: `crates/metallic_script/src/api/`. Tests: `crates/metallic_script/tests/{api,remote,cframe,pivot}.rs`.

## Where scripts run
In combined mode, `ScriptHost::start` runs in tree order: every enabled `Script` under Workspace or ServerScriptService (server VM) and every
enabled `LocalScript` under StarterPlayerScripts or a `PlayerGui` (client VM). Scripts added later to those places (for example
StarterGui contents cloned into the `PlayerGui`) start on the next `step`. `script` is the script's own Instance.
The runtime (`Game::start`) copies the children of `StarterGui` into `Player1.PlayerGui` once when the game starts (M1 has
no `ResetOnSpawn`: the GUI survives respawns).

`ScriptHost::new_server(world, output)` selects a fixed server-only host. It starts only the server VM and eligible
`Script`s; `LocalScript`s stay inactive, including those created, enabled or reparented while running. It creates no
synthetic local Player, PlayerGui or PlayerAdded event, and preserves existing world instances. `Players.LocalPlayer`
is nil; no player is admitted by this constructor. Server events, Heartbeat, waits and script-owner cleanup use the
existing host paths. Client-targeted RemoteEvents have no receiver and are discarded at the next delivery step.
Host-injected client action events likewise have no receiver and are discarded on the next step; InputAction state
access keeps its existing client-only restriction.
The default `ScriptHost::new` and the current Game/Studio combined mode keep their existing behavior.

This role is a scripting foundation only. It adds no network transport, player admission, dedicated-server process,
ModuleScript/require loader or playable multiplayer. The separate scheduler correctness work in #173 and the later
process/transport/isolation work remain necessary for eventual hosting; M2 epic #15 acceptance is unchanged.

## Globals
`game`, `workspace`, `script`, `print`, `warn`, `task` (`wait`, `spawn`, `delay`), `Instance`, `Vector3`, `Vector2`, `CFrame`, `Color3`, `UDim2`, `Enum`.
All API tables are read-only.

## Instance
- `Instance.new(class, parent?)`. Services cannot be created; an unknown class errors.
- Members: `Name`, `Parent`, `ClassName` (read-only), `FindFirstChild(name, recursive?)`, `WaitForChild(name, timeout?)`,
  `GetChildren()`, `GetDescendants()`, `IsA(class)`, `Destroy()`, `Clone()`, and every registry property of the class by name.
- `instance.ChildName` finds a child. Unknown member: `<Name> is not a valid member of <ClassName>`.
- Member keys must be strings; numeric keys are rejected. Invalid UTF-8 bytes in a string key keep the existing
  replacement-character behavior for lookup and errors.
- Wrong property type: `invalid value for property Position: expected Vector3, got number`. Numbers and vectors must be finite.
- The same instance is always the same Lua object (`a == b`, usable as a table key).
- A destroyed instance stays readable, like in Roblox: `Name`, `ClassName` and its other properties keep their last
  values, `Parent` is `nil`, `FindFirstChild` returns `nil`, `GetChildren`/`GetDescendants` return `{}` and `IsA`
  still answers. Setting `Parent` errors (`The Parent property of Brick is locked: it has been destroyed`); setting any
  other property works and later reads see it, but the instance stays destroyed. `Clone`, `WaitForChild` and its
  events error; `Destroy` again does nothing. Services
  cannot be destroyed, cloned or reparented.
- `WaitForChild` yields; `step` resumes it. With a timeout it returns nil afterwards. Without one it warns once after 5 s.

```lua
local part = Instance.new("Part", workspace)
part.Position = Vector3.new(0, 10, 0)
print(workspace:WaitForChild("Spawn", 5), #workspace:GetChildren())
```

## Datatypes
- `Vector3.new(x, y, z)`: `X Y Z Magnitude Unit`, `+ - *` (vector * number, number * vector, vector * vector), also `/ number`,
  unary `-`, `==`.
  Components retain the engine's f64 precision in userdata.
- `Vector2.new(x, y)`: `X Y Magnitude Unit`, `+ -`, `*` and `/` (by a number or a `Vector2`), unary `-`, `==`. It is not a
  property type; input actions, `RemoteEvent` and scripts use it. `Vector2.new()` is `(0, 0)`.
- `CFrame`: a position and a rotation (Roblox conventions: the rotation's columns are the right, up and back axes;
  angles in radians). Code: `metallic_core::CFrame` and `crates/metallic_script/src/api/cframe.rs`.
  - Constructors: `CFrame.new()`, `new(pos)`, `new(pos, lookAt)`, `new(x, y, z)`, `new(x, y, z, qx, qy, qz, qw)`,
    `new(x, y, z, R00, R01, R02, R10, R11, R12, R20, R21, R22)`, `CFrame.identity`, `lookAt(at, target, up?)`,
    `Angles(rx, ry, rz)` = `fromEulerAnglesXYZ` (Rx * Ry * Rz: Z is applied first), `fromEulerAnglesYXZ` =
    `fromOrientation` (Ry * Rx * Rz), `fromEulerAngles(rx, ry, rz, order?)` with `Enum.RotationOrder` (default XYZ),
    `fromAxisAngle(axis, angle)`, `fromMatrix(pos, vX, vY, vZ?)`.
  - Fields: `Position`, `X Y Z`, `Rotation`, `LookVector`, `RightVector`, `UpVector`, `XVector`, `YVector`, `ZVector`.
  - Methods: `Inverse`, `Lerp(goal, alpha)` (position linear, rotation along the shortest arc), `ToWorldSpace`,
    `ToObjectSpace`, `PointToWorldSpace`, `PointToObjectSpace`, `VectorToWorldSpace`, `VectorToObjectSpace` (each takes
    several values and returns as many), `GetComponents` (also `components`), `ToEulerAnglesXYZ`, `ToEulerAnglesYXZ`,
    `ToOrientation`, `ToEulerAngles(order?)`, `ToAxisAngle`, `Orthonormalize`, `FuzzyEq(other, epsilon?)`.
  - Operators: `CFrame * CFrame`, `CFrame * Vector3` (a point), `CFrame + Vector3`, `CFrame - Vector3`, `==`;
    `tostring` gives the 12 components. Fields cannot be assigned.
- `BasePart.CFrame` reads `Position` and `Orientation` as one frame and writes both. `Orientation` keeps degrees
  applied Y, then X, then Z, so `part.CFrame:ToOrientation()` is `Orientation` in radians. A non-finite CFrame is
  refused and changes nothing. `BasePart.CFrame` itself is not stored; `PivotOffset` and `WorldPivot` are stored
  CFrame properties (scene files keep them). Remote payloads do not carry CFrames yet.
- `Color3.fromRGB(r, g, b)`: `R G B` read as 0..1.
- `UDim2.new(xs, xo, ys, yo)`: `X` and `Y` read as `{Scale, Offset}`.

```lua
part.Color = Color3.fromRGB(255, 80, 0)
part.Size = Vector3.new(4, 1, 2) * 2
```

## Pivots, size and scale
Roblox's `PVInstance` API on `BasePart` and `Model` (`IsA("PVInstance")` is true for both; it is not a registry class).
Code: `metallic_core::transform`, shared with Studio's building tools. Spec: `docs/design/building-tools.md` sections 4.3,
4.4 and 16.

- `GetPivot()`: for a part `CFrame * PivotOffset`. For a Model, its `PrimaryPart`'s pivot when that is a BasePart inside
  the model; otherwise `WorldPivot`.
- `PivotTo(cframe)`: moves the part, or every BasePart descendant of the model (through Folders and nested Models),
  so the pivot lands on `cframe`. The WorldPivots of nested Models move with them. On a Model without a PrimaryPart,
  `WorldPivot` becomes `cframe`.
- `BasePart.PivotOffset` (CFrame, default identity): the pivot relative to the part.
- `Model.WorldPivot` (CFrame): until it is first set (directly or by `PivotTo`) it reads as the centre of the model's
  bounding box with world axes, and follows the parts; after that it stays where it was put. It is ignored while a
  valid `PrimaryPart` is set. `Model.PrimaryPart` (BasePart or nil) is ignored when it is not a BasePart inside the model.
- `BasePart:Resize(face, delta)`: `face` is `Enum.NormalId` (`Right Top Back Left Bottom Front`, values 0 to 5; the number
  or the name also work). The face moves `delta` studs along its normal and the opposite face stays; sizes stay within
  0.001 to 2048 studs.
  A Ball grows on all axes, a Cylinder's Y and Z together. Returns false and changes nothing when the part has
  `CanCollide` and the grown part would newly overlap (by more than 0.0001 studs) another `CanCollide` part in
  Workspace; true otherwise.
- `Model:ScaleTo(scale)` / `GetScale()`: `scale` (positive, finite) is absolute, relative to the model's size at scale 1,
  as in Roblox: `ScaleTo(2)` then `ScaleTo(0.5)` gives half the original size, and `ScaleTo(1)` (or
  `ScaleTo(GetScale() * 0.5)` after `ScaleTo(2)`) restores it. Part positions and sizes scale about the pivot, which stays
  put; orientations stay; `PivotOffset` positions and nested WorldPivots scale too; nested Models' `GetScale` multiplies.
- **Limits.** A part's `Size` is 0.001 to 2048 studs per axis. Assigning `Size` clamps each axis, as Roblox does, and
  so does `Resize`. `ScaleTo` and `PivotTo` compute and check every new value first. If a part would leave the size
  limits, or a position would overflow, they error and change nothing (`GetScale` included).
- **CFrame properties.** `PivotOffset`, `WorldPivot`, `BasePart.CFrame` and `PivotTo`'s target are made orthonormal on
  write. A scaled or skewed matrix keeps its right axis and corrects the others (`CFrame:Orthonormalize`), so
  `PivotTo(cf)` then `GetPivot()` returns `cf:Orthonormalize()`. A degenerate rotation (for example all zeros) errors.
- `Model:GetBoundingBox()` returns `(CFrame, Vector3)`: the box around every BasePart descendant, oriented like the
  PrimaryPart (or the pivot when there is none). An empty model returns its pivot and size 0.
  `Model:GetExtentsSize()` returns that size.

```lua
local door = workspace.Door
door.PrimaryPart = door.Hinge
door:PivotTo(door:GetPivot() * CFrame.Angles(0, math.rad(90), 0))   -- swing about the hinge
local cf, size = door:GetBoundingBox()
```

## Signals
`signal:Connect(fn)` returns a connection with `:Disconnect()` and `.Connected`. Handlers run in their own thread (they may
`task.wait`). A `Connect` made inside a handler does not run in the same dispatch; `Disconnect` inside a handler is safe.

| Signal | Arguments | Fired when |
|---|---|---|
| `BasePart.Touched` | `hit` | the runtime calls `ScriptHost::touched(part, hit)`; when a character touches the part, `hit` is the character part touching it: a body part (`Head`, `UpperTorso`, `LeftHand`, ...) whose touch volume follows the animated avatar, or the `HumanoidRootPart` for the movement capsule (either way `hit.Parent` is the character Model). Each character part fires its own `Touched` |
| `BasePart.TouchEnded` | `hit` | the runtime calls `ScriptHost::touch_ended(part, hit)` when a character part stops touching the part (it separated, the character died, or the character was destroyed or replaced); pairs with `Touched`. If the
character part was destroyed by then, `hit` is the destroyed instance, still readable (`hit.Name`; `hit.Parent` is nil). Part-to-part contacts report `Touched` only |
| `Humanoid.Died` | none | the runtime calls `ScriptHost::humanoid_died` |
| `Player.CharacterAdded` | `model` | the runtime calls `ScriptHost::character_added` (also sets `Player.Character`) |
| `Players.PlayerAdded` | `player` | first `step` after combined-host `start`, for `Player1`; no synthetic event in the server-only host |
| `RunService.Heartbeat` | `dt` | every `step` |

```lua
workspace.Lava.Touched:Connect(function(hit)
    local hum = hit.Parent:FindFirstChild("Humanoid")
    if hum then hum:TakeDamage(25) end
end)
```

## Services
`game:GetService(name)` (or `game.Name`) for Workspace, Players, ReplicatedStorage, ServerScriptService, StarterPlayer,
StarterGui, Lighting, RunService, UserInputService, ContextActionService. Others error: `'Foo' is not a valid Service name`.

- `Players.LocalPlayer` (client VM; nil on the server), `Players:GetPlayers()` (players whose `PlayerAdded` was delivered),
  `Players:GetPlayerFromCharacter(model)`.
- Player: `UserId`, `Character`, `CharacterAdded`. Combined startup creates a `PlayerGui` for its local player;
  server-only startup adds no player or GUI.
- `Humanoid:TakeDamage(n)` only lowers `Health` (not below 0). Death is detected by the character controller.
- Humanoid tuning is clamped on write: `Health` and `MaxHealth` to 0..1e9, `WalkSpeed` and `JumpPower` to 0..1000,
  `MaxSlopeAngle` to 0..89. Setting a NaN or infinite number raises an error and keeps the old value. A character
  dies when `Health` is 0, when it falls below `Workspace.FallenPartsDestroyHeight`, or when its root goes more than
  1,000,000 studs from the origin on any axis.
- `RunService:IsServer()`, `RunService:IsClient()`, `RunService.Heartbeat`.

```lua
game:GetService("Players").PlayerAdded:Connect(function(player)
    print(player.Name, game:GetService("RunService"):IsServer())
end)
```

## Tasks and coroutines

- `task.spawn(functionOrThread, ...)` resumes immediately; `task.delay(seconds, functionOrThread, ...)` queues
  the original function/thread and complete argument tuple. A thread already queued by the scheduler cannot
  be scheduled again before its current admission completes.
- `task.wait(seconds?)` and `task.delay(nil, callback, ...)` default nil duration to zero. Numeric strings are
  coerced to numbers; negative durations use zero. Invalid duration values raise positioned argument errors.
- While a task thread is queued, `coroutine.resume(thread, ...)` returns false and an ownership error, and a
  wrapped coroutine raises that error. Foreign arguments do not replace the original callback or wake values.
  Cancelled script tasks remain blocked while a user retains their thread handles.
- A user coroutine that calls `task.wait` is adopted by the scheduler and resumes at its due time with the
  elapsed duration. Its initial resume/wrap call exposes no internal scheduler values. Ordinary later yields
  release queued ownership so manual resumes work again, preserving nil/multiple values and native errors.
- Native `pcall` and `xpcall` are unchanged. Ordinary coroutine creation, yielding, status and close behavior
  remain available; internally tagged results are consumed by the scheduler/admission helpers.

## RemoteEvent
`FireServer(...)` (client only), `FireClient(player, ...)` and `FireAllClients(...)` (server only), `OnServerEvent(player, ...)`
(server only), `OnClientEvent(...)` (client only). Wrong side: `FireClient can only be called from the server` and so on.

- Allowed values: nil, boolean, number, string, Vector3, Vector2, Color3, UDim2, Instance, and tables of these up to 16 levels deep.
  They are deep-copied (each handler gets its own copy); Instances pass by reference (a destroyed one arrives as nil).
- Delivery is on the next `step`, never in the same call.
- Refused: `cannot send function` (also thread/other userdata), `cannot send cyclic table`, tables deeper than 16 levels.

```lua
-- server
remote.OnServerEvent:Connect(function(player, amount) print(player.Name, amount) end)
-- client
remote:FireServer(5)
```

## Limits
- Pattern matching: the 10 million charged-step complexity allowance applies separately to each starting-position
  attempt. Deadline polling continues across attempts and substitutions; the script's 0.1 s execution budget still
  applies to the complete call. A larger linear operation can therefore time out even when it passes the complexity
  check. The separate 200-level matcher depth, 32-capture and 64 MB gsub-result limits are unchanged.
- Remote: at most 32 arguments, 10,000 values (tables and leaves, shared tables counted per reference) and 1 MB of string bytes per
  call (`remote payload too large`); at most 1,000 messages and 16 MB queued per VM per step (`remote queue full`).
- Signals: at most 10,000 connections per signal (`too many connections`). All handlers of a step share a 0.25 s budget per VM;
  the rest run on the next step, in order. A script whose handlers time out 3 times in a row is disconnected (error printed once).
- A script that is destroyed, disabled (`Enabled = false`) or moved out of a place where scripts run stops: its connections and
  threads are removed. Ownership is tracked by the script's instance id, so clones with the same full name stop independently.
- Userdata metatables are protected (`getmetatable` returns false, `setmetatable` errors).
- Errors raised by API calls and operators are plain strings with `<script>:<line>:` so `pcall` shows them cleanly.

## Milestone 1 limits
In combined mode the client VM writes to the same World as the server VM (single process). Nothing stops a LocalScript from changing server-owned
state; M2 replication will separate them. Accepted for M1.

## Input (client only)
Spec: `docs/design/input-actions.md` section 5. Everything here needs the client VM (a `LocalScript`).

### Enums
`Enum.KeyCode` (`Unknown`, letters, digits, `F1` to `F12`, `Space`, arrows, `MouseButton1`, `ButtonA`, `Thumbstick1`, ...: the
controls of the input runtime), `Enum.UserInputType` (`MouseButton1` to `3`, `MouseWheel`, `MouseMovement`, `Touch`, `Keyboard`,
`Focus`, `Gamepad1` to `4`, `TextInput`, `None`), `Enum.UserInputState` (`Begin`, `Change`, `End`, `Cancel`, `None`),
`Enum.InputActionType`, `Enum.InputTrigger`, `Enum.TouchSlot` (Metallic), `Enum.ContextActionResult` (`Sink`, `Pass`),
`Enum.PreferredInput` (`KeyboardAndMouse`, `Gamepad`, `Touch`), `Enum.RotationOrder` (`XYZ`, `XZY`, `YZX`, `YXZ`, `ZXY`, `ZYX`; used by `CFrame`), `Enum.NormalId` (used by `Resize`), `Enum.ContextActionPriority` (`Low` 1000, `Medium` 2000,
`High` 3000). Items have `Name` and `Value`, compare with `==` and print as `Enum.KeyCode.Space`. A wrong member errors
(`Nope is not a valid member of Enum.KeyCode`). Raw `UserInputService` events (#80) will reuse these names.

### InputAction
`GetState()` (boolean, number, `Vector2` or `Vector3` by the action's `Type`; zero or false when the action or its context is
disabled), `GetHoldProgress()` (0 to 1), and the signals `Pressed()`, `Released()`, `Triggered(value)`, `StateChanged(value)`.
The value is the action's value as the runtime reported it. On the server any of these errors with
`InputAction can only be used from a LocalScript`; other members still work.

### ContextActionService
- `BindAction(name, handler, createTouchButton, ...inputs)` and `BindActionAtPriority(name, handler, createTouchButton,
  priority, ...inputs)`. Inputs are `Enum.KeyCode` or `Enum.UserInputType` values (`Keyboard` binds every key, `Gamepad1` to
  `4` bind the gamepad buttons, `MouseMovement` binds `MouseDelta`). The call creates a hidden `InputAction` (one binding per
  key) in a hidden `InputContext` named `ContextActionService@<priority>` under the local player; binding a name again
  replaces it only once the new one is complete (a failed call changes nothing). A script may have at most 256 bound
  actions; past that `BindAction` errors with `too many bound actions`. Default priority is 2000.
- `handler(actionName, state, inputObject)` runs with `Begin`, `End`, `Change` (axis actions) or `Cancel` (the action was
  unbound or its context disabled while pressed). `inputObject` has `KeyCode` (`Unknown`), `UserInputType`, `UserInputState`,
  `Position`, `Delta`. Return `Enum.ContextActionResult.Pass` to let lower consumers see the event; anything else sinks.
- `UnbindAction(name)`, `SetTitle(name, text)`, `SetImage(name, image)`, `SetPosition(name, udim2)` (set `TouchLabel`,
  `TouchImage`, and `TouchLayout = Custom` with `TouchPosition`). `GetButton(name)` returns the touch overlay's button of an
  action bound with `createTouchButton`, or nil when there is none or the overlay did not place it. It is a read-only
  `TouchButton` object (the overlay is drawn by the Player, not by GUI instances): `Name`, `ClassName`, `Text`, `Image`,
  `Visible`, `AbsolutePosition` and `AbsoluteSize` (`Vector2`, dp from the top-left of the screen), `Position` and `Size`
  (`UDim2` offsets in dp).

### UserInputService (last input only)
`GetLastInputType()`, `LastInputTypeChanged(type)`, `PreferredInput`, `TouchEnabled`, `KeyboardEnabled`, `GamepadEnabled`.
Raw events stay with #80.

### Host side
`ScriptHost::input_action_event`, `set_action_state`, `set_input_devices`, `set_preferred_input`, `set_last_input_type` and
`take_bound_results` connect the input runtime to the scripts.

### Limits
Handlers share the 0.25 s step budget and run under the 0.1 s resume budget like all handlers. At most 16 action events are
dispatched per `step`, oldest event time first; the rest wait for the next steps (the queue holds 1024, the oldest are
dropped beyond that).

```lua
local dash = game:GetService("ReplicatedStorage").Input.Gameplay.Dash
dash.Triggered:Connect(function() request:FireServer() end)   -- send the intent, the server checks the cooldown
```
