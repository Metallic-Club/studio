//! `CFrame` and `BasePart.CFrame` against values worked out by hand.

mod common;

use common::Rig;
use metallic_core::{Value, Vec3};

/// Shared Luau helpers: `near(a, b)` for numbers and Vector3s, `same(a, b)` for CFrames.
const HELPERS: &str = r#"
local EPS = 1e-9
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
"#;

fn run(body: &str) -> Vec<String> {
    let mut rig = Rig::new();
    rig.server_script("S", &format!("{HELPERS}\n{body}\nprint('ok')"));
    rig.host.start().expect("host starts");
    rig.lines()
}

#[test]
fn constructors_match_roblox() {
    let lines = run(r#"
        local V = Vector3.new
        near(CFrame.new().Position, V(0, 0, 0), "identity")
        same(CFrame.identity, CFrame.new(), "identity constant")
        near(CFrame.new(1, 2, 3) * V(1, 0, 0), V(2, 2, 3), "translation")
        near(CFrame.new(V(4, 5, 6)).Position, V(4, 5, 6), "from Vector3")
        -- Quaternion overload: a quarter turn about Y is (0, sin 45, 0, cos 45).
        local h = math.sqrt(0.5)
        same(CFrame.new(0, 0, 0, 0, h, 0, h), CFrame.Angles(0, math.pi / 2, 0), "quaternion")
        -- Twelve numbers are the position then the rotation row by row.
        local m = CFrame.new(1, 2, 3, 0, 0, 1, 0, 1, 0, -1, 0, 0)
        same(m, CFrame.new(1, 2, 3) * CFrame.Angles(0, math.pi / 2, 0), "matrix")
        -- new(pos, lookAt) and lookAt: looking along +X puts the right vector on +Z.
        local look = CFrame.lookAt(V(1, 0, 0), V(11, 0, 0))
        near(look.LookVector, V(1, 0, 0), "lookAt look")
        near(look.RightVector, V(0, 0, 1), "lookAt right")
        near(look.UpVector, V(0, 1, 0), "lookAt up")
        same(CFrame.new(V(1, 0, 0), V(11, 0, 0)), look, "new(pos, target)")
        near(CFrame.lookAt(V(0, 0, 0), V(0, 10, 0)).LookVector, V(0, 1, 0), "look straight up")
        same(CFrame.fromAxisAngle(V(0, 2, 0), math.pi / 2), CFrame.Angles(0, math.pi / 2, 0), "axis angle")
        same(CFrame.fromMatrix(V(0, 0, 0), V(0, 0, -1), V(0, 1, 0)), CFrame.Angles(0, math.pi / 2, 0), "fromMatrix")
    "#);
    assert_eq!(lines, ["print:ok"]);
}

#[test]
fn euler_angle_orders() {
    let lines = run(r#"
        local A = CFrame.Angles
        local x, y, z = 0.3, -0.5, 1.2
        -- XYZ: Z is applied first, then Y, then X.
        same(A(x, y, z), A(x, 0, 0) * A(0, y, 0) * A(0, 0, z), "Angles")
        same(CFrame.fromEulerAnglesXYZ(x, y, z), A(x, y, z), "fromEulerAnglesXYZ")
        -- YXZ (Orientation order): Z first, then X, then Y.
        local yxz = A(0, y, 0) * A(x, 0, 0) * A(0, 0, z)
        same(CFrame.fromEulerAnglesYXZ(x, y, z), yxz, "fromEulerAnglesYXZ")
        same(CFrame.fromOrientation(x, y, z), yxz, "fromOrientation")
        same(CFrame.fromEulerAngles(x, y, z), A(x, y, z), "default order")
        same(CFrame.fromEulerAngles(x, y, z, Enum.RotationOrder.ZYX), A(0, 0, z) * A(0, y, 0) * A(x, 0, 0), "ZYX")
        local rx, ry, rz = A(x, y, z):ToEulerAnglesXYZ()
        near(rx, x, "rx"); near(ry, y, "ry"); near(rz, z, "rz")
        rx, ry, rz = yxz:ToOrientation()
        near(rx, x, "ox"); near(ry, y, "oy"); near(rz, z, "oz")
        rx, ry, rz = yxz:ToEulerAnglesYXZ()
        near(ry, y, "yxz y")
        rx, ry, rz = CFrame.fromEulerAngles(x, y, z, Enum.RotationOrder.ZXY):ToEulerAngles(Enum.RotationOrder.ZXY)
        near(rx, x, "zxy x"); near(ry, y, "zxy y"); near(rz, z, "zxy z")
        local axis, angle = A(0, 0.5, 0):ToAxisAngle()
        near(axis, Vector3.new(0, 1, 0), "axis"); near(angle, 0.5, "angle")
    "#);
    assert_eq!(lines, ["print:ok"]);
}

#[test]
fn vectors_spaces_and_operators() {
    let lines = run(r#"
        local V = Vector3.new
        local turn = CFrame.Angles(0, math.pi / 2, 0)
        -- A quarter turn about Y: look (-Z) goes to -X, right (+X) goes to -Z.
        near(turn.LookVector, V(-1, 0, 0), "look")
        near(turn.RightVector, V(0, 0, -1), "right")
        near(turn.UpVector, V(0, 1, 0), "up")
        near(turn.XVector, turn.RightVector, "XVector")
        near(turn.ZVector, -turn.LookVector, "ZVector")
        local cf = CFrame.new(10, 0, 0) * turn
        near(cf:PointToWorldSpace(V(1, 0, 0)), V(10, 0, -1), "point to world")
        near(cf:PointToObjectSpace(V(10, 0, -1)), V(1, 0, 0), "point to object")
        near(cf:VectorToWorldSpace(V(1, 0, 0)), V(0, 0, -1), "vector to world")
        near(cf:VectorToObjectSpace(V(0, 0, -1)), V(1, 0, 0), "vector to object")
        local a, b = cf:PointToWorldSpace(V(0, 0, 0), V(0, 1, 0))
        near(a, V(10, 0, 0), "tuple 1"); near(b, V(10, 1, 0), "tuple 2")
        same(CFrame.new(5, 0, 0):ToObjectSpace(CFrame.new(7, 1, 0)), CFrame.new(2, 1, 0), "to object")
        same(CFrame.new(5, 0, 0):ToWorldSpace(CFrame.new(2, 1, 0)), CFrame.new(7, 1, 0), "to world")
        local any = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.2, 0.1)
        same(any:Inverse() * any, CFrame.new(), "inverse")
        same(any * any:Inverse(), CFrame.new(), "inverse right")
        same(cf + V(0, 5, 0), CFrame.new(10, 5, 0) * turn, "plus")
        same(cf - V(10, 0, 0), turn, "minus")
        near(cf.Position, V(10, 0, 0), "Position")
        near(cf.X + cf.Y + cf.Z, 10, "XYZ")
        same(cf.Rotation, turn, "Rotation")
        -- Lerp: position halfway, rotation half the turn.
        local mid = CFrame.new():Lerp(cf, 0.5)
        near(mid.Position, V(5, 0, 0), "lerp position")
        local _, ry = mid:ToEulerAnglesXYZ()
        near(ry, math.pi / 4, "lerp angle")
        same(CFrame.new():Lerp(cf, 0), CFrame.new(), "lerp 0")
        same(CFrame.new():Lerp(cf, 1), cf, "lerp 1")
        print(table.concat({CFrame.new(1, 2, 3):GetComponents()}, " "))
        print(tostring(CFrame.new(1, 2, 3)))
        print(CFrame.new(1, 2, 3) == CFrame.new(1, 2, 3), CFrame.new(1, 2, 3) == CFrame.new(1, 2, 4), typeof(cf))
    "#);
    assert_eq!(
        lines,
        [
            "print:1 2 3 1 0 0 0 1 0 0 0 1",
            "print:1, 2, 3, 1, 0, 0, 0, 1, 0, 0, 0, 1",
            "print:true false CFrame",
            "print:ok"
        ]
    );
}

#[test]
fn errors_read_like_roblox() {
    let lines = run(r#"
        local cf = CFrame.new()
        print(pcall(function() return cf.Nope end))
        print(pcall(function() cf.X = 1 end))
        print(pcall(function() return cf * 5 end))
        print(pcall(function() return CFrame.new("a") end))
        print(pcall(function() return CFrame.fromEulerAngles(0, 0, 0, Enum.KeyCode.Space) end))
    "#);
    assert!(
        lines[0].contains("Nope is not a valid member of CFrame"),
        "{lines:?}"
    );
    assert!(lines[1].contains("X cannot be assigned to"), "{lines:?}");
    assert!(
        lines[2].contains("multiplied by a CFrame or a Vector3"),
        "{lines:?}"
    );
    assert!(lines[3].contains("CFrame.new"), "{lines:?}");
    assert!(lines[4].contains("Enum.RotationOrder"), "{lines:?}");
    assert!(
        lines
            .iter()
            .all(|l| l.starts_with("print:false") || l == "print:ok"),
        "{lines:?}"
    );
}

#[test]
fn part_cframe_maps_onto_position_and_orientation() {
    let mut rig = Rig::new();
    let ws = rig.root("Workspace");
    let part = rig.add("Part", "Brick", ws);
    let other = rig.add("Part", "Other", ws);
    rig.server_script(
        "S",
        &format!(
            "{HELPERS}\n{}",
            r#"
            local p, o = workspace.Brick, workspace.Other
            p.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0, math.rad(90), 0)
            near(p.Position, Vector3.new(1, 2, 3), "position")
            near(p.Orientation, Vector3.new(0, 90, 0), "orientation")
            o.Position = Vector3.new(4, 5, 6)
            o.Orientation = Vector3.new(30, 45, 60)
            local x, y, z = o.CFrame:ToOrientation()
            near(x, math.rad(30), "x"); near(y, math.rad(45), "y"); near(z, math.rad(60), "z")
            same(o.CFrame, CFrame.new(4, 5, 6) * CFrame.fromOrientation(math.rad(30), math.rad(45), math.rad(60)), "read")
            -- Straight up (gimbal lock) still stores a usable Orientation.
            o.CFrame = CFrame.Angles(math.pi / 2, 0, 0)
            near(o.Orientation, Vector3.new(90, 0, 0), "gimbal")
            print(pcall(function() p.CFrame = Vector3.new() end))
            print(pcall(function() p.CFrame = CFrame.new(0 / 0, 0, 0) end))
            print(pcall(function() return workspace.CFrame end))
            print("ok")
            "#
        ),
    );
    rig.host.start().expect("host starts");
    let lines = rig.lines();
    assert!(
        lines[0].contains("expected CFrame, got userdata"),
        "{lines:?}"
    );
    assert!(lines[1].contains("finite"), "{lines:?}");
    assert!(
        lines[2].contains("CFrame is not a valid member of Workspace"),
        "{lines:?}"
    );
    assert_eq!(lines[3], "print:ok", "{lines:?}");
    let world = rig.world.borrow();
    // The failed non-finite write changed nothing.
    assert_eq!(
        world.get(part, "Position").unwrap(),
        Value::Vector3(Vec3::new(1.0, 2.0, 3.0))
    );
    let Value::Vector3(o) = world.get(other, "Orientation").unwrap() else {
        panic!("Orientation is a Vector3");
    };
    assert!(
        (o.x - 90.0).abs() < 1e-9 && o.y.abs() < 1e-9 && o.z.abs() < 1e-9,
        "{o:?}"
    );
}
