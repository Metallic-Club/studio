//! `TweenService`, `TweenInfo` and `Tween` (#79).

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Capture, Rig};
use metallic_core::{Color3, InstanceId, UDim2, Value, Vec3, World};
use metallic_script::ScriptHost;

const DT: f64 = 1.0 / 60.0;

fn number(rig: &Rig, id: InstanceId, prop: &str) -> f64 {
    match rig.world.borrow().get(id, prop).unwrap() {
        Value::Number(n) => n,
        other => panic!("{prop} is {other:?}"),
    }
}

fn vector(rig: &Rig, id: InstanceId, prop: &str) -> Vec3 {
    match rig.world.borrow().get(id, prop).unwrap() {
        Value::Vector3(v) => v,
        other => panic!("{prop} is {other:?}"),
    }
}

fn started(rig: &mut Rig) {
    rig.host.start().expect("host starts");
}

fn no_errors(rig: &Rig) {
    let lines = rig.lines();
    assert!(
        !lines.iter().any(|l| l.starts_with("error:")),
        "script errors: {lines:?}"
    );
}

/// Reference values from the standard easing formulas (the ones Roblox documents), checked at
/// a few alphas through `TweenService:GetValue`.
#[test]
fn easing_values_match_reference_numbers() {
    let mut rig = Rig::new();
    rig.server_script(
        "Ease",
        r#"
        local TS = game:GetService("TweenService")
        local S, D = Enum.EasingStyle, Enum.EasingDirection
        local cases = {
            {0.3, S.Linear, D.In}, {0.5, S.Quad, D.In}, {0.5, S.Quad, D.Out},
            {0.25, S.Quad, D.InOut}, {0.75, S.Quad, D.InOut}, {0.5, S.Cubic, D.In},
            {0.5, S.Quart, D.In}, {0.5, S.Quint, D.In}, {0.5, S.Sine, D.In},
            {0.5, S.Sine, D.Out}, {0.5, S.Back, D.In}, {0.5, S.Exponential, D.In},
            {0.5, S.Circular, D.In}, {0.5, S.Bounce, D.Out}, {0.5, S.Elastic, D.Out},
            {0.2, S.Bounce, D.In}, {0, S.Elastic, D.In}, {1, S.Back, D.Out},
            {1.5, S.Linear, D.In}, {-0.5, S.Quad, D.Out},
        }
        for _, c in cases do
            print(string.format("%.6f", TS:GetValue(c[1], c[2], c[3])))
        end
        "#,
    );
    started(&mut rig);
    no_errors(&rig);
    let got: Vec<f64> = rig
        .lines()
        .iter()
        .map(|l| l.trim_start_matches("print:").parse().unwrap())
        .collect();
    let s = 1.70158_f64;
    let bounce = |t: f64| {
        // easeOutBounce
        let (n, d) = (7.5625, 2.75);
        if t < 1.0 / d {
            n * t * t
        } else if t < 2.0 / d {
            let t = t - 1.5 / d;
            n * t * t + 0.75
        } else if t < 2.5 / d {
            let t = t - 2.25 / d;
            n * t * t + 0.9375
        } else {
            let t = t - 2.625 / d;
            n * t * t + 0.984375
        }
    };
    let expected = [
        0.3,
        0.25,
        0.75,
        0.125,
        0.875,
        0.125,
        0.0625,
        0.03125,
        1.0 - (std::f64::consts::FRAC_PI_4).cos(),
        (std::f64::consts::FRAC_PI_4).sin(),
        0.25 * ((s + 1.0) * 0.5 - s),
        2f64.powf(-5.0),
        1.0 - 0.75f64.sqrt(),
        0.765625,
        1.015625,
        1.0 - bounce(0.8),
        0.0,
        1.0,
        1.0,
        0.0,
    ];
    assert_eq!(got.len(), expected.len(), "{:?}", rig.lines());
    for (i, (g, e)) in got.iter().zip(expected).enumerate() {
        assert!((g - e).abs() < 2e-6, "case {i}: got {g}, expected {e}");
    }
}

#[test]
fn tween_info_defaults_and_fields() {
    let mut rig = Rig::new();
    rig.server_script(
        "Info",
        r#"
        local a = TweenInfo.new()
        print(a.Time, a.EasingStyle == Enum.EasingStyle.Quad, a.EasingDirection == Enum.EasingDirection.Out, a.RepeatCount, a.Reverses, a.DelayTime)
        local b = TweenInfo.new(2, Enum.EasingStyle.Bounce, Enum.EasingDirection.InOut, 3, true, 0.5)
        print(b.Time, b.EasingStyle, b.EasingDirection, b.RepeatCount, b.Reverses, b.DelayTime, typeof(b))
        print(TweenInfo.new(2) == TweenInfo.new(2), TweenInfo.new(2) == TweenInfo.new(3))
        print(pcall(TweenInfo.new, 1, Enum.EasingDirection.In))
        print(pcall(TweenInfo.new, "fast"))
        "#,
    );
    started(&mut rig);
    let lines = rig.lines();
    assert_eq!(lines[0], "print:1 true true 0 false 0");
    assert_eq!(
        lines[1],
        "print:2 Enum.EasingStyle.Bounce Enum.EasingDirection.InOut 3 true 0.5 TweenInfo"
    );
    assert_eq!(lines[2], "print:true false");
    assert!(
        lines[3].starts_with("print:false") && lines[3].contains("Enum.EasingStyle"),
        "{lines:?}"
    );
    assert!(
        lines[4].starts_with("print:false") && lines[4].contains("must be a number"),
        "{lines:?}"
    );
}

#[test]
fn number_tween_advances_on_the_step_clock_and_completes_once() {
    let mut rig = Rig::new();
    let part = rig.add("Part", "Glass", rig.root("Workspace"));
    rig.server_script(
        "Fade",
        r#"
        local TS = game:GetService("TweenService")
        local t = TS:Create(workspace.Glass, TweenInfo.new(1, Enum.EasingStyle.Linear), {Transparency = 1})
        print(t.PlaybackState, t.Instance == workspace.Glass, t.ClassName, t:IsA("Tween"), typeof(t))
        t.Completed:Connect(function(state) print("completed", state, t.PlaybackState) end)
        t:Play()
        print(t.PlaybackState)
        _G.tween = t
        "#,
    );
    started(&mut rig);
    no_errors(&rig);
    assert_eq!(
        number(&rig, part, "Transparency"),
        0.0,
        "Play writes nothing until a step"
    );
    let now = rig.run(0.0, 30);
    assert!((number(&rig, part, "Transparency") - 0.5).abs() < 1e-9);
    rig.run(now, 60);
    assert_eq!(number(&rig, part, "Transparency"), 1.0);
    let lines = rig.lines();
    assert_eq!(
        lines,
        [
            "print:Enum.PlaybackState.Begin true Tween true Instance",
            "print:Enum.PlaybackState.Playing",
            "print:completed Enum.PlaybackState.Completed Enum.PlaybackState.Completed",
        ]
    );
}

#[test]
fn every_tweenable_type_reaches_its_goal() {
    let mut rig = Rig::new();
    let ws = rig.root("Workspace");
    let part = rig.add("Part", "Box", ws);
    let gui = rig.add("ScreenGui", "Hud", rig.root("StarterGui"));
    let frame = rig.add("Frame", "Panel", gui);
    rig.server_script(
        "Types",
        r#"
        local TS = game:GetService("TweenService")
        local info = TweenInfo.new(0.5, Enum.EasingStyle.Sine, Enum.EasingDirection.InOut)
        TS:Create(workspace.Box, info, {
            Size = Vector3.new(4, 6, 8),
            Color = Color3.fromRGB(255, 0, 128),
            Anchored = true,
        }):Play()
        TS:Create(game.StarterGui.Hud.Panel, info, {Size = UDim2.new(0.5, 10, 0.25, -4)}):Play()
        "#,
    );
    started(&mut rig);
    no_errors(&rig);
    let now = rig.run(0.0, 15);
    let size = vector(&rig, part, "Size");
    assert!(
        (size.y - 3.5).abs() < 1e-9,
        "half-way (Sine InOut is 0.5 at 0.5) {size:?}"
    );
    assert_eq!(
        rig.world.borrow().get(part, "Anchored").unwrap(),
        Value::Bool(false),
        "a bool keeps its start value until the end"
    );
    rig.run(now, 30);
    let world = rig.world.borrow();
    assert_eq!(
        world.get(part, "Size").unwrap(),
        Value::Vector3(Vec3::new(4.0, 6.0, 8.0))
    );
    assert_eq!(
        world.get(part, "Color").unwrap(),
        Value::Color3(Color3::new(255.0, 0.0, 128.0))
    );
    assert_eq!(world.get(part, "Anchored").unwrap(), Value::Bool(true));
    assert_eq!(
        world.get(frame, "Size").unwrap(),
        Value::UDim2(UDim2::new(0.5, 10.0, 0.25, -4.0))
    );
}

#[test]
fn part_cframe_tween_moves_and_turns_the_part() {
    let mut rig = Rig::new();
    let part = rig.add("Part", "Door", rig.root("Workspace"));
    rig.server_script(
        "Swing",
        r#"
        local TS = game:GetService("TweenService")
        local goal = CFrame.new(10, 2, 0) * CFrame.Angles(0, math.rad(90), 0)
        TS:Create(workspace.Door, TweenInfo.new(1, Enum.EasingStyle.Linear), {CFrame = goal}):Play()
        "#,
    );
    started(&mut rig);
    no_errors(&rig);
    let now = rig.run(0.0, 30);
    let mid = vector(&rig, part, "Orientation");
    assert!((mid.y - 45.0).abs() < 1e-6, "half-way turn {mid:?}");
    assert!((vector(&rig, part, "Position").x - 5.0).abs() < 1e-9);
    rig.run(now, 31);
    let p = vector(&rig, part, "Position");
    let o = vector(&rig, part, "Orientation");
    assert!((p - Vec3::new(10.0, 2.0, 0.0)).magnitude() < 1e-9, "{p:?}");
    assert!((o.y - 90.0).abs() < 1e-6, "{o:?}");
}

#[test]
fn newer_tween_on_a_property_cancels_the_older_one() {
    let mut rig = Rig::new();
    let part = rig.add("Part", "Block", rig.root("Workspace"));
    rig.server_script(
        "Override",
        r#"
        local TS = game:GetService("TweenService")
        local p = workspace.Block
        local linear = TweenInfo.new(1, Enum.EasingStyle.Linear)
        local a = TS:Create(p, linear, {Position = Vector3.new(10, 0, 0), Transparency = 0.5})
        local b = TS:Create(p, linear, {Position = Vector3.new(0, 10, 0)})
        local other = TS:Create(p, linear, {Size = Vector3.new(2, 2, 2)})
        for name, t in {a = a, b = b, other = other} do
            t.Completed:Connect(function(state) print(name, state.Name) end)
        end
        a:Play()
        other:Play()
        local frames = 0
        game:GetService("RunService").Heartbeat:Connect(function()
            frames += 1
            if frames == 30 then
                b:Play()
                print("b played", a.PlaybackState.Name, other.PlaybackState.Name)
            end
        end)
        "#,
    );
    started(&mut rig);
    rig.run(0.0, 120);
    no_errors(&rig);
    let mut lines = rig.lines();
    lines.sort();
    assert_eq!(
        lines,
        [
            "print:a Cancelled",
            "print:b Completed",
            "print:b played Cancelled Playing",
            "print:other Completed",
        ]
    );
    // a stopped half-way (x = 5, Transparency 0.25); b moved from there to its goal.
    assert_eq!(vector(&rig, part, "Position"), Vec3::new(0.0, 10.0, 0.0));
    assert!((number(&rig, part, "Transparency") - 0.25).abs() < 1e-9);
}

#[test]
fn cancel_pause_and_replay() {
    let mut rig = Rig::new();
    let part = rig.add("Part", "Lift", rig.root("Workspace"));
    rig.server_script(
        "Control",
        r#"
        local TS = game:GetService("TweenService")
        local t = TS:Create(workspace.Lift, TweenInfo.new(1, Enum.EasingStyle.Linear), {Transparency = 1})
        local events = 0
        t.Completed:Connect(function(state)
            events += 1
            print("completed", state.Name, events)
        end)
        _G.t = t
        t:Play()
        "#,
    );
    started(&mut rig);
    let now = rig.run(0.0, 30);
    assert!((number(&rig, part, "Transparency") - 0.5).abs() < 1e-9);
    rig.server_script("Pause", "_G.t:Pause() print(_G.t.PlaybackState.Name)");
    let now = rig.run(now, 30);
    assert!(
        (number(&rig, part, "Transparency") - 0.5).abs() < 1e-9,
        "paused"
    );
    rig.server_script("Resume", "_G.t:Play()");
    let now = rig.run(now, 15);
    // The new script starts in the first of those steps, before the tweens advance.
    assert!((number(&rig, part, "Transparency") - 0.75).abs() < 1e-9);
    rig.server_script(
        "Cancel",
        "_G.t:Cancel() _G.t:Cancel() print(_G.t.PlaybackState.Name)",
    );
    let now = rig.run(now, 30);
    assert!(
        (number(&rig, part, "Transparency") - 0.75).abs() < 1e-9,
        "cancel leaves the value"
    );
    // Playing again starts from the current value and takes the full time.
    rig.server_script("Again", "_G.t:Play()");
    let now = rig.run(now, 30);
    assert!((number(&rig, part, "Transparency") - 0.875).abs() < 1e-9);
    rig.run(now, 40);
    assert_eq!(number(&rig, part, "Transparency"), 1.0);
    no_errors(&rig);
    assert_eq!(
        rig.lines(),
        [
            "print:Paused",
            "print:Cancelled",
            "print:completed Cancelled 1",
            "print:completed Completed 2",
        ]
    );
}

/// Roblox waits `DelayTime` before every pass: the first, each reverse and each repeat.
#[test]
fn delay_comes_before_every_pass_reverses_and_repeats_included() {
    let mut rig = Rig::new();
    let part = rig.add("Part", "Pulse", rig.root("Workspace"));
    rig.server_script(
        "Pulse",
        r#"
        local TS = game:GetService("TweenService")
        -- 2 cycles of (0.25 s delay, 0.25 s out, 0.25 s delay, 0.25 s back): done at 2 s.
        local t = TS:Create(workspace.Pulse, TweenInfo.new(0.25, Enum.EasingStyle.Linear, Enum.EasingDirection.Out, 1, true, 0.25), {Transparency = 1})
        t.Completed:Connect(function(state) print("done", state.Name) end)
        t:Play()
        local last
        game:GetService("RunService").Heartbeat:Connect(function()
            if t.PlaybackState ~= last then
                last = t.PlaybackState
                print(last.Name)
            end
        end)
        "#,
    );
    started(&mut rig);
    let mut now = 0.0;
    let mut at = |rig: &mut Rig, frames: usize| {
        now = rig.run(now, frames);
        number(rig, part, "Transparency")
    };
    let near = |got: f64, want: f64, what: &str| {
        assert!((got - want).abs() < 1e-6, "{what}: {got}, want {want}");
    };
    near(at(&mut rig, 12), 0.0, "0.2 s: first delay");
    near(at(&mut rig, 12), 0.6, "0.4 s: out");
    near(
        at(&mut rig, 12),
        1.0,
        "0.6 s: delay before the reverse, held at the goal",
    );
    near(at(&mut rig, 18), 0.4, "0.9 s: back");
    near(
        at(&mut rig, 12),
        0.0,
        "1.1 s: delay before the repeat, held at the start",
    );
    near(at(&mut rig, 18), 0.6, "1.4 s: out again");
    near(
        at(&mut rig, 12),
        1.0,
        "1.6 s: delay before the last reverse",
    );
    assert!(!rig.lines().iter().any(|l| l.contains("done")));
    near(at(&mut rig, 25), 0.0, "2.0 s: done, back at the start");
    no_errors(&rig);
    let delayed_playing = ["print:Delayed", "print:Playing"];
    let mut expected: Vec<&str> = delayed_playing.repeat(4);
    expected.extend(["print:done Completed", "print:Completed"]);
    assert_eq!(rig.lines(), expected);
}

/// A tween's Rust-side state is charged to the Lua memory budget while its Lua object lives.
#[test]
fn created_tweens_are_charged_to_the_lua_memory_budget() {
    let mut rig = Rig::new();
    rig.add("Part", "P", rig.root("Workspace"));
    rig.server_script(
        "Many",
        r#"
        local TS = game:GetService("TweenService")
        local info = TweenInfo.new()
        local goals = {Transparency = 1, Position = Vector3.new(1, 2, 3), Color = Color3.fromRGB(255, 0, 0)}
        local kept = {}
        local n = 2000
        local before = collectgarbage("count")
        for i = 1, n do
            kept[i] = TS:Create(workspace.P, info, goals)
        end
        local per = (collectgarbage("count") - before) * 1024 / n
        print(string.format("%d", per))
        "#,
    );
    started(&mut rig);
    no_errors(&rig);
    let per: f64 = rig.lines()[0].trim_start_matches("print:").parse().unwrap();
    // The Lua object and table slot are well under 200 bytes; the charged buffer adds the Rust
    // state (the Rc'd TweenData and three goals, several hundred bytes).
    assert!(
        per > 500.0,
        "{per} bytes per tween: the Rust state is not charged"
    );
}

#[test]
fn create_rejects_bad_goals() {
    let mut rig = Rig::new();
    rig.add("Part", "P", rig.root("Workspace"));
    rig.server_script(
        "Bad",
        r#"
        local TS = game:GetService("TweenService")
        local info = TweenInfo.new()
        local p = workspace.P
        for _, goals in {{Nope = 1}, {Name = "x"}, {Transparency = Vector3.new()}, {[1] = 2}} do
            local ok, err = pcall(TS.Create, TS, p, info, goals)
            print(ok, err)
        end
        print(pcall(TS.Create, TS, "part", info, {}))
        print(pcall(TS.Create, TS, p, {}, {}))
        print(game:GetService("TweenService") == TS, TS.ClassName)
        "#,
    );
    started(&mut rig);
    let lines = rig.lines();
    let expect = [
        "no property named 'Nope'",
        "is a String, which cannot be tweened",
        "invalid value for property Transparency",
        "goal keys must be property names",
        "argument 1 must be an Instance",
        "argument 2 must be a TweenInfo",
    ];
    for (line, text) in lines.iter().zip(expect) {
        assert!(
            line.starts_with("print:false") && line.contains(text),
            "{line} should mention {text}"
        );
    }
    assert_eq!(lines[6], "print:true TweenService");
}

/// Two identical runs give bit-identical values: the tweens follow the host's step clock.
#[test]
fn tweens_are_deterministic_on_a_server_only_host() {
    let run = || {
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let part = world.create("Part", Some(ws)).unwrap();
        world
            .set(part, "Name", Value::String("Ball".into()))
            .unwrap();
        let sss = world.root("ServerScriptService").unwrap();
        let script = world.create("Script", Some(sss)).unwrap();
        world
            .set(
                script,
                "Source",
                Value::String(
                    r#"
                    local TS = game:GetService("TweenService")
                    TS:Create(workspace.Ball, TweenInfo.new(0.7, Enum.EasingStyle.Elastic, Enum.EasingDirection.Out, -1, true), {Position = Vector3.new(3, 7, 1)}):Play()
                    "#
                    .into(),
                ),
            )
            .unwrap();
        let world = Rc::new(RefCell::new(world));
        let out = Rc::new(Capture::default());
        let mut host = ScriptHost::new_server(world.clone(), out);
        host.start().unwrap();
        let mut samples = Vec::new();
        let mut now = 0.0;
        // Uneven steps, as a headless simulation might take.
        for i in 0..200 {
            let dt = if i % 3 == 0 { DT * 2.0 } else { DT };
            now += dt;
            host.step(now, dt);
            let Value::Vector3(p) = world.borrow().get(part, "Position").unwrap() else {
                panic!()
            };
            samples.push(p);
        }
        samples
    };
    let a = run();
    let b = run();
    assert_eq!(a, b);
    assert!(a.iter().any(|p| p.y > 7.0), "Elastic overshoots the goal");
}

/// Playing tweens allocate nothing per tween: a step costs the same with 20 or 200 of them.
#[test]
fn playing_tweens_do_not_allocate_per_step() {
    let steady = |tweens: usize| {
        let mut rig = Rig::new();
        rig.add("Part", "P", rig.root("Workspace"));
        rig.server_script(
            "Many",
            &format!(
                r#"
                local TS = game:GetService("TweenService")
                for i = 1, {tweens} do
                    local p = Instance.new("Part")
                    p.Parent = workspace
                    TS:Create(p, TweenInfo.new(100), {{Transparency = 1, Position = Vector3.new(i, 0, 0), Color = Color3.fromRGB(255, 0, 0)}}):Play()
                end
                "#
            ),
        );
        started(&mut rig);
        let now = rig.run(0.0, 5);
        no_errors(&rig);
        let before = alloc_count::get();
        rig.run(now, 20);
        alloc_count::get() - before
    };
    let few = steady(20);
    let many = steady(200);
    assert_eq!(
        few, many,
        "allocations in 20 steps: {few} with 20 tweens, {many} with 200"
    );
}

mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<u64> = const { Cell::new(0) };
    }

    struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(ptr, layout, size) }
        }
    }

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    /// Allocations made by this thread so far.
    pub fn get() -> u64 {
        COUNT.with(|c| c.get())
    }
}
