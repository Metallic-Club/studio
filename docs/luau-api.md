# Luau API (Metallic Engine, milestone 1)

Scripts are sandboxed Luau with Roblox-style names. `Script`s run in the **server VM**, `LocalScript`s in the **client VM**
(one local player, `Player1`). Errors read `<Instance full name>:<line>: <message>` and go to the output sink; one failing
script or handler never stops the others. Every call into a script (start, handler, Heartbeat, remote) has a 0.1 s budget.

Code: `crates/metallic_script/src/api/`. Tests: `crates/metallic_script/tests/{api,remote}.rs`.

## Where scripts run
`ScriptHost::start` runs, in tree order: every enabled `Script` under Workspace or ServerScriptService (server VM) and every
enabled `LocalScript` under StarterPlayerScripts or a `PlayerGui` (client VM). Scripts added later to those places (for example
StarterGui contents cloned into the `PlayerGui`) start on the next `step`. `script` is the script's own Instance.
The runtime (`Game::start`) copies the children of `StarterGui` into `Player1.PlayerGui` once when the game starts (M1 has
no `ResetOnSpawn`: the GUI survives respawns).

## Globals
`game`, `workspace`, `script`, `print`, `warn`, `task` (`wait`, `spawn`, `delay`), `Instance`, `Vector3`, `Color3`, `UDim2`.
All API tables are read-only.

## Instance
- `Instance.new(class, parent?)`. Services cannot be created; an unknown class errors.
- Members: `Name`, `Parent`, `ClassName` (read-only), `FindFirstChild(name, recursive?)`, `WaitForChild(name, timeout?)`,
  `GetChildren()`, `GetDescendants()`, `IsA(class)`, `Destroy()`, `Clone()`, and every registry property of the class by name.
- `instance.ChildName` finds a child. Unknown member: `<Name> is not a valid member of <ClassName>`.
- Wrong property type: `invalid value for property Position: expected Vector3, got number`. Numbers and vectors must be finite.
- The same instance is always the same Lua object (`a == b`, usable as a table key).
- A destroyed instance errors on use: `Brick has been destroyed`. Services cannot be destroyed, cloned or reparented.
- `WaitForChild` yields; `step` resumes it. With a timeout it returns nil afterwards. Without one it warns once after 5 s.

```lua
local part = Instance.new("Part", workspace)
part.Position = Vector3.new(0, 10, 0)
print(workspace:WaitForChild("Spawn", 5), #workspace:GetChildren())
```

## Datatypes
- `Vector3.new(x, y, z)`: `X Y Z Magnitude Unit`, `+ - *` (vector * number, number * vector, vector * vector), also `/ number`,
  unary `-`, `==`.
- `Color3.fromRGB(r, g, b)`: `R G B` read as 0..1.
- `UDim2.new(xs, xo, ys, yo)`: `X` and `Y` read as `{Scale, Offset}`.

```lua
part.Color = Color3.fromRGB(255, 80, 0)
part.Size = Vector3.new(4, 1, 2) * 2
```

## Signals
`signal:Connect(fn)` returns a connection with `:Disconnect()` and `.Connected`. Handlers run in their own thread (they may
`task.wait`). A `Connect` made inside a handler does not run in the same dispatch; `Disconnect` inside a handler is safe.

| Signal | Arguments | Fired when |
|---|---|---|
| `BasePart.Touched` | `hit` | the runtime calls `ScriptHost::touched(part, hit)`; when a character touches the part, `hit` is its `HumanoidRootPart` (so `hit.Parent` is the character Model) |
| `Humanoid.Died` | none | the runtime calls `ScriptHost::humanoid_died` |
| `Player.CharacterAdded` | `model` | the runtime calls `ScriptHost::character_added` (also sets `Player.Character`) |
| `Players.PlayerAdded` | `player` | first `step` after `start`, for `Player1` |
| `RunService.Heartbeat` | `dt` | every `step` |

```lua
workspace.Lava.Touched:Connect(function(hit)
    local hum = hit.Parent:FindFirstChild("Humanoid")
    if hum then hum:TakeDamage(25) end
end)
```

## Services
`game:GetService(name)` (or `game.Name`) for Workspace, Players, ReplicatedStorage, ServerScriptService, StarterPlayer,
StarterGui, Lighting, RunService. Others error: `'Foo' is not a valid Service name`.

- `Players.LocalPlayer` (client VM; nil on the server), `Players:GetPlayers()` (players whose `PlayerAdded` was delivered),
  `Players:GetPlayerFromCharacter(model)`.
- Player: `UserId`, `Character`, `CharacterAdded`. Each Player has a `PlayerGui` child.
- `Humanoid:TakeDamage(n)` only lowers `Health` (not below 0). Death is detected by the character controller.
- `RunService:IsServer()`, `RunService:IsClient()`, `RunService.Heartbeat`.

```lua
game:GetService("Players").PlayerAdded:Connect(function(player)
    print(player.Name, game:GetService("RunService"):IsServer())
end)
```

## RemoteEvent
`FireServer(...)` (client only), `FireClient(player, ...)` and `FireAllClients(...)` (server only), `OnServerEvent(player, ...)`
(server only), `OnClientEvent(...)` (client only). Wrong side: `FireClient can only be called from the server` and so on.

- Allowed values: nil, boolean, number, string, Vector3, Color3, UDim2, Instance, and tables of these up to 16 levels deep.
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
- Each VM has a 256 MiB memory cap. `string.gmatch` iterators retain shared subject
  and pattern strings inside that cap; many iterators over one string share its
  storage. Retained strings are released when the iterators become collectible.
- Remote: at most 32 arguments, 10,000 values (tables and leaves, shared tables counted per reference) and 1 MB of string bytes per
  call (`remote payload too large`); at most 1,000 messages and 16 MB queued per VM per step (`remote queue full`).
- Signals: at most 10,000 connections per signal (`too many connections`). All handlers of a step share a 0.25 s budget per VM;
  the rest run on the next step, in order. A script whose handlers time out 3 times in a row is disconnected (error printed once).
- A script that is destroyed, disabled (`Enabled = false`) or moved out of a place where scripts run stops: its connections and
  threads are removed. Ownership is tracked by the script's instance id, so clones with the same full name stop independently.
- Userdata metatables are protected (`getmetatable` returns false, `setmetatable` errors).
- Errors raised by API calls and operators are plain strings with `<script>:<line>:` so `pcall` shows them cleanly.

## Milestone 1 limits
The client VM writes to the same World as the server VM (single process). Nothing stops a LocalScript from changing server-owned
state; M2 replication will separate them. Accepted for M1.
