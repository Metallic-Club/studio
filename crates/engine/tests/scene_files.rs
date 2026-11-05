//! The engine's scenes survive the scene format (#186): the Player's demo game and every stress scene, saved,
//! loaded into a new game and saved again, give the same file.
//!
//! The test lives here rather than in `metallic_project` because the scenes are built by the engine (the stress
//! builders) or by running the demo's script, and `metallic_project` may not depend on the engine (ADR 0001).

use metallic_core::DataModel;
use metallic_engine::stress::{StressKind, StressScene};
use metallic_project::{SceneIds, load_scene, save_scene};

/// The seed the Player builds stress scenes from.
const SEED: u64 = 1;

/// Saves `dm`, loads the file into a new game, saves that again, and checks both files are the same, with the
/// loaded ids and with a fresh numbering. Returns the file.
fn assert_round_trips(dm: &DataModel, scene: &str) -> String {
    let saved = save_scene(dm, &mut SceneIds::new()).unwrap_or_else(|e| panic!("{scene}: {e}"));
    assert_eq!(saved.warnings, [], "{scene}: save warnings");

    let mut loaded_dm = metallic_classes::new_game().expect("a new game");
    let loaded = load_scene(&mut loaded_dm, &saved.text).unwrap_or_else(|e| panic!("{scene}: {e}"));
    assert_eq!(loaded.warnings, [], "{scene}: load warnings");
    assert_eq!(
        loaded_dm.len(),
        dm.len() - non_saved_instances(dm),
        "{scene}: instance count"
    );

    let mut ids = loaded.ids;
    let again = save_scene(&loaded_dm, &mut ids).unwrap_or_else(|e| panic!("{scene}: {e}"));
    assert_same(&saved.text, &again.text, &format!("{scene}, saved again"));
    let fresh = save_scene(&loaded_dm, &mut SceneIds::new()).unwrap_or_else(|e| panic!("{scene}: {e}"));
    assert_same(&saved.text, &fresh.text, &format!("{scene}, numbered afresh"));
    saved.text
}

/// Checks two scene files are the same, naming the first line that differs (the files are too large to print).
fn assert_same(first: &str, second: &str, what: &str) {
    if first == second {
        return;
    }
    let line = first
        .lines()
        .zip(second.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| first.lines().count().min(second.lines().count()));
    let context = |text: &str| {
        text.lines()
            .skip(line.saturating_sub(8))
            .take(9)
            .collect::<Vec<_>>()
            .join("\n")
    };
    panic!(
        "{what}: the files differ at line {}\nfirst:\n{}\nsecond:\n{}",
        line + 1,
        context(first),
        context(second)
    );
}

/// How many more instances `dm` has than a new game under the services that are not saved. A scene does not
/// hold them, so a game loaded from it has only a new game's.
fn non_saved_instances(dm: &DataModel) -> usize {
    let new_game = metallic_classes::new_game().expect("a new game");
    let registry = dm.registry();
    let outside = |dm: &DataModel| -> usize {
        registry
            .services()
            .iter()
            .filter(|&&class| registry.class(class).is_some_and(|info| !info.persisted()))
            .filter_map(|&class| dm.service(class))
            .map(|service| dm.descendants(service).map_or(0, Iterator::count))
            .sum()
    };
    outside(dm) - outside(&new_game)
}

#[test]
fn stress_scenes_round_trip() {
    for kind in StressKind::ALL {
        let mut dm = metallic_classes::new_game().expect("a new game");
        StressScene::build(&mut dm, kind, SEED).unwrap_or_else(|e| panic!("{}: {e}", kind.name()));
        assert_round_trips(&dm, kind.name());
    }
}

/// The Player's demo game builds its scene from a script when it starts; the scene saved after that holds the
/// parts, the lights, `Lighting`'s settings and the script itself, pointing at its source.
#[cfg(feature = "luau")]
#[test]
fn the_demo_scene_round_trips() {
    use metallic_core::Value;
    use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};
    use metallic_script::{CollectOutput, OutputLevel};

    const DEMO: &str = include_str!("../../../apps/player/src/scenes/demo.luau");

    let sources = MemorySources::new().with("Demo.luau", DEMO);
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    let dm = engine.data_model_mut();
    let registry = dm.registry().clone();
    let scripts = dm
        .service(registry.class_id("ServerScriptService").expect("class"))
        .expect("service");
    let script = dm.create(registry.class_id("Script").expect("Script")).expect("script");
    dm.set_name(script, "Demo".into()).expect("name");
    dm.set_by_name(script, "Source", Value::String("Demo.luau".into()))
        .expect("source");
    dm.set_parent(script, Some(scripts)).expect("parent");
    engine.start().expect("start");
    let errors: Vec<&str> = engine
        .host()
        .output()
        .of_level(OutputLevel::Error)
        .map(|line| line.message.as_str())
        .collect();
    assert!(errors.is_empty(), "script errors: {errors:#?}");

    let text = assert_round_trips(engine.data_model(), "demo");
    assert!(
        text.contains("\"Source\": \"Demo.luau\""),
        "the script points at its source"
    );
    assert!(
        text.contains("\"class\": \"SpotLight\""),
        "the demo's parts and lights are saved"
    );
}
