//! Pivots, Resize, ScaleTo and GetBoundingBox (issue #216) against values worked out by hand.

mod common;

use common::Rig;

/// `near(a, b, what)` for numbers and Vector3s, `same(a, b, what)` for CFrames, and `part(...)`.
const HELPERS: &str = r#"
local EPS = 1e-6
local V = Vector3.new
local function near(a, b, what)
    local ok
    if type(a) == "number" then
        ok = math.abs(a - b) < EPS
    else
        ok = (a - b).Magnitude < EPS
    end
    if not ok then error(what .. ": got " .. tostring(a) .. ", want " .. tostring(b), 2) end
end
local function same(a, b, what)
    if not a:FuzzyEq(b, EPS) then
        error(what .. ": got " .. tostring(a) .. ", want " .. tostring(b), 2)
    end
end
local function part(name, parent, position, size)
    local p = Instance.new("Part")
    p.Name = name
    p.Anchored = true
    p.Position = position
    if size then p.Size = size end
    p.Parent = parent
    return p
end
"#;

fn run(body: &str) -> Vec<String> {
    let mut rig = Rig::new();
    rig.server_script("S", &format!("{HELPERS}\n{body}\nprint('ok')"));
    rig.host.start().expect("host starts");
    rig.lines()
}

fn assert_ok(lines: Vec<String>) {
    assert_eq!(lines, ["print:ok"]);
}

#[test]
fn part_pivot_round_trip_on_a_rotated_part() {
    assert_ok(run(r#"
        local p = part("P", workspace, V(0, 0, 0))
        near(p.PivotOffset.Position, V(0, 0, 0), "default offset")
        same(p:GetPivot(), p.CFrame, "pivot is the centre by default")
        p.CFrame = CFrame.new(10, 0, 0) * CFrame.Angles(0, math.pi / 2, 0)
        p.PivotOffset = CFrame.new(0, -1, 2)
        -- A quarter turn about Y maps local (0, -1, 2) to world (2, -1, 0).
        near(p:GetPivot().Position, V(12, -1, 0), "rotated pivot")
        same(p:GetPivot(), p.CFrame * p.PivotOffset, "pivot = CFrame * PivotOffset")
        -- The pivot lands on the target; the part sits at target * PivotOffset:Inverse().
        p:PivotTo(CFrame.new(0, 5, 0))
        same(p:GetPivot(), CFrame.new(0, 5, 0), "pivot landed")
        near(p.Position, V(0, 6, -2), "part position")
        near(p.Orientation, V(0, 0, 0), "part orientation")
        local before = p.CFrame
        p:PivotTo(p:GetPivot())
        same(p.CFrame, before, "round trip")
    "#));
}

#[test]
fn model_without_primary_part() {
    assert_ok(run(r#"
        local m = Instance.new("Model", workspace)
        local a = part("A", m, V(0, 0, 0), V(2, 2, 2))
        local b = part("B", m, V(10, 0, 0), V(2, 2, 2))
        if m.PrimaryPart ~= nil then error("PrimaryPart defaults to nil") end
        -- Never set: the pivot is the centre of the bounding box.
        same(m:GetPivot(), CFrame.new(5, 0, 0), "bbox pivot")
        same(m.WorldPivot, CFrame.new(5, 0, 0), "WorldPivot reads the bbox centre")
        b.Position = V(20, 0, 0)
        same(m:GetPivot(), CFrame.new(10, 0, 0), "follows the parts while unset")
        b.Position = V(10, 0, 0)
        m:PivotTo(CFrame.new(5, 10, 0))
        near(a.Position, V(0, 10, 0), "a moved")
        near(b.Position, V(10, 10, 0), "b moved")
        same(m.WorldPivot, CFrame.new(5, 10, 0), "PivotTo sets WorldPivot")
        -- Once set, the pivot no longer follows the parts.
        b.Position = V(20, 10, 0)
        same(m:GetPivot(), CFrame.new(5, 10, 0), "set pivot stays")
        m.WorldPivot = CFrame.new(0, 10, 0)
        m:PivotTo(CFrame.new(1, 10, 0))
        near(a.Position, V(1, 10, 0), "a by WorldPivot")
        near(b.Position, V(21, 10, 0), "b by WorldPivot")
        -- Turning a quarter about Y around the pivot (0, 10, 0) after moving it there.
        m.WorldPivot = CFrame.new(0, 10, 0)
        m:PivotTo(CFrame.new(0, 10, 0) * CFrame.Angles(0, math.pi / 2, 0))
        near(a.Position, V(0, 10, -1), "a turned")
        near(b.Position, V(0, 10, -21), "b turned")
        near(a.Orientation, V(0, 90, 0), "a orientation")
        near(m:GetPivot().LookVector, V(-1, 0, 0), "pivot turned")
    "#));
}

#[test]
fn model_with_primary_part() {
    assert_ok(run(r#"
        local m = Instance.new("Model", workspace)
        local a = part("A", m, V(0, 0, 0))
        local b = part("B", m, V(4, 0, 0))
        local outside = part("Outside", workspace, V(100, 0, 0))
        m.PrimaryPart = a
        a.PivotOffset = CFrame.new(0, 1, 0)
        same(m:GetPivot(), CFrame.new(0, 1, 0), "PrimaryPart pivot")
        m:PivotTo(CFrame.new(0, 11, 0))
        near(a.Position, V(0, 10, 0), "a")
        near(b.Position, V(4, 10, 0), "b")
        same(m:GetPivot(), CFrame.new(0, 11, 0), "landed")
        -- A PrimaryPart outside the model is ignored: the unset WorldPivot is the bbox centre.
        -- a spans x -2..2 (default size 4, 1, 2), b spans 2..6.
        m.PrimaryPart = outside
        same(m:GetPivot(), CFrame.new(2, 10, 0), "outside PrimaryPart ignored")
        near(outside.Position, V(100, 0, 0), "outside untouched")
    "#));
}

#[test]
fn nested_models_move_and_keep_their_pivots() {
    assert_ok(run(r#"
        local outer = Instance.new("Model", workspace)
        local p1 = part("P1", outer, V(0, 0, 0))
        local folder = Instance.new("Folder", outer)
        local f1 = part("F1", folder, V(0, 0, 3))
        local inner = Instance.new("Model", outer)
        local p2 = part("P2", inner, V(6, 0, 0))
        local p3 = part("P3", inner, V(8, 0, 0))
        inner.WorldPivot = CFrame.new(7, 0, 0)
        outer.WorldPivot = CFrame.new(0, 0, 0)
        -- A quarter turn about Y maps (x, 0, z) to (z, 0, -x), then up 5.
        outer:PivotTo(CFrame.new(0, 5, 0) * CFrame.Angles(0, math.pi / 2, 0))
        near(p1.Position, V(0, 5, 0), "p1")
        near(f1.Position, V(3, 5, 0), "part in a folder")
        near(p2.Position, V(0, 5, -6), "p2")
        near(p3.Position, V(0, 5, -8), "p3")
        same(inner:GetPivot(), CFrame.new(0, 5, -7) * CFrame.Angles(0, math.pi / 2, 0), "inner pivot moved")
        -- Moving the inner model moves only its parts.
        inner:PivotTo(inner:GetPivot() + V(0, 1, 0))
        near(p2.Position, V(0, 6, -6), "p2 up")
        near(p3.Position, V(0, 6, -8), "p3 up")
        near(p1.Position, V(0, 5, 0), "p1 stays")
        near(outer:GetPivot().Position, V(0, 5, 0), "outer pivot stays")
    "#));
}

#[test]
fn resize_each_face() {
    let lines = run(r#"
        local p = part("P", workspace, V(0, 0, 0), V(4, 1, 2))
        print(p:Resize(Enum.NormalId.Top, 2))
        near(p.Size, V(4, 3, 2), "top size"); near(p.Position, V(0, 1, 0), "top position")
        print(p:Resize(Enum.NormalId.Bottom, 1))
        near(p.Size, V(4, 4, 2), "bottom size"); near(p.Position, V(0, 0.5, 0), "bottom position")
        print(p:Resize(Enum.NormalId.Right, 1))
        near(p.Size, V(5, 4, 2), "right size"); near(p.Position, V(0.5, 0.5, 0), "right position")
        print(p:Resize(Enum.NormalId.Left, 1))
        near(p.Size, V(6, 4, 2), "left size"); near(p.Position, V(0, 0.5, 0), "left position")
        print(p:Resize(Enum.NormalId.Back, 2))
        near(p.Size, V(6, 4, 4), "back size"); near(p.Position, V(0, 0.5, 1), "back position")
        -- Shrinking the front (-Z) face moves the centre towards +Z.
        print(p:Resize(Enum.NormalId.Front, -1))
        near(p.Size, V(6, 4, 3), "front size"); near(p.Position, V(0, 0.5, 1.5), "front position")
        -- The number (1 = Top) and the name work too.
        p:Resize(1, 1); p:Resize("Top", 1)
        near(p.Size, V(6, 6, 3), "number and name")
        -- The size stops at 0.001 and the opposite face stays where it was (bottom at y = -1.5).
        p:Resize(Enum.NormalId.Top, -100)
        near(p.Size.Y, 0.001, "floor"); near(p.Position.Y, -1.5 + 0.0005, "floor position")

        -- A part turned a quarter about Y: its local +X (Right) points to world -Z.
        local q = part("Q", workspace, V(0, 20, 0), V(4, 1, 2))
        q.Orientation = V(0, 90, 0)
        q:Resize(Enum.NormalId.Right, 2)
        near(q.Size, V(6, 1, 2), "rotated size"); near(q.Position, V(0, 20, -1), "rotated position")

        -- Ball grows on every axis; a Cylinder's Y and Z grow together.
        local ball = part("Ball", workspace, V(0, 40, 0), V(2, 2, 2))
        ball.Shape = "Ball"
        ball:Resize(Enum.NormalId.Top, 2)
        near(ball.Size, V(4, 4, 4), "ball size"); near(ball.Position, V(0, 41, 0), "ball position")
        local cyl = part("Cyl", workspace, V(0, 60, 0), V(4, 2, 2))
        cyl.Shape = "Cylinder"
        cyl:Resize(Enum.NormalId.Back, 2)
        near(cyl.Size, V(4, 4, 4), "cylinder size"); near(cyl.Position, V(0, 60, 1), "cylinder position")
        print(pcall(function() p:Resize(Enum.KeyCode.Space, 1) end))
    "#);
    let (last, rest) = lines.split_last().expect("output");
    assert_eq!(last, "print:ok", "{lines:?}");
    let (bad, rest) = rest.split_last().expect("output");
    assert!(
        bad.starts_with("print:false") && bad.contains("Enum.NormalId"),
        "{bad}"
    );
    assert!(
        rest.iter().all(|l| l == "print:true") && rest.len() == 6,
        "{lines:?}"
    );
}

#[test]
fn resize_refuses_to_grow_into_a_colliding_part() {
    let lines = run(r#"
        -- Wall spans x 9..11.
        local wall = part("Wall", workspace, V(10, 0, 0), V(2, 10, 10))
        local p = part("P", workspace, V(0, 0, 0), V(4, 1, 2))
        print(p:Resize(Enum.NormalId.Right, 5))   -- right face 2 -> 7
        print(p:Resize(Enum.NormalId.Right, 2))   -- 7 -> 9: touching is allowed
        print(p:Resize(Enum.NormalId.Right, 1))   -- 9 -> 10: would overlap
        near(p.Size, V(11, 1, 2), "unchanged after refusal")
        near(p.Position, V(3.5, 0, 0), "position unchanged")
        print(p:Resize(Enum.NormalId.Left, 1))    -- the other side is free
        wall.CanCollide = false
        print(p:Resize(Enum.NormalId.Right, 1))   -- a non-colliding wall does not block
        p.CanCollide = false
        wall.CanCollide = true
        print(p:Resize(Enum.NormalId.Right, 1))   -- nor does it block a non-colliding part
    "#);
    assert_eq!(
        lines,
        [
            "print:true",
            "print:true",
            "print:false",
            "print:true",
            "print:true",
            "print:true",
            "print:ok"
        ]
    );
}

#[test]
fn scale_to_keeps_relative_offsets() {
    assert_ok(
        run(r#"
        local m = Instance.new("Model", workspace)
        local a = part("A", m, V(0, 0, 0), V(4, 1, 2))
        local b = part("B", m, V(10, 0, 0), V(2, 2, 2))
        b.Orientation = V(0, 90, 0)
        b.PivotOffset = CFrame.new(1, 0, 0)
        local inner = Instance.new("Model", m)
        local c = part("C", inner, V(0, 4, 0), V(1, 1, 1))
        m.WorldPivot = CFrame.new(0, 0, 0)
        near(m:GetScale(), 1, "initial scale")
        m:ScaleTo(2)
        near(m:GetScale(), 2, "scale 2")
        near(a.Size, V(8, 2, 4), "a size")
        near(b.Size, V(4, 4, 4), "b size")
        near(b.Position - a.Position, V(20, 0, 0), "offset doubled")
        near(c.Position, V(0, 8, 0), "nested part")
        near(b.Orientation, V(0, 90, 0), "orientation kept")
        near(b.PivotOffset.Position, V(2, 0, 0), "PivotOffset scaled")
        near(inner:GetScale(), 2, "nested scale")
        same(m:GetPivot(), CFrame.new(0, 0, 0), "pivot fixed")
        -- Scale is absolute, as in Roblox: 0.5 is half the original size.
        m:ScaleTo(0.5)
        near(a.Size, V(2, 0.5, 1), "half size")
        near(b.Position, V(5, 0, 0), "half position")
        -- 2 and then half of that returns the original sizes.
        m:ScaleTo(2)
        m:ScaleTo(m:GetScale() * 0.5)
        near(m:GetScale(), 1, "back to 1")
        near(a.Size, V(4, 1, 2), "a original")
        near(b.Size, V(2, 2, 2), "b original")
        near(b.Position, V(10, 0, 0), "b position original")
        near(c.Position, V(0, 4, 0), "c original")
        near(b.PivotOffset.Position, V(1, 0, 0), "PivotOffset original")
        near(inner:GetScale(), 1, "nested original")

        -- Without a set pivot the model scales about its bounding-box centre.
        local n = Instance.new("Model", workspace)
        local d = part("D", n, V(100, 0, 0), V(2, 2, 2))
        local e = part("E", n, V(104, 0, 0), V(2, 2, 2))
        n:ScaleTo(3)
        near(d.Position, V(96, 0, 0), "d about centre")
        near(e.Position, V(108, 0, 0), "e about centre")
        print(pcall(function() n:ScaleTo(0) end))
        print(pcall(function() n:ScaleTo(-1) end))
    "#)
        .into_iter()
        .filter(|l| !(l.starts_with("print:false") && l.contains("positive")))
        .collect(),
    );
}

#[test]
fn bounding_box_of_rotated_parts() {
    assert_ok(run(r#"
        local m = Instance.new("Model", workspace)
        local p = part("P", m, V(0, 0, 0), V(4, 1, 2))
        p.Orientation = V(0, 90, 0)
        local cf, size = m:GetBoundingBox()
        same(cf, CFrame.new(), "box frame")
        near(size, V(2, 1, 4), "quarter turn swaps X and Z")
        -- At 45 degrees each horizontal extent is 4 cos 45 + 2 sin 45 = 3 * sqrt(2).
        p.Orientation = V(0, 45, 0)
        cf, size = m:GetBoundingBox()
        near(size, V(3 * math.sqrt(2), 1, 3 * math.sqrt(2)), "45 degrees")
        near(m:GetExtentsSize(), size, "GetExtentsSize")
        -- With a PrimaryPart the box takes its orientation and fits the part exactly.
        m.PrimaryPart = p
        cf, size = m:GetBoundingBox()
        near(size, V(4, 1, 2), "aligned with PrimaryPart")
        same(cf, p.CFrame, "frame of the PrimaryPart")

        local two = Instance.new("Model", workspace)
        part("A", two, V(0, 0, 0), V(2, 2, 2))
        part("B", two, V(10, 4, 0), V(2, 2, 2))
        cf, size = two:GetBoundingBox()
        near(size, V(12, 6, 2), "two parts size")
        same(cf, CFrame.new(5, 2, 0), "two parts centre")

        local empty = Instance.new("Model", workspace)
        empty.WorldPivot = CFrame.new(3, 3, 3)
        cf, size = empty:GetBoundingBox()
        same(cf, CFrame.new(3, 3, 3), "empty model: its pivot")
        near(size, V(0, 0, 0), "empty model: size 0")
    "#));
}

#[test]
fn members_and_errors() {
    let lines = run(r#"
        local m = Instance.new("Model", workspace)
        local p = part("P", m, V(0, 0, 0))
        local f = Instance.new("Folder", workspace)
        print(p:IsA("PVInstance"), m:IsA("PVInstance"), f:IsA("PVInstance"))
        print(pcall(function() return f:GetPivot() end))
        print(pcall(function() return p:ScaleTo(2) end))
        print(pcall(function() return m:Resize(Enum.NormalId.Top, 1) end))
        print(pcall(function() return p.WorldPivot end))
        print(pcall(function() return m.ScaleFactor end))
        print(pcall(function() m.WorldPivotSet = true end))
        print(pcall(function() m:PivotTo(V(1, 2, 3)) end))
        print(pcall(function() p.PivotOffset = V(1, 2, 3) end))
        -- A clone keeps its pivot settings.
        m.WorldPivot = CFrame.new(1, 2, 3)
        m:ScaleTo(2)
        local copy = m:Clone()
        same(copy:GetPivot(), CFrame.new(1, 2, 3), "clone pivot")
        near(copy:GetScale(), 2, "clone scale")
    "#);
    assert_eq!(lines[0], "print:true true false");
    let expect = [
        "GetPivot is not a valid member of Folder",
        "ScaleTo is not a valid member of Part",
        "Resize is not a valid member of Model",
        "WorldPivot is not a valid member of Part",
        "ScaleFactor is not a valid member of Model",
        "WorldPivotSet is not a valid member of Model",
        "PivotTo: expected a CFrame",
        "invalid value for property PivotOffset: expected CFrame",
    ];
    for (line, want) in lines[1..].iter().zip(expect) {
        assert!(
            line.starts_with("print:false") && line.contains(want),
            "{line} lacks {want}"
        );
    }
    assert_eq!(
        lines.last().map(String::as_str),
        Some("print:ok"),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 10, "{lines:?}");
}

#[test]
fn scale_to_and_pivot_to_change_nothing_when_they_fail() {
    let lines = run(r#"
        local m = Instance.new("Model", workspace)
        local a = part("A", m, V(0, 0, 0), V(4, 1, 2))
        local inner = Instance.new("Model", m)
        local b = part("B", inner, V(10, 0, 0), V(2, 2, 2))
        m.WorldPivot = CFrame.new()
        m:ScaleTo(2)
        -- 1000 would make A 4000 studs long: past the 2048 limit.
        print(pcall(function() m:ScaleTo(1000) end))
        near(m:GetScale(), 2, "scale kept")
        near(inner:GetScale(), 2, "nested scale kept")
        near(a.Size, V(8, 2, 4), "a size kept")
        near(b.Size, V(4, 4, 4), "b size kept")
        near(b.Position, V(20, 0, 0), "b position kept")

        -- Sizes in range, but a position would overflow to infinity.
        local far = Instance.new("Model", workspace)
        local near0 = part("Near", far, V(0, 0, 0), V(0.001, 0.001, 0.001))
        local edge = part("Edge", far, V(1e303, 0, 0), V(0.001, 0.001, 0.001))
        far.WorldPivot = CFrame.new()
        print(pcall(function() far:ScaleTo(1e6) end))
        near(far:GetScale(), 1, "far scale kept")
        near(near0.Size, V(0.001, 0.001, 0.001), "near size kept")
        if edge.Position.X ~= 1e303 then error("edge moved") end

        -- PivotTo: Edge (at 1e308) would land at 2e308, past the largest number; Near is moved
        -- first in tree order and must not move.
        edge.Position = V(1e308, 0, 0)
        print(pcall(function() far:PivotTo(CFrame.new(1e308, 0, 0)) end))
        near(near0.Position, V(0, 0, 0), "near position kept")
        same(far.WorldPivot, CFrame.new(), "WorldPivot kept")
    "#);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[0].starts_with("print:false") && lines[0].contains("2048"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("print:false") && lines[1].contains("nothing was changed"),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("print:false") && lines[2].contains("nothing was changed"),
        "{lines:?}"
    );
    assert_eq!(lines[3], "print:ok");
}

#[test]
fn part_size_is_capped_at_2048_studs() {
    assert_ok(
        run(r#"
        local p = part("P", workspace, V(0, 0, 0), V(4, 1, 2))
        p.CanCollide = false
        p.Size = V(5000, 1, 0)
        near(p.Size, V(2048, 1, 0.001), "assignment clamps")
        p.Size = V(4, 1, 2)
        -- Resize stops at 2048 and keeps the left face (x = -2) where it was.
        print(p:Resize(Enum.NormalId.Right, 5000))
        near(p.Size, V(2048, 1, 2), "resize clamps")
        near(p.Position, V(-2 + 1024, 0, 0), "left face kept")
    "#)
        .into_iter()
        .filter(|l| l != "print:true")
        .collect(),
    );
}

#[test]
fn pivot_offset_is_made_orthonormal() {
    let lines = run(r#"
        local p = part("P", workspace, V(0, 0, 0))
        -- A scaled rotation (2 on the diagonal) is stored as the plain rotation.
        p.PivotOffset = CFrame.new(1, 2, 3, 2, 0, 0, 0, 2, 0, 0, 0, 2)
        same(p.PivotOffset, CFrame.new(1, 2, 3), "scaled matrix")
        -- A skewed one: right (1, 0, 0), up (1, 1, 0); Gram-Schmidt keeps right, fixes up.
        p.PivotOffset = CFrame.new(0, -1, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1)
        same(p.PivotOffset, CFrame.new(0, -1, 0), "skewed matrix")
        local target = CFrame.new(5, 6, 7) * CFrame.Angles(0.3, 0.6, 0.9)
        p:PivotTo(target)
        same(p:GetPivot(), target, "round trip")
        -- A skewed target (fromMatrix keeps its axes as given) lands as its orthonormal version.
        local skewed = CFrame.fromMatrix(V(1, 1, 1), V(1, 0, 0), V(1, 1, 0), V(0, 0, 1))
        p:PivotTo(skewed)
        same(p:GetPivot(), skewed:Orthonormalize(), "skewed target")
        local zero = CFrame.fromMatrix(V(0, 0, 0), V(0, 0, 0), V(0, 0, 0), V(0, 0, 0))
        print(pcall(function() p.PivotOffset = zero end))
        print(pcall(function() p:PivotTo(zero) end))
    "#);
    assert_eq!(lines.len(), 3, "{lines:?}");
    for line in &lines[..2] {
        assert!(
            line.starts_with("print:false") && line.contains("degenerate"),
            "{lines:?}"
        );
    }
    assert_eq!(lines[2], "print:ok");
}
