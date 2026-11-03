--!strict
-- The Player's built-in demo game (the M3 milestone demo): every part shape, every material, see-through glass,
-- the three kinds of light, parts moved on Heartbeat and a camera driven by UserInputService.
--
-- Controls
--   W A S D      move the camera (any movement leaves the automatic orbit)
--   Q / E        down / up
--   Shift        move faster
--   right mouse  hold to look around (the cursor stays where it was)
--   arrow keys   look around without a mouse
--   C            back to the automatic orbit
--   Space        pause / resume the moving parts
--
-- It runs as a Script in a standalone (RunContext.Local) game. The Player embeds this file in its binary; games
-- loaded from projects replace it in M4.

local Lighting = game:GetService("Lighting")
local RunService = game:GetService("RunService")
local UserInputService = game:GetService("UserInputService")

---------------------------------------------------------------------------------------------------------------------
-- The scene
---------------------------------------------------------------------------------------------------------------------

local function part(
    name: string,
    shape: EnumPartShape,
    material: EnumMaterial,
    size: Vector3,
    cframe: CFrame,
    color: Color3,
    transparency: number?
): Part
    local p = Instance.new("Part")
    p.Name = name
    p.Shape = shape
    p.Material = material
    p.Size = size
    p.CFrame = cframe
    p.Color = color
    p.Transparency = transparency or 0
    p.Anchored = true
    p.Parent = workspace
    return p
end

local Block, Ball, Cylinder, Wedge =
    Enum.PartShape.Block, Enum.PartShape.Ball, Enum.PartShape.Cylinder, Enum.PartShape.Wedge
local M = Enum.Material

-- The baseplate, and a darker path across it.
part("Baseplate", Block, M.Concrete, Vector3.new(160, 2, 160), CFrame.new(0, -1, 0), Color3.fromRGB(99, 95, 98))
part("Path", Block, M.SmoothPlastic, Vector3.new(6, 0.2, 120), CFrame.new(0, 0.1, 0), Color3.fromRGB(64, 66, 72))

-- A gallery: every shape in every material, one row per material. Each part rests on the baseplate.
local materials = { M.Plastic, M.SmoothPlastic, M.Metal, M.Wood, M.Concrete, M.Grass, M.Glass, M.Neon }
local colors = {
    Color3.fromRGB(196, 40, 28),
    Color3.fromRGB(13, 105, 172),
    Color3.fromRGB(163, 162, 165),
    Color3.fromRGB(160, 95, 53),
    Color3.fromRGB(175, 170, 160),
    Color3.fromRGB(75, 151, 75),
    Color3.fromRGB(175, 221, 255),
    Color3.fromRGB(0, 255, 191),
}
local shapes = { Block, Ball, Cylinder, Wedge }
for row, material in materials do
    local z = -24 + (row - 1) * 5
    local transparency = if material == M.Glass then 0.55 else 0
    for column, shape in shapes do
        local x = 10 + (column - 1) * 5
        local size = if shape == Cylinder then Vector3.new(3, 2.5, 2.5) else Vector3.new(3, 3, 3)
        part(
            `{material.Name}{shape.Name}`,
            shape,
            material,
            size,
            CFrame.new(x, size.Y / 2, z),
            colors[row],
            transparency
        )
    end
end

-- See-through glass panes in front of the gallery, with bright parts behind them.
for i = 0, 2 do
    part(
        `GlassPane{i + 1}`,
        Block,
        M.Glass,
        Vector3.new(0.3, 6, 8),
        CFrame.new(5, 3, -14 + i * 10),
        Color3.fromRGB(175, 221, 255),
        0.6
    )
end

-- A spinning carousel: a metal hub, and neon and plastic cars around it (moved on Heartbeat).
local hub = part("CarouselHub", Cylinder, M.Metal, Vector3.new(1, 8, 8), CFrame.new(-20, 0.5, -10) * CFrame.Angles(0, 0, math.pi / 2), Color3.fromRGB(120, 124, 130))
local pole = part("CarouselPole", Cylinder, M.Metal, Vector3.new(10, 1, 1), CFrame.new(-20, 5, -10) * CFrame.Angles(0, 0, math.pi / 2), Color3.fromRGB(200, 200, 205))
local cars: { Part } = {}
local carCount = 12
for i = 1, carCount do
    local material = if i % 3 == 0 then M.Neon else M.Plastic
    local color = Color3.fromHSV((i - 1) / carCount, 0.8, 1)
    table.insert(cars, part(`Car{i}`, if i % 2 == 0 then Ball else Block, material, Vector3.new(1.6, 1.6, 1.6), CFrame.new(-20, 3, -10), color))
end

-- Bobbing balls along the path, and a tumbling wedge.
local bobbers: { Part } = {}
for i = 1, 6 do
    table.insert(
        bobbers,
        part(`Bobber{i}`, Ball, if i % 2 == 0 then M.Neon else M.SmoothPlastic, Vector3.new(2, 2, 2), CFrame.new(-6, 2, -30 + i * 9), Color3.fromHSV(i / 7, 0.7, 1))
    )
end
local tumbler = part("Tumbler", Wedge, M.Wood, Vector3.new(4, 4, 4), CFrame.new(-20, 4, 14), Color3.fromRGB(160, 95, 53))
local orb = part("GlassOrb", Ball, M.Glass, Vector3.new(6, 6, 6), CFrame.new(-20, 4, 26), Color3.fromRGB(150, 200, 255), 0.5)
local core = part("OrbCore", Block, M.Neon, Vector3.new(1.5, 1.5, 1.5), CFrame.new(-20, 4, 26), Color3.fromRGB(255, 120, 40))

-- Lights. They render once the renderer has them (#81 for the sun, #83 for point and spot lights); until then the
-- renderer's temporary directional light shades everything.
Lighting.ClockTime = 15
Lighting.Brightness = 2

local sun = Instance.new("DirectionalLight")
sun.Name = "Sun"
sun.Brightness = 1
sun.Color = Color3.fromRGB(255, 244, 214)
sun.CFrame = CFrame.lookAt(Vector3.new(0, 0, 0), Vector3.new(-0.4, -1, -0.3))
sun.Parent = workspace

-- A lamp post: a point light inside a neon bulb, so it follows the bulb.
part("LampPost", Cylinder, M.Metal, Vector3.new(7, 0.6, 0.6), CFrame.new(-8, 3.5, 20) * CFrame.Angles(0, 0, math.pi / 2), Color3.fromRGB(40, 40, 45))
local bulb = part("LampBulb", Ball, M.Neon, Vector3.new(1.4, 1.4, 1.4), CFrame.new(-8, 7.4, 20), Color3.fromRGB(255, 214, 140))
local lamp = Instance.new("PointLight")
lamp.Name = "Lamp"
lamp.Color = Color3.fromRGB(255, 214, 140)
lamp.Brightness = 3
lamp.Range = 18
lamp.Parent = bulb

-- A spotlight on a gantry over the gallery, pointing straight down (forward is -Z, so tilt -Z to -Y).
part("GantryLeft", Block, M.Metal, Vector3.new(0.6, 12, 0.6), CFrame.new(8, 6, -6), Color3.fromRGB(40, 40, 45))
part("GantryRight", Block, M.Metal, Vector3.new(0.6, 12, 0.6), CFrame.new(28, 6, -6), Color3.fromRGB(40, 40, 45))
part("GantryBeam", Block, M.Metal, Vector3.new(20.6, 0.6, 0.6), CFrame.new(18, 12, -6), Color3.fromRGB(40, 40, 45))
part("SpotHead", Cylinder, M.Metal, Vector3.new(1.2, 1.2, 1.2), CFrame.new(18, 11.2, -6) * CFrame.Angles(0, 0, math.pi / 2), Color3.fromRGB(30, 30, 30))
local spot = Instance.new("SpotLight")
spot.Name = "Spot"
spot.Color = Color3.fromRGB(180, 210, 255)
spot.Brightness = 5
spot.Range = 24
spot.Angle = 45
spot.CFrame = CFrame.lookAt(Vector3.new(18, 11, -6), Vector3.new(18, 0, -6))
spot.Parent = workspace

-- A coloured point light circling the carousel.
local firefly = part("Firefly", Ball, M.Neon, Vector3.new(0.8, 0.8, 0.8), CFrame.new(-20, 6, -10), Color3.fromRGB(120, 255, 120))
local fireflyLight = Instance.new("PointLight")
fireflyLight.Name = "FireflyLight"
fireflyLight.Color = Color3.fromRGB(120, 255, 120)
fireflyLight.Brightness = 2
fireflyLight.Range = 12
fireflyLight.Parent = firefly

---------------------------------------------------------------------------------------------------------------------
-- Moving parts (Heartbeat)
---------------------------------------------------------------------------------------------------------------------

local paused = false
local t = 0
RunService.Heartbeat:Connect(function(deltaTime: number)
    if paused then
        return
    end
    t += deltaTime

    local spin = t * 0.8
    pole.CFrame = CFrame.new(-20, 5, -10) * CFrame.Angles(0, spin, math.pi / 2)
    hub.CFrame = CFrame.new(-20, 0.5, -10) * CFrame.Angles(0, spin, math.pi / 2)
    for i, car in cars do
        local a = spin + (i - 1) / carCount * math.pi * 2
        local height = 3 + math.sin(t * 2 + i) * 0.8
        car.CFrame = CFrame.new(-20 + math.cos(a) * 8, height, -10 + math.sin(a) * 8) * CFrame.Angles(0, -a, 0)
    end

    for i, bobber in bobbers do
        bobber.Position = Vector3.new(-6, 2 + math.abs(math.sin(t * 2 + i * 0.7)) * 3, -30 + i * 9)
    end

    tumbler.CFrame = CFrame.new(-20, 4 + math.sin(t) * 0.5, 14) * CFrame.Angles(t * 0.7, t * 1.1, 0)
    core.CFrame = CFrame.new(-20, 4, 26) * CFrame.Angles(t, t * 1.3, t * 0.4)
    orb.Transparency = 0.45 + math.sin(t * 1.5) * 0.15

    local f = t * 1.4
    firefly.Position = Vector3.new(-20 + math.cos(f) * 12, 6 + math.sin(f * 2), -10 + math.sin(f) * 12)
    spot.Angle = 40 + math.sin(t) * 10
end)

---------------------------------------------------------------------------------------------------------------------
-- The camera (RenderStepped, so it moves before the frame is drawn)
---------------------------------------------------------------------------------------------------------------------

local camera = workspace.CurrentCamera :: Camera
camera.FieldOfView = 70

local orbiting = true
local orbitAngle = 0.6
local position = Vector3.new(30, 14, 30)
local yaw, pitch = 0, 0
local looking = false

local SPEED = 16
local FAST = 4
local TURN = 2 -- radians per second with the arrow keys
local MOUSE_TURN = 0.004 -- radians per pixel

local function down(key: EnumKeyCode): number
    return if UserInputService:IsKeyDown(key) then 1 else 0
end

local function leaveOrbit()
    if orbiting then
        orbiting = false
        position = camera.CFrame.Position
        local look = camera.CFrame.LookVector
        yaw = math.atan2(-look.X, -look.Z)
        pitch = math.asin(math.clamp(look.Y, -1, 1))
    end
end

RunService.RenderStepped:Connect(function(deltaTime: number)
    -- Hold the right mouse button to look around: the cursor stays where it was, and its movement turns the camera.
    local wantLook = UserInputService:IsMouseButtonPressed(Enum.UserInputType.MouseButton2)
    if wantLook ~= looking then
        looking = wantLook
        UserInputService.MouseBehavior = if looking
            then Enum.MouseBehavior.LockCurrentPosition
            else Enum.MouseBehavior.Default
    end

    local forward = down(Enum.KeyCode.W) - down(Enum.KeyCode.S)
    local right = down(Enum.KeyCode.D) - down(Enum.KeyCode.A)
    local up = down(Enum.KeyCode.E) - down(Enum.KeyCode.Q)
    local turnX = down(Enum.KeyCode.Right) - down(Enum.KeyCode.Left)
    local turnY = down(Enum.KeyCode.Up) - down(Enum.KeyCode.Down)
    local mouse = if looking then UserInputService:GetMouseDelta() else Vector2.zero

    if forward ~= 0 or right ~= 0 or up ~= 0 or turnX ~= 0 or turnY ~= 0 or mouse.Magnitude > 0 then
        leaveOrbit()
    end

    if orbiting then
        orbitAngle += deltaTime * 0.15
        local eye = Vector3.new(math.sin(orbitAngle) * 50, 22, math.cos(orbitAngle) * 50)
        camera.CFrame = CFrame.lookAt(eye, Vector3.new(0, 0, -2))
        return
    end

    yaw -= turnX * TURN * deltaTime + mouse.X * MOUSE_TURN
    pitch += turnY * TURN * deltaTime - mouse.Y * MOUSE_TURN
    pitch = math.clamp(pitch, -1.5, 1.5)
    local rotation = CFrame.fromOrientation(pitch, yaw, 0)

    local speed = SPEED * (if UserInputService:IsKeyDown(Enum.KeyCode.LeftShift) then FAST else 1)
    local move = rotation.LookVector * forward + rotation.RightVector * right + Vector3.yAxis * up
    if move.Magnitude > 0 then
        position += move.Unit * speed * deltaTime
    end
    camera.CFrame = CFrame.new(position) * rotation
end)

UserInputService.InputBegan:Connect(function(input: InputObject)
    if input.KeyCode == Enum.KeyCode.Space then
        paused = not paused
    elseif input.KeyCode == Enum.KeyCode.C then
        orbiting = true
    end
end)

print(`Metallic demo: {#workspace:GetChildren()} objects in Workspace. WASD to move, hold the right mouse button to look.`)
