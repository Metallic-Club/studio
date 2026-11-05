//! The asset index of a `.mtproj` project: ids given, kept across moves and renames, missing assets reported, and
//! `assets.json` round-tripping through saved archives.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use metallic_project::manifest::MANIFEST_FILE;
use metallic_project::{
    AssetIndex, AssetIndexError, AssetMove, Fingerprint, INDEX_FILE, Manifest, Project, ProjectError, Template,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const START: &str = "Scenes/Main.scene";
const GRASS: &str = "Assets/grass.png";
const HELLO: &str = "Scripts/Hello.luau";
const ROCK: &str = "Assets/rock.glb";

/// A new project made from the Baseplate template, in an empty folder for one test.
fn new_project(test: &str) -> Project {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("asset_index").join(test);
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

/// Moves a member the way an editor would: read it, write it at its new path, remove the old one.
fn rename(project: &mut Project, from: &str, to: &str) {
    let bytes = project.read(from).expect("the member");
    project.write(to, bytes).expect("written");
    assert!(project.remove(from));
}

#[test]
fn ids_are_given_and_the_index_round_trips_through_a_saved_project() {
    let mut project = new_project("round_trip");
    assert!(project.assets().is_empty(), "a new project has no index yet");

    project.write(GRASS, b"\x89PNG grass".to_vec()).unwrap();
    project.write(HELLO, "print('hello')").unwrap();
    let report = project.refresh_assets().unwrap();
    let added: Vec<&str> = report.added.iter().map(|asset| asset.path.as_str()).collect();
    assert_eq!(added, [GRASS, START, HELLO]);
    assert!(report.moved.is_empty() && report.missing.is_empty());
    assert!(project.contains(INDEX_FILE));
    assert_eq!(
        project.assets().id(MANIFEST_FILE),
        None,
        "metallic.toml is not an asset"
    );
    let index = project.assets().clone();

    let mut project = save_and_reopen(project);
    assert_eq!(*project.assets(), index, "the ids are read back on open");
    let bytes = project.read(INDEX_FILE).unwrap();
    assert_eq!(
        bytes,
        index.to_json().into_bytes(),
        "assets.json is the index's own text"
    );
    assert_eq!(AssetIndex::from_bytes(&bytes).unwrap().to_json().into_bytes(), bytes);

    // Nothing changed, so refreshing changes nothing and leaves nothing to save.
    let report = project.refresh_assets().unwrap();
    assert!(report.added.is_empty() && report.moved.is_empty() && report.missing.is_empty());
    assert!(!project.is_modified());
}

#[test]
fn saved_members_are_fingerprinted_from_the_archive_like_unsaved_ones() {
    let mut project = new_project("fingerprints");
    let png = b"\x89PNG stored, not deflated".to_vec();
    let script = "print('deflated')".repeat(100);
    project.write(GRASS, png.clone()).unwrap();
    project.write(HELLO, script.clone()).unwrap();
    assert_eq!(project.fingerprint(GRASS).unwrap(), Fingerprint::of(&png));
    let project = save_and_reopen(project);
    assert_eq!(project.fingerprint(GRASS).unwrap(), Fingerprint::of(&png));
    assert_eq!(project.fingerprint(HELLO).unwrap(), Fingerprint::of(script.as_bytes()));
    assert!(matches!(
        project.fingerprint("Assets/none.png"),
        Err(ProjectError::MissingMember { .. })
    ));
}

#[test]
fn moves_renames_additions_and_deletions_between_runs_keep_ids_and_report_missing_assets() {
    let mut project = new_project("moves");
    project.write(GRASS, b"\x89PNG grass".to_vec()).unwrap();
    project.write(HELLO, "print('hello')").unwrap();
    project.write(ROCK, b"glTF rock".to_vec()).unwrap();
    project.refresh_assets().unwrap();
    let mut project = save_and_reopen(project);
    let id = |project: &Project, path: &str| project.assets().id(path).expect("indexed");
    let (grass, hello, rock, scene) = (
        id(&project, GRASS),
        id(&project, HELLO),
        id(&project, ROCK),
        id(&project, START),
    );

    // Between runs: move the texture into a folder, rename the script, delete the model, add a sound.
    rename(&mut project, GRASS, "Assets/Textures/grass.png");
    rename(&mut project, HELLO, "Scripts/Greeting.luau");
    assert!(project.remove(ROCK));
    project.write("Assets/wind.ogg", b"OggS wind".to_vec()).unwrap();
    let report = project.refresh_assets().unwrap();

    assert_eq!(
        report.moved,
        [
            AssetMove {
                id: grass,
                from: GRASS.into(),
                to: "Assets/Textures/grass.png".into()
            },
            AssetMove {
                id: hello,
                from: HELLO.into(),
                to: "Scripts/Greeting.luau".into()
            },
        ]
    );
    let added: Vec<&str> = report.added.iter().map(|asset| asset.path.as_str()).collect();
    assert_eq!(added, ["Assets/wind.ogg"]);
    assert_eq!(report.missing.len(), 1);
    assert_eq!((report.missing[0].id, report.missing[0].path.as_str()), (rock, ROCK));

    // The ids survive a save, and the missing model is still reported on the next run.
    let mut project = save_and_reopen(project);
    assert_eq!(id(&project, "Assets/Textures/grass.png"), grass);
    assert_eq!(id(&project, "Scripts/Greeting.luau"), hello);
    assert_eq!(id(&project, START), scene);
    assert_eq!(project.assets().path(rock), Some(ROCK));
    let report = project.refresh_assets().unwrap();
    assert_eq!(report.missing.len(), 1);
    assert_eq!(report.missing[0].id, rock);

    // Forgetting it removes it for good.
    assert_eq!(project.forget_asset(rock).as_deref(), Some(ROCK));
    let mut project = save_and_reopen(project);
    assert_eq!(project.assets().path(rock), None);
    assert!(project.refresh_assets().unwrap().missing.is_empty());
}

#[test]
fn assets_json_is_changed_only_through_the_index() {
    let mut project = new_project("guarded");
    project.refresh_assets().unwrap();
    assert!(matches!(
        project.write(INDEX_FILE, "{}"),
        Err(ProjectError::IndexMember { .. })
    ));
    assert!(!project.remove(INDEX_FILE));
}

#[test]
fn a_project_whose_assets_json_is_refused_does_not_open() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("asset_index")
        .join("refused");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Bad.mtproj");
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    let manifest = Manifest::new("Bad").to_toml();
    let members: [(&str, &[u8]); 3] = [
        (MANIFEST_FILE, manifest.as_bytes()),
        (START, b"{}"),
        (INDEX_FILE, br#"{"format": 99, "assets": {}}"#),
    ];
    for (member, bytes) in members {
        writer.start_file(member, SimpleFileOptions::default()).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    let error = Project::open(&path).unwrap_err();
    assert!(
        matches!(
            error,
            ProjectError::AssetIndex {
                source: AssetIndexError::UnsupportedFormat { found: 99 },
                ..
            }
        ),
        "{error}"
    );
}
