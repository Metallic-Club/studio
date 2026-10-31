//! `LookOverride` from Luau (#222): display names are stored as ids, unknown styles error.

mod common;

use common::Rig;

#[test]
fn luau_sets_look_override_by_display_name_and_reads_the_id() {
    let mut rig = Rig::new();
    rig.server_script(
        "S",
        r#"
        local part = Instance.new("Part")
        part.Parent = workspace
        print(part.LookOverride)
        part.LookOverride = "Retro / PS1"
        print(part.LookOverride)
        local model = Instance.new("Model")
        model.Parent = workspace
        model.LookOverride = "Pixel 3D"
        print(model.LookOverride)
        print(pcall(function() part.LookOverride = "comic" end))
        print(part.LookOverride)
        part.LookOverride = "inherit"
        print(part.LookOverride)
        "#,
    );
    rig.host.start().expect("host starts");
    let lines = rig.lines();
    assert_eq!(lines[0], "print:inherit");
    assert_eq!(lines[1], "print:retro-32");
    assert_eq!(lines[2], "print:pixel-3d");
    assert!(lines[3].starts_with("print:false"), "{lines:?}");
    assert_eq!(lines[4], "print:retro-32", "a refused write changes nothing");
    assert_eq!(lines[5], "print:inherit");
    assert_eq!(lines.len(), 6, "{lines:?}");
}
