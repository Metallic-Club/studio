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
