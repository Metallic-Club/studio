//! Tags through the script API: CollectionService, instance methods, signals, VMs, scenes.

mod common;

use common::Rig;
use metallic_core::{World, load_scene, save_scene};

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
fn add_has_remove_and_list_through_the_service_and_the_instance() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local p = workspace.P
         print(CS == game.CollectionService, CS:HasTag(p, 'Lava'), p:HasTag('Lava'))
         CS:AddTag(p, 'Lava')
         p:AddTag('Hazard')
         p:AddTag('Lava')
         print(CS:HasTag(p, 'Lava'), p:HasTag('Hazard'), #p:GetTags(), #CS:GetTags(p))
         print(table.concat(p:GetTags(), ','))
         print(table.concat(CS:GetAllTags(), ','))
         p:RemoveTag('Lava')
         CS:RemoveTag(p, 'Never')
         print(p:HasTag('Lava'), table.concat(CS:GetTags(p), ','))",
    );
    assert_eq!(
        lines,
        [
            "print:true false false",
            "print:true true 2 2",
            "print:Hazard,Lava",
            "print:Hazard,Lava",
            "print:false Hazard"
        ],
        "{lines:?}"
    );
}

#[test]
fn bad_arguments_raise_clear_errors() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local p = workspace.P
         local function try(f, ...) local ok, e = pcall(f, ...) print(ok, e) end
         try(function() p:AddTag('') end)
         try(function() p:AddTag(string.rep('x', 300)) end)
         try(function() p:AddTag('bad\\ttab') end)
         try(function() CS:AddTag(5, 'x') end)
         try(function() CS:AddTag(nil, 'x') end)
         try(function() p:AddTag(5) end)  -- a number is read as text, like any string argument
         try(function() return CS.Nope end)
         p:Destroy()
         try(function() p:AddTag('x') end)
         try(function() return CS:HasTag(p, 'x') end)",
    );
    let falses: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("print:false"))
        .collect();
    // `HasTag` on the destroyed part answers with the tags it had (none).
    assert_eq!(falses.len(), 7, "{lines:?}");
    assert_eq!(
        lines.last().map(String::as_str),
        Some("print:true false"),
        "{lines:?}"
    );
    assert!(falses[0].contains("cannot be empty"), "{}", falses[0]);
    assert!(falses[1].contains("255"));
    assert!(falses[2].contains("control"));
    assert!(falses[3].contains("Instance expected"));
    assert!(falses[4].contains("Instance expected"));
    assert!(falses[5].contains("not a valid member of CollectionService"));
    assert!(falses[6].contains("has been destroyed"));
}

#[test]
fn get_tagged_uses_the_game_tree_and_returns_the_same_objects() {
    let mut rig = Rig::new();
    part(&rig, "A");
    part(&rig, "B");
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local a, b = workspace.A, workspace.B
         a:AddTag('T') b:AddTag('T')
         local list = CS:GetTagged('T')
         print(#list, list[1] == a, list[2] == b, #CS:GetTagged('Nothing'))
         b.Parent = nil
         print(#CS:GetTagged('T'), b:HasTag('T'))
         b.Parent = game.ReplicatedStorage
         print(#CS:GetTagged('T'))
         a:Destroy()
         print(#CS:GetTagged('T'), #CS:GetAllTags())
         local copy = b:Clone()
         print(copy:HasTag('T'), #CS:GetTagged('T'))
         copy.Parent = workspace
         print(#CS:GetTagged('T'))",
    );
    assert_eq!(
        lines,
        [
            "print:2 true true 0",
            "print:1 true",
            "print:2",
            "print:1 1",
            "print:true 1",
            "print:2"
        ],
        "{lines:?}"
    );
}

#[test]
fn added_and_removed_signals_fire_in_the_next_step_with_the_instance() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local CS = game:GetService('CollectionService')
         local p = workspace.P
         CS:GetInstanceAddedSignal('Lava'):Connect(function(i) print('added', i.Name, i == p) end)
         CS:GetInstanceRemovedSignal('Lava'):Connect(function(i) print('removed', i.Name, i:HasTag('Lava')) end)
         p:AddTag('Lava')
         print('tagged')
         task.wait(0.1)
         p:AddTag('Lava')      -- already tagged: nothing
         p:AddTag('Other')     -- another tag: nothing
         p:RemoveTag('Lava')
         p:RemoveTag('Lava')   -- not tagged: nothing",
    );
    rig.host.start().unwrap();
    assert_eq!(rig.lines(), ["print:tagged"], "not inside AddTag");
    rig.run(0.0, 30);
    assert_eq!(
        rig.lines(),
        [
            "print:tagged",
            "print:added P true",
            "print:removed P false"
        ]
    );
}

#[test]
fn entering_leaving_and_destroying_fire_the_signals() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.server_script(
        "Main",
        "local CS = game:GetService('CollectionService')
         local p = workspace.P
         p:AddTag('T')
         task.wait(0.1)
         CS:GetInstanceAddedSignal('T'):Connect(function(i) print('in', i == p) end)
         CS:GetInstanceRemovedSignal('T'):Connect(function(i) print('out', i == p) end)
         p.Parent = nil
         task.wait(0.1)
         p.Parent = workspace
         task.wait(0.1)
         p.Parent = game.ReplicatedStorage      -- stays in the game: nothing
         task.wait(0.1)
         p:Destroy()
         task.wait(0.1)
         print('end')",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 60);
    assert_eq!(
        rig.lines(),
        [
            "print:out true",
            "print:in true",
            "print:out true",
            "print:end"
        ]
    );
}

#[test]
fn signals_are_per_tag_and_disconnect() {
    let mut rig = Rig::new();
    part(&rig, "P");
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local p = workspace.P
         local c = CS:GetInstanceAddedSignal('A'):Connect(function() print('A') end)
         CS:GetInstanceAddedSignal('B'):Connect(function() print('B') end)
         p:AddTag('B')
         task.wait(0.05)
         c:Disconnect()
         p:AddTag('A')
         task.wait(0.05)
         print('end')",
    );
    assert_eq!(lines, ["print:B", "print:end"], "{lines:?}");
}

#[test]
fn the_server_and_client_vms_both_see_changes_and_so_does_the_editor() {
    let mut rig = Rig::new();
    part(&rig, "P");
    rig.local_script(
        "Client",
        "local CS = game:GetService('CollectionService')
         CS:GetInstanceAddedSignal('Flag'):Connect(function(i) print('client', i.Name) end)",
    );
    rig.server_script(
        "Server",
        "local CS = game:GetService('CollectionService')
         CS:GetInstanceAddedSignal('Flag'):Connect(function(i) print('server', i.Name) end)
         CS:GetInstanceAddedSignal('Editor'):Connect(function(i) print('editor tag', i.Name) end)
         workspace.P:AddTag('Flag')",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 4);
    let p = rig
        .world
        .borrow()
        .find_first_child(rig.root("Workspace"), "P")
        .unwrap();
    rig.world.borrow_mut().add_tag(p, "Editor").unwrap();
    rig.run(0.0, 4);
    let lines = rig.lines();
    for want in ["print:client P", "print:server P", "print:editor tag P"] {
        assert!(lines.contains(&want.to_string()), "{want}: {lines:?}");
    }
}

#[test]
fn only_a_bounded_number_of_signal_tags_per_vm() {
    let mut rig = Rig::new();
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local failed = 0
         for i = 1, 5000 do
             if not pcall(CS.GetInstanceAddedSignal, CS, 'n' .. i) then failed += 1 end
         end
         print(failed > 0)",
    );
    assert!(lines.contains(&"print:true".to_string()), "{lines:?}");
}

#[test]
fn tags_loaded_from_a_scene_are_visible_to_scripts() {
    let dir = std::env::temp_dir().join(format!("tags-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    for name in ["One", "Two", "Three"] {
        let p = w.create("Part", Some(ws)).unwrap();
        w.set(p, "Name", metallic_core::Value::String(name.into()))
            .unwrap();
        if name != "Three" {
            w.add_tag(p, "Checkpoint").unwrap();
        }
    }
    save_scene(&w, &path).unwrap();
    let loaded = load_scene(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();

    let mut rig = Rig::new();
    *rig.world.borrow_mut() = loaded;
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local names = {}
         for _, i in CS:GetTagged('Checkpoint') do table.insert(names, i.Name) end
         table.sort(names)
         print(table.concat(names, ','), workspace.Three:HasTag('Checkpoint'))",
    );
    assert_eq!(lines, ["print:One,Two false"], "{lines:?}");
}

#[test]
fn collection_service_is_a_service_scripts_cannot_create() {
    let mut rig = Rig::new();
    let lines = run(
        &mut rig,
        "print(pcall(Instance.new, 'CollectionService'))
         local CS = game:GetService('CollectionService')
         print(tostring(CS), CS.Name, CS.ClassName)",
    );
    assert!(lines[0].starts_with("print:false"), "{lines:?}");
    assert_eq!(
        lines[1],
        "print:CollectionService CollectionService CollectionService"
    );
}

#[test]
fn signal_tags_are_validated_like_tags() {
    let mut rig = Rig::new();
    let lines = run(
        &mut rig,
        "local CS = game:GetService('CollectionService')
         local function try(tag)
             print(pcall(CS.GetInstanceAddedSignal, CS, tag))
             print(pcall(CS.GetInstanceRemovedSignal, CS, tag))
         end
         try('')
         try(string.rep('x', 5000))
         try('bad\ttab')
         try('Fine')",
    );
    let bad = lines
        .iter()
        .filter(|l| l.starts_with("print:false") && l.contains("invalid tag"))
        .count();
    assert_eq!(bad, 6, "{lines:?}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("print:true")).count(),
        2,
        "{lines:?}"
    );
}

#[test]
fn destroying_thousands_of_tagged_instances_fires_every_removal() {
    let mut rig = Rig::new();
    rig.server_script(
        "Main",
        "local CS = game:GetService('CollectionService')
         local model = Instance.new('Model', workspace)
         local parts = {}
         for i = 1, 6000 do
             local p = Instance.new('Part', model)
             p:AddTag('Cleanup')
             parts[i] = p
         end
         task.wait(0.5)
         local removed = 0
         CS:GetInstanceRemovedSignal('Cleanup'):Connect(function() removed += 1 end)
         model:Destroy()
         task.wait(3)
         print('removed', removed)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 400);
    let lines = rig.lines();
    assert!(
        lines.contains(&"print:removed 6000".to_string()),
        "{lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("lost") || l.contains("dropped")),
        "{lines:?}"
    );
}

#[test]
fn twenty_thousand_tagged_destroys_reach_the_manager_without_losing_one() {
    let mut rig = Rig::new();
    rig.server_script(
        "Manager",
        "local CS = game:GetService('CollectionService')
         local live, adds, removes = {}, 0, 0
         CS:GetInstanceAddedSignal('Managed'):Connect(function(i) live[i] = true adds += 1 end)
         CS:GetInstanceRemovedSignal('Managed'):Connect(function(i) live[i] = nil removes += 1 end)
         local model = Instance.new('Model', workspace)
         for batch = 1, 20 do
             for i = 1, 1000 do
                 Instance.new('Part', model):AddTag('Managed')
             end
             task.wait()
         end
         task.wait(1)
         local function size() local n = 0 for _ in live do n += 1 end return n end
         print('before', size(), adds)
         model:Destroy()
         while size() > 0 do task.wait() end
         print('after', size(), adds, removes)",
    );
    rig.host.start().unwrap();
    rig.run(0.0, 3000);
    let lines = rig.lines();
    assert!(
        lines.contains(&"print:before 20000 20000".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"print:after 0 20000 20000".to_string()),
        "{lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("lost") || l.contains("dropped")),
        "{lines:?}"
    );
}

#[test]
fn deferred_removal_handlers_still_read_twenty_thousand_destroyed_instances() {
    let mut rig = Rig::new();
    // Made and destroyed by the engine: no Luau object of a part exists before its removal
    // handler runs, so each handler reads the host's record of the destroyed part.
    let model = rig.add("Model", "Crowd", rig.root("Workspace"));
    {
        let mut w = rig.world.borrow_mut();
        for i in 0..20_000 {
            let id = w.create("Part", Some(model)).unwrap();
            w.set(id, "Name", metallic_core::Value::String(format!("P{i}")))
                .unwrap();
            w.set_attribute(id, "N", metallic_core::Value::Number(f64::from(i)))
                .unwrap();
            w.add_tag(id, "Managed").unwrap();
        }
    }
    rig.server_script(
        "Manager",
        "local CS = game:GetService('CollectionService')
         local steps, removed, readable, first, last = 0, 0, 0, nil, nil
         game:GetService('RunService').Heartbeat:Connect(function() steps += 1 end)
         CS:GetInstanceRemovedSignal('Managed'):Connect(function(i)
             removed += 1
             first = first or steps
             last = steps
             local n = i:GetAttribute('N')
             if i.Name == 'P' .. n and i.ClassName == 'Part' and i.Parent == nil
                 and i:HasTag('Managed') and #i:GetTags() == 1 and i:GetAttributes().N == n then
                 readable += 1
             end
             if removed == 20000 then
                 print('done', removed, readable, last - first > 2)
             end
         end)",
    );
    rig.host.start().unwrap();
    let now = rig.run(0.0, 120);
    rig.world.borrow_mut().destroy(model);
    rig.run(now, 3000);
    let lines = rig.lines();
    assert!(
        lines.contains(&"print:done 20000 20000 true".to_string()),
        "{lines:?}"
    );
    assert_eq!(rig.host.destroyed_records(), 0, "kept after delivery");
}
