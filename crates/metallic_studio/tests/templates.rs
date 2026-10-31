//! Template listing and project creation, against temporary template folders.

use std::fs;
use std::path::Path;

use metallic_studio::StudioError;
use metallic_studio::templates::{create_from_template, list_templates};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// Makes a template folder `<root>/<folder>` with a valid `template.toml`.
fn make_template(root: &Path, folder: &str, id: &str, order: i64) -> std::path::PathBuf {
    let dir = root.join(folder);
    write(
        &dir.join("template.toml"),
        &format!(
            "id = \"{id}\"\nname = \"Name {id}\"\ndescription = \"About {id}\"\npreview = \"preview.png\"\norder = {order}\n"
        ),
    );
    write(
        &dir.join("metallic.toml"),
        "name = \"Template\"\nversion = 1\n",
    );
    write(&dir.join("scenes/main.scene"), "scene data");
    dir
}

#[test]
fn lists_valid_templates_in_order_skipping_broken() {
    let root = tempfile::tempdir().expect("tmp");
    make_template(root.path(), "b", "second", 20);
    make_template(root.path(), "a", "first", 10);
    // Missing key (no `order`).
    write(
        &root.path().join("c/template.toml"),
        "id = \"c\"\nname = \"C\"\ndescription = \"d\"\npreview = \"p.png\"\n",
    );
    // Not TOML at all.
    write(&root.path().join("d/template.toml"), "this is [[ not toml");
    // A folder without template.toml and a loose file.
    fs::create_dir_all(root.path().join("e")).expect("mkdir");
    write(&root.path().join("loose.txt"), "x");

    let found = list_templates(root.path());
    let ids: Vec<&str> = found.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["first", "second"]);
    assert_eq!(found[0].name, "Name first");
    assert_eq!(found[0].description, "About first");
    assert_eq!(
        found[0].preview_path,
        root.path().join("a").join("preview.png")
    );
    assert_eq!(found[0].dir, root.path().join("a"));
    assert_eq!(found[0].order, 10);
}

#[test]
fn listing_a_missing_folder_is_empty() {
    assert!(list_templates(Path::new("/definitely/not/here")).is_empty());
}

#[test]
fn creates_into_unicode_path_with_spaces() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "t", "parkour", 1);
    let info = list_templates(root.path()).remove(0);
    assert_eq!(info.dir, dir);

    let target = root.path().join("Mes Jeux \u{d1}").join("Course");
    create_from_template(&info, &target, "Course").expect("create");
    assert_eq!(
        fs::read_to_string(target.join("scenes/main.scene")).expect("scene"),
        "scene data"
    );
    assert!(target.join("metallic.toml").is_file());
    assert!(
        !target.join("template.toml").exists(),
        "template.toml must not be copied"
    );
}

#[test]
fn refuses_non_empty_folder_and_writes_nothing() {
    let root = tempfile::tempdir().expect("tmp");
    make_template(root.path(), "t", "parkour", 1);
    let info = list_templates(root.path()).remove(0);

    let target = root.path().join("busy");
    write(&target.join("keep.txt"), "mine");
    let err = create_from_template(&info, &target, "Busy").expect_err("must refuse");
    assert!(matches!(err, StudioError::FolderNotEmpty(_)));
    assert!(
        err.to_string()
            .contains("This folder isn't empty. Choose an empty folder or a new one.")
    );
    let names: Vec<_> = fs::read_dir(&target)
        .expect("dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["keep.txt"]);
    assert_eq!(
        fs::read_to_string(target.join("keep.txt")).expect("read"),
        "mine"
    );
}

#[test]
fn accepts_an_existing_empty_folder() {
    let root = tempfile::tempdir().expect("tmp");
    make_template(root.path(), "t", "parkour", 1);
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("empty");
    fs::create_dir_all(&target).expect("mkdir");
    create_from_template(&info, &target, "Empty").expect("create");
    assert!(target.join("metallic.toml").is_file());
}

#[test]
fn sets_project_name() {
    let root = tempfile::tempdir().expect("tmp");
    make_template(root.path(), "t", "parkour", 1);
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "My 'Quoted' Game # \u{d1}").expect("create");
    let table: toml::Table = fs::read_to_string(target.join("metallic.toml"))
        .expect("read")
        .parse()
        .expect("toml");
    assert_eq!(table["name"].as_str(), Some("My 'Quoted' Game # \u{d1}"));
    assert_eq!(
        table["version"].as_integer(),
        Some(1),
        "other keys are kept"
    );
}

#[test]
fn creates_metallic_toml_when_template_has_none() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "t", "parkour", 1);
    fs::remove_file(dir.join("metallic.toml")).expect("rm");
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Fresh").expect("create");
    let table: toml::Table = fs::read_to_string(target.join("metallic.toml"))
        .expect("read")
        .parse()
        .expect("toml");
    assert_eq!(table["name"].as_str(), Some("Fresh"));
}

#[test]
fn rejects_bad_project_names() {
    let root = tempfile::tempdir().expect("tmp");
    make_template(root.path(), "t", "parkour", 1);
    let info = list_templates(root.path()).remove(0);
    for (i, bad) in [
        "..\\x",
        "a\"b",
        "CON",
        "nul",
        "trailing.",
        "",
        "  ",
        "com1",
        "LPT9",
        "Aux.txt",
        "prn.",
    ]
    .iter()
    .enumerate()
    {
        let target = root.path().join(format!("out{i}"));
        let err = create_from_template(&info, &target, bad).expect_err(bad);
        assert!(matches!(err, StudioError::InvalidName(_)), "{bad:?}: {err}");
        assert!(!target.exists(), "{bad:?} must write nothing");
    }
    for ok in ["CONSOLE", "Com10", "My Game", "a.b"] {
        metallic_studio::templates::validate_project_name(ok).expect(ok);
    }
    let msg = metallic_studio::templates::validate_project_name("CON")
        .expect_err("CON")
        .to_string();
    assert!(msg.contains("reserved"), "{msg}");
}

#[test]
fn failed_copy_leaves_no_target() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "t", "parkour", 1);
    // A broken metallic.toml makes the copy succeed but the name step fail.
    write(&dir.join("metallic.toml"), "this is = = not toml");
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Game").expect_err("must fail");
    assert!(!target.exists(), "folder this call created must be gone");

    // An existing empty folder is kept, but emptied.
    fs::create_dir_all(&target).expect("mkdir");
    create_from_template(&info, &target, "Game").expect_err("must fail");
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(&target).expect("read").count(), 0);
}

const SETTINGS: &str = r#"
[[setting]]
attribute = "WalkSpeed"
label = "Walking speed"
group = "Movement"
kind = "number"
min = 8
max = 24
step = 1
help = "How quickly the character walks."

[[setting]]
attribute = "RoundMode"
label = "Round mode"
group = "Rules"
kind = "choice"
choices = ["practice", "timed"]
help = "Choose a practice course or a timed round."
"#;

fn read_table(path: &Path) -> toml::Table {
    fs::read_to_string(path)
        .expect("read TOML")
        .parse()
        .expect("valid TOML")
}

#[test]
fn copied_project_settings_and_other_metadata_survive_project_rename() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    write(
        &dir.join("metallic.toml"),
        &format!(
            "name = \"Original\"\nmain_scene = \"main.mscene\"\ndimension = \"3D\"\n{SETTINGS}\n[custom]\nvalues = [1, 2, 3]\n"
        ),
    );
    let mut expected = read_table(&dir.join("metallic.toml"));
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("Creator Games Ñ").join("Remix");
    create_from_template(&info, &target, "  My Remix  ").expect("create");
    let mut actual = read_table(&target.join("metallic.toml"));
    assert_eq!(
        actual
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned)),
        Some("My Remix".to_owned())
    );
    expected.remove("name");
    assert_eq!(
        actual, expected,
        "all settings, ranges, help and extension keys must survive"
    );
    let config =
        metallic_runtime::ProjectConfig::read(&target).expect("runtime accepts project metadata");
    assert_eq!(config.name, "My Remix");
    assert_eq!(config.main_scene, Path::new("main.mscene"));
}

#[test]
fn legacy_template_settings_are_migrated_to_the_project_file_without_source_changes() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let original_metadata = format!(
        "{}{SETTINGS}",
        fs::read_to_string(dir.join("template.toml")).expect("metadata")
    );
    write(&dir.join("template.toml"), &original_metadata);
    write(
        &dir.join("metallic.toml"),
        "name = \"Original\"\nmain_scene = \"main.mscene\"\n",
    );
    let expected = read_table(&dir.join("template.toml"))["setting"].clone();
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Legacy Remix").expect("create");
    let project = read_table(&target.join("metallic.toml"));
    assert_eq!(project.get("setting"), Some(&expected));
    assert!(!target.join("template.toml").exists());
    assert_eq!(
        fs::read_to_string(dir.join("template.toml")).expect("source"),
        original_metadata
    );
    assert_eq!(
        metallic_runtime::ProjectConfig::read(&target)
            .expect("runtime config")
            .name,
        "Legacy Remix"
    );
}

#[test]
fn conflicting_legacy_and_project_settings_fail_without_leaving_a_partial_project() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let metadata = fs::read_to_string(dir.join("template.toml")).expect("metadata");
    write(&dir.join("template.toml"), &format!("{metadata}{SETTINGS}"));
    write(
        &dir.join("metallic.toml"),
        &format!(
            "name = \"Original\"\nmain_scene = \"main.mscene\"\n{}",
            SETTINGS.replace("max = 24", "max = 100")
        ),
    );
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    let error = create_from_template(&info, &target, "Conflict")
        .expect_err("ambiguous settings must not be discarded");
    assert!(matches!(error, StudioError::Metadata { .. }));
    assert!(error.to_string().contains("setting"));
    assert!(!target.exists());
    fs::create_dir(&target).expect("empty folder");
    create_from_template(&info, &target, "Conflict").expect_err("same conflict");
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(&target).expect("read target").count(), 0);
}

#[test]
fn identical_legacy_and_project_settings_are_not_duplicated() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let metadata = fs::read_to_string(dir.join("template.toml")).expect("metadata");
    write(&dir.join("template.toml"), &format!("{metadata}{SETTINGS}"));
    write(
        &dir.join("metallic.toml"),
        &format!("name = \"Original\"\nmain_scene = \"main.mscene\"\n{SETTINGS}"),
    );
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Remix").expect("matching settings");
    assert_eq!(
        read_table(&target.join("metallic.toml"))["setting"]
            .as_array()
            .expect("settings")
            .len(),
        2
    );
}

#[test]
fn template_previews_and_root_tests_are_excluded_but_project_content_is_kept() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let mut metadata = read_table(&dir.join("template.toml"));
    metadata.insert(
        "preview".into(),
        toml::Value::String("cards/../cards/card.png".into()),
    );
    metadata.insert(
        "preview_video".into(),
        toml::Value::String("cards/card.webm".into()),
    );
    write(
        &dir.join("template.toml"),
        &toml::to_string(&metadata).expect("metadata"),
    );
    for relative in [
        "preview.png",
        "preview.webm",
        "tests/smoke.mscene",
        "tests/fixtures/data.txt",
        "cards/card.png",
        "cards/card.webm",
    ] {
        write(&dir.join(relative), "template-only fixture");
    }
    for relative in [
        "tutorial.toml",
        "assets/preview.png",
        "assets/template.toml",
        "assets/tests/data.txt",
        "scripts/game.luau",
    ] {
        write(&dir.join(relative), "creator project content");
    }
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Remix").expect("create");
    for relative in [
        "template.toml",
        "preview.png",
        "preview.webm",
        "tests",
        "cards/card.png",
        "cards/card.webm",
    ] {
        assert!(
            !target.join(relative).exists(),
            "template-only {relative} was copied"
        );
    }
    for relative in [
        "tutorial.toml",
        "assets/preview.png",
        "assets/template.toml",
        "assets/tests/data.txt",
        "scripts/game.luau",
        "scenes/main.scene",
    ] {
        assert_eq!(
            fs::read(target.join(relative)).expect(relative),
            fs::read(dir.join(relative)).expect(relative)
        );
    }
}

#[test]
fn an_incorrect_preview_reference_cannot_discard_the_project_file() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let mut metadata = read_table(&dir.join("template.toml"));
    metadata.insert(
        "preview".into(),
        toml::Value::String("metallic.toml".into()),
    );
    write(
        &dir.join("template.toml"),
        &toml::to_string(&metadata).expect("metadata"),
    );
    write(
        &dir.join("metallic.toml"),
        &format!("name = \"Original\"\nmain_scene = \"main.mscene\"\n{SETTINGS}"),
    );
    let settings = read_table(&dir.join("metallic.toml"))["setting"].clone();
    let info = list_templates(root.path()).remove(0);
    let target = root.path().join("out");
    create_from_template(&info, &target, "Remix").expect("create");
    assert_eq!(
        read_table(&target.join("metallic.toml"))["setting"],
        settings
    );
    assert_eq!(
        metallic_runtime::ProjectConfig::read(&target)
            .expect("runtime config")
            .main_scene,
        Path::new("main.mscene")
    );
}

#[test]
fn the_shipped_parkour_template_creates_a_loadable_game() {
    let templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let info = list_templates(&templates)
        .into_iter()
        .find(|template| template.id == "parkour")
        .expect("shipped Parkour template");
    let root = tempfile::tempdir().expect("tmp");
    let target = root.path().join("Created Parkour");
    create_from_template(&info, &target, "My Parkour").expect("create shipped project");
    let config = metallic_runtime::ProjectConfig::read(&target).expect("project config");
    assert_eq!(config.name, "My Parkour");
    assert_eq!(
        fs::read(target.join(&config.main_scene)).expect("copied scene"),
        fs::read(info.dir.join(&config.main_scene)).expect("source scene")
    );
    metallic_runtime::Game::load(&target)
        .expect("created game loads without template catalog files");
    for relative in ["template.toml", "preview.png", "preview.webm", "tests"] {
        assert!(!target.join(relative).exists(), "excluded {relative}");
    }
}

#[test]
fn direct_template_info_without_catalog_still_creates_a_project() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = make_template(root.path(), "source", "parkour", 1);
    let mut info = list_templates(root.path()).remove(0);
    fs::remove_file(dir.join("template.toml")).expect("remove catalog");
    info.preview_path = dir.join("card.png");
    write(&info.preview_path, "catalog image");
    write(
        &dir.join("metallic.toml"),
        &format!("name = \"Original\"\n{SETTINGS}"),
    );
    let expected = read_table(&dir.join("metallic.toml"))["setting"].clone();
    let target = root.path().join("out");
    create_from_template(&info, &target, "Direct")
        .expect("catalog is optional for direct TemplateInfo");
    assert!(!target.join("card.png").exists());
    assert_eq!(
        read_table(&target.join("metallic.toml"))["setting"],
        expected
    );
}
