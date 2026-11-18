//! `CollectionService` (#809) driven through the engine: tags from Luau, `GetTagged`, and the added and removed
//! signals, which fire at the start of the frame after the change.
#![cfg(feature = "luau")]

use metallic_core::{DataModel, Value};
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};
use metallic_script::{CollectOutput, OutputLevel};

const MAIN: &str = r#"
local CollectionService = game:GetService("CollectionService")
_G.log = {}
local function note(text) table.insert(_G.log, text) end
CollectionService:GetInstanceAddedSignal("Lava"):Connect(function(part) note("added " .. part.Name) end)
CollectionService:GetInstanceRemovedSignal("Lava"):Connect(function(part) note("removed " .. part.Name) end)

local a = Instance.new("Part")
a.Name = "A"
a.Parent = workspace
CollectionService:AddTag(a, "Lava")
local b = Instance.new("Part")
b.Name = "B"
b:AddTag("Lava")
b:AddTag("Lava")
_G.a, _G.b = a, b
"#;

/// A started standalone game running [`MAIN`].
fn started() -> Engine<CollectOutput> {
    let sources = MemorySources::new().with("Main.luau", MAIN);
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    add_script(engine.data_model_mut());
    engine.start().expect("start");
    engine
}

fn add_script(dm: &mut DataModel) {
    let class = dm.registry().class_id("Script").expect("Script");
    let service = dm.registry().class_id("ServerScriptService").expect("service class");
    let service = dm.service(service).expect("ServerScriptService");
    let script = dm.create(class).expect("create");
    dm.set_name(script, "Main".into()).expect("name");
    dm.set_by_name(script, "Source", Value::String("Main.luau".into()))
        .expect("source");
    dm.set_parent(script, Some(service)).expect("parent");
}

/// Runs `code` in a throwaway script and returns what it prints.
fn probe(engine: &mut Engine<CollectOutput>, code: &str) -> String {
    let (dm, host) = engine.parts_mut();
    let class = dm.registry().class_id("Script").expect("Script");
    let script = dm.create(class).expect("probe");
    let before = host.output().lines.len();
    host.run_script(dm, script, "Probe.luau", code).expect("probe runs");
    let printed: Vec<String> = host.output().lines[before..]
        .iter()
        .map(|line| {
            assert_eq!(line.level, OutputLevel::Print, "probe failed: {}", line.message);
            line.message.clone()
        })
        .collect();
    host.output_mut().lines.truncate(before);
    printed.join("\n")
}

fn log(engine: &mut Engine<CollectOutput>) -> String {
    probe(engine, "print(table.concat(_G.log, \", \"))")
}

#[test]
fn tags_are_found_and_their_signals_fire_a_frame_later() {
    let mut engine = started();
    let errors: Vec<String> = engine
        .host()
        .output()
        .of_level(OutputLevel::Error)
        .map(|l| l.message.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // Deferred: nothing yet.
    assert_eq!(log(&mut engine), "");
    engine.tick();
    // B has no parent, so only A came into the game with the tag.
    assert_eq!(log(&mut engine), "added A");
    assert_eq!(
        probe(
            &mut engine,
            "local cs = game:GetService(\"CollectionService\")
             print(#cs:GetTagged(\"Lava\"), cs:HasTag(_G.b, \"Lava\"), _G.a:HasTag(\"Lava\"), \
                   table.concat(_G.b:GetTags(), \",\"), table.concat(cs:GetAllTags(), \",\"))
             print(typeof(cs:GetInstanceAddedSignal(\"Lava\")))"
        ),
        "1 true true Lava Lava\nRBXScriptSignal"
    );

    // B comes into the game, A loses the tag, then a tagged clone arrives and is destroyed in one frame.
    probe(
        &mut engine,
        "_G.b.Parent = workspace
         _G.a:RemoveTag(\"Lava\")",
    );
    engine.tick();
    assert_eq!(log(&mut engine), "added A, added B, removed A");
    probe(
        &mut engine,
        "local c = _G.b:Clone()
         c.Name = \"C\"
         c.Parent = workspace
         _G.b:Destroy()",
    );
    engine.tick();
    assert_eq!(log(&mut engine), "added A, added B, removed A, removed B, added C");
    assert_eq!(
        probe(
            &mut engine,
            "print(#game:GetService(\"CollectionService\"):GetTagged(\"Lava\"))"
        ),
        "1"
    );
}
