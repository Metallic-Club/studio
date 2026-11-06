//! File operations on a project (#190): creating, renaming, moving and deleting members, with asset ids kept, path
//! references rewritten and deletes reported, checked by saving, reopening and loading the start scene.
//!
//! A save that fails part-way is tested in `src/project/file_ops.rs`, where a failing writer can be injected.

use std::fs;
use std::path::{Path, PathBuf};

use metallic_core::{DataModel, Id, Value};
use metallic_project::manifest::MANIFEST_FILE;
use metallic_project::{
    AssetId, INDEX_FILE, MemberMove, Project, ProjectError, SceneIds, SceneReference, Template, save_scene,
};

const START: &str = "Scenes/Main.scene";
const ROCK: &str = "Assets/rock.glb";
const DOOR: &str = "Scripts/Door.luau";

/// A new project made from the Baseplate template, in an empty folder for one test.
fn new_project(test: &str) -> Project {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("file_ops").join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("the Baseplate template");
    Project::create(dir.join("Game.mtproj"), &template, "Game").expect("a new project")
}

/// Saves the project and opens it again from disk, so nothing survives but what the archive holds.
fn save_and_reopen(mut project: Project) -> Project {
    project.save().expect("saved");
    let path: PathBuf = project.path().to_path_buf();
    drop(project);
    Project::open(path).expect("reopened")
}

fn game() -> DataModel {
    metallic_classes::new_game().expect("a game")
}

fn workspace(dm: &DataModel) -> Id {
    dm.service(dm.registry().class_id("Workspace").expect("the class"))
        .expect("the service")
}

fn new_in(dm: &mut DataModel, class: &str, name: &str) -> Id {
    let id = dm
        .create(dm.registry().class_id(class).expect("the class"))
        .expect("created");
    dm.set_name(id, name.into()).expect("named");
    dm.set_parent(id, Some(workspace(dm))).expect("parented");
    id
}

fn child(dm: &DataModel, name: &str) -> Id {
    dm.find_first_child(workspace(dm), name, false)
        .expect("Workspace")
        .unwrap_or_else(|| panic!("no `{name}` in Workspace"))
}

fn text(value: &str) -> Value {
    Value::String(value.into())
}

/// A project whose start scene has a `MeshPart` drawing the rock (by asset id) and a `Script` running the door
/// script (by path), plus a member nothing refers to. Returns it with the rock's id, saved.
fn referencing_project(test: &str) -> (Project, AssetId) {
    let mut project = new_project(test);
    let cube =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/textured_cube.glb")).expect("set up");
    let rock = project.create_member(ROCK, cube).expect("set up");
    project.create_member(DOOR, "print('door')").expect("set up");
    project
        .create_member("Assets/notes.txt", "nothing refers to this")
        .expect("set up");

    let mut dm = game();
    let statue = new_in(&mut dm, "MeshPart", "Statue");
    dm.set_by_name(statue, "MeshId", text(&rock.to_string()))
        .expect("set up");
    let script = new_in(&mut dm, "Script", "Door");
    dm.set_by_name(script, "Source", text(DOOR)).expect("set up");
    let saved = save_scene(&dm, &mut SceneIds::new()).expect("set up");
    project.write(START, saved.text).expect("set up");
    // Indexes the template's scene, which came without an id.
    let report = project.refresh_assets().expect("set up");
    assert_eq!(report.added.len(), 1);
    (save_and_reopen(project), rock)
}

#[test]
fn moved_assets_keep_loading_after_save_and_reopen() {
    let (mut project, rock) = referencing_project("move");
    let start_id = project.assets().id(START).expect("the start scene is indexed");

    // By id: only the index changes.
    let report = project.rename_member(ROCK, "Assets/Models/Rock.glb").unwrap();
    assert_eq!(
        report.moved,
        [MemberMove {
            from: ROCK.to_owned(),
            to: "Assets/Models/Rock.glb".to_owned(),
            id: Some(rock),
        }]
    );
    assert!(report.scenes.is_empty(), "no scene names the rock by path");
    assert!(!report.start_scene);
    assert_eq!(project.assets().id("Assets/Models/Rock.glb"), Some(rock), "same id");
    assert_eq!(project.assets().id(ROCK), None);
    let scene_before = project.read(START).unwrap();

    // By path: a folder of scripts moves, and the scene's `Source` follows.
    let report = project.rename_folder("Scripts", "Code/Doors").unwrap();
    assert_eq!(report.moved.len(), 1);
    assert_eq!(report.moved[0].to, "Code/Doors/Door.luau");
    assert_eq!(report.scenes, [START]);
    assert_eq!(
        String::from_utf8(project.read(START).unwrap()).unwrap(),
        String::from_utf8(scene_before)
            .unwrap()
            .replace(&format!("\"{DOOR}\""), "\"Code/Doors/Door.luau\""),
        "only the path changed in the scene's text"
    );

    // The start scene itself: `metallic.toml` follows it.
    let report = project.rename_member(START, "Levels/One.scene").unwrap();
    assert!(report.start_scene);
    assert_eq!(report.moved[0].id, Some(start_id));
    assert_eq!(project.manifest().start_scene, "Levels/One.scene");

    let mut project = save_and_reopen(project);
    assert_eq!(project.manifest().start_scene, "Levels/One.scene");
    assert_eq!(project.assets().id("Levels/One.scene"), Some(start_id));
    assert!(!project.contains(ROCK) && !project.contains(DOOR) && !project.contains(START));
    let mut dm = game();
    let loaded = project.load_start_scene(&mut dm).unwrap();
    assert_eq!(loaded.warnings, []);
    let statue = child(&dm, "Statue");
    assert_eq!(dm.get_by_name(statue, "MeshId"), Ok(text(&rock.to_string())));
    let script = child(&dm, "Door");
    assert_eq!(dm.get_by_name(script, "Source"), Ok(text("Code/Doors/Door.luau")));
    assert_eq!(project.read_text("Code/Doors/Door.luau").unwrap(), "print('door')");
    // The rock still decodes by its id, now from its new member.
    let mut reader = project.asset_reader();
    assert_eq!(reader.member(&rock.to_string()).unwrap().1, "Assets/Models/Rock.glb");
    reader.load_mesh(&rock.to_string()).unwrap();

    // The rewritten scene is exactly what saving the loaded game writes.
    let mut ids = loaded.ids;
    let resaved = save_scene(&dm, &mut ids).unwrap();
    assert_eq!(project.read_text("Levels/One.scene").unwrap(), resaved.text);
}

#[test]
fn deleting_reports_what_referred_to_the_member() {
    let (mut project, rock) = referencing_project("delete");

    let preview = project.references(ROCK).unwrap();
    let report = project.delete_member(ROCK).unwrap();
    assert_eq!(report, preview, "references() shows what a delete will report");
    assert_eq!(report.id, Some(rock));
    assert!(!report.start_scene);
    assert_eq!(
        report.scenes,
        [SceneReference {
            scene: START.to_owned(),
            instance: report.scenes[0].instance,
            class: "MeshPart".to_owned(),
            name: "Statue".to_owned(),
            property: "MeshId".to_owned(),
        }]
    );
    assert!(!project.contains(ROCK));
    assert_eq!(
        project.assets().path(rock),
        Some(ROCK),
        "a referenced asset stays indexed, as missing"
    );

    let report = project.delete_member(DOOR).unwrap();
    assert_eq!(report.scenes.len(), 1);
    assert_eq!(
        (report.scenes[0].class.as_str(), report.scenes[0].property.as_str()),
        ("Script", "Source")
    );

    let notes = project.assets().id("Assets/notes.txt").unwrap();
    let report = project.delete_member("Assets/notes.txt").unwrap();
    assert!(report.is_empty());
    assert_eq!(
        project.assets().path(notes),
        None,
        "an unreferenced asset leaves the index"
    );

    let report = project.delete_member(START).unwrap();
    assert!(report.start_scene, "the manifest's start scene is a reference");

    // After a save and reopen, the deletes hold.
    let project = save_and_reopen(project);
    assert!(!project.contains(ROCK) && !project.contains(DOOR) && !project.contains(START));
    assert_eq!(project.assets().path(rock), Some(ROCK));
}

#[test]
fn create_gives_ids_and_every_new_path_is_checked() {
    let mut project = new_project("create");
    let id = project
        .create_member("Assets/grass.png", b"\x89PNG grass".to_vec())
        .unwrap();
    assert_eq!(project.assets().id("Assets/grass.png"), Some(id));
    let other = project
        .create_member("Assets/sand.png", b"\x89PNG sand".to_vec())
        .unwrap();
    assert_ne!(id, other);

    for bad in ["../evil.txt", "/abs.txt", "a\\b.txt", "Assets//x.png", "C:x.png"] {
        assert!(
            matches!(project.create_member(bad, "x"), Err(ProjectError::UnsafeMember { .. })),
            "{bad}"
        );
        assert!(
            matches!(
                project.rename_member("Assets/grass.png", bad),
                Err(ProjectError::UnsafeMember { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        project.create_member(MANIFEST_FILE, "x"),
        Err(ProjectError::ManifestMember { .. })
    ));
    assert!(matches!(
        project.rename_member("Assets/grass.png", INDEX_FILE),
        Err(ProjectError::IndexMember { .. })
    ));
    assert!(matches!(
        project.rename_member(MANIFEST_FILE, "Assets/m.toml"),
        Err(ProjectError::ManifestMember { .. })
    ));
    assert!(matches!(
        project.delete_member(INDEX_FILE),
        Err(ProjectError::IndexMember { .. })
    ));
    assert!(matches!(
        project.create_member("Assets/grass.png", "again"),
        Err(ProjectError::MemberExists { .. })
    ));
    assert!(matches!(
        project.rename_member("Assets/grass.png", "Assets/sand.png"),
        Err(ProjectError::MemberExists { .. })
    ));
    assert!(matches!(
        project.rename_member("Assets/none.png", "Assets/x.png"),
        Err(ProjectError::MissingMember { .. })
    ));
    assert!(matches!(
        project.rename_folder("Assets", "Assets/Old"),
        Err(ProjectError::FolderIntoItself { .. })
    ));
    assert!(matches!(
        project.rename_folder("Nothing", "Else"),
        Err(ProjectError::MissingMember { .. })
    ));
    project.create_member("Textures/sand.png", "taken").unwrap();
    assert!(matches!(
        project.rename_folder("Assets", "Textures"),
        Err(ProjectError::MemberExists { .. })
    ));
    assert!(project.contains("Assets/grass.png"), "a refused move changes nothing");
    assert!(matches!(
        project.delete_member("Assets/none.png"),
        Err(ProjectError::MissingMember { .. })
    ));

    // A folder move keeps every id.
    let report = project.rename_folder("Assets", "Art").unwrap();
    assert_eq!(report.moved.len(), 2);
    let project = save_and_reopen(project);
    assert_eq!(project.assets().id("Art/grass.png"), Some(id));
    assert_eq!(project.assets().id("Art/sand.png"), Some(other));
    assert_eq!(
        project.members(),
        [
            "Art/grass.png",
            "Art/sand.png",
            START,
            "Textures/sand.png",
            INDEX_FILE,
            MANIFEST_FILE
        ]
    );
}
