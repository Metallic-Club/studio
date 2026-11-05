//! A scene written in an older format loads with warnings, not errors (#186).
//!
//! `fixtures/v0.scene` is a hand-written format 0 scene: format 0 kept every instance's name in a `Name` property,
//! which format 1 moved to a `name` key. It must load through the migration, with one warning for the upgrade and
//! nothing else, and save back as the current format.

use metallic_core::{DataModel, Id, Value};
use metallic_project::{SCENE_FORMAT, SceneWarning, load_scene, save_scene};

const V0: &str = include_str!("fixtures/v0.scene");

fn service(dm: &DataModel, class: &str) -> Id {
    let class = dm.registry().class_id(class).expect("service class");
    dm.service(class).expect("service")
}

fn child(dm: &DataModel, parent: Id, name: &str) -> Id {
    dm.find_first_child(parent, name, false)
        .expect("parent exists")
        .unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn a_format_0_scene_loads_with_a_warning() {
    let mut dm = metallic_classes::new_game().expect("a new game");
    let loaded = load_scene(&mut dm, V0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.format, 0);
    assert!(
        matches!(
            loaded.warnings.as_slice(),
            [SceneWarning::Migrated { from: 0, to: 1, .. }]
        ),
        "{:?}",
        loaded.warnings
    );

    let workspace = service(&dm, "Workspace");
    let names: Vec<&str> = dm
        .children(workspace)
        .expect("Workspace")
        .map(|c| dm.name(c).expect("name"))
        .collect();
    assert_eq!(names, ["Camera", "Baseplate", "Lamp"]);
    let glow = child(&dm, child(&dm, workspace, "Lamp"), "Glow");
    assert_eq!(dm.get_by_name(glow, "Range"), Ok(Value::Number(24.0)));
    let camera = child(&dm, workspace, "Camera");
    assert_eq!(dm.get_by_name(workspace, "CurrentCamera"), Ok(Value::Ref(Some(camera))));
    let lighting = service(&dm, "Lighting");
    assert_eq!(dm.get_by_name(lighting, "ClockTime"), Ok(Value::Number(18.5)));
    let spin = child(&dm, service(&dm, "ServerScriptService"), "Spin");
    assert_eq!(
        dm.get_by_name(spin, "Source"),
        Ok(Value::String("Scripts/Spin.luau".into()))
    );

    // Saved again, it is a current scene that keeps the file's ids and loads with no warnings.
    let mut ids = loaded.ids;
    let saved = save_scene(&dm, &mut ids).expect("save");
    assert_eq!(saved.warnings, []);
    assert!(!saved.text.contains("\"Name\""), "{}", saved.text);
    assert!(saved.text.contains("\"name\": \"Baseplate\""));
    let doc: serde_json::Value = serde_json::from_str(&saved.text).expect("JSON");
    assert_eq!(doc["format"], SCENE_FORMAT);
    let mut again = metallic_classes::new_game().expect("a new game");
    let reloaded = load_scene(&mut again, &saved.text).expect("load");
    assert_eq!(reloaded.warnings, []);
    let mut ids = reloaded.ids;
    assert_eq!(save_scene(&again, &mut ids).expect("save").text, saved.text);
}
