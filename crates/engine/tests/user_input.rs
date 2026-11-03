//! `UserInputService` driven through the engine: synthetic platform events pushed into the input queue of a
//! headless engine reach a Luau script as `InputBegan`/`InputEnded`, `IsKeyDown` and `GetMouseDelta`, and the
//! script's `MouseBehavior` reaches a fake window's cursor grab.
#![cfg(all(feature = "luau", feature = "render"))]

use std::sync::{Arc, Mutex};

use metallic_core::{DataModel, Value};
use metallic_engine::{Engine, EngineConfig, InputEvent, MemorySources, RunContext};
use metallic_platform::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use metallic_platform::{
    ButtonState, CursorGrab, DeviceEvent, Key, KeyCode, KeyEvent, PlatformError, Position, Size, Window, WindowBackend,
    WindowEvent, WindowId, WindowMode,
};
use metallic_script::{CollectOutput, OutputLevel};

const WINDOW: WindowId = WindowId::from_raw(1);

/// A started standalone game running `source` as one `Script`.
fn started(source: &str) -> Engine<CollectOutput> {
    let sources = MemorySources::new().with("Main.luau", source);
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    add_script(engine.data_model_mut());
    engine.start().expect("start");
    assert_no_errors(&engine);
    engine
}

fn add_script(dm: &mut DataModel) {
    let class = dm.registry().class_id("Script").expect("Script");
    let service = dm.registry().class_id("ServerScriptService").expect("service class");
    let service = dm.service(service).expect("ServerScriptService");
    let script = dm.create(class).expect("create");
    dm.set_name(script, "Main".into()).expect("name");
    dm.set_by_name(script, "Source", Value::String("Main.luau".into()))
        .expect("source");
    dm.set_parent(script, Some(service)).expect("parent");
}

/// Evaluates the Luau expression `expr` in a throwaway script and returns what it prints.
fn probe(engine: &mut Engine<CollectOutput>, expr: &str) -> String {
    let (dm, host) = engine.parts_mut();
    let class = dm.registry().class_id("Script").expect("Script");
    let script = dm.create(class).expect("probe");
    let before = host.output().lines.len();
    host.run_script(dm, script, "Probe.luau", &format!("print(tostring({expr}))"))
        .expect("probe runs");
    let line = host.output().lines.get(before).expect("the probe printed");
    assert_eq!(line.level, OutputLevel::Print, "probe failed: {}", line.message);
    let printed = line.message.clone();
    host.output_mut().lines.truncate(before);
    printed
}

/// Runs the Luau statement `code` in a throwaway script, as a script changing settings between frames would.
fn run(engine: &mut Engine<CollectOutput>, code: &str) {
    let (dm, host) = engine.parts_mut();
    let class = dm.registry().class_id("Script").expect("Script");
    let script = dm.create(class).expect("runner");
    host.run_script(dm, script, "Run.luau", code).expect("runs");
}

fn assert_no_errors(engine: &Engine<CollectOutput>) {
    let errors: Vec<&str> = engine
        .host()
        .output()
        .of_level(OutputLevel::Error)
        .map(|l| l.message.as_str())
        .collect();
    assert!(errors.is_empty(), "script errors: {errors:#?}");
}

fn key(code: KeyCode, text: &str, state: ButtonState) -> InputEvent {
    InputEvent::Window {
        window: WINDOW,
        event: WindowEvent::Key(KeyEvent {
            key_code: code,
            key: Key::Character(text.into()),
            text: state.is_pressed().then(|| text.to_owned()),
            state,
            repeat: false,
        }),
    }
}

fn motion(dx: f64, dy: f64) -> InputEvent {
    InputEvent::Device(DeviceEvent::MouseMotion { dx, dy })
}

fn cursor(x: f64, y: f64) -> InputEvent {
    InputEvent::Window {
        window: WINDOW,
        event: WindowEvent::CursorMoved(Position { x, y }),
    }
}

fn focus(focused: bool) -> InputEvent {
    InputEvent::Window {
        window: WINDOW,
        event: WindowEvent::Focused(focused),
    }
}

const KEYS: &str = r#"
local UserInputService = game:GetService("UserInputService")
local RunService = game:GetService("RunService")
_G.began = {}
_G.ended = {}
UserInputService.InputBegan:Connect(function(input, gameProcessedEvent)
    table.insert(_G.began, input.KeyCode.Name)
    _G.isW = input.KeyCode == Enum.KeyCode.W
    _G.keyboard = input.UserInputType == Enum.UserInputType.Keyboard
    _G.processed = gameProcessedEvent
end)
UserInputService.InputEnded:Connect(function(input)
    table.insert(_G.ended, `{input.KeyCode.Name} {input.UserInputState.Name}`)
end)
RunService.Heartbeat:Connect(function()
    _G.wDown = UserInputService:IsKeyDown(Enum.KeyCode.W)
    _G.held = #UserInputService:GetKeysPressed()
end)
"#;

#[test]
fn a_key_press_reaches_scripts_as_input_began_is_key_down_and_input_ended() {
    let mut engine = started(KEYS);
    engine.push_input(key(KeyCode::KeyW, "w", ButtonState::Pressed));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(probe(&mut engine, "table.concat(_G.began, ',')"), "W");
    assert_eq!(probe(&mut engine, "_G.isW"), "true");
    assert_eq!(probe(&mut engine, "_G.keyboard"), "true");
    assert_eq!(probe(&mut engine, "_G.processed"), "false");
    assert_eq!(probe(&mut engine, "_G.wDown"), "true");
    assert_eq!(probe(&mut engine, "_G.held"), "1");

    // Held across frames with no events.
    engine.run_headless(3);
    assert_eq!(probe(&mut engine, "_G.wDown"), "true");

    engine.push_input(key(KeyCode::KeyW, "w", ButtonState::Released));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(probe(&mut engine, "table.concat(_G.ended, ',')"), "W End");
    assert_eq!(probe(&mut engine, "_G.wDown"), "false");
    assert_eq!(probe(&mut engine, "_G.held"), "0");
    assert_eq!(
        probe(&mut engine, "table.concat(_G.began, ',')"),
        "W",
        "InputBegan fired once"
    );
}

/// Tracks held inputs the Roblox way: a table keyed by the input objects.
const HELD: &str = r#"
local UserInputService = game:GetService("UserInputService")
local held = {}
_G.log = {}
UserInputService.InputBegan:Connect(function(input)
    held[input] = true
    table.insert(_G.log, `began {input.KeyCode.Name} {input.UserInputState.Name}`)
end)
UserInputService.InputEnded:Connect(function(input)
    table.insert(_G.log, `ended {input.KeyCode.Name} {input.UserInputState.Name} held {held[input] == true}`)
    held[input] = nil
end)
_G.heldCount = function()
    local n = 0
    for _ in held do
        n += 1
    end
    return n
end
"#;

#[test]
fn a_tap_within_one_frame_begins_then_ends_with_one_object() {
    let mut engine = started(HELD);
    engine.push_input(key(KeyCode::KeyW, "w", ButtonState::Pressed));
    engine.push_input(key(KeyCode::KeyW, "w", ButtonState::Released));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(probe(&mut engine, "table.concat(_G.log, '; ')"), "began W Begin");
    assert_eq!(probe(&mut engine, "_G.heldCount()"), "1");
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(
        probe(&mut engine, "table.concat(_G.log, '; ')"),
        "began W Begin; ended W End held true",
        "InputEnded passes the object InputBegan passed, now in End"
    );
    assert_eq!(probe(&mut engine, "_G.heldCount()"), "0");
}

#[test]
fn ten_thousand_presses_leave_the_instance_count_flat() {
    let mut engine = started(HELD);
    engine.tick();
    let instances = engine.data_model().len();
    for _ in 0..10_000 {
        engine.push_input(key(KeyCode::KeyE, "e", ButtonState::Pressed));
        engine.push_input(key(KeyCode::KeyE, "e", ButtonState::Released));
        engine.tick();
    }
    engine.run_headless(2);
    assert_no_errors(&engine);
    assert_eq!(
        engine.data_model().len(),
        instances,
        "every press's object was destroyed"
    );
    assert_eq!(probe(&mut engine, "_G.heldCount()"), "0");
    assert_eq!(probe(&mut engine, "#_G.log"), "20000");
}

const MOUSE: &str = r#"
local UserInputService = game:GetService("UserInputService")
local RunService = game:GetService("RunService")
UserInputService.MouseBehavior = Enum.MouseBehavior.LockCenter
_G.deltas = {}
RunService.Heartbeat:Connect(function()
    local delta = UserInputService:GetMouseDelta()
    table.insert(_G.deltas, `{delta.X},{delta.Y}`)
    local location = UserInputService:GetMouseLocation()
    _G.location = `{location.X},{location.Y}`
end)
"#;

#[test]
fn mouse_delta_under_lock_center_is_the_frames_movement() {
    let mut engine = started(MOUSE);
    let grabs = attach_fake_window(&mut engine);
    engine.push_input(cursor(400.0, 300.0));
    engine.push_input(motion(3.0, -1.0));
    engine.push_input(motion(2.0, 4.0));
    engine.tick();
    engine.tick();
    engine.push_input(motion(-1.0, 0.0));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(
        probe(&mut engine, "table.concat(_G.deltas, ' ')"),
        "5,3 0,0 -1,0",
        "accumulated within a frame, zero in a frame without movement"
    );
    assert_eq!(probe(&mut engine, "_G.location"), "400,300");
    assert_eq!(*grabs.grabs.lock().expect("grabs"), [CursorGrab::Lock]);
    assert_eq!(*grabs.visible.lock().expect("visible"), [true]);
}

#[test]
fn mouse_behavior_and_icon_reach_the_window() {
    let mut engine = started(
        r#"
        local UserInputService = game:GetService("UserInputService")
        _G.set = function(behavior, icon)
            UserInputService.MouseBehavior = behavior
            UserInputService.MouseIconEnabled = icon
        end
        "#,
    );
    let window = attach_fake_window(&mut engine);
    engine.tick();
    run(&mut engine, "_G.set(Enum.MouseBehavior.LockCurrentPosition, false)");
    engine.tick();
    engine.tick();
    run(&mut engine, "_G.set(Enum.MouseBehavior.Default, true)");
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(
        *window.grabs.lock().expect("grabs"),
        [CursorGrab::None, CursorGrab::Lock, CursorGrab::None],
        "applied once per change"
    );
    assert_eq!(*window.visible.lock().expect("visible"), [true, false, true]);

    // A new window is told the current state.
    run(&mut engine, "_G.set(Enum.MouseBehavior.LockCenter, true)");
    engine.detach_window();
    let window = attach_fake_window(&mut engine);
    engine.tick();
    assert_eq!(*window.grabs.lock().expect("grabs"), [CursorGrab::Lock]);
}

/// Records each frame's `GetMouseDelta`, every mouse `InputChanged` delta and the frame's `GetMouseLocation`, and
/// lets the test set `MouseBehavior`.
const CURSOR: &str = r#"
local UserInputService = game:GetService("UserInputService")
local RunService = game:GetService("RunService")
_G.deltas = {}
_G.changed = {}
_G.set = function(behavior)
    UserInputService.MouseBehavior = behavior
end
UserInputService.InputChanged:Connect(function(input)
    if input.UserInputType == Enum.UserInputType.MouseMovement then
        table.insert(_G.changed, `{input.Delta.X},{input.Delta.Y}`)
    end
end)
RunService.Heartbeat:Connect(function()
    local delta = UserInputService:GetMouseDelta()
    table.insert(_G.deltas, `{delta.X},{delta.Y}`)
    local location = UserInputService:GetMouseLocation()
    _G.location = `{location.X},{location.Y}`
end)
"#;

/// The `_G` list `name` as one string, then emptied.
fn take(engine: &mut Engine<CollectOutput>, name: &str) -> String {
    let got = probe(engine, &format!("table.concat(_G.{name}, ' ')"));
    run(engine, &format!("table.clear(_G.{name})"));
    got
}

#[test]
fn lock_center_without_a_lock_holds_the_cursor_at_the_centre() {
    let mut engine = started(CURSOR);
    let window = attach_confining_window(&mut engine);
    run(&mut engine, "_G.set(Enum.MouseBehavior.LockCenter)");
    engine.push_input(cursor(100.0, 50.0));
    engine.tick();
    assert_eq!(*window.grabs.lock().expect("grabs"), [CursorGrab::Lock]);
    assert!(window.moves().is_empty(), "no focus yet: the cursor is left alone");
    assert_eq!(probe(&mut engine, "_G.location"), "100,50");

    // Focus: the cursor goes to the centre at each input step.
    engine.push_input(focus(true));
    engine.tick();
    assert_eq!(probe(&mut engine, "_G.location"), "400,300");
    // The user pushes the mouse: the OS moves the cursor off the centre and reports raw motion.
    engine.push_input(cursor(410.0, 295.0));
    engine.push_input(motion(10.0, -5.0));
    engine.tick();
    assert_eq!(probe(&mut engine, "_G.location"), "400,300");
    // The next frame has only the engine's move back to the centre in it, as a cursor event: had it counted, it
    // would read as a push of -10,5.
    engine.push_input(cursor(400.0, 300.0));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(window.moves(), [(400.0, 300.0); 3], "once per input step");
    assert_eq!(
        take(&mut engine, "deltas"),
        "0,0 0,0 10,-5 0,0",
        "raw motion only; neither the engine's moves nor the cursor's position count"
    );
    assert_eq!(
        take(&mut engine, "changed"),
        "0,0 10,-5",
        "the first cursor event, then the push; the engine's moves fire nothing"
    );

    // Focus lost: the cursor is free (and its events count again) until focus comes back.
    engine.push_input(focus(false));
    engine.push_input(cursor(50.0, 60.0));
    engine.tick();
    assert_eq!(window.moves().len(), 3);
    assert_eq!(probe(&mut engine, "_G.location"), "50,60");
    engine.push_input(focus(true));
    engine.tick();
    assert_eq!(window.moves().len(), 4);
    assert_eq!(probe(&mut engine, "_G.location"), "400,300");

    // Back to `Default`: the engine lets go.
    run(&mut engine, "_G.set(Enum.MouseBehavior.Default)");
    engine.tick();
    engine.push_input(cursor(10.0, 20.0));
    engine.tick();
    assert_eq!(window.moves().len(), 4);
    assert_eq!(probe(&mut engine, "_G.location"), "10,20");
    assert_eq!(
        *window.grabs.lock().expect("grabs"),
        [CursorGrab::Lock, CursorGrab::Lock, CursorGrab::Lock, CursorGrab::None],
        "grabbed again each time focus came back"
    );
}

#[test]
fn lock_current_position_without_a_lock_holds_the_cursor_where_it_was() {
    let mut engine = started(CURSOR);
    let window = attach_confining_window(&mut engine);
    engine.push_input(focus(true));
    engine.push_input(cursor(120.0, 80.0));
    engine.tick();
    assert!(window.moves().is_empty(), "Default: the cursor is free");
    run(&mut engine, "_G.set(Enum.MouseBehavior.LockCurrentPosition)");
    engine.tick();
    engine.push_input(cursor(130.0, 85.0));
    engine.push_input(motion(10.0, 5.0));
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(window.moves(), [(120.0, 80.0); 2]);
    assert_eq!(probe(&mut engine, "_G.location"), "120,80");
    assert_eq!(take(&mut engine, "deltas"), "0,0 0,0 10,5");
}

#[test]
fn a_window_that_locks_gets_the_cursor_at_the_centre_before_each_grab() {
    let mut engine = started(CURSOR);
    let window = attach_fake_window(&mut engine);
    run(&mut engine, "_G.set(Enum.MouseBehavior.LockCenter)");
    engine.push_input(focus(true));
    engine.push_input(cursor(100.0, 50.0));
    engine.tick();
    assert_eq!(window.moves(), [(400.0, 300.0)], "moved once, then locked there");
    engine.push_input(cursor(400.0, 300.0));
    engine.push_input(motion(3.0, 4.0));
    engine.tick();
    engine.tick();
    assert_no_errors(&engine);
    assert_eq!(window.moves().len(), 1, "the OS holds a locked cursor");
    assert_eq!(*window.grabs.lock().expect("grabs"), [CursorGrab::Lock]);
    assert_eq!(probe(&mut engine, "_G.location"), "400,300");
    assert_eq!(take(&mut engine, "deltas"), "0,0 3,4 0,0");
    assert_eq!(
        take(&mut engine, "changed"),
        "0,0 3,4",
        "the first cursor event, then the push; the engine's move fires nothing"
    );

    // The system may let go of the cursor while another window has focus: back in, it is moved and locked again.
    engine.push_input(focus(false));
    engine.push_input(focus(true));
    engine.tick();
    assert_eq!(window.moves(), [(400.0, 300.0); 2]);
    assert_eq!(
        *window.grabs.lock().expect("grabs"),
        [CursorGrab::Lock, CursorGrab::Lock]
    );
}

#[test]
fn a_stopped_game_tracks_keys_but_fires_nothing() {
    let sources = MemorySources::new();
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    let uis = engine.user_input_service();
    assert_eq!(engine.data_model().class_name(uis), Ok("UserInputService"));
    engine.push_input(key(KeyCode::Space, " ", ButtonState::Pressed));
    engine.tick();
    let dm = engine.data_model();
    assert_eq!(dm.pending_events(), 0);
    assert_eq!(dm.get_by_name(uis, "KeyboardEnabled"), Ok(Value::Bool(true)));
}

/// A window with no OS behind it that records what the engine asks of its cursor.
#[derive(Clone, Default)]
struct FakeWindow {
    /// Whether the window cannot lock the cursor and confines it instead, as winit does on X11.
    confine_only: bool,
    grabs: Arc<Mutex<Vec<CursorGrab>>>,
    visible: Arc<Mutex<Vec<bool>>>,
    /// Where the engine moved the cursor.
    moves: Arc<Mutex<Vec<(f64, f64)>>>,
}

impl FakeWindow {
    fn moves(&self) -> Vec<(f64, f64)> {
        self.moves.lock().expect("moves").clone()
    }
}

fn attach_fake_window(engine: &mut Engine<CollectOutput>) -> FakeWindow {
    attach(engine, FakeWindow::default())
}

/// A fake window that confines the cursor when asked to lock it.
fn attach_confining_window(engine: &mut Engine<CollectOutput>) -> FakeWindow {
    attach(
        engine,
        FakeWindow {
            confine_only: true,
            ..FakeWindow::default()
        },
    )
}

fn attach(engine: &mut Engine<CollectOutput>, fake: FakeWindow) -> FakeWindow {
    engine
        .attach_window(Window::from_backend(Arc::new(fake.clone())))
        .expect("attach");
    fake
}

impl HasWindowHandle for FakeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::NotSupported)
    }
}

impl HasDisplayHandle for FakeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::NotSupported)
    }
}

impl WindowBackend for FakeWindow {
    fn id(&self) -> WindowId {
        WINDOW
    }
    /// Odd, so the centre has to round to a whole pixel.
    fn inner_size(&self) -> Size {
        Size::new(801, 601)
    }
    fn scale_factor(&self) -> f64 {
        1.0
    }
    fn set_title(&self, _title: &str) {}
    fn set_mode(&self, _mode: WindowMode) -> Result<(), PlatformError> {
        Ok(())
    }
    fn set_cursor_grab(&self, grab: CursorGrab) -> Result<CursorGrab, PlatformError> {
        self.grabs.lock().expect("grabs").push(grab);
        Ok(match grab {
            CursorGrab::Lock if self.confine_only => CursorGrab::Confine,
            grab => grab,
        })
    }
    fn set_cursor_visible(&self, visible: bool) {
        self.visible.lock().expect("visible").push(visible);
    }
    fn set_cursor_position(&self, position: Position) -> Result<(), PlatformError> {
        self.moves.lock().expect("moves").push((position.x, position.y));
        Ok(())
    }
    fn request_redraw(&self) {}
}
