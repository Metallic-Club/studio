--!strict
-- Type-checked in CI against api/metallic.d.luau with luau-lsp (the `luau-types` job in .github/workflows/ci.yml).
-- It must check cleanly.

local part = Instance.new("Part")
part.Name = "Door"
part.Position = Vector3.new(0, 5, 0)
part.Size = Vector3.one * 4
part.Color = Color3.fromRGB(200, 120, 40)
part.Material = Enum.Material.Plastic
part.Shape = Enum.PartShape.Ball
part.Anchored = true
part.Parent = workspace

local height: number = part.Position.Y + part.Size.Magnitude
local pivot: CFrame = part:GetPivot() * CFrame.Angles(0, math.pi / 2, 0)
part:PivotTo(pivot + Vector3.yAxis)
local material: EnumMaterial = part.Material
local materialName: string = material.Name

local found = workspace:FindFirstChild("Door")
if found and found:IsA("BasePart") then
    print(found:GetFullName())
end

local connection = part.Changed:Connect(function(property: string)
    print("changed", property)
end)
part:GetPropertyChangedSignal("Position"):Once(function()
    print("moved")
end)
workspace.ChildAdded:Connect(function(child: Instance)
    print(child.Name, child.ClassName)
end)

local score = Instance.new("IntValue", part)
score.Changed:Connect(function(value: number)
    print("score", value + 1)
end)

local ask = Instance.new("BindableFunction")
ask.OnInvoke = function(question: string): string
    return question .. "?"
end

local camera = workspace.CurrentCamera
if camera then
    camera.FieldOfView = 70
end

task.spawn(function()
    local waited: number = task.wait(0.5)
    print(waited, time(), height, materialName)
    connection:Disconnect()
end)

for _, item in Enum.Material:GetEnumItems() do
    print(item.Name, item.Value)
end
