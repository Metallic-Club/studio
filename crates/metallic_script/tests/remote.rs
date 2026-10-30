//! RemoteEvent behaviour: delivery on the next step, argument copying and the refusals.

mod common;

use common::Rig;

fn rig_with_remote() -> Rig {
    let rig = Rig::new();
    rig.add("RemoteEvent", "Ping", rig.root("ReplicatedStorage"));
    rig
}

#[test]
fn remote_fire_server_delivers_next_step_with_player_first() {
    let mut rig = rig_with_remote();
    rig.server_script(
        "Server",
        r#"
        local ping = game:GetService("ReplicatedStorage").Ping
        ping.OnServerEvent:Connect(function(player, a, b)
            print("server got", player.Name, a, b)
        end)
        "#,
    );
    rig.local_script(
        "Client",
        r#"
        local ping = game:GetService("ReplicatedStorage"):WaitForChild("Ping")
        ping:FireServer(7, "x")
        print("fired")
        "#,
    );
    rig.host.start().unwrap();
    assert_eq!(
        rig.lines(),
        ["print:fired"],
        "not delivered in the same call"
    );
    rig.run(0.0, 1);
    assert_eq!(rig.lines(), ["print:fired", "print:server got Player1 7 x"]);
    rig.run(1.0, 2);
    assert_eq!(rig.lines().len(), 2, "delivered once");
}

#[test]
fn remote_to_client_and_back() {
    let mut rig = rig_with_remote();
    rig.server_script(
        "Server",
        r#"
        local ping = game:GetService("ReplicatedStorage").Ping
        local Players = game:GetService("Players")
        Players.PlayerAdded:Connect(function(p)
            ping:FireClient(p, "hello", workspace)
            ping:FireAllClients("everyone")
        end)
        "#,
    );
    rig.local_script(
        "Client",
        r#"
        local ping = game:GetService("ReplicatedStorage"):WaitForChild("Ping")
        ping.OnClientEvent:Connect(function(a, b) print("client got", a, b and b.Name) end)
        "#,
    );
    rig.host.start().unwrap();
    let t = rig.run(0.0, 1);
    assert!(
        rig.lines().is_empty(),
        "fired during step 1 (PlayerAdded), delivered in step 2: {:?}",
        rig.lines()
    );
    rig.run(t, 1);
    assert_eq!(
        rig.lines(),
        [
            "print:client got hello Workspace",
            "print:client got everyone nil"
        ]
    );
}

#[test]
fn remote_values_deep_copied() {
    let mut rig = rig_with_remote();
    rig.server_script(
        "Server",
        r#"
        local ping = game:GetService("ReplicatedStorage").Ping
        ping.OnServerEvent:Connect(function(player, t, v, c, u, inst, flag)
            print(t.list[1], t.nested.deep.n, #t.list, v.X, c.R, u.X.Scale, inst.Name, flag, t.skip)
            t.list[1] = "changed on server"
        end)
        ping.OnServerEvent:Connect(function(player, t) print("second handler sees", t.list[1]) end)
        "#,
    );
    rig.local_script(
        "Client",
        r#"
        local ping = game:GetService("ReplicatedStorage"):WaitForChild("Ping")
        local shared = { n = 1 }
        local t = { list = { "orig", 2 }, nested = { deep = shared }, skip = nil }
        ping:FireServer(t, Vector3.new(1, 2, 3), Color3.fromRGB(255, 0, 0), UDim2.new(0.5, 0, 0, 0), workspace, true)
        t.list[1] = "mutated after fire"
        task.delay(0.1, function() print("client still has", t.list[1]) end)
        "#,
    );
    rig.host.start().unwrap();
    let t = rig.run(0.0, 1);
    assert_eq!(
        rig.lines(),
        [
            "print:orig 1 2 1 1 0.5 Workspace true nil",
            "print:second handler sees orig",
        ],
        "the copy was taken when fired; handlers get separate copies"
    );
    rig.run(t, 10);
    assert_eq!(
        rig.lines().last().unwrap(),
        "print:client still has mutated after fire"
    );
}

#[test]
fn remote_rejects_function_and_cycle() {
    let mut rig = rig_with_remote();
    rig.local_script(
        "Client",
        r#"
        local ping = game:GetService("ReplicatedStorage"):WaitForChild("Ping")
        print(pcall(function() ping:FireServer(function() end) end))
        print(pcall(function() ping:FireServer({ f = print }) end))
        local cyc = {}
        cyc.self = cyc
        print(pcall(function() ping:FireServer(cyc) end))
        local deep = {}
        local cur = deep
        for i = 1, 20 do cur.next = {} cur = cur.next end
        print(pcall(function() ping:FireServer(deep) end))
        local ok = {}
        cur = ok
        for i = 1, 15 do cur.next = {} cur = cur.next end
        print(pcall(function() ping:FireServer(ok) end))
        print(pcall(function() ping:FireServer(coroutine.create(print)) end))
        local shared = {}
        print(pcall(function() ping:FireServer({ shared, shared }) end))
        "#,
    );
    rig.host.start().unwrap();
    let lines = rig.lines();
    assert!(
        lines[0].starts_with("print:false") && lines[0].contains("cannot send function"),
        "{lines:?}"
    );
    assert!(lines[1].contains("cannot send function"), "{lines:?}");
    assert!(lines[2].contains("cannot send cyclic table"), "{lines:?}");
    assert!(lines[3].contains("deeper than 16"), "{lines:?}");
    assert_eq!(lines[4], "print:true");
    assert!(lines[5].contains("cannot send thread"), "{lines:?}");
    assert_eq!(
        lines[6], "print:true",
        "a shared (not cyclic) small table is fine; big expansions are limited in limits.rs"
    );
}

#[test]
fn fire_client_from_client_errors() {
    let mut rig = rig_with_remote();
    rig.local_script(
        "Client",
        r#"
        local ping = game:GetService("ReplicatedStorage"):WaitForChild("Ping")
        local me = game.Players.LocalPlayer
        print(pcall(function() ping:FireClient(me, 1) end))
        print(pcall(function() ping:FireAllClients(1) end))
        print(pcall(function() return ping.OnServerEvent end))
        "#,
    );
    rig.server_script(
        "Server",
        r#"
        local ping = game:GetService("ReplicatedStorage").Ping
        print(pcall(function() ping:FireServer(1) end))
        print(pcall(function() ping:FireClient(workspace, 1) end))
        print(pcall(function() return ping.OnClientEvent end))
        print(pcall(function() return workspace.FireServer end))
        "#,
    );
    rig.host.start().unwrap();
    let joined = rig.lines().join("\n");
    assert!(
        joined.contains("FireClient can only be called from the server"),
        "{joined}"
    );
    assert!(
        joined.contains("FireAllClients can only be called from the server"),
        "{joined}"
    );
    assert!(
        joined.contains("OnServerEvent can only be used from the server"),
        "{joined}"
    );
    assert!(
        joined.contains("FireServer can only be called from the client"),
        "{joined}"
    );
    assert!(joined.contains("FireClient expects a Player"), "{joined}");
    assert!(
        joined.contains("OnClientEvent can only be used from the client"),
        "{joined}"
    );
    assert!(
        joined.contains("FireServer is not a valid member of Workspace"),
        "{joined}"
    );
}
