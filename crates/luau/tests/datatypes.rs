//! Data types from Luau, checked against `metallic_core`'s maths and round-tripped through properties.

mod support;

use metallic_core::glam::{EulerRot, Quat};
use metallic_core::{CFrame, Color3, ScriptValue, Value, Vector3};
use metallic_script::Resume;
use support::Game;

/// Runs `source` and returns what it returned.
fn eval(game: &mut Game, source: &str) -> Vec<ScriptValue> {
    let run = game.run("Eval", source);
    game.assert_no_errors();
    match run.resume {
        Resume::Finished(values) => values,
        other => panic!("{other:?}"),
    }
}

fn cframe(value: &ScriptValue) -> CFrame {
    match value {
        ScriptValue::CFrame(cf) => *cf,
        other => panic!("not a CFrame: {other:?}"),
    }
}

fn vector(value: &ScriptValue) -> Vector3 {
    match value {
        ScriptValue::Vector3(v) => *v,
        other => panic!("not a Vector3: {other:?}"),
    }
}

fn number(value: &ScriptValue) -> f64 {
    match value {
        ScriptValue::Number(n) => *n,
        other => panic!("not a number: {other:?}"),
    }
}

#[test]
fn vector3_api() {
    let mut game = Game::new();
    let out = eval(
        &mut game,
        r##"
        local a, b = Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
        return a + b, a - b, a * 2, b / 2, -a, a:Dot(b), a:Cross(b), a:Lerp(b, 0.5), Vector3.new(3, 4, 0).Magnitude,
            Vector3.new(0, 3, 0).Unit, a == Vector3.new(1, 2, 3), a.X + a.Y + a.Z, Vector3.zero, Vector3.one,
            typeof(a), Vector3.new(), a * b
        "##,
    );
    let a = Vector3::new(1.0, 2.0, 3.0);
    let b = Vector3::new(4.0, 5.0, 6.0);
    assert_eq!(vector(&out[0]), a + b);
    assert_eq!(vector(&out[1]), a - b);
    assert_eq!(vector(&out[2]), a * 2.0);
    assert_eq!(vector(&out[3]), b / 2.0);
    assert_eq!(vector(&out[4]), -a);
    assert_eq!(number(&out[5]), f64::from(a.dot(b)));
    assert_eq!(vector(&out[6]), a.cross(b));
    assert_eq!(vector(&out[7]), a.lerp(b, 0.5));
    assert_eq!(number(&out[8]), 5.0);
    assert_eq!(vector(&out[9]), Vector3::Y);
    assert_eq!(out[10], ScriptValue::Bool(true));
    assert_eq!(number(&out[11]), 6.0);
    assert_eq!(vector(&out[12]), Vector3::ZERO);
    assert_eq!(vector(&out[13]), Vector3::ONE);
    assert_eq!(out[14], ScriptValue::from("Vector3"));
    assert_eq!(vector(&out[15]), Vector3::ZERO);
    assert_eq!(vector(&out[16]), a * b);
}

#[test]
fn cframe_api_matches_core() {
    let mut game = Game::new();
    let out = eval(
        &mut game,
        r##"
        local cf = CFrame.new(1, 2, 3) * CFrame.Angles(math.rad(30), math.rad(45), math.rad(60))
        local look = CFrame.new(Vector3.new(0, 0, 0), Vector3.new(0, 0, -10))
        local o = CFrame.fromOrientation(0.1, 0.2, 0.3)
        local rx, ry, rz = o:ToOrientation()
        return cf, cf:Inverse(), cf * Vector3.new(1, 0, 0), cf.Position, cf.LookVector, cf.RightVector,
            cf.UpVector, cf:Lerp(CFrame.new(), 0.5), look.LookVector, rx, ry, rz, cf * cf:Inverse(),
            cf + Vector3.new(1, 1, 1), select("#", cf:GetComponents()), typeof(cf)
        "##,
    );
    let expected = CFrame::new(Vector3::new(1.0, 2.0, 3.0))
        * CFrame::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            30f32.to_radians(),
            45f32.to_radians(),
            60f32.to_radians(),
        ));
    assert!(cframe(&out[0]).approx_eq(expected, 1e-5));
    // `CFrame.Angles` takes radians and agrees with core's degree constructor.
    assert!(cframe(&out[0]).approx_eq(
        CFrame::new(Vector3::new(1.0, 2.0, 3.0)) * CFrame::from_euler_xyz_deg(30.0, 45.0, 60.0),
        1e-5
    ));
    assert!(cframe(&out[1]).approx_eq(expected.inverse(), 1e-5));
    assert!(vector(&out[2]).abs_diff_eq(expected * Vector3::X, 1e-5));
    assert_eq!(vector(&out[3]), Vector3::new(1.0, 2.0, 3.0));
    assert!(vector(&out[4]).abs_diff_eq(expected.look_vector(), 1e-5));
    assert!(vector(&out[5]).abs_diff_eq(expected.right_vector(), 1e-5));
    assert!(vector(&out[6]).abs_diff_eq(expected.up_vector(), 1e-5));
    assert!(cframe(&out[7]).approx_eq(expected.lerp(CFrame::IDENTITY, 0.5), 1e-5));
    assert!(vector(&out[8]).abs_diff_eq(Vector3::NEG_Z, 1e-6));
    for (value, angle) in out[9..12].iter().zip([0.1, 0.2, 0.3]) {
        assert!((number(value) - angle).abs() < 1e-5, "{value:?} vs {angle}");
    }
    assert!(cframe(&out[12]).approx_eq(CFrame::IDENTITY, 1e-5));
    assert_eq!(cframe(&out[13]).position(), Vector3::new(2.0, 3.0, 4.0));
    assert_eq!(number(&out[14]), 12.0);
    assert_eq!(out[15], ScriptValue::from("CFrame"));
}

#[test]
fn color3_and_vector2_api() {
    let mut game = Game::new();
    let out = eval(
        &mut game,
        r##"
        local c = Color3.fromRGB(255, 128, 0)
        local v = Vector2.new(3, 4)
        return c, Color3.fromHex("#FF8000") == c, c:ToHex(), c:Lerp(Color3.new(0, 0, 0), 0.5), c.G,
            v.Magnitude, v + Vector2.one, v * 2, v:Dot(Vector2.new(1, 0)), typeof(c), typeof(v), tostring(v)
        "##,
    );
    assert_eq!(out[0], ScriptValue::Color3(Color3::from_rgb_u8(255, 128, 0)));
    assert_eq!(out[1], ScriptValue::Bool(true));
    assert_eq!(out[2], ScriptValue::from("ff8000"));
    assert_eq!(
        out[3],
        ScriptValue::Color3(Color3::from_rgb_u8(255, 128, 0).lerp(Color3::BLACK, 0.5))
    );
    assert!((number(&out[4]) - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(number(&out[5]), 5.0);
    assert_eq!(out[6], ScriptValue::Vector2(metallic_core::Vector2::new(4.0, 5.0)));
    assert_eq!(out[7], ScriptValue::Vector2(metallic_core::Vector2::new(6.0, 8.0)));
    assert_eq!(number(&out[8]), 3.0);
    assert_eq!(out[9], ScriptValue::from("Color3"));
    assert_eq!(out[10], ScriptValue::from("Vector2"));
    assert_eq!(out[11], ScriptValue::from("3, 4"));
}

#[test]
fn values_round_trip_through_properties() {
    let mut game = Game::new();
    let out = eval(
        &mut game,
        r##"
        local part = Instance.new("Part", workspace)
        part.Name = "Round"
        local cf = CFrame.new(5, 6, 7) * CFrame.fromOrientation(0.3, 0.2, 0.1)
        part.CFrame = cf
        part.Color = Color3.new(0.25, 0.5, 0.75)
        part.Size = Vector3.new(2, 3, 4)
        part.Orientation = Vector3.new(0, 90, 0)
        return part.CFrame == cf, part.Color, part.Size, part.Position, part.Orientation
        "##,
    );
    assert_eq!(out[0], ScriptValue::Bool(false), "Orientation was written after CFrame");
    assert_eq!(out[1], ScriptValue::Color3(Color3::new(0.25, 0.5, 0.75)));
    assert_eq!(vector(&out[2]), Vector3::new(2.0, 3.0, 4.0));
    assert_eq!(vector(&out[3]), Vector3::new(5.0, 6.0, 7.0));
    assert!(vector(&out[4]).abs_diff_eq(Vector3::new(0.0, 90.0, 0.0), 1e-3));
    let workspace = game.service("Workspace");
    let part = game.dm.find_first_child(workspace, "Round", false).unwrap().unwrap();
    let Ok(Value::CFrame(stored)) = game.dm.get_by_name(part, "CFrame") else {
        panic!("no CFrame");
    };
    assert!(stored.approx_eq(
        CFrame::new(Vector3::new(5.0, 6.0, 7.0)) * CFrame::from_orientation_deg(0.0, 90.0, 0.0),
        1e-5
    ));
}
