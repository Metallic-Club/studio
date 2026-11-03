//! Headless Luau games driven by the engine loop: RunService's events, `task.wait` timing, the phase order, the
//! catch-up cap, and pause and step.
#![cfg(feature = "luau")]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use metallic_core::{DataModel, Value};
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext, RunState, Simulation};
use metallic_script::{CollectOutput, OutputLevel};

/// An engine for `context` whose game holds one script per `(name, source)`: `Script`s in `ServerScriptService` on
/// a server and in a standalone game, `LocalScript`s in `Workspace` on a client. Not started.
fn engine_with(config: EngineConfig, scripts: &[(&str, &str)]) -> Engine<CollectOutput> {
    let mut sources = MemorySources::new();
    for (name, source) in scripts {
        sources.insert(&format!("{name}.luau"), source);
    }
    let context = config.context;
    let mut engine = Engine::with_output(config.with_sources(sources), CollectOutput::new()).expect("engine");
    let dm = engine.data_model_mut();
    let (class, service) = match context {
        RunContext::Client => ("LocalScript", "Workspace"),
        RunContext::Server | RunContext::Local => ("Script", "ServerScriptService"),
    };
    for (name, _) in scripts {
        add_script(dm, class, service, name);
    }
    engine
}

fn add_script(dm: &mut DataModel, class: &str, service: &str, name: &str) {
    let class = dm.registry().class_id(class).expect("script class");
    let service = dm.registry().class_id(service).expect("service class");
    let service = dm.service(service).expect("service");
    let script = dm.create(class).expect("create");
    dm.set_name(script, name.into()).expect("name");
    dm.set_by_name(script, "Source", Value::String(format!("{name}.luau").into()))
        .expect("source");
    dm.set_parent(script, Some(service)).expect("parent");
}

fn started(context: RunContext, scripts: &[(&str, &str)]) -> Engine<CollectOutput> {
    let mut engine = engine_with(EngineConfig::new(context), scripts);
    engine.start().expect("start");
    assert_no_errors(&engine);
    engine
}

/// Evaluates the Luau expression `expr` in a throwaway script and returns what it prints, so tests can read the
/// state the game's scripts left in `_G`.
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

fn assert_no_errors(engine: &Engine<CollectOutput>) {
    let errors: Vec<&str> = engine
        .host()
        .output()
        .of_level(OutputLevel::Error)
        .map(|l| l.message.as_str())
        .collect();
    assert!(errors.is_empty(), "script errors: {errors:#?}");
}

const HEARTBEAT_COUNTER: &str = r#"
local RunService = game:GetService("RunService")
_G.heartbeats = 0
_G.worst = 0
_G.total = 0
RunService.Heartbeat:Connect(function(deltaTime)
    _G.heartbeats += 1
    _G.total += deltaTime
    _G.worst = math.max(_G.worst, math.abs(deltaTime - 1 / 60))
end)
"#;

#[test]
fn heartbeat_fires_every_tick_with_a_sixtieth_of_a_second() {
    let mut engine = started(RunContext::Server, &[("Counter", HEARTBEAT_COUNTER)]);
    let last = engine.run_headless(600);
    assert_no_errors(&engine);
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "600");
    let worst: f64 = probe(&mut engine, "_G.worst").parse().expect("a number");
    assert!(worst < 1e-6, "deltaTime strayed from 1/60 by {worst}");
    let total: f64 = probe(&mut engine, "_G.total").parse().expect("a number");
    assert!(
        (total - 10.0).abs() < 1e-5,
        "600 ticks are 10 s of game time, the deltas summed to {total}"
    );
    assert_eq!(last.frame, 600);
    assert_eq!(last.sim_steps, 1);
    assert!((engine.game_time() - 10.0).abs() < 1e-5);
}

#[test]
fn task_wait_one_second_resumes_on_the_sixtieth_tick() {
    let script = r#"
        _G.waited = task.wait(1)
        _G.wokeAt = time()
    "#;
    let mut engine = started(RunContext::Server, &[("Waiter", script)]);
    let mut woke_on = None;
    for tick in 1..=70 {
        engine.tick();
        if probe(&mut engine, "_G.waited") != "nil" {
            woke_on = Some(tick);
            break;
        }
    }
    assert_no_errors(&engine);
    let woke_on = woke_on.expect("task.wait(1) never returned within 70 ticks");
    assert!((59..=61).contains(&woke_on), "woke on tick {woke_on}");
    assert_eq!(
        woke_on, 60,
        "the step is rounded up, so the 60th tick is at or past 1 s"
    );
    let waited: f64 = probe(&mut engine, "_G.waited").parse().expect("a number");
    assert!((1.0..1.0 + 1e-6).contains(&waited), "task.wait returned {waited}");
    let woke_at: f64 = probe(&mut engine, "_G.wokeAt").parse().expect("a number");
    assert!((woke_at - 1.0).abs() < 1e-6, "time() at wake was {woke_at}");
}

const PHASE_LOG: &str = r#"
local RunService = game:GetService("RunService")
_G.log = {}
for _, name in {"RenderStepped", "PreSimulation", "PostSimulation", "Heartbeat"} do
    RunService[name]:Connect(function()
        table.insert(_G.log, name)
    end)
end
"#;

#[test]
fn a_script_sees_the_phases_in_frame_order() {
    let mut engine = started(RunContext::Local, &[("Phases", PHASE_LOG)]);
    engine.run_headless(2);
    assert_no_errors(&engine);
    assert_eq!(
        probe(&mut engine, "table.concat(_G.log, ',')"),
        "RenderStepped,PreSimulation,PostSimulation,Heartbeat,RenderStepped,PreSimulation,PostSimulation,Heartbeat"
    );
}

#[test]
fn a_server_never_fires_render_stepped() {
    let mut engine = started(RunContext::Server, &[("Phases", PHASE_LOG)]);
    engine.run_headless(60);
    assert_no_errors(&engine);
    let log = probe(&mut engine, "table.concat(_G.log, ',')");
    assert!(!log.contains("RenderStepped"), "{log}");
    assert_eq!(
        log.split(',').count(),
        180,
        "PreSimulation, PostSimulation and Heartbeat, 60 times each"
    );
    assert!(log.starts_with("PreSimulation,PostSimulation,Heartbeat,PreSimulation"));
}

#[test]
fn a_client_fires_render_stepped_for_local_scripts() {
    let mut engine = started(RunContext::Client, &[("Phases", PHASE_LOG)]);
    engine.run_headless(1);
    assert_no_errors(&engine);
    assert_eq!(
        probe(&mut engine, "table.concat(_G.log, ',')"),
        "RenderStepped,PreSimulation,PostSimulation,Heartbeat"
    );
}

#[test]
fn run_service_reports_the_context() {
    let flags = r#"
        local RunService = game:GetService("RunService")
        _G.flags = `{RunService:IsStudio()} {RunService:IsRunning()} {RunService:IsClient()} {RunService:IsServer()}`
    "#;
    let mut server = started(RunContext::Server, &[("Flags", flags)]);
    assert_eq!(probe(&mut server, "_G.flags"), "false true false true");

    let mut client = started(RunContext::Client, &[("Flags", flags)]);
    assert_eq!(probe(&mut client, "_G.flags"), "false true true false");

    let mut studio = engine_with(
        EngineConfig::new(RunContext::Local).with_studio(true),
        &[("Flags", flags)],
    );
    studio.start().expect("start");
    assert_eq!(probe(&mut studio, "_G.flags"), "true true true true");

    assert!(studio.stop());
    let running = "game:GetService('RunService'):IsRunning()";
    assert_eq!(probe(&mut studio, running), "false");
    assert_eq!(studio.state(), RunState::Stopped);
}

/// Counts the fixed steps the engine runs.
struct CountSteps(Rc<Cell<u32>>);

impl Simulation for CountSteps {
    fn step(&mut self, _dm: &mut DataModel, dt: Duration) {
        assert_eq!(dt, Duration::from_nanos(16_666_667));
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn a_one_second_stall_runs_at_most_the_catch_up_cap() {
    let config = EngineConfig::new(RunContext::Server)
        .with_max_catch_up(5)
        .with_max_frame_time(Duration::from_secs(2));
    let mut engine = engine_with(config, &[("Counter", HEARTBEAT_COUNTER)]);
    let steps = Rc::new(Cell::new(0));
    engine.set_simulation(Box::new(CountSteps(Rc::clone(&steps))));
    engine.start().expect("start");

    let stall = engine.frame(Duration::from_secs(1));
    assert_eq!(stall.sim_steps, 5);
    assert_eq!(steps.get(), 5);
    assert!(stall.dropped_steps > 0);
    assert!(
        (stall.dt - 1.0).abs() < 1e-9,
        "game time follows the stall up to max_frame_time"
    );
    // One frame of scripts, whatever the stall: Heartbeat fired once.
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "1");

    // The next frame is an ordinary one: no debt was carried.
    let next = engine.tick();
    assert!(next.sim_steps <= 1, "{next:?}");
    assert_no_errors(&engine);
}

#[test]
fn a_long_frame_moves_game_time_by_the_longest_frame_only() {
    let mut engine = started(RunContext::Server, &[]);
    let stats = engine.frame(Duration::from_secs(10));
    assert!((stats.dt - 0.25).abs() < 1e-9);
    assert!((engine.game_time() - 0.25).abs() < 1e-9);
    assert_eq!(stats.sim_steps, 8, "the default catch-up cap");
}

#[test]
fn pause_freezes_the_game_and_step_frame_runs_one_frame() {
    let script = r#"
        local RunService = game:GetService("RunService")
        _G.heartbeats = 0
        RunService.Heartbeat:Connect(function()
            _G.heartbeats += 1
            _G.time = time()
        end)
    "#;
    let mut engine = started(RunContext::Server, &[("Counter", script)]);
    let steps = Rc::new(Cell::new(0));
    engine.set_simulation(Box::new(CountSteps(Rc::clone(&steps))));
    engine.run_headless(10);
    let before = engine.game_time();

    assert!(engine.pause());
    assert!(!engine.pause(), "already paused");
    assert_eq!(engine.state(), RunState::Paused);
    let frame = engine.run_headless(30);
    assert!(!frame.ran);
    assert_eq!(frame.dt, 0.0);
    assert_eq!(engine.frame_count(), 40, "paused frames still count");
    assert_eq!(engine.game_time(), before, "game time stands still");
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "10");
    assert_eq!(steps.get(), 10);

    let stepped = engine.step_frame().expect("a paused game steps");
    assert!(stepped.ran);
    assert_eq!(stepped.sim_steps, 1);
    assert_eq!(engine.state(), RunState::Paused, "stepping does not resume");
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "11");
    assert_eq!(steps.get(), 11);
    let time: f64 = probe(&mut engine, "_G.time").parse().expect("a number");
    assert!((time - 11.0 / 60.0).abs() < 1e-6, "time() is game time: {time}");

    assert!(engine.resume());
    engine.run_headless(5);
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "16");
    assert_no_errors(&engine);
}

#[test]
fn a_stopped_game_runs_no_scripts_and_keeps_time_still() {
    let mut engine = engine_with(EngineConfig::new(RunContext::Server), &[("Counter", HEARTBEAT_COUNTER)]);
    let frame = engine.run_headless(5);
    assert!(!frame.ran);
    assert_eq!(engine.game_time(), 0.0);
    assert!(engine.step_frame().is_none());
    assert!(engine.host().output().lines.is_empty());

    engine.start().expect("start");
    assert!(matches!(
        engine.start(),
        Err(metallic_engine::EngineError::AlreadyRunning)
    ));
    engine.run_headless(3);
    assert_eq!(probe(&mut engine, "_G.heartbeats"), "3");
}

#[test]
fn script_errors_reach_the_output_and_the_loop_goes_on() {
    let script = r#"
        local RunService = game:GetService("RunService")
        RunService.Heartbeat:Connect(function()
            error("boom")
        end)
    "#;
    let mut engine = started(RunContext::Server, &[("Broken", script)]);
    engine.run_headless(3);
    let errors = engine.host().output().of_level(OutputLevel::Error).count();
    assert_eq!(errors, 3, "one error per Heartbeat, each ending only its own thread");
}
