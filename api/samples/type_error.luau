--!strict
-- Deliberately wrong: CI checks that luau-lsp rejects this file with api/metallic.d.luau, which proves the
-- definitions are loaded and actually constrain scripts. Each line below is one type error.

local part = Instance.new("Part")
part.Position = "up"
part.Material = Enum.PartShape.Ball
local missing = part:FindFirstChild(42)
print(missing, part.Size.Q)
