//! A small game from Luau: script classes started by the host, `require`, `BindableFunction`, lifecycle and
//! cleanup.

mod support;

use support::Game;

#[test]
fn scripts_share_a_module_table_through_require() {
    let mut game = Game::new();
    let storage = game.service("ReplicatedStorage");
    let workspace = game.service("Workspace");
    game.add(
        "ModuleScript",
        Some(storage),
        "Counter",
        Some(
            r##"
            print("loading", script.Name)
            local Counter = { count = 0 }
            function Counter.add(n)
                Counter.count += n
                return Counter.count
            end
            return Counter
            "##,
        ),
    );
    let user = r##"
        local Counter = require(game.ReplicatedStorage.Counter)
        print(script.Name, Counter.add(1), Counter == require(game.ReplicatedStorage.Counter))
    "##;
    game.add("Script", Some(workspace), "First", Some(user));
    game.add("Script", Some(workspace), "Second", Some(user));
    game.start();
    game.assert_no_errors();
    assert_eq!(game.prints(), ["loading Counter", "First 1 true", "Second 2 true"]);
}

#[test]
fn require_errors_name_the_module_and_cycles() {
    let mut game = Game::new();
    let storage = game.service("ReplicatedStorage");
    let workspace = game.service("Workspace");
    game.add(
        "ModuleScript",
        Some(storage),
        "Broken",
        Some("local x = 1\nerror(\"no config\")\nreturn x\n"),
    );
    game.add(
        "ModuleScript",
        Some(storage),
        "Ping",
        Some("return require(script.Parent.Pong)\n"),
    );
    game.add(
        "ModuleScript",
        Some(storage),
        "Pong",
        Some("return require(script.Parent.Ping)\n"),
    );
    game.add(
        "ModuleScript",
        Some(storage),
        "Slow",
        Some("task.wait(0.2)\nreturn { ready = true }\n"),
    );
    game.add(
        "Script",
        Some(workspace),
        "Main",
        Some(
            r##"
            print(pcall(require, game.ReplicatedStorage.Broken))
            print(pcall(require, game.ReplicatedStorage.Ping))
            print(pcall(require, workspace))
            print("slow", require(game.ReplicatedStorage.Slow).ready)
            "##,
        ),
    );
    game.start();
    for _ in 0..30 {
        game.frame(1.0 / 60.0);
    }
    let prints = game.prints();
    assert_eq!(prints.len(), 4, "{prints:#?}");
    assert!(
        prints[0].starts_with("false")
            && prints[0].contains("Requested module experienced an error while loading")
            && prints[0].contains("ReplicatedStorage.Broken:2:")
            && prints[0].contains("no config"),
        "{}",
        prints[0]
    );
    assert!(
        prints[1].contains(
            "Requested module was required recursively: ReplicatedStorage.Pong requires ReplicatedStorage.Ping \
             requires ReplicatedStorage.Pong"
        ),
        "{}",
        prints[1]
    );
    assert!(
        prints[2].contains("Attempted to call require with invalid argument(s)."),
        "{}",
        prints[2]
    );
    assert_eq!(prints[3], "slow true");
    // The modules' own errors are reported once each, as theirs.
    let errors = game.errors();
    assert!(
        errors.iter().any(|e| e.starts_with("ReplicatedStorage.Broken:2:")),
        "{errors:#?}"
    );
}

#[test]
fn bindable_function_round_trip() {
    let mut game = Game::new();
    let workspace = game.service("Workspace");
    game.add("BindableFunction", Some(workspace), "Add", None);
    // The caller starts first: its Invoke waits until the server assigns OnInvoke.
    game.add(
        "Script",
        Some(workspace),
        "Caller",
        Some(
            r##"
            local add = workspace.Add
            local sum, label = add:Invoke(2, 3)
            print("sum", sum, label)
            print(pcall(function() return add.OnInvoke end))
            print(pcall(add.Invoke, add, "boom"))
            "##,
        ),
    );
    game.add(
        "Script",
        Some(workspace),
        "Server",
        Some(
            r##"
            workspace.Add.OnInvoke = function(a, b)
                if a == "boom" then
                    error("cannot add boom")
                end
                task.wait(0.1)
                return a + b, "done"
            end
            "##,
        ),
    );
    game.start();
    for _ in 0..30 {
        game.frame(1.0 / 60.0);
    }
    game.assert_no_errors();
    let prints = game.prints();
    assert_eq!(prints[0], "sum 5 done");
    assert!(
        prints[1].starts_with("false") && prints[1].contains("OnInvoke is a callback member of BindableFunction"),
        "{}",
        prints[1]
    );
    assert!(
        prints[2].starts_with("false") && prints[2].contains("Workspace.Server:4: cannot add boom"),
        "{}",
        prints[2]
    );
    assert_eq!(game.host.callback_count(), 1);
}

#[test]
fn a_script_disables_another_and_waits_survive_destruction() {
    let mut game = Game::new();
    let workspace = game.service("Workspace");
    let looper = game.add(
        "Script",
        Some(workspace),
        "Looper",
        Some(
            r##"
            while true do
                print("tick")
                task.wait(0.5)
            end
            "##,
        ),
    );
    game.add(
        "Script",
        Some(workspace),
        "Stopper",
        Some(
            r##"
            local event = Instance.new("BindableEvent")
            task.spawn(function()
                print(pcall(function() return event.Event:Wait() end))
            end)
            task.wait(0.1)
            workspace.Looper.Enabled = false
            event:Destroy()
            "##,
        ),
    );
    game.start();
    for _ in 0..120 {
        game.frame(1.0 / 60.0);
    }
    game.assert_no_errors();
    assert!(!game.host.is_running(looper));
    let prints = game.prints();
    assert_eq!(prints.iter().filter(|p| *p == "tick").count(), 1, "{prints:#?}");
    assert!(
        prints.iter().any(|p| p.starts_with("false")
            && p.contains("the instance whose signal this thread was waiting on was destroyed")),
        "{prints:#?}"
    );
    assert_eq!(game.host.waiter_count(), 0);
}

#[test]
fn destroying_a_thousand_connected_parts_leaks_nothing() {
    let mut game = Game::new();
    let workspace = game.service("Workspace");
    game.add(
        "Script",
        Some(workspace),
        "Spawner",
        Some(
            r##"
            local folder = Instance.new("Folder")
            folder.Name = "Parts"
            folder.Parent = workspace
            local touched = 0
            for i = 1, 1000 do
                local part = Instance.new("Part")
                part.Changed:Connect(function() touched += 1 end)
                part:GetPropertyChangedSignal("Transparency"):Connect(function() end)
                part.Parent = folder
            end
            "##,
        ),
    );
    game.start();
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(game.host.listener_count(), 2000);
    let baseline_connections = game.dm.connection_count() - 2000;

    let folder = game
        .dm
        .find_first_child(workspace, "Parts", false)
        .expect("lookup")
        .expect("the folder");
    game.dm.destroy(folder).expect("destroy");
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(game.host.listener_count(), 0);
    assert_eq!(game.dm.connection_count(), baseline_connections);
}
