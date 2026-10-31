//! GUI element events through `ScriptHost::gui_event` (#79).

mod common;

use common::Rig;
use metallic_script::{GuiEvent, GuiEventKind, GuiPointer, MAX_GUI_EVENTS_PER_STEP};

fn event(target: metallic_core::InstanceId, kind: GuiEventKind) -> GuiEvent {
    GuiEvent {
        target,
        kind,
        pointer: GuiPointer::Mouse,
        position: [12.4, 30.6],
    }
}

#[test]
fn button_and_frame_events_reach_local_scripts_in_order() {
    let mut rig = Rig::new();
    let gui = rig.add("ScreenGui", "Hud", rig.root("Workspace"));
    let button = rig.add("TextButton", "Go", gui);
    let label = rig.add("TextLabel", "Title", gui);
    rig.local_script(
        "Ui",
        r##"
        local hud = workspace.Hud
        hud.Go.Activated:Connect(function(input, clicks)
            print("activated", input.UserInputType.Name, input.UserInputState.Name, input.Position, clicks)
        end)
        hud.Go.MouseButton1Click:Connect(function(...) print("click", select("#", ...)) end)
        hud.Go.MouseButton1Down:Connect(function(x, y) print("down", x, y) end)
        hud.Title.MouseEnter:Connect(function(x, y) print("enter", x, y) end)
        hud.Title.InputBegan:Connect(function(input) print("began", input.UserInputType.Name) end)
        print(pcall(function() return hud.Title.Activated end))
        print(hud.Go:IsA("GuiButton"), hud.Go:IsA("Frame"), hud.Go.Text, hud.Go.AutoButtonColor, hud.Go.ZIndex)
        "##,
    );
    rig.host.start().unwrap();
    for (target, kind) in [
        (label, GuiEventKind::MouseEnter),
        (label, GuiEventKind::InputBegan),
        (button, GuiEventKind::MouseButton1Down),
        (button, GuiEventKind::MouseButton1Click),
        (button, GuiEventKind::Activated),
    ] {
        rig.host.gui_event(event(target, kind));
    }
    rig.run(0.0, 2);
    let lines = rig.lines();
    assert!(
        lines[0].starts_with("print:false") && lines[0].contains("Activated is not a valid member"),
        "a TextLabel is not a button: {lines:?}"
    );
    assert_eq!(
        &lines[1..],
        [
            "print:true true Button true 1",
            "print:enter 12 31",
            "print:began MouseButton1",
            "print:down 12 31",
            "print:click 0",
            "print:activated MouseButton1 End 12, 31, 0 1",
        ]
    );
}

#[test]
fn the_gui_event_queue_is_bounded_per_step() {
    let mut rig = Rig::new();
    let gui = rig.add("ScreenGui", "Hud", rig.root("Workspace"));
    let button = rig.add("TextButton", "Go", gui);
    rig.local_script(
        "Ui",
        r#"
        local n = 0
        workspace.Hud.Go.MouseButton1Click:Connect(function() n += 1 end)
        game:GetService("RunService").Heartbeat:Connect(function()
            if n > 0 then print(n) n = 0 end
        end)
        "#,
    );
    rig.host.start().unwrap();
    for _ in 0..MAX_GUI_EVENTS_PER_STEP + 50 {
        rig.host
            .gui_event(event(button, GuiEventKind::MouseButton1Click));
    }
    let now = rig.run(0.0, 1);
    rig.run(now, 2);
    assert_eq!(rig.lines(), [format!("print:{MAX_GUI_EVENTS_PER_STEP}")]);
}

/// The cap never splits a click: hover and moves leave room for it, and a click that does not
/// fit is dropped whole.
#[test]
fn the_gui_event_cap_keeps_clicks_whole() {
    let mut rig = Rig::new();
    let gui = rig.add("ScreenGui", "Hud", rig.root("Workspace"));
    let button = rig.add("TextButton", "Go", gui);
    rig.local_script(
        "Ui",
        r#"
        local go = workspace.Hud.Go
        local seen = {}
        for _, e in {"MouseButton1Down", "MouseButton1Up", "MouseButton1Click", "Activated"} do
            go[e]:Connect(function() table.insert(seen, e) end)
        end
        game:GetService("RunService").Heartbeat:Connect(function()
            if #seen > 0 then print(table.concat(seen, " ")) table.clear(seen) end
        end)
        "#,
    );
    rig.host.start().unwrap();
    let click = [
        GuiEventKind::MouseButton1Up,
        GuiEventKind::MouseButton1Click,
        GuiEventKind::Activated,
    ];
    // A flood of hover events, then a click: the click is kept.
    for _ in 0..MAX_GUI_EVENTS_PER_STEP * 2 {
        rig.host.gui_event(event(button, GuiEventKind::MouseEnter));
    }
    for kind in click {
        rig.host.gui_event(event(button, kind));
    }
    let now = rig.run(0.0, 1);
    assert_eq!(
        rig.lines(),
        ["print:MouseButton1Up MouseButton1Click Activated"]
    );
    // Presses fill the queue to two places short: the next click is dropped whole.
    for _ in 0..MAX_GUI_EVENTS_PER_STEP - 2 {
        rig.host
            .gui_event(event(button, GuiEventKind::MouseButton1Down));
    }
    for kind in click {
        rig.host.gui_event(event(button, kind));
    }
    rig.run(now, 1);
    // `lines` keeps everything printed so far: the second step's line is the last.
    let lines = rig.lines();
    assert_eq!(lines.len(), 2);
    let second = &lines[1];
    assert!(second.starts_with("print:MouseButton1Down"));
    assert!(
        !second.contains("Up") && !second.contains("Click") && !second.contains("Activated"),
        "the click was split"
    );
}

/// `ScreenGui.ZIndexBehavior` takes `Enum.ZIndexBehavior` items (or their names) and reads back
/// as the item.
#[test]
fn zindex_behavior_is_an_enum_property() {
    let mut rig = Rig::new();
    rig.add("ScreenGui", "Hud", rig.root("Workspace"));
    rig.local_script(
        "Ui",
        r#"
        local hud = workspace.Hud
        print(hud.ZIndexBehavior == Enum.ZIndexBehavior.Sibling, Enum.ZIndexBehavior.Global.Value)
        hud.ZIndexBehavior = Enum.ZIndexBehavior.Global
        print(hud.ZIndexBehavior.Name)
        hud.ZIndexBehavior = "Sibling"
        print(hud.ZIndexBehavior.Name)
        print(pcall(function() hud.ZIndexBehavior = Enum.KeyCode.A end))
        "#,
    );
    rig.host.start().unwrap();
    rig.run(0.0, 1);
    let lines = rig.lines();
    assert_eq!(
        &lines[..3],
        ["print:true 0", "print:Global", "print:Sibling"]
    );
    assert!(lines[3].starts_with("print:false"), "{lines:?}");
}

/// Connecting to `UserInputService.InputBegan`/`InputEnded` warns once per VM that only
/// pointers are reported until #80.
#[test]
fn uis_input_began_warns_once_that_only_pointers_are_wired() {
    let mut rig = Rig::new();
    rig.local_script(
        "Ui",
        r#"
        local UIS = game:GetService("UserInputService")
        UIS.InputBegan:Connect(function() end)
        UIS.InputBegan:Connect(function() end)
        UIS.InputEnded:Connect(function() end)
        "#,
    );
    rig.host.start().unwrap();
    rig.run(0.0, 1);
    let warns: Vec<String> = rig
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("warn:"))
        .collect();
    assert_eq!(warns.len(), 1, "{warns:?}");
    assert!(warns[0].contains("#80"), "{warns:?}");
}
