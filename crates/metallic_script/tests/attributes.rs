//! Attributes through the script API: values, names, signals, VMs, clones.

mod common;

use common::Rig;

fn run(rig: &mut Rig, src: &str) -> Vec<String> {
    rig.server_script("Main", src);
    rig.host.start().unwrap();
    rig.run(0.0, 30);
    rig.lines()
}

fn part(rig: &Rig, name: &str) {
    rig.add("Part", name, rig.root("Workspace"));
}

#[test]
fn every_type_round_trips_and_nil_removes() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         p:SetAttribute('B', true)
         p:SetAttribute('N', 2.5)
         p:SetAttribute('S', 'text')
         p:SetAttribute('V', Vector3.new(1, 2, 3))
         p:SetAttribute('C', Color3.fromRGB(128, 64, 0))
         p:SetAttribute('U', UDim2.new(0.5, 4, 1, -2))
         p:SetAttribute('F', CFrame.new(1, 2, 3))
         print(p:GetAttribute('B'), p:GetAttribute('N'), p:GetAttribute('S'))
         print(p:GetAttribute('V') == Vector3.new(1, 2, 3), p:GetAttribute('C') == Color3.fromRGB(128, 64, 0))
         print(p:GetAttribute('U') == UDim2.new(0.5, 4, 1, -2), p:GetAttribute('F').Position == Vector3.new(1, 2, 3))
         print(p:GetAttribute('Missing'))
         p:SetAttribute('N', nil)
         print(p:GetAttribute('N'))
         local all = p:GetAttributes()
         local count = 0
         for _ in all do count += 1 end
         print(count, all.S, all.N)",
    );
    assert!(
        lines.contains(&"print:true 2.5 text".to_string()),
        "{lines:?}"
    );
    assert!(lines.contains(&"print:true true".to_string()), "{lines:?}");
    assert!(lines.contains(&"print:nil".to_string()), "{lines:?}");
    assert!(lines.contains(&"print:6 text nil".to_string()), "{lines:?}");
}

#[test]
fn bad_names_and_values_raise_clear_errors() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         local function try(...) local ok, e = pcall(p.SetAttribute, p, ...) print(ok, e) end
         try('', 1)
         try('has space', 1)
         try('RBXThing', 1)
         try(string.rep('a', 101), 1)
         try('Fn', function() end)
         try('Tbl', {})
         try('Inst', workspace)
         try('Nan', 0/0)
         try('Inf', math.huge)
         try('Ok_1', 1)",
    );
    let falses: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("print:false"))
        .collect();
    assert_eq!(falses.len(), 9, "{lines:?}");
    assert!(falses[0].contains("cannot be empty"), "{}", falses[0]);
    assert!(falses[1].contains("letters, digits and underscores"));
    assert!(falses[2].contains("RBX"));
    assert!(falses[3].contains("100"));
    assert!(falses[4].contains("function"));
    assert!(falses[5].contains("table"));
    assert!(lines.contains(&"print:true nil".to_string()), "{lines:?}");
}

#[test]
fn attribute_changed_fires_next_step_with_the_name() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         p.AttributeChanged:Connect(function(name) print('changed', name, p:GetAttribute(name)) end)
         p:SetAttribute('Health', 10)
         print('set done')
         task.wait(0.1)
         p:SetAttribute('Health', 10)   -- same value: no event
         p:SetAttribute('Health', 5)
         p:SetAttribute('Health', nil)",
    );
    rig.host.start().unwrap();
    let before = rig.lines();
    assert_eq!(
        before,
        ["print:set done"],
        "handlers do not run inside the setter"
    );
    rig.run(0.0, 30);
    let lines = rig.lines();
    assert_eq!(
        lines,
        [
            "print:set done",
            "print:changed Health 10",
            // Two changes in one step: each fires, and the handler reads the current value.
            "print:changed Health nil",
            "print:changed Health nil"
        ],
        "{lines:?}"
    );
}

#[test]
fn get_attribute_changed_signal_fires_only_for_its_attribute_without_arguments() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         p:GetAttributeChangedSignal('A'):Connect(function(...) print('A', select('#', ...), p:GetAttribute('A')) end)
         p:GetAttributeChangedSignal('B'):Connect(function() print('B') end)
         p:SetAttribute('A', 1)
         p:SetAttribute('C', 3)",
    );
    assert_eq!(lines, ["print:A 0 1"], "{lines:?}");
}

#[test]
fn signals_are_per_instance_and_disconnect() {
    let mut rig = Rig::new();
    part(&rig, "P");
    part(&rig, "Q");
    let lines = run(
        &mut rig,
        "local p, q = workspace.P, workspace.Q
         local c = p:GetAttributeChangedSignal('X'):Connect(function() print('P') end)
         q:GetAttributeChangedSignal('X'):Connect(function() print('Q') end)
         q:SetAttribute('X', 1)
         task.wait(0.05)
         c:Disconnect()
         p:SetAttribute('X', 1)
         task.wait(0.05)
         print('end')",
    );
    assert_eq!(lines, ["print:Q", "print:end"], "{lines:?}");
}

#[test]
fn a_handler_that_sets_an_attribute_causes_one_more_event_next_step() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         local n = 0
         p:GetAttributeChangedSignal('Count'):Connect(function()
             n += 1
             if n < 3 then p:SetAttribute('Count', n) end
             print('event', n)
         end)
         p:SetAttribute('Count', 0)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 10);
    assert_eq!(
        rig.lines(),
        ["print:event 1", "print:event 2", "print:event 3"]
    );
}

#[test]
fn the_server_and_client_vms_both_see_a_change_made_by_either() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.local_script(
        "Client",
        "local p = workspace.P
         p:GetAttributeChangedSignal('Flag'):Connect(function() print('client sees', p:GetAttribute('Flag')) end)
         p:SetAttribute('FromClient', true)",
    );
    rig.server_script(
        "Server",
        "local p = workspace.P
         p:GetAttributeChangedSignal('FromClient'):Connect(function() print('server sees client') end)
         p:SetAttribute('Flag', 'up')",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 4);
    let lines = rig.lines();
    assert!(
        lines.contains(&"print:client sees up".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"print:server sees client".to_string()),
        "{lines:?}"
    );
}

#[test]
fn changes_made_outside_scripts_fire_signals_too() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         p:GetAttributeChangedSignal('FromEditor'):Connect(function() print('saw', p:GetAttribute('FromEditor')) end)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 2);
    let p = rig
        .world
        .borrow()
        .find_first_child(rig.root("Workspace"), "P")
        .unwrap();
    rig.world
        .borrow_mut()
        .set_attribute(p, "FromEditor", metallic_core::Value::Number(9.0))
        .unwrap();
    rig.run(0.0, 2);
    assert_eq!(rig.lines(), ["print:saw 9"]);
}

#[test]
fn destroyed_instances_raise_the_usual_error_and_stop_firing() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         p:GetAttributeChangedSignal('X'):Connect(function() print('should not fire') end)
         p:SetAttribute('X', 1)
         p:Destroy()
         print(pcall(function() return p:GetAttribute('X') end))
         print(pcall(function() p:SetAttribute('X', 2) end))",
    );
    assert!(
        !lines.contains(&"print:should not fire".to_string()),
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("print:false") && l.contains("has been destroyed"))
            .count(),
        2,
        "{lines:?}"
    );
}

#[test]
fn clones_copy_attributes_and_stay_independent() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         p:SetAttribute('Team', 'Red')
         local c = p:Clone()
         c:SetAttribute('Team', 'Blue')
         print(p:GetAttribute('Team'), c:GetAttribute('Team'))",
    );
    assert_eq!(lines, ["print:Red Blue"], "{lines:?}");
}

#[test]
fn services_and_non_parts_carry_attributes() {
    let mut rig = Rig::new();
    let lines = run(
        &mut rig,
        "workspace:SetAttribute('Mode', 'Race')
         game:GetService('ReplicatedStorage'):SetAttribute('Round', 3)
         print(workspace:GetAttribute('Mode'), game.ReplicatedStorage:GetAttribute('Round'))",
    );
    assert_eq!(lines, ["print:Race 3"], "{lines:?}");
}

#[test]
fn only_a_bounded_number_of_signal_names_per_vm() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         local failed = 0
         for i = 1, 5000 do
             local ok = pcall(p.GetAttributeChangedSignal, p, 'n' .. i)
             if not ok then failed += 1 end
         end
         print(failed > 0)",
    );
    assert!(lines.contains(&"print:true".to_string()), "{lines:?}");
}

#[test]
fn signal_names_are_validated_like_attribute_names() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local p = workspace.P
         local function try(name) local ok, e = pcall(p.GetAttributeChangedSignal, p, name) print(ok, e) end
         try('')
         try('has space')
         try('RBXThing')
         try(string.rep('a', 5000))
         try('Fine_1')",
    );
    let falses = lines
        .iter()
        .filter(|l| l.starts_with("print:false") && l.contains("invalid attribute name"))
        .count();
    assert_eq!(falses, 4, "{lines:?}");
    assert!(
        lines.iter().any(|l| l.starts_with("print:true")),
        "{lines:?}"
    );
}

#[test]
fn thousands_of_changes_in_one_step_all_arrive_in_order() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         local seen, last, ordered = 0, -1, true
         p.AttributeChanged:Connect(function(name)
             seen += 1
             local n = tonumber(string.sub(name, 2))
             if n ~= (last + 1) % 100 then ordered = false end
             last = n
         end)
         for i = 0, 5999 do p:SetAttribute('a' .. (i % 100), i) end
         task.wait(2)
         print(seen, ordered)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 300);
    let lines = rig.lines();
    assert!(lines.contains(&"print:6000 true".to_string()), "{lines:?}");
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("lost") || l.contains("dropped")),
        "{lines:?}"
    );
}

#[test]
fn the_host_trims_the_log_it_has_delivered() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let before = rig.world.borrow().attribute_cursor();
    rig.server_script(
        "Main",
        "local p = workspace.P
         for i = 1, 100 do p:SetAttribute('x', i) end",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 3);
    // Both VMs have read everything, so the log was trimmed past a cursor taken before it.
    assert_eq!(
        rig.world.borrow().attribute_changes_since(before).0,
        metallic_core::AttributeChanges::Overflow
    );
    let now = rig.world.borrow().attribute_cursor();
    assert_eq!(
        rig.world.borrow().attribute_changes_since(now).0,
        metallic_core::AttributeChanges::Changes(Vec::new())
    );
}

#[test]
fn a_burst_of_changes_is_delivered_in_order_over_several_steps() {
    let mut rig = Rig::new();
    rig.host.set_step_handler_limit(Some(500));
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         local seen, last, ordered = 0, -1, true
         p.AttributeChanged:Connect(function(name)
             seen += 1
             local n = tonumber(string.sub(name, 2))
             if n ~= (last + 1) % 100 then ordered = false end
             last = n
         end)
         for i = 0, 29999 do p:SetAttribute('a' .. (i % 100), i) end
         while seen < 30000 do task.wait() end
         print(seen, ordered)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 3000);
    let lines = rig.lines();
    assert!(lines.contains(&"print:30000 true".to_string()), "{lines:?}");
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("lost") || l.contains("dropped")),
        "{lines:?}"
    );
}

#[test]
fn a_backlog_beyond_the_log_loses_only_the_dropped_events_and_says_so() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local p = workspace.P
         local seen = 0
         p:GetAttributeChangedSignal('Late'):Connect(function() seen += 1 end)
         p.AttributeChanged:Connect(function(name) if name == 'Early' then seen += 1000 end end)
         task.wait(1)
         print('seen', seen)",
    );
    rig.host.start().unwrap();
    let t = rig.run(0.0, 2);
    let p = rig
        .world
        .borrow()
        .find_first_child(rig.root("Workspace"), "P")
        .unwrap();
    {
        // More unread changes than the log keeps, between two steps: the oldest are dropped,
        // the newest (the 'Late' one) survives.
        let mut w = rig.world.borrow_mut();
        w.set_attribute(p, "Early", metallic_core::Value::Number(1.0))
            .unwrap();
        for i in 0..300_000 {
            w.set_attribute(p, "N", metallic_core::Value::Number(f64::from(i)))
                .unwrap();
        }
        w.set_attribute(p, "Late", metallic_core::Value::Number(1.0))
            .unwrap();
    }
    rig.run(t, 90);
    let lines = rig.lines();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("warn:") && l.contains("were lost")),
        "{lines:?}"
    );
    // The surviving newest change was delivered (1), the dropped oldest one (1000) was not.
    assert!(lines.contains(&"print:seen 1".to_string()), "{lines:?}");
}
