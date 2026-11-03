//! Opens a window and runs a Luau script in it: the engine driven by the platform's event loop, drawn by the
//! renderer.
//!
//! ```text
//! cargo run -p metallic_engine --example windowed            # until the window is closed
//! cargo run -p metallic_engine --example windowed -- 5       # or for 5 seconds
//! ```
//!
//! The script builds a small scene (every part shape, a few materials and two see-through parts) and moves
//! `Workspace.CurrentCamera` around it on `RenderStepped`, so the view follows the camera the script sets. It prints
//! once a second on `Heartbeat`, with how many `RenderStepped` events it saw in that second, and prints every key
//! and button `UserInputService.InputBegan` reports; Space pauses the camera, and Escape toggles a locked, hidden
//! cursor (`MouseBehavior.LockCenter`), and while it is locked the once-a-second line adds how far the mouse moved
//! (`GetMouseDelta`). The render hook is the engine's [`SceneRenderer`], wrapped to print what it drew once a
//! second. The loop is capped at 60 frames per second on top of the window's vsync.

use std::error::Error;
use std::time::{Duration, Instant};

use metallic_core::Value;
use metallic_engine::{
    Engine, EngineConfig, EngineMode, MemorySources, RenderFrame, RenderHook, RenderHookError, RunContext,
    SceneRenderer, WindowedOptions, run_windowed,
};
use metallic_platform::{Size, Window, WindowConfig};

const MAIN: &str = r#"
local RunService = game:GetService("RunService")
print(`started: IsClient {RunService:IsClient()}, IsServer {RunService:IsServer()}`)

-- The scene.
local function part(shape, material, size, position, color, transparency)
    local p = Instance.new("Part")
    p.Shape = shape
    p.Material = material
    p.Size = size
    p.Position = position
    p.Color = color
    p.Transparency = transparency or 0
    p.Anchored = true
    p.Parent = workspace
    return p
end
part(Enum.PartShape.Block, Enum.Material.Concrete, Vector3.new(40, 1, 40), Vector3.new(0, -0.5, 0), Color3.fromRGB(128, 126, 120))
part(Enum.PartShape.Block, Enum.Material.Plastic, Vector3.new(3, 3, 3), Vector3.new(-6, 1.5, -1), Color3.fromRGB(196, 40, 28))
part(Enum.PartShape.Ball, Enum.Material.SmoothPlastic, Vector3.new(3.5, 3.5, 3.5), Vector3.new(-1.5, 1.75, -3), Color3.fromRGB(13, 105, 172))
part(Enum.PartShape.Cylinder, Enum.Material.Wood, Vector3.new(5, 2, 2), Vector3.new(3, 1, -1), Color3.fromRGB(160, 95, 53))
part(Enum.PartShape.Wedge, Enum.Material.Grass, Vector3.new(3, 2.5, 4), Vector3.new(7, 1.25, -2), Color3.fromRGB(75, 151, 75))
part(Enum.PartShape.Ball, Enum.Material.Metal, Vector3.new(2, 2, 2), Vector3.new(0.5, 1, 2), Color3.fromRGB(226, 155, 64))
part(Enum.PartShape.Block, Enum.Material.Neon, Vector3.new(1, 1, 1), Vector3.new(-3, 0.5, 3), Color3.fromRGB(0, 255, 191))
part(Enum.PartShape.Block, Enum.Material.Glass, Vector3.new(5, 3, 0.3), Vector3.new(-4, 1.5, 5), Color3.fromRGB(175, 221, 255), 0.6)
local bouncer = part(Enum.PartShape.Ball, Enum.Material.Plastic, Vector3.new(2, 2, 2), Vector3.new(4, 3.5, 3), Color3.fromRGB(255, 89, 89), 0.35)

-- The camera circles the scene; the renderer draws from wherever it is.
local camera = workspace.CurrentCamera
camera.FieldOfView = 60
local paused = false
local angle = 0
RunService.RenderStepped:Connect(function(deltaTime)
    if not paused then
        angle += deltaTime * 0.4
    end
    local eye = Vector3.new(math.sin(angle) * 16, 7, math.cos(angle) * 16)
    camera.CFrame = CFrame.lookAt(eye, Vector3.new(0, 1, 0))
    bouncer.Position = Vector3.new(4, 3.5 + math.sin(time() * 2), 3)
end)

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
        print(`Heartbeat: {seconds} s, {rendered} RenderStepped this second, camera at {camera.CFrame.Position}`)
        rendered = 0
        if UserInputService.MouseBehavior ~= Enum.MouseBehavior.Default then
            print(`  mouse moved {moved} this second`)
        end
        moved = Vector2.zero
    end
end)

UserInputService.InputBegan:Connect(function(input, gameProcessedEvent)
    if input.UserInputType == Enum.UserInputType.Keyboard then
        print(`InputBegan: {input.KeyCode}, gameProcessedEvent {gameProcessedEvent}`)
        if input.KeyCode == Enum.KeyCode.Space then
            paused = not paused
        end
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
"#;

/// The engine's renderer, printing what it drew and how long the GPU took once a second.
struct Reporting {
    inner: SceneRenderer,
    last_report: Instant,
}

impl RenderHook for Reporting {
    fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
        self.inner.attach(window)?;
        if let Some(renderer) = self.inner.renderer() {
            println!("render hook: attached, {:?}", renderer.settings().color_format);
        }
        Ok(())
    }

    fn detach(&mut self) {
        self.inner.detach();
    }

    fn resize(&mut self, size: Size, scale_factor: f64) {
        self.inner.resize(size, scale_factor);
    }

    fn render(&mut self, frame: &mut RenderFrame<'_>) {
        self.inner.render(frame);
        if self.last_report.elapsed() >= Duration::from_secs(1) {
            self.last_report = Instant::now();
            let stats = self.inner.draw_stats();
            let gpu = self
                .inner
                .renderer()
                .and_then(|renderer| renderer.gpu_timings())
                .map_or(0.0, |timings| timings.total_ms());
            println!(
                "render: {} opaque + {} transparent draws, {} instances, {} triangles, GPU {gpu:.3} ms",
                stats.opaque_draws, stats.transparent_draws, stats.instances, stats.triangles
            );
        }
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

    engine.set_render_hook(Box::new(Reporting {
        inner: SceneRenderer::default(),
        last_report: Instant::now(),
    }))?;
    engine.start()?;

    let mut options = WindowedOptions::new(WindowConfig::new("Metallic: windowed engine example").with_size(960, 540))
        .with_frame_cap(60);
    if let Some(seconds) = std::env::args().nth(1).and_then(|arg| arg.parse::<f64>().ok()) {
        options = options.with_exit_after(Duration::from_secs_f64(seconds));
    }
    let engine = run_windowed(engine, options)?;
    println!(
        "window closed: {} frames, {:.2} s of game time, game {:?}",
        engine.frame_count(),
        engine.game_time(),
        engine.state()
    );
    Ok(())
}
