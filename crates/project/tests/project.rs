//! `.mtproj` projects: creating one from the Baseplate template, opening it, editing and saving it, and every way
//! opening one fails.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use metallic_core::DataModel;
use metallic_project::manifest::MANIFEST_FILE;
use metallic_project::project::{TEMP_SUFFIX, compression_for};
use metallic_project::{Manifest, Project, ProjectError, Template, check_member_path};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const START: &str = "Scenes/Main.scene";

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("project").join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

fn template() -> Template {
    Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("the Baseplate template")
}

fn game() -> DataModel {
    metallic_classes::new_game().expect("a game")
}

/// The names of `Workspace`'s children.
fn workspace_children(dm: &DataModel) -> Vec<String> {
    let workspace = dm
        .service(dm.registry().class_id("Workspace").expect("the class"))
        .expect("the service");
    dm.children(workspace)
        .expect("Workspace")
        .map(|child| dm.name(child).expect("a child").to_owned())
        .collect()
}

/// Writes a zip archive at `path` holding `members`, deflated.
fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
    let mut writer = ZipWriter::new(File::create(path).expect("the archive"));
    for (member, bytes) in members {
        writer
            .start_file(*member, SimpleFileOptions::default())
            .expect("a member");
        writer.write_all(bytes).expect("written");
    }
    writer.finish().expect("finished");
}

fn manifest() -> Vec<u8> {
    Manifest::new("Test").to_toml().into_bytes()
}

fn scene() -> Vec<u8> {
    template().member(START).expect("the start scene").to_vec()
}

fn open_error(path: &Path) -> ProjectError {
    Project::open(path).expect_err("the project is refused")
}

#[test]
fn a_project_made_from_the_template_opens_and_shows_the_baseplate() {
    let dir = scratch("create");
    let path = dir.join("My Game.mtproj");
    let project = Project::create(&path, &template(), "My Game").unwrap();
    assert_eq!(project.manifest().name, "My Game");
    assert_eq!(project.manifest().start_scene, START);
    assert_eq!(project.members(), [START, MANIFEST_FILE]);
    assert!(!project.is_modified());
    assert!(
        !dir.join(format!("My Game.mtproj{TEMP_SUFFIX}")).exists(),
        "the temporary archive was renamed"
    );

    // Reopened from disk, the start scene loads into a game.
    drop(project);
    let mut project = Project::open(&path).unwrap();
    assert_eq!(project.warnings(), []);
    let mut dm = game();
    let loaded = project.load_start_scene(&mut dm).unwrap();
    assert_eq!(loaded.warnings, []);
    assert_eq!(workspace_children(&dm), ["Camera", "Baseplate", "SpawnLocation"]);
}

#[test]
fn the_archive_has_the_manifest_first_and_text_deflated() {
    let dir = scratch("layout");
    let path = dir.join("Game.mtproj");
    let mut project = Project::create(&path, &template(), "Game").unwrap();
    project.write("Assets/Logo.png", vec![0x89, b'P', b'N', b'G']).unwrap();
    project.write("Scripts/Main.luau", "print('hi')").unwrap();
    project.save().unwrap();

    let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
    let names: Vec<String> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
    assert_eq!(
        names,
        [MANIFEST_FILE, "Assets/Logo.png", START, "Scripts/Main.luau"],
        "the manifest first, then sorted"
    );
    for (member, method) in [
        (MANIFEST_FILE, CompressionMethod::Deflated),
        (START, CompressionMethod::Deflated),
        ("Scripts/Main.luau", CompressionMethod::Deflated),
        ("Assets/Logo.png", CompressionMethod::Stored),
    ] {
        assert_eq!(archive.by_name(member).unwrap().compression(), method, "{member}");
    }
}

#[test]
fn compression_follows_the_format() {
    for stored in ["a.png", "Assets/B.JPG", "c.jpeg", "d.glb", "Music/e.ogg", "f.mp3"] {
        assert_eq!(compression_for(stored), CompressionMethod::Stored, "{stored}");
    }
    for deflated in [
        MANIFEST_FILE,
        START,
        "a.luau",
        "b.gltf",
        "c.json",
        "README",
        "png/readme",
    ] {
        assert_eq!(compression_for(deflated), CompressionMethod::Deflated, "{deflated}");
    }
}

#[test]
fn creating_the_same_project_twice_gives_the_same_bytes() {
    let dir = scratch("deterministic");
    Project::create(dir.join("a.mtproj"), &template(), "Game").unwrap();
    Project::create(dir.join("b.mtproj"), &template(), "Game").unwrap();
    assert_eq!(
        fs::read(dir.join("a.mtproj")).unwrap(),
        fs::read(dir.join("b.mtproj")).unwrap()
    );
}

#[test]
fn create_never_replaces_a_file() {
    let dir = scratch("exists");
    let path = dir.join("Game.mtproj");
    fs::write(&path, "precious").unwrap();
    let error = Project::create(&path, &template(), "Game").unwrap_err();
    assert!(matches!(error, ProjectError::AlreadyExists { .. }), "{error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "precious");
}

#[test]
fn edits_stay_in_memory_until_saved() {
    let dir = scratch("edits");
    let path = dir.join("Game.mtproj");
    let mut project = Project::create(&path, &template(), "Game").unwrap();
    let before = fs::read(&path).unwrap();

    project.write("Scripts/Main.luau", "print('hi')").unwrap();
    let mut manifest = project.manifest().clone();
    manifest.description = "Edited".to_owned();
    project.set_manifest(manifest).unwrap();
    assert!(project.is_modified());
    assert_eq!(project.read_text("Scripts/Main.luau").unwrap(), "print('hi')");
    assert_eq!(fs::read(&path).unwrap(), before, "nothing is written before a save");

    project.save().unwrap();
    assert!(!project.is_modified());
    let mut reopened = Project::open(&path).unwrap();
    assert_eq!(reopened.manifest().description, "Edited");
    assert_eq!(reopened.read_text("Scripts/Main.luau").unwrap(), "print('hi')");
    // The unchanged scene was copied over as it was.
    assert_eq!(reopened.read(START).unwrap(), scene());

    // Removing a member and saving again: the project still saves over the file it has open.
    assert!(project.remove("Scripts/Main.luau"));
    assert!(!project.remove("Scripts/Main.luau"));
    assert!(!project.remove(MANIFEST_FILE), "the manifest stays");
    project.save().unwrap();
    drop(reopened);
    let reopened = Project::open(&path).unwrap();
    assert_eq!(reopened.members(), [START, MANIFEST_FILE]);
}

#[test]
fn a_failed_save_leaves_the_project_as_it_was() {
    let dir = scratch("failed-save");
    let path = dir.join("Game.mtproj");
    let mut project = Project::create(&path, &template(), "Game").unwrap();
    let before = fs::read(&path).unwrap();
    project.write("Scripts/Main.luau", "print('hi')").unwrap();

    // A folder where the temporary archive goes makes the save fail before anything is renamed.
    let temp = dir.join(format!("Game.mtproj{TEMP_SUFFIX}"));
    fs::create_dir(&temp).unwrap();
    let error = project.save().unwrap_err();
    assert!(matches!(error, ProjectError::Save { .. }), "{error}");
    assert!(error.to_string().contains("the file on disk is unchanged"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(project.is_modified(), "the edits are kept");

    fs::remove_dir(&temp).unwrap();
    project.save().unwrap();
    assert!(Project::open(&path).unwrap().contains("Scripts/Main.luau"));
}

#[test]
fn a_missing_file_is_refused() {
    let dir = scratch("missing");
    let error = open_error(&dir.join("Nowhere.mtproj"));
    assert!(matches!(error, ProjectError::NotFound { .. }), "{error}");
    assert!(
        error.to_string().ends_with("Nowhere.mtproj: no such project file"),
        "{error}"
    );
}

#[test]
fn a_file_that_is_not_a_zip_is_refused() {
    let dir = scratch("not-zip");
    let path = dir.join("Game.mtproj");
    for contents in [&b""[..], b"[project]\nname = \"Game\"\n", &[0u8; 4096]] {
        fs::write(&path, contents).unwrap();
        let error = open_error(&path);
        assert!(matches!(error, ProjectError::NotAZip { .. }), "{error}");
        assert!(error.to_string().contains("not a Metallic project"), "{error}");
    }
}

#[test]
fn a_missing_manifest_is_refused() {
    let dir = scratch("no-manifest");
    let path = dir.join("Game.mtproj");
    // In a folder is not at the root.
    write_zip(&path, &[(START, &scene()), ("Game/metallic.toml", &manifest())]);
    let error = open_error(&path);
    assert!(matches!(error, ProjectError::MissingManifest { .. }), "{error}");
    assert!(
        error.to_string().ends_with("no metallic.toml at the project's root"),
        "{error}"
    );
}

#[test]
fn an_invalid_manifest_is_refused() {
    let dir = scratch("bad-manifest");
    let path = dir.join("Game.mtproj");
    for (text, message) in [
        (&b"[project\n"[..], "metallic.toml is not valid TOML"),
        (b"[project]\nname = \"Game\"\n", "`project.kind` is missing"),
        (b"\xFF\xFE", "metallic.toml is not UTF-8 text"),
    ] {
        write_zip(&path, &[(MANIFEST_FILE, text), (START, &scene())]);
        let error = open_error(&path);
        assert!(matches!(error, ProjectError::Manifest { .. }), "{error}");
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn a_missing_start_scene_is_refused() {
    let dir = scratch("no-scene");
    let path = dir.join("Game.mtproj");
    write_zip(&path, &[(MANIFEST_FILE, &manifest()), ("Scenes/Other.scene", &scene())]);
    // Opening reads only the manifest; loading the start scene says what is missing.
    let mut project = Project::open(&path).unwrap();
    let mut dm = game();
    let error = project.load_start_scene(&mut dm).unwrap_err();
    assert!(matches!(error, ProjectError::MissingStartScene { .. }), "{error}");
    assert!(
        error
            .to_string()
            .ends_with("the start scene `Scenes/Main.scene` is not in the project"),
        "{error}"
    );
    assert_eq!(workspace_children(&dm), ["Camera"], "the game is unchanged");

    // A template without its start scene makes no project.
    let template_dir = dir.join("template");
    fs::create_dir_all(&template_dir).unwrap();
    fs::write(template_dir.join(MANIFEST_FILE), manifest()).unwrap();
    let error = Project::create(
        dir.join("New.mtproj"),
        &Template::from_dir(&template_dir).unwrap(),
        "New",
    )
    .unwrap_err();
    assert!(matches!(error, ProjectError::MissingStartScene { .. }), "{error}");
    assert!(!dir.join("New.mtproj").exists());
}

#[test]
fn a_start_scene_that_does_not_load_is_refused() {
    let dir = scratch("bad-scene");
    let path = dir.join("Game.mtproj");
    write_zip(&path, &[(MANIFEST_FILE, &manifest()), (START, b"{\"format\": 99}")]);
    let mut project = Project::open(&path).unwrap();
    let error = project.load_start_scene(&mut game()).unwrap_err();
    assert!(matches!(error, ProjectError::Scene { .. }), "{error}");
    assert!(error.to_string().contains("the scene is in format 99"), "{error}");
}

#[test]
fn path_traversal_members_are_refused() {
    let dir = scratch("traversal");
    let path = dir.join("Game.mtproj");
    for evil in [
        "../evil.luau",
        "Scripts/../../evil.luau",
        "/etc/evil",
        "Scripts\\..\\..\\evil.luau",
        "C:/Windows/evil.dll",
        "Scripts/./Main.luau",
    ] {
        write_zip(
            &path,
            &[
                (MANIFEST_FILE, &manifest()),
                (START, &scene()),
                (evil, b"print('pwned')"),
            ],
        );
        let error = open_error(&path);
        assert!(matches!(error, ProjectError::UnsafeMember { .. }), "{evil}: {error}");
        assert!(error.to_string().contains(&format!("`{evil}`")), "{error}");
    }

    // Nor can a start scene or an edit point outside the project.
    let mut bad = Manifest::new("Test");
    bad.start_scene = "../Main.scene".to_owned();
    write_zip(&path, &[(MANIFEST_FILE, bad.to_toml().as_bytes()), (START, &scene())]);
    assert!(matches!(open_error(&path), ProjectError::UnsafeMember { .. }));
    let mut project = Project::create(dir.join("Good.mtproj"), &template(), "Good").unwrap();
    assert!(matches!(
        project.write("../outside.luau", "x"),
        Err(ProjectError::UnsafeMember { .. })
    ));
    assert!(matches!(
        project.set_manifest(bad),
        Err(ProjectError::UnsafeMember { .. })
    ));
    assert!(matches!(
        project.write(MANIFEST_FILE, "x"),
        Err(ProjectError::ManifestMember { .. })
    ));
    assert!(!project.is_modified());
}

#[test]
fn folder_entries_are_allowed_and_skipped() {
    let dir = scratch("folders");
    let path = dir.join("Game.mtproj");
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    writer.add_directory("Scenes/", SimpleFileOptions::default()).unwrap();
    for (member, bytes) in [(MANIFEST_FILE, manifest()), (START, scene())] {
        writer.start_file(member, SimpleFileOptions::default()).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
    let project = Project::open(&path).unwrap();
    assert_eq!(project.members(), [START, MANIFEST_FILE]);
}

#[test]
fn member_paths_are_checked() {
    for good in [
        "metallic.toml",
        "Scenes/Main.scene",
        "Assets/With Spaces/a.png",
        "..a/b..",
        "日本語.luau",
    ] {
        assert_eq!(check_member_path(good), Ok(()), "{good}");
    }
    for bad in ["", "/a", "a\\b", "C:a", "a//b", "a/", "./a", "a/../b", "..", "a\0b"] {
        assert!(check_member_path(bad).is_err(), "{bad:?}");
    }
}
