//! #79: in Studio's Play mode a click on a game GUI button fires its script events and does not
//! reach game input, through the same `PlayInput` path the viewport uses.

use eframe::egui::{self, Event, Modifiers, PointerButton, Rect};
use metallic_core::{InstanceId, UDim2, Value, World};
use metallic_runtime::TICK;
use metallic_studio::output::OutputLog;
use metallic_studio::play::input::PlayInput;
use metallic_studio::play::{LocalPlaySession, PlaySession};

fn add(world: &mut World, class: &str, name: &str, parent: InstanceId) -> InstanceId {
    let id = world.create(class, Some(parent)).unwrap();
    world.set(id, "Name", Value::String(name.into())).unwrap();
    id
}

/// A button at the viewport's top-left (100 x 50 points) whose `Activated` prints, and an
/// action on the left mouse button that prints when the game gets a click.
fn world() -> World {
    let mut w = World::new_with_services();
    let starter = w.root("StarterGui").unwrap();
    let hud = add(&mut w, "ScreenGui", "Hud", starter);
    let button = add(&mut w, "TextButton", "Go", hud);
    w.set(
        button,
        "Size",
        Value::UDim2(UDim2::new(0.0, 100.0, 0.0, 50.0)),
    )
    .unwrap();
    let script = add(&mut w, "LocalScript", "Ui", hud);
    w.set(
        script,
        "Source",
        Value::String(
            r#"
            script.Parent.Go.Activated:Connect(function() print("activated") end)
            game:GetService("ContextActionService"):BindAction("Fire", function(_, state)
                print("fire " .. state.Name)
                return Enum.ContextActionResult.Pass
            end, false, Enum.UserInputType.MouseButton1)
            "#
            .into(),
        ),
    )
    .unwrap();
    w
}

fn click(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// One Studio frame: egui input to the session's view, then its update.
fn frame(session: &mut LocalPlaySession, input: &mut PlayInput, events: Vec<Event>) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events,
        focused: true,
        ..Default::default()
    };
    let viewport = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let mut out = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        let view = session.view_mut().expect("a local session has a view");
        input.feed(&ctx, view, viewport, true);
    });
    out.textures_delta.clear();
    session.update(TICK);
}

fn printed(log: &mut OutputLog) -> Vec<String> {
    log.drain();
    log.lines().map(|l| l.text.clone()).collect()
}

#[test]
fn a_click_on_a_game_button_in_play_mode_activates_it_and_not_game_input() {
    let mut log = OutputLog::new(64);
    let mut session = LocalPlaySession::start("Gui", &world(), log.sink()).unwrap();
    let mut input = PlayInput::new();
    frame(&mut session, &mut input, Vec::new());
    let on = egui::pos2(40.0, 20.0);
    frame(
        &mut session,
        &mut input,
        vec![Event::PointerMoved(on), click(on, true)],
    );
    frame(&mut session, &mut input, vec![click(on, false)]);
    frame(&mut session, &mut input, Vec::new());
    let lines = printed(&mut log);
    assert!(lines.iter().any(|l| l == "activated"), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("fire")),
        "the game saw the click: {lines:?}"
    );
    let off = egui::pos2(500.0, 400.0);
    frame(
        &mut session,
        &mut input,
        vec![Event::PointerMoved(off), click(off, true)],
    );
    frame(&mut session, &mut input, vec![click(off, false)]);
    frame(&mut session, &mut input, Vec::new());
    let lines = printed(&mut log);
    assert!(lines.iter().any(|l| l == "fire Begin"), "{lines:?}");
    assert_eq!(
        lines.iter().filter(|l| *l == "activated").count(),
        1,
        "{lines:?}"
    );
}
