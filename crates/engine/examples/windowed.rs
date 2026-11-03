//! Opens a window and runs a Luau script in it: the engine driven by the platform's event loop.
//!
//! ```text
//! cargo run -p metallic_engine --example windowed            # until the window is closed
//! cargo run -p metallic_engine --example windowed -- 5       # or for 5 seconds
//! ```
//!
//! The script prints once a second on `Heartbeat`, with how many `RenderStepped` events it saw in that second, and
//! prints every key and button `UserInputService.InputBegan` reports; Escape toggles a locked, hidden cursor
//! (`MouseBehavior.LockCenter`), and while it is locked the once-a-second line adds how far the mouse moved
//! (`GetMouseDelta`) and where the cursor is (`GetMouseLocation`, the window's centre). Nothing is drawn yet (the
//! renderer is #75 to #77): a stand-in render hook counts the frames it is asked to draw and reports the window's
//! size, to show where the renderer plugs in. The loop is capped at 60 frames per second, since no vsync paces it.

use std::cell::Cell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use metallic_core::Value;
use metallic_engine::{
    Engine, EngineConfig, EngineMode, MemorySources, RenderFrame, RenderHook, RenderHookError, RunContext,
    WindowedOptions, run_windowed,
};
use metallic_platform::{Size, Window, WindowConfig};

const MAIN: &str = r#"
local RunService = game:GetService("RunService")
print(`started: IsClient {RunService:IsClient()}, IsServer {RunService:IsServer()}`)

local rendered = 0
RunService.RenderStepped:Connect(function()
    rendered += 1
end)

local UserInputService = game:GetService("UserInputService")

local seconds = 0
local since = 0
local moved = Vector2.zero
RunService.Heartbeat:Connect(function(deltaTime)
    moved += UserInputService:GetMouseDelta()
    since += deltaTime
    if since >= 1 then
        since -= 1
        seconds += 1
        print(`Heartbeat: {seconds} s, time() = {string.format("%.2f", time())}, {rendered} RenderStepped this second`)
        rendered = 0
        if UserInputService.MouseBehavior ~= Enum.MouseBehavior.Default then
            print(`  mouse moved {moved} this second; GetMouseLocation() = {UserInputService:GetMouseLocation()}`)
        end
        moved = Vector2.zero
    end
end)

UserInputService.InputBegan:Connect(function(input, gameProcessedEvent)
    if input.UserInputType == Enum.UserInputType.Keyboard then
        print(`InputBegan: {input.KeyCode}, gameProcessedEvent {gameProcessedEvent}`)
    elseif input.UserInputType ~= Enum.UserInputType.MouseMovement then
        print(`InputBegan: {input.UserInputType} at {input.Position}`)
    end
end)
UserInputService.InputEnded:Connect(function(input)
    if input.KeyCode == Enum.KeyCode.Escape then
        -- Escape toggles a first-person style locked cursor.
        local locked = UserInputService.MouseBehavior ~= Enum.MouseBehavior.Default
        UserInputService.MouseBehavior = if locked then Enum.MouseBehavior.Default else Enum.MouseBehavior.LockCenter
        UserInputService.MouseIconEnabled = locked
        print(`MouseBehavior: {UserInputService.MouseBehavior}`)
    end
end)
UserInputService.WindowFocused:Connect(function()
    print("WindowFocused")
end)
UserInputService.WindowFocusReleased:Connect(function()
    print("WindowFocusReleased")
end)
"#;

/// Counts the frames the engine asks it to draw; a real renderer would sync and draw here.
struct CountingHook {
    frames: Rc<Cell<u64>>,
}

impl RenderHook for CountingHook {
    fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
        println!("render hook: attached to {:?}", window.id());
        Ok(())
    }

    fn detach(&mut self) {
        println!("render hook: detached");
    }

    fn resize(&mut self, size: Size, scale_factor: f64) {
        println!(
            "render hook: {}x{} physical pixels, scale {scale_factor}",
            size.width, size.height
        );
    }

    fn render(&mut self, _frame: &mut RenderFrame<'_>) {
        self.frames.set(self.frames.get() + 1);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let sources = MemorySources::new().with("Main.luau", MAIN);
    let config = EngineConfig::new(RunContext::Local)
        .with_mode(EngineMode::Windowed)
        .with_sources(sources);
    let mut engine = Engine::new(config)?;

    let dm = engine.data_model_mut();
    let registry = dm.registry().clone();
    let script_class = registry.class_id("Script").ok_or("no Script class")?;
    let workspace_class = registry.class_id("Workspace").ok_or("no Workspace class")?;
    let workspace = dm.service(workspace_class).ok_or("no Workspace")?;
    let script = dm.create(script_class)?;
    dm.set_name(script, "Main".into())?;
    dm.set_by_name(script, "Source", Value::String("Main.luau".into()))?;
    dm.set_parent(script, Some(workspace))?;

    let frames = Rc::new(Cell::new(0));
    engine.set_render_hook(Box::new(CountingHook {
        frames: Rc::clone(&frames),
    }))?;
    engine.start()?;

    let mut options = WindowedOptions::new(WindowConfig::new("Metallic: windowed engine example").with_size(960, 540))
        .with_frame_cap(60);
    if let Some(seconds) = std::env::args().nth(1).and_then(|arg| arg.parse::<f64>().ok()) {
        options = options.with_exit_after(Duration::from_secs_f64(seconds));
    }
    let engine = run_windowed(engine, options)?;
    println!(
        "window closed: {} frames, {} drawn, {:.2} s of game time, game {:?}",
        engine.frame_count(),
        frames.get(),
        engine.game_time(),
        engine.state()
    );
    Ok(())
}
