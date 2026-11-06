//! A game run from its build (#191): a project with a script and a mesh part is packed into a `game.pak`, and the
//! engine loads its start scene, runs its script and reads the part's mesh from the pak alone, through the same
//! [`GameFiles`] interface it uses for the project.
#![cfg(feature = "luau")]

use std::path::Path;

use metallic_core::glam::Vec3;
use metallic_core::{DataModel, Id, Value};
use metallic_engine::metallic_project::{GameFiles, Pak, Project, SceneIds, Template, save_scene};
use metallic_engine::{Engine, EngineConfig, RunContext, project_sources};
use metallic_script::{CollectOutput, OutputLevel};

const MODEL: &str = "Assets/Models/textured_cube.glb";
const SCRIPT: &str = "Scripts/Main.luau";

fn service(dm: &DataModel, class: &str) -> Id {
    dm.service(dm.registry().class_id(class).expect("the class"))
        .expect("the service")
}

fn new_in(dm: &mut DataModel, class: &str, parent: Id, name: &str) -> Id {
    let id = dm
        .create(dm.registry().class_id(class).expect("the class"))
        .expect("made");
    dm.set_name(id, name.into()).expect("named");
    dm.set_parent(id, Some(parent)).expect("parented");
    id
}

/// A project in a fresh folder whose start scene has a script (which prints) and a mesh part naming the model, saved
/// and packed into `game.pak` beside it. Returns the pak's path; the project file is deleted, so only the pak is
/// left to read.
fn packed_game() -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("game_pak");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("the Baseplate template");
    let path = dir.join("Game.mtproj");
    let mut project = Project::create(&path, &template, "Packed").expect("a project");
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../project/tests/fixtures/textured_cube.glb");
    project
        .write(MODEL, std::fs::read(model).expect("the model"))
        .expect("written");
    project
        .write(SCRIPT, "print('running from ' .. game.Workspace.Crate.Name)")
        .expect("written");
    project.refresh_assets().expect("indexed");
    let mesh = project.assets().id(MODEL).expect("indexed").to_string();

    let mut dm = metallic_classes::new_game().expect("a game");
    let workspace = service(&dm, "Workspace");
    let part = new_in(&mut dm, "MeshPart", workspace, "Crate");
    dm.set_by_name(part, "MeshId", Value::String(mesh.into())).expect("set");
    dm.set_by_name(part, "Size", Value::Vector3(Vec3::splat(4.0)))
        .expect("set");
    let scripts = service(&dm, "ServerScriptService");
    let script = new_in(&mut dm, "Script", scripts, "Main");
    dm.set_by_name(script, "Source", Value::String(SCRIPT.into()))
        .expect("set");
    let scene = save_scene(&dm, &mut SceneIds::new()).expect("saved");
    let start = project.manifest().start_scene.clone();
    project.write(&start, scene.text).expect("written");
    project.save().expect("saved");

    let pak = dir.join("game.pak");
    Pak::pack(&mut project, &pak).expect("packed");
    drop(project);
    std::fs::remove_file(&path).expect("the project is gone");
    pak
}

#[test]
fn a_game_loads_and_runs_from_its_pak() {
    let mut pak = Pak::open(packed_game()).expect("the pak opens");
    let files: &mut dyn GameFiles = &mut pak;

    let config = EngineConfig::new(RunContext::Local).with_sources(project_sources(files).expect("the scripts"));
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("an engine");
    let loaded = files
        .load_start_scene(engine.data_model_mut())
        .expect("the start scene");
    assert_eq!(loaded.warnings, []);
    engine.start().expect("started");
    let output = engine.host().output();
    let errors: Vec<&str> = output
        .of_level(OutputLevel::Error)
        .map(|line| line.message.as_str())
        .collect();
    assert!(errors.is_empty(), "script errors: {errors:#?}");
    assert!(
        output.lines.iter().any(|line| line.message == "running from Crate"),
        "{:?}",
        output.lines
    );

    // The part's mesh, by the id its scene names, from the pak on the asset worker's reader.
    let dm = engine.data_model_mut();
    let workspace = service(dm, "Workspace");
    let part = dm
        .children(workspace)
        .expect("Workspace")
        .find(|&child| dm.name(child).is_ok_and(|name| name == "Crate"))
        .expect("the mesh part");
    let Ok(Value::String(mesh)) = dm.get_by_name(part, "MeshId") else {
        panic!("the part names its mesh");
    };
    let mut reader = files.asset_reader();
    let mesh = reader.load_mesh(&mesh).expect("the mesh loads from the pak");
    assert_eq!(mesh.triangles(), 12);
    assert_eq!(mesh.size, [2.0, 2.0, 2.0]);
}
