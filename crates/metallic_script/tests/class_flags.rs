//! `Instance.new` follows the class registry's flags (#460, the class half of #93).

mod common;

use common::Rig;

#[test]
fn instance_new_refuses_abstract_classes_and_services_with_the_existing_text() {
    for class in [
        // Abstract.
        "Instance",
        "BasePart",
        "PVInstance",
        "GuiBase2d",
        "GuiObject",
        "LuaSourceContainer",
        "BaseScript",
        "BaseRemoteEvent",
        "ServiceProvider",
        "DataModel",
        // Services.
        "Workspace",
        "Players",
        "ReplicatedStorage",
        "ServerScriptService",
        "ServerStorage",
        "StarterPlayer",
        "StarterGui",
        "Lighting",
        "RunService",
        // Services that are host objects, not registry classes yet.
        "UserInputService",
        "ContextActionService",
        "CollectionService",
    ] {
        let mut rig = Rig::new();
        rig.server_script(
            "S",
            &format!("local ok, message = pcall(Instance.new, '{class}')\nprint(ok, message)"),
        );
        rig.host.start().expect("host starts");
        let lines = rig.lines();
        assert!(
            lines[0].starts_with("print:false")
                && lines[0].contains(&format!("Unable to create an Instance of type \"{class}\"")),
            "{class}: {lines:?}"
        );
    }
}

#[test]
fn instance_new_still_creates_concrete_classes_and_is_a_sees_the_inserted_classes() {
    let mut rig = Rig::new();
    rig.server_script(
        "S",
        r#"
        local part = Instance.new("Part")
        local script = Instance.new("Script")
        local module = Instance.new("ModuleScript")
        local frame = Instance.new("TextLabel")
        local remote = Instance.new("RemoteEvent")
        local model = Instance.new("Model")
        print(part:IsA("PVInstance"), part:IsA("BasePart"), part:IsA("Instance"))
        print(model:IsA("PVInstance"), model:IsA("BasePart"))
        print(script:IsA("BaseScript"), script:IsA("LuaSourceContainer"))
        print(module:IsA("LuaSourceContainer"), module:IsA("BaseScript"))
        print(frame:IsA("GuiObject"), frame:IsA("GuiBase2d"), frame:IsA("Frame"))
        print(remote:IsA("BaseRemoteEvent"), part.ClassName, part.Name)
        "#,
    );
    rig.host.start().expect("host starts");
    assert_eq!(
        rig.lines(),
        [
            "print:true true true",
            "print:true false",
            "print:true true",
            "print:true false",
            "print:true true true",
            "print:true Part Part",
        ]
    );
}

#[test]
fn abstract_class_members_stay_as_they_were_for_scripts() {
    let mut rig = Rig::new();
    rig.server_script(
        "S",
        r#"
        local script = Instance.new("Script")
        script.Source = "return 1"
        print(script.Source, script.Enabled)
        local frame = Instance.new("Frame")
        print(frame.Visible, frame.ZIndex)
        "#,
    );
    rig.host.start().expect("host starts");
    assert_eq!(rig.lines(), ["print:return 1 true", "print:true 1"]);
}

/// `hidden` on the resolved descriptor is what the binding filters on, and what `hide` sets for a
/// subclass; no built-in class uses `hide` yet, so a hidden built-in property stands in for it.
#[test]
fn a_hidden_member_is_not_a_valid_member_for_scripts() {
    let mut rig = Rig::new();
    rig.server_script(
        "S",
        r#"
        local model = Instance.new("Model")
        print(pcall(function() return model.ScaleFactor end))
        print(pcall(function() model.ScaleFactor = 2 end))
        "#,
    );
    rig.host.start().expect("host starts");
    let lines = rig.lines();
    for line in &lines {
        assert!(
            line.starts_with("print:false")
                && line.contains("ScaleFactor is not a valid member of Model"),
            "{lines:?}"
        );
    }
    assert_eq!(lines.len(), 2);
}
