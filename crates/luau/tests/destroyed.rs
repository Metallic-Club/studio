//! Destroyed instances from Luau: their members still read, writes fail with the script's position, and their
//! snapshots go once no script refers to them.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use metallic_core::{ListenerKey, ScriptValue, Value};
use metallic_luau::LuauLanguage;
use metallic_script::{
    ChunkId, CompileError, HostContext, MemorySources, ModuleKey, Resume, ScriptFailure, ScriptHost, ScriptLanguage,
    ThreadId,
};
use support::Game;

#[test]
fn a_destroyed_part_still_reads() {
    let mut game = Game::new();
    game.run(
        "Reader",
        r#"
        local part = Instance.new("Part", workspace)
        part.Name = "Door"
        part.Transparency = 0.5
        part.Position = Vector3.new(1, 2, 3)
        part:SetAttribute("Team", "Red")
        Instance.new("Folder", part).Name = "Hinge"
        local same = part
        part:Destroy()
        print(part.Name, part.Transparency, part.Parent)
        print(part.ClassName, part.Position, part:IsA("BasePart"), part:GetFullName(), tostring(part))
        print(#part:GetChildren(), part:FindFirstChild("Hinge"), part:GetAttribute("Team"), same == part)
        part:Destroy()
        part:ClearAllChildren()
        local fresh = Instance.new("Part")
        print(fresh == part, fresh.Name, part.Name)
        "#,
    );
    game.assert_no_errors();
    assert_eq!(
        game.prints(),
        [
            "Door 0.5 nil",
            "Part 1, 2, 3 true Door Door",
            "0 nil Red true",
            "false Part Door",
        ]
    );
}

#[test]
fn handlers_read_destroyed_instances() {
    let mut game = Game::new();
    let workspace = game.service("Workspace");
    let folder = game.add("Folder", Some(workspace), "Box", None);
    let lid = game.add("Part", Some(folder), "Lid", None);
    game.dm
        .set_by_name(lid, "Transparency", Value::Number(0.75))
        .expect("set");
    game.run(
        "Watcher",
        r#"
        local part = Instance.new("Part", workspace)
        part.Name = "Lamp"
        part.Transparency = 0.25
        part.Destroying:Connect(function()
            print("destroying", part.Name, part.Transparency, part.Parent)
        end)
        workspace.Box.ChildRemoved:Connect(function(child)
            print("removed", child.Name, child.Transparency, child.Parent)
        end)
        part:Destroy()
        "#,
    );
    // The script never touched the lid: the data model keeps it readable while its events are delivered.
    game.dm.destroy(lid).expect("destroy");
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(game.prints(), ["destroying Lamp 0.25 nil", "removed Lid 0.75 nil"]);
}

#[test]
fn writes_to_a_destroyed_instance_fail_with_the_line() {
    let mut game = Game::new();
    game.run(
        "Writer",
        "local p = Instance.new('Part')\np:Destroy()\np.Transparency = 1\n",
    );
    game.run(
        "Mover",
        "local p = Instance.new('Part')\np:Destroy()\n\np.Parent = workspace\n",
    );
    game.run("Renamer", "local p = Instance.new('Part')\np:Destroy()\np.Name = 'x'\n");
    game.run("Cloner", "local p = Instance.new('Part')\np:Destroy()\np:Clone()\n");
    game.run(
        "Caught",
        "local p = Instance.new('Part')\np:Destroy()\nprint(pcall(function() p.Anchored = true end))\n",
    );
    assert_eq!(
        game.errors(),
        [
            "Workspace.Writer:3: cannot set Transparency: the instance was destroyed",
            "Workspace.Mover:4: cannot set Parent: the instance was destroyed",
            "Workspace.Renamer:3: cannot set Name: the instance was destroyed",
            "Workspace.Cloner:3: the instance was destroyed",
        ]
    );
    // Inside the VM the position names the chunk (`Scripts/Caught.luau#<uid>`); the host's output names the script.
    let prints = game.prints();
    assert_eq!(prints.len(), 1, "{prints:?}");
    assert!(
        prints[0].starts_with("false Scripts/Caught.luau")
            && prints[0].ends_with(":3: cannot set Anchored: the instance was destroyed"),
        "{prints:?}"
    );
}

/// A `ScriptLanguage` that shares its Luau backend, so the test can run the backend's garbage collector between
/// calls while the host owns the language.
struct SharedLuau(Rc<RefCell<LuauLanguage>>);

impl ScriptLanguage for SharedLuau {
    fn name(&self) -> &str {
        "luau"
    }

    fn extensions(&self) -> &[&str] {
        &["luau"]
    }

    fn compile(&mut self, source: &str, chunk_name: &str) -> Result<ChunkId, CompileError> {
        self.0.borrow_mut().compile(source, chunk_name)
    }

    fn spawn(&mut self, chunk: ChunkId, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        self.0.borrow_mut().spawn(chunk, ctx)
    }

    fn spawn_listener(&mut self, listener: ListenerKey, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        self.0.borrow_mut().spawn_listener(listener, ctx)
    }

    fn spawn_module(
        &mut self,
        chunk: ChunkId,
        key: ModuleKey,
        ctx: &mut HostContext<'_>,
    ) -> Result<ThreadId, ScriptFailure> {
        self.0.borrow_mut().spawn_module(chunk, key, ctx)
    }

    fn release_module(&mut self, key: ModuleKey) {
        self.0.borrow_mut().release_module(key);
    }

    fn resume(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        self.0.borrow_mut().resume(thread, args, ctx)
    }

    fn resume_error(&mut self, thread: ThreadId, message: &str, ctx: &mut HostContext<'_>) -> Resume {
        self.0.borrow_mut().resume_error(thread, message, ctx)
    }

    fn release_thread(&mut self, thread: ThreadId) {
        self.0.borrow_mut().release_thread(thread);
    }

    fn release_listener(&mut self, listener: ListenerKey) {
        self.0.borrow_mut().release_listener(listener);
    }

    fn memory_used(&self) -> usize {
        self.0.borrow().memory_used()
    }

    fn set_memory_limit(&mut self, bytes: Option<usize>) {
        self.0.borrow_mut().set_memory_limit(bytes);
    }
}

#[test]
fn dropping_the_references_frees_every_snapshot() {
    const PARTS: usize = 200;
    let dm = metallic_classes::new_game().expect("game");
    let luau = Rc::new(RefCell::new(LuauLanguage::new(dm.registry().clone()).expect("Luau VM")));
    let mut host = ScriptHost::new();
    host.register_language(Box::new(SharedLuau(Rc::clone(&luau))))
        .expect("register");
    let mut game = Game {
        dm,
        host,
        now: 0.0,
        sources: MemorySources::new(),
    };

    game.run(
        "Holder",
        &format!(
            r#"
            _G.parts = {{}}
            for i = 1, {PARTS} do
                local part = Instance.new("Part", workspace)
                part.Transparency = i / {PARTS}
                _G.parts[i] = part
            end
            for _, part in _G.parts do
                part:Destroy()
            end
            "#
        ),
    );
    game.assert_no_errors();
    assert_eq!(game.dm.snapshot_count(), PARTS, "every held part stays readable");
    game.run(
        "Reader",
        &format!("print(_G.parts[{PARTS}].Transparency, _G.parts[1].Parent)"),
    );
    assert_eq!(game.prints(), ["1 nil"]);

    // While a script holds them, a full collection frees nothing.
    luau.borrow_mut().collect_garbage(&mut game.dm).expect("gc");
    assert_eq!(game.dm.snapshot_count(), PARTS);

    game.run("Dropper", "_G.parts = nil");
    game.assert_no_errors();
    luau.borrow_mut().collect_garbage(&mut game.dm).expect("gc");
    assert_eq!(game.dm.snapshot_count(), 0, "every snapshot was freed");
}
