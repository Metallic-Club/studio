//! The sandbox, budgets and errors: no escapes, runaway loops stop alone, compile errors and memory limits.

mod support;

use std::time::Duration;

use metallic_core::Value;
use metallic_script::{HostError, Resume, ScriptFailure};
use support::Game;

#[test]
fn no_file_os_or_environment_escapes() {
    let mut game = Game::new();
    game.run(
        "Probe",
        r##"
        print(io, os.execute, os.getenv, os.exit, os.remove, loadstring, getfenv, setfenv, dofile,
            loadfile, package, collectgarbage, debug.getinfo, debug.sethook, string.dump)
        -- `require` is ours: it takes a ModuleScript, never a path.
        print(typeof(os.clock()), typeof(debug.traceback), typeof(tick()), typeof(time()), typeof(require))
        print(pcall(require, "Scripts/Door.luau"))
        print(pcall(function() math.pi = 3 end))
        print(pcall(function() string.foo = 1 end))
        print(pcall(function() getmetatable(Vector3.zero).__index = nil end))
        _G.shared_value = 42
        shared.answer = 1
        local ok = pcall(function() Instance.foo = 1 end)
        print(ok, _G.shared_value, shared.answer)
        "##,
    );
    game.run("Second", "print(_G.shared_value, x_global)");
    game.assert_no_errors();
    let mut prints = game.prints();
    assert_eq!(prints[0], "nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil");
    assert_eq!(prints[1], "number function number number function");
    assert!(
        prints
            .remove(2)
            .contains("Attempted to call require with invalid argument(s)."),
        "require refuses a path"
    );
    assert!(
        prints[2].starts_with("false") && prints[2].contains("readonly"),
        "{}",
        prints[2]
    );
    assert!(prints[3].starts_with("false"), "{}", prints[3]);
    assert!(prints[4].starts_with("false"), "{}", prints[4]);
    assert_eq!(prints[5], "false 42 1");
    assert_eq!(prints[6], "42 nil");
}

#[test]
fn globals_of_one_script_do_not_leak_into_another() {
    let mut game = Game::new();
    game.run("A", "secret = 1\nprint(script.Name, secret)");
    game.run("B", "print(script.Name, secret)");
    game.assert_no_errors();
    assert_eq!(game.prints(), ["A 1", "B nil"]);
}

#[test]
fn a_runaway_loop_is_interrupted_and_other_scripts_keep_running() {
    let mut game = Game::new();
    game.host.set_resume_budget(Duration::from_millis(20));
    game.run("Ticker", "while true do\n    task.wait()\n    print('tick')\nend\n");
    let run = game.run("Spinner", "local n = 0\nwhile true do\n    n += 1\nend\n");
    assert!(
        matches!(
            run.resume,
            Resume::Errored(ScriptFailure::Timeout { line: Some(2..=3) })
        ),
        "{:?}",
        run.resume
    );
    let looped = game.run(
        "Protected",
        "local ok = pcall(function() while true do end end)\nprint('caught', ok)\nwhile true do end\n",
    );
    assert!(matches!(looped.resume, Resume::Errored(ScriptFailure::Timeout { .. })));
    game.frame(1.0 / 60.0);
    game.frame(1.0 / 60.0);
    assert_eq!(game.prints().iter().filter(|p| *p == "tick").count(), 2);
    let errors = game.errors();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].starts_with("Workspace.Spinner:"), "{errors:?}");
    assert!(errors[0].contains("time budget"), "{errors:?}");
}

#[test]
fn compile_errors_name_the_file_and_line() {
    let mut game = Game::new();
    let script = game.script("Broken");
    let error = game
        .host
        .run_script(&mut game.dm, script, "Scripts/Broken.luau", "local x = 1\nlocal = 2\n")
        .expect_err("does not compile");
    let HostError::Compile(error) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.chunk, "Scripts/Broken.luau");
    assert_eq!(error.line, 2);
    assert_eq!(game.errors().len(), 1);
}

#[test]
fn the_memory_limit_fails_the_allocating_thread() {
    let mut game = Game::new();
    game.run("Before", "print('fine')");
    let used = game.host.memory_used();
    assert!(used > 0);
    game.host.set_memory_limit(Some(used + 512 * 1024));
    let run = game.run(
        "Hog",
        "local t = {}\nfor i = 1, 1e7 do\n    t[i] = tostring(i) .. 'xxxxxxxxxxxxxxxx'\nend\n",
    );
    assert!(
        matches!(run.resume, Resume::Errored(ScriptFailure::OutOfMemory)),
        "{:?}",
        run.resume
    );
    game.host.set_memory_limit(None);
    game.run("After", "print('still fine')");
    assert_eq!(game.prints(), ["fine", "still fine"]);
}

#[test]
fn enums_come_from_the_registry() {
    let mut game = Game::new();
    game.run(
        "Enums",
        r##"
        local part = Instance.new("Part", workspace)
        part.Name = "P"
        print(part.Material, part.Material == Enum.Material.Plastic, typeof(Enum.Material.Wood))
        part.Material = Enum.Material.Neon
        print(part.Material.Name, part.Material.Value, part.Material.EnumType, part.Material == Enum.Material.Neon)
        part.Material = "Wood"
        print(part.Material, #Enum.Material:GetEnumItems(), Enum.PartShape.Ball)
        print(pcall(function() part.Material = Enum.PartShape.Ball end))
        print(pcall(function() return Enum.Material.Lava end))
        "##,
    );
    let prints = game.prints();
    game.assert_no_errors();
    assert_eq!(prints[0], "Enum.Material.Plastic true EnumItem");
    assert_eq!(prints[1], "Neon 5 Material true");
    assert_eq!(prints[2], "Enum.Material.Wood 8 Enum.PartShape.Ball");
    assert!(
        prints[3].starts_with("false") && prints[3].contains("Enum.Material"),
        "{}",
        prints[3]
    );
    assert!(
        prints[4].contains("Lava is not a valid member of \"Enum.Material\""),
        "{}",
        prints[4]
    );
    let workspace = game.service("Workspace");
    let part = game.dm.find_first_child(workspace, "P", false).unwrap().unwrap();
    assert_eq!(
        game.dm.get_by_name(part, "Material"),
        Ok(Value::Enum(game.dm.registry().enum_item("Material", "Wood").unwrap()))
    );
}

#[test]
fn methods_as_values_children_and_instance_identity() {
    let mut game = Game::new();
    game.run(
        "Values",
        r##"
        local folder = Instance.new("Folder", workspace)
        folder.Name = "Box"
        local part = Instance.new("Part", folder)
        part.Name = "Lid"
        local find = folder.FindFirstChild
        print(find(folder, "Lid") == part, workspace.Box.Lid == part, #folder:GetChildren())
        local seen = {}
        seen[part] = true
        print(seen[folder:FindFirstChild("Lid")], rawequal(part, workspace.Box.Lid))
        print(pcall(find, 1, "Lid"))
        local destroy = part.Destroy
        destroy(part)
        print(folder:FindFirstChild("Lid"), part.Parent)
        "##,
    );
    let prints = game.prints();
    assert_eq!(prints[0], "true true 1");
    assert_eq!(prints[1], "true true");
    assert!(
        prints[2].starts_with("false") && prints[2].contains("Expected ':'"),
        "{}",
        prints[2]
    );
    assert_eq!(prints.len(), 3, "{prints:?}");
    // A destroyed instance's members cannot be read: the data model refuses its id.
    assert_eq!(game.errors().len(), 1, "{:?}", game.errors());
}
