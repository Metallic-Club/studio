//! Instances from Luau: creating parts, properties (computed ones too), methods, children by name, errors.

mod support;

use metallic_core::{CFrame, Value, Vector3};
use support::Game;

#[test]
fn a_script_builds_a_part_and_reads_it_back() {
    let mut game = Game::new();
    game.run(
        "Builder",
        r#"
        local part = Instance.new("Part")
        part.Name = "Door"
        part.Position = Vector3.new(1, 2, 3)
        part.Transparency = 0.5
        part.Anchored = true
        part.Parent = workspace
        print(part.Name, part.Position, part.Transparency, part.Anchored, part.ClassName)
        print(part:IsA("BasePart"), part:IsA("Model"), typeof(part), typeof(part.Position))
        print(workspace.Door == part, workspace:FindFirstChild("Door") == part, tostring(part))
        print(script.Name, game.Workspace == workspace)
        "#,
    );
    game.assert_no_errors();
    assert_eq!(
        game.prints(),
        [
            "Door 1, 2, 3 0.5 true Part",
            "true false Instance Vector3",
            "true true Door",
            "Builder true",
        ]
    );
    let workspace = game.service("Workspace");
    let door = game.dm.find_first_child(workspace, "Door", false).unwrap().unwrap();
    assert_eq!(
        game.dm.get_by_name(door, "CFrame"),
        Ok(Value::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0))))
    );
}

#[test]
fn errors_name_the_script_and_line() {
    let mut game = Game::new();
    game.run(
        "Door",
        "local part = Instance.new('Part')\npart.Parent = workspace\nprint(part.Bogus)\n",
    );
    game.run("Other", "\n\nerror('boom')\n");
    game.run("ReadOnly", "local p = Instance.new('Part')\np.ClassName = 'x'\n");
    game.run("Assign", "local p = Instance.new('Part')\n\np.Transparency = 'x'\n");
    game.run("Method", "workspace:Nope()\n");
    assert_eq!(
        game.errors(),
        [
            "Workspace.Door:3: Bogus is not a valid member of Part \"Workspace.Part\"",
            "Workspace.Other:3: boom",
            "Workspace.ReadOnly:2: Unable to assign property ClassName. Property is read only",
            "Workspace.Assign:3: Unable to assign property Transparency. expected number, got string",
            "Workspace.Method:1: Nope is not a valid member of Workspace \"Workspace\"",
        ]
    );
}
