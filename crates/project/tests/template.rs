//! `templates/baseplate-3d`, the 3D Baseplate template: its files are what the code below produces.
//!
//! The start scene is built in code and written with `save_scene`, so it is always a scene this engine reads. After a
//! deliberate change to the template (or to the scene format), run with `METALLIC_UPDATE_REFERENCES=1` to write the
//! files again, and commit them.

use std::path::{Path, PathBuf};

use metallic_core::{CFrame, Color3, DataModel, Id, Value, Vector3};
use metallic_project::manifest::MANIFEST_FILE;
use metallic_project::{Manifest, SceneIds, Template, load_scene, save_scene};

/// The template's folder.
fn template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")
}

fn workspace(dm: &DataModel) -> Id {
    dm.service(dm.registry().class_id("Workspace").expect("the class"))
        .expect("the service")
}

/// Adds an anchored `Part` called `name` to `Workspace`.
fn part(dm: &mut DataModel, name: &str, size: Vector3, position: Vector3, rgb: [u8; 3], material: &str) {
    let material = dm.registry().enum_item("Material", material).expect("the material");
    let part = dm
        .create(dm.registry().class_id("Part").expect("the class"))
        .expect("a part");
    dm.set_name(part, name.into()).expect("named");
    for (prop, value) in [
        ("Size", Value::Vector3(size)),
        ("CFrame", Value::CFrame(CFrame::new(position))),
        ("Color", Value::Color3(Color3::from_rgb_u8(rgb[0], rgb[1], rgb[2]))),
        ("Material", Value::Enum(material)),
        ("Anchored", Value::Bool(true)),
    ] {
        dm.set_by_name(part, prop, value).expect("a part property");
    }
    let parent = workspace(dm);
    dm.set_parent(part, Some(parent)).expect("parented");
}

/// The template's start scene: a baseplate whose top is at y = 0, a spawn location in its middle, a camera looking
/// at it, and `Lighting` as a new game has it (the default lighting).
fn baseplate_scene() -> String {
    let mut dm = metallic_classes::new_game().expect("a game");
    part(
        &mut dm,
        "Baseplate",
        Vector3::new(512.0, 16.0, 512.0),
        Vector3::new(0.0, -8.0, 0.0),
        [99, 95, 98],
        "Concrete",
    );
    // A plain part until there is a SpawnLocation class (#198): it marks where players will appear.
    part(
        &mut dm,
        "SpawnLocation",
        Vector3::new(12.0, 1.0, 12.0),
        Vector3::new(0.0, 0.5, 0.0),
        [163, 162, 165],
        "SmoothPlastic",
    );

    let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace(&dm), "CurrentCamera") else {
        panic!("a new game has a camera");
    };
    let view = CFrame::look_at(Vector3::new(0.0, 20.0, 40.0), Vector3::ZERO, Vector3::Y);
    dm.set_by_name(camera, "CFrame", Value::CFrame(view)).expect("the view");

    let saved = save_scene(&dm, &mut SceneIds::new()).expect("saved");
    assert_eq!(saved.warnings, []);
    saved.text
}

/// Checks that the file at `member` in the template holds `expected`, or writes it when updating.
fn check(member: &str, expected: &str) {
    let path = template_dir().join(member);
    if std::env::var("METALLIC_UPDATE_REFERENCES").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("the folder made");
        std::fs::write(&path, expected).expect("written");
        return;
    }
    let found = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}; run with METALLIC_UPDATE_REFERENCES=1", path.display()));
    // A checkout that turned line ends into CRLF still holds the same template.
    assert!(
        found.replace("\r\n", "\n") == expected,
        "{} is not what the code produces; run with METALLIC_UPDATE_REFERENCES=1 to write it",
        path.display()
    );
}

#[test]
fn the_checked_in_template_is_what_the_code_produces() {
    let manifest = Manifest::new("Baseplate").to_toml();
    check(MANIFEST_FILE, &manifest);
    let start = Manifest::from_toml(&manifest).unwrap().manifest.start_scene;
    check(&start, &baseplate_scene());

    // Nothing else is in the folder, so a stray file is not packed into every new project.
    let template = Template::from_dir(template_dir()).unwrap();
    let members: Vec<&str> = template.members().map(|(member, _)| member).collect();
    assert_eq!(members, [start.as_str(), MANIFEST_FILE]);
}

#[test]
fn the_template_scene_loads_without_warnings() {
    let template = Template::from_dir(template_dir()).unwrap();
    let start = template.manifest().unwrap().start_scene;
    let text = String::from_utf8(template.member(&start).unwrap().to_vec()).unwrap();
    let mut dm = metallic_classes::new_game().unwrap();
    let loaded = load_scene(&mut dm, &text).unwrap();
    assert_eq!(loaded.warnings, []);
    let workspace = workspace(&dm);
    for name in ["Baseplate", "SpawnLocation", "Camera"] {
        assert!(dm.find_first_child(workspace, name, false).unwrap().is_some(), "{name}");
    }
}
