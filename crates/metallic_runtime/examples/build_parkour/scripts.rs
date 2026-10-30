//! Luau sources of the Parkour template. Each creator-facing script starts with one comment line
//! saying what it does and is meant to be read by someone learning Luau.

/// Lava folder: knocks out anyone who touches a lava part.
pub const LAVA: &str = r#"-- Lava: any player who touches a lava part in this folder is knocked out.
local lavaFolder = script.Parent

local function onTouched(hit)
    -- hit is the body part that touched the lava; its Parent is the character.
    local humanoid = hit.Parent:FindFirstChild("Humanoid")
    if humanoid then
        humanoid.Health = 0
    end
end

for _, lava in ipairs(lavaFolder:GetChildren()) do
    if lava:IsA("BasePart") then
        lava.Touched:Connect(onTouched)
    end
end
"#;

/// The moving platform between the lava walk and Checkpoint 2.
pub const MOVING_PLATFORM: &str = r#"-- Moves this platform back and forth between the two docks, forever.
local RunService = game:GetService("RunService")

local platform = script.Parent
local middle = platform.Position
local distance = 7        -- TRY: how far it goes each way, in studs
local secondsPerTrip = 7  -- TRY: a smaller number makes it faster

local elapsed = 0
RunService.Heartbeat:Connect(function(dt)
    elapsed = elapsed + dt
    local wave = math.sin(elapsed / secondsPerTrip * 2 * math.pi)
    platform.Position = middle + Vector3.new(0, 0, wave * distance)
end)
"#;

/// The spinning bar on the arena after Checkpoint 2.
pub const SPINNING_BAR: &str = r#"-- Spins this bar around the middle of the arena. Jump over it!
local RunService = game:GetService("RunService")

local bar = script.Parent
local degreesPerSecond = 70  -- TRY: 40 is easy, 120 is hard

local angle = 0
RunService.Heartbeat:Connect(function(dt)
    angle = (angle + degreesPerSecond * dt) % 360
    bar.Orientation = Vector3.new(0, angle, 0)
end)
"#;

/// Checkpoints folder: tells the player's screen about a new checkpoint.
pub const CHECKPOINTS: &str = r#"-- Tells a player's screen when they reach a new checkpoint.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local checkpointFolder = script.Parent
local checkpointReached = ReplicatedStorage:WaitForChild("CheckpointReached")
local bestOrder = {}  -- the highest checkpoint each player has reached

local checkpoints = {}
for _, child in ipairs(checkpointFolder:GetChildren()) do
    if child:IsA("Checkpoint") then
        table.insert(checkpoints, child)
    end
end

for _, checkpoint in ipairs(checkpoints) do
    checkpoint.Touched:Connect(function(hit)
        local player = Players:GetPlayerFromCharacter(hit.Parent)
        if player and checkpoint.Order > (bestOrder[player] or 0) then
            bestOrder[player] = checkpoint.Order
            checkpointReached:FireClient(player, checkpoint.Order, #checkpoints)
        end
    end)
end
"#;

/// The Finish pad.
pub const FINISH: &str = r#"-- When a player first reaches the finish, tells them how long the run took.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")

local finishPad = script.Parent
local finished = ReplicatedStorage:WaitForChild("Finished")
local secondsPlayed = 0
local someoneFinished = false  -- only the first finish counts while this server runs

RunService.Heartbeat:Connect(function(dt)
    secondsPlayed = secondsPlayed + dt
end)

finishPad.Touched:Connect(function(hit)
    local player = Players:GetPlayerFromCharacter(hit.Parent)
    if player and not someoneFinished then
        someoneFinished = true
        finished:FireClient(player, secondsPlayed)
    end
end)
"#;

/// The HUD's LocalScript.
pub const HUD: &str = r#"-- Shows the goal, checkpoint progress and the finish time at the top of the screen.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local hud = script.Parent
local message = hud:WaitForChild("Banner"):WaitForChild("Message")
local player = Players.LocalPlayer
local status = "Reach the finish!"  -- TRY: change the first message

local function show(text)
    message.Text = text
end

local function watchCharacter(character)
    show(status)
    local humanoid = character:WaitForChild("Humanoid")
    humanoid.Died:Connect(function()
        show("Oops! Back to your checkpoint...")
    end)
end

ReplicatedStorage:WaitForChild("CheckpointReached").OnClientEvent:Connect(function(order, total)
    status = "Checkpoint " .. order .. "/" .. total
    show(status)
end)

ReplicatedStorage:WaitForChild("Finished").OnClientEvent:Connect(function(seconds)
    status = string.format("Finished in %.1f s", seconds)
    show(status)
end)

player.CharacterAdded:Connect(watchCharacter)
show(status)
"#;

/// Server half of the smoke test (only in `tests/smoke.mscene`).
pub const SMOKE_SERVER: &str = r#"-- Smoke test: respawn at Checkpoint 2 and exactly one Finished event.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local function fail(message)
    error("PARKOUR_SMOKE_FAIL " .. message)
end

local player = Players:WaitForChild("Player1")
while player.Character == nil do
    task.wait()
end
local checkpoint = workspace.Checkpoints.Checkpoint2
local finishPad = workspace.Finish

local function root()
    return player.Character:WaitForChild("HumanoidRootPart")
end

local function standOn(part)
    root().Position = part.Position + Vector3.new(0, part.Size.Y / 2 + 3, 0)
end

task.wait(0.5)
standOn(checkpoint)
task.wait(0.5)
if player.RespawnLocation ~= checkpoint then
    fail("Checkpoint 2 did not become the respawn location")
end

root().Position = Vector3.new(0, workspace.FallenPartsDestroyHeight - 50, 0)
task.wait(6)
local distance = (root().Position - checkpoint.Position).Magnitude
if distance > 4 then
    fail("respawned " .. distance .. " studs from Checkpoint 2")
end

standOn(finishPad)
task.wait(0.5)
standOn(checkpoint)
task.wait(0.5)
standOn(finishPad)
task.wait(1)

local reported = false
ReplicatedStorage.SmokeReport.OnServerEvent:Connect(function(_, count)
    reported = true
    if count ~= 1 then
        fail("expected 1 Finished event, got " .. tostring(count))
    end
    print("PARKOUR_SMOKE_OK")
end)
ReplicatedStorage.SmokeQuery:FireClient(player)
task.wait(2)
if not reported then
    fail("the client never reported its Finished count")
end
"#;

/// Client half of the smoke test.
pub const SMOKE_CLIENT: &str = r#"-- Smoke test helper: counts Finished events and reports the count when asked.
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local count = 0
ReplicatedStorage:WaitForChild("Finished").OnClientEvent:Connect(function()
    count = count + 1
end)
ReplicatedStorage:WaitForChild("SmokeQuery").OnClientEvent:Connect(function()
    ReplicatedStorage.SmokeReport:FireServer(count)
end)
"#;
