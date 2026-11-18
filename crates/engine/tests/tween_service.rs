//! `TweenService` (#810) driven through the engine: `TweenInfo` from Luau, a tween moving a part a frame at a time,
//! pausing, cancelling, reversing, repeating and `Completed`.
#![cfg(feature = "luau")]

use metallic_core::{DataModel, Value};
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};
use metallic_script::{CollectOutput, OutputLevel};

/// A started standalone game with a part `P` in Workspace.
fn started() -> Engine<CollectOutput> {
    let sources = MemorySources::new().with("Main.luau", "");
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    add_part(engine.data_model_mut());
    engine.start().expect("start");
    engine
}

fn add_part(dm: &mut DataModel) {
    let class = dm.registry().class_id("Part").expect("Part");
    let workspace = dm.registry().class_id("Workspace").expect("Workspace class");
    let workspace = dm.service(workspace).expect("Workspace");
    let part = dm.create(class).expect("create");
    dm.set_name(part, "P".into()).expect("name");
    dm.set_by_name(part, "Anchored", Value::Bool(true)).expect("anchored");
    dm.set_parent(part, Some(workspace)).expect("parent");
}

/// Runs `code` in a throwaway script and returns what it prints, one line per `print`.
fn run(engine: &mut Engine<CollectOutput>, code: &str) -> String {
    let (dm, host) = engine.parts_mut();
    let class = dm.registry().class_id("Script").expect("Script");
    let script = dm.create(class).expect("runner");
    // In a service, so it is not reaped with its connections at the next frame.
    let service = dm.registry().class_id("ServerScriptService").expect("service class");
    let service = dm.service(service).expect("ServerScriptService");
    dm.set_parent(script, Some(service)).expect("parent");
    let before = host.output().lines.len();
    host.run_script(dm, script, "Run.luau", code).expect("runs");
    let printed: Vec<String> = host.output().lines[before..]
        .iter()
        .map(|line| {
            assert_eq!(line.level, OutputLevel::Print, "failed: {}", line.message);
            line.message.clone()
        })
        .collect();
    host.output_mut().lines.truncate(before);
    printed.join("\n")
}

/// Steps frames until `seconds` of game time passed.
fn advance(engine: &mut Engine<CollectOutput>, seconds: f64) {
    let until = engine.game_time() + seconds - 1e-6;
    while engine.game_time() < until {
        engine.tick();
    }
}

/// The part's transparency, rounded to two places.
fn transparency(engine: &mut Engine<CollectOutput>) -> String {
    run(engine, "print(string.format(\"%.2f\", workspace.P.Transparency))")
}

#[test]
fn tween_info_reads_as_roblox_s() {
    let mut engine = started();
    assert_eq!(
        run(
            &mut engine,
            "local info = TweenInfo.new()
             print(typeof(info), info.Time, info.EasingStyle.Name, info.EasingDirection.Name, info.RepeatCount, \
                   info.Reverses, info.DelayTime)
             local other = TweenInfo.new(2, Enum.EasingStyle.Linear, Enum.EasingDirection.InOut, -1, true, 0.5)
             print(tostring(other))
             print(TweenInfo.new(2, Enum.EasingStyle.Linear) == TweenInfo.new(2, Enum.EasingStyle.Linear))
             print(game:GetService(\"TweenService\"):GetValue(0.5, Enum.EasingStyle.Quad, Enum.EasingDirection.In))
             print(pcall(TweenInfo.new, 1, Enum.EasingDirection.In))"
        ),
        "TweenInfo 1 Quad Out 0 false 0\n\
         Time:2 DelayTime:0.5 RepeatCount:-1 Reverses:True EasingDirection:InOut EasingStyle:Linear\n\
         true\n\
         0.25\n\
         false bad argument #2 to 'new' (Enum.EasingStyle expected, got EnumItem)"
    );
}

#[test]
fn a_tween_moves_pauses_and_completes() {
    let mut engine = started();
    run(
        &mut engine,
        "local TweenService = game:GetService(\"TweenService\")
         local info = TweenInfo.new(1, Enum.EasingStyle.Linear)
         _G.tween = TweenService:Create(workspace.P, info, { Transparency = 1, Position = Vector3.new(10, 0, 0) })
         _G.done = {}
         _G.tween.Completed:Connect(function(state) table.insert(_G.done, state.Name) end)
         print(_G.tween.PlaybackState.Name, _G.tween.Instance == workspace.P)
         _G.tween:Play()",
    );
    advance(&mut engine, 0.5);
    assert_eq!(transparency(&mut engine), "0.50");
    assert_eq!(run(&mut engine, "print(_G.tween.PlaybackState.Name)"), "Playing");
    run(&mut engine, "_G.tween:Pause()");
    advance(&mut engine, 0.5);
    assert_eq!(transparency(&mut engine), "0.50", "paused");
    run(&mut engine, "_G.tween:Play()");
    advance(&mut engine, 0.6);
    assert_eq!(transparency(&mut engine), "1.00");
    assert_eq!(
        run(
            &mut engine,
            "print(_G.tween.PlaybackState.Name, table.concat(_G.done, \",\"), workspace.P.Position.X)"
        ),
        "Completed Completed 10"
    );
}

#[test]
fn cancel_reverse_repeat_and_a_newer_tween_of_the_same_property() {
    let mut engine = started();
    run(
        &mut engine,
        "local TweenService = game:GetService(\"TweenService\")
         _G.done = {}
         _G.back = TweenService:Create(workspace.P, TweenInfo.new(0.5, Enum.EasingStyle.Linear, \
             Enum.EasingDirection.Out, 1, true), { Transparency = 1 })
         _G.back.Completed:Connect(function(state) table.insert(_G.done, \"back \" .. state.Name) end)
         _G.back:Play()",
    );
    // Up and back down, twice.
    advance(&mut engine, 0.5);
    assert_eq!(transparency(&mut engine), "1.00");
    advance(&mut engine, 0.25);
    assert_eq!(transparency(&mut engine), "0.50");
    advance(&mut engine, 1.3);
    assert_eq!(transparency(&mut engine), "0.00");
    assert_eq!(
        run(&mut engine, "print(table.concat(_G.done, \",\"))"),
        "back Completed"
    );

    // A newer tween of the property cancels the older one; Cancel stops where it is.
    run(
        &mut engine,
        "local TweenService = game:GetService(\"TweenService\")
         _G.slow = TweenService:Create(workspace.P, TweenInfo.new(10, Enum.EasingStyle.Linear), { Transparency = 1 })
         _G.slow.Completed:Connect(function(state) table.insert(_G.done, \"slow \" .. state.Name) end)
         _G.slow:Play()",
    );
    advance(&mut engine, 1.0);
    run(
        &mut engine,
        "local TweenService = game:GetService(\"TweenService\")
         _G.fast = TweenService:Create(workspace.P, TweenInfo.new(1, Enum.EasingStyle.Linear), { Transparency = 0 })
         _G.fast.Completed:Connect(function(state) table.insert(_G.done, \"fast \" .. state.Name) end)
         _G.fast:Play()",
    );
    advance(&mut engine, 0.5);
    run(&mut engine, "_G.fast:Cancel()");
    let held = transparency(&mut engine);
    advance(&mut engine, 0.5);
    assert_eq!(transparency(&mut engine), held, "cancelled where it was");
    assert_eq!(
        run(
            &mut engine,
            "print(table.concat(_G.done, \",\"), _G.slow.PlaybackState.Name, _G.fast.PlaybackState.Name)"
        ),
        "back Completed,slow Cancelled,fast Cancelled Cancelled Cancelled"
    );
    // A property that cannot be tweened is refused.
    assert!(
        run(
            &mut engine,
            "print(pcall(function() game:GetService(\"TweenService\"):Create(workspace.P, TweenInfo.new(), \
             { Name = \"x\" }) end))"
        )
        .starts_with("false")
    );
}
