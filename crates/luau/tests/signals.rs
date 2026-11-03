//! Signals and the `task` library from Luau, through the host's scheduler.

mod support;

use metallic_script::Phase;
use support::Game;

#[test]
fn changed_and_property_changed_signals_fire_with_their_arguments() {
    let mut game = Game::new();
    game.run(
        "Watcher",
        r##"
        local part = Instance.new("Part", workspace)
        part.Changed:Connect(function(name) print("changed", name) end)
        part:GetPropertyChangedSignal("Transparency"):Connect(function(...)
            print("transparency", select("#", ...), part.Transparency)
        end)
        part.Transparency = 0.25
        part.Name = "Lamp"
        "##,
    );
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(
        game.prints(),
        ["changed Transparency", "transparency 0 0.25", "changed Name"]
    );
}

#[test]
fn once_wait_and_disconnect() {
    let mut game = Game::new();
    game.run(
        "Events",
        r##"
        local event = Instance.new("BindableEvent")
        local count = 0
        local connection = event.Event:Connect(function(n) count += n end)
        event.Event:Once(function(n) print("once", n) end)
        task.spawn(function()
            local a, b = event.Event:Wait()
            print("waited", a, b)
        end)
        print("connected", connection.Connected, typeof(connection), typeof(event.Event))
        event:Fire(1, "x")
        task.wait()
        print("count", count)
        connection:Disconnect()
        print("connected", connection.Connected)
        event:Fire(10)
        task.wait()
        print("count", count)
        "##,
    );
    game.frame(1.0 / 60.0);
    game.frame(1.0 / 60.0);
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(
        game.prints(),
        [
            "connected true RBXScriptConnection RBXScriptSignal",
            "once 1",
            "waited 1 x",
            "count 1",
            "connected false",
            "count 1",
        ]
    );
}

#[test]
fn bindable_events_carry_engine_values_between_scripts() {
    let mut game = Game::new();
    game.run(
        "Receiver",
        r##"
        local event = Instance.new("BindableEvent")
        event.Name = "Ping"
        event.Parent = workspace
        event.Event:Connect(function(part, v, cf, color, material, list)
            print(part.Name, v, cf.Position, color.R, material, #list, list[2])
        end)
        "##,
    );
    game.run(
        "Sender",
        r##"
        local part = Instance.new("Part")
        part.Name = "Crate"
        workspace.Ping:Fire(part, Vector3.new(1, 2, 3), CFrame.new(4, 5, 6), Color3.new(0.5, 0, 0),
            Enum.Material.Wood, {1, 2, 3})
        "##,
    );
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(game.prints(), ["Crate 1, 2, 3 4, 5, 6 0.5 Enum.Material.Wood 3 2"]);
}

#[test]
fn task_wait_resumes_across_heartbeats_with_the_time_that_passed() {
    let mut game = Game::new();
    game.run(
        "Sleeper",
        r##"
        print("start", time())
        local elapsed = task.wait(0.1)
        print("woke", elapsed >= 0.1, time())
        "##,
    );
    assert_eq!(game.prints(), ["start 0"]);
    for _ in 0..5 {
        game.frame(0.025);
    }
    // Not yet: 0.1 s is reached on the fourth frame's Heartbeat.
    game.assert_no_errors();
    assert_eq!(game.prints(), ["start 0", "woke true 0.1"]);
}

#[test]
fn spawn_defer_and_delay_run_in_the_hosts_order() {
    let mut game = Game::new();
    game.run(
        "Order",
        r##"
        print("main 1")
        task.spawn(function(a)
            print("spawn 1", a)
            task.wait()
            print("spawn 2")
        end, "arg")
        task.defer(function(t) print("defer", t.n) end, { n = 7 })
        task.delay(0, function() print("delay") end)
        local cancelled = task.delay(0, function() print("never") end)
        task.cancel(cancelled)
        print("main 2")
        "##,
    );
    assert_eq!(game.prints(), ["main 1", "spawn 1 arg", "main 2"]);
    game.host.step(&mut game.dm, Phase::PreSimulation, 0.0);
    assert_eq!(game.prints(), ["main 1", "spawn 1 arg", "main 2", "defer 7"]);
    game.host.step(&mut game.dm, Phase::Heartbeat, 0.0);
    game.assert_no_errors();
    // Sleepers wake in the order they went to sleep: the spawned thread slept before the delay was asked for.
    assert_eq!(
        game.prints(),
        ["main 1", "spawn 1 arg", "main 2", "defer 7", "spawn 2", "delay"]
    );
}

#[test]
fn a_failing_listener_ends_alone() {
    let mut game = Game::new();
    game.run(
        "Listener",
        r##"
        local part = Instance.new("Part", workspace)
        part.Changed:Connect(function(name)
            if name == "Name" then error("bad name") end
            print("ok", name)
        end)
        part.Name = "X"
        part.Transparency = 1
        "##,
    );
    game.frame(1.0 / 60.0);
    assert_eq!(game.prints(), ["ok Transparency"]);
    assert_eq!(game.errors(), ["Workspace.Listener:4: bad name"]);
}
