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
