//! A dedicated server in miniature: no window, no platform loop, one Luau script, three seconds of game time paced in
//! real time at the fixed tick rate.
//!
//! ```text
//! cargo run -p metallic_engine --example headless --no-default-features --features physics,luau
//! ```
//!
//! The script prints the run flags at start, then a line on `Heartbeat` once per second of game time.

use std::error::Error;
use std::time::Instant;

use metallic_core::Value;
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};

const MAIN: &str = r#"
local RunService = game:GetService("RunService")
print(`started: IsServer {RunService:IsServer()}, IsClient {RunService:IsClient()}, IsRunning {RunService:IsRunning()}`)

local seconds = 0
local since = 0
RunService.Heartbeat:Connect(function(deltaTime)
    since += deltaTime
    if since >= 1 then
        since -= 1
        seconds += 1
        print(`Heartbeat: {seconds} s of game time, time() = {string.format("%.3f", time())}`)
    end
end)
"#;

/// Ticks to run: three seconds at 60 Hz.
const TICKS: u32 = 180;

fn main() -> Result<(), Box<dyn Error>> {
    let sources = MemorySources::new().with("Main.luau", MAIN);
    let mut engine = Engine::new(EngineConfig::new(RunContext::Server).with_sources(sources))?;

    let dm = engine.data_model_mut();
    let registry = dm.registry().clone();
    let script_class = registry.class_id("Script").ok_or("no Script class")?;
    let service_class = registry
        .class_id("ServerScriptService")
        .ok_or("no ServerScriptService class")?;
    let service = dm.service(service_class).ok_or("no ServerScriptService")?;
    let script = dm.create(script_class)?;
    dm.set_name(script, "Main".into())?;
    dm.set_by_name(script, "Source", Value::String("Main.luau".into()))?;
    dm.set_parent(script, Some(service))?;

    engine.start()?;
    let step = engine.fixed_step();
    let began = Instant::now();
    for tick in 1..=TICKS {
        engine.tick();
        // A server ticks at a fixed rate: sleep until the next tick is due rather than running ahead.
        let due = step * tick;
        let elapsed = began.elapsed();
        if elapsed < due {
            std::thread::sleep(due - elapsed);
        }
    }
    engine.stop();
    println!(
        "ran {} ticks: {:.3} s of game time in {:.3} s of wall time",
        engine.frame_count(),
        engine.game_time(),
        began.elapsed().as_secs_f64()
    );
    Ok(())
}
