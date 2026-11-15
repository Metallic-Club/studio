//! The Assets browser (#676): navigating by the tree, the tiles, the breadcrumbs and Back; the search; and creating,
//! renaming, duplicating, moving by dragging and deleting files and folders, with asset ids kept and paths that name
//! a moved file rewritten, in the scene being edited too.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::Value;

use super::*;

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
}

/// A project with a picture in `Assets/Trees` and a script, the Assets tab in front.
fn browsing(test: &str) -> Harness<'static, Test> {
    let mut harness = editing(settings_file(test, None), &format!("Browse {test}"));
    harness.set_size(egui::vec2(1280.0, 1000.0));
    let editor = editor(&mut harness);
    if !editor.project.contains("Assets/Trees/oak.png") {
        editor
            .project
            .create_member("Assets/Trees/oak.png", vec![7; 300])
            .unwrap();
        editor
            .project
            .create_member("Scripts/Door.server.luau", b"print('door')".to_vec())
            .unwrap();
    }
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);
    harness
}

/// Double-clicks the tile `name`, the clicks timed as `double_click`'s are (#662).
fn double_click_tile(harness: &mut Harness<'_, Test>, name: &str) {
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::GridCell, name).click();
    harness.step();
    harness.input_mut().time = Some(time + 0.05);
    harness.get_by_role_and_label(Role::GridCell, name).click();
    settle(harness);
    harness.input_mut().time = None;
}

fn tile_exists(harness: &Harness<'_, Test>, name: &str) -> bool {
    harness.query_by_role_and_label(Role::GridCell, name).is_some()
}

#[test]
fn folders_open_from_the_tree_the_tiles_the_breadcrumbs_and_back() {
    let mut harness = browsing("assets-navigate");
    // The root: the project's folders, as tiles and in the tree.
    for name in ["Assets", "Scenes", "Scripts"] {
        assert!(tile_exists(&harness, name), "{name}");
        harness.get_by_role_and_label(Role::TreeItem, name);
    }
    harness.get_by_role_and_label(Role::TreeItem, "Project");

    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    assert!(tile_exists(&harness, "Trees") && !tile_exists(&harness, "Scenes"));
    assert_eq!(editor(&mut harness).browser.open, "Assets");
    harness.get_by_role_and_label(Role::Link, "Open Assets");

    double_click_tile(&mut harness, "Trees");
    assert!(
        tile_exists(&harness, "oak"),
        "a file's tile is its name without the extension"
    );
    harness.get_by_role_and_label(Role::Link, "Open Trees");
    // The tree opened down to it.
    harness.get_by_role_and_label(Role::TreeItem, "oak.png");

    harness.get_by_role_and_label(Role::Button, "Back").click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).browser.open, "Assets");
    harness.get_by_role_and_label(Role::Link, "Open Project").click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).browser.open, "");
    assert!(tile_exists(&harness, "Scenes"));

    // A file's row in the tree shows it in its folder, selected.
    harness.get_by_role_and_label(Role::TreeItem, "oak.png").click();
    settle(&mut harness);
    let browser = &editor(&mut harness).browser;
    assert_eq!(browser.open, "Assets/Trees");
    assert!(browser.selected.contains("Assets/Trees/oak.png"));
}

#[test]
fn the_search_finds_files_anywhere_by_name() {
    let mut harness = browsing("assets-search");
    harness
        .get_by_role_and_label(Role::TextInput, "Search the assets")
        .click();
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TextInput, "Search the assets")
        .type_text("OAK");
    settle(&mut harness);
    assert!(tile_exists(&harness, "oak"));
    assert!(!tile_exists(&harness, "Assets") && !tile_exists(&harness, "Door"));
    editor(&mut harness).browser.query = "zebra".into();
    settle(&mut harness);
    harness.get_by_label("Nothing is called \u{201C}zebra\u{201D}.");
}

#[test]
fn a_new_folder_is_named_in_place_and_files_are_renamed_duplicated_and_moved() {
    let mut harness = browsing("assets-edit");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);

    // New Folder from the empty space's menu, then named where it is.
    harness.get_by_role_and_label(Role::Grid, "Assets").click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "New Folder").click();
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TextInput, "Name of New Folder")
        .type_text("Rocks");
    harness.key_press(Key::Enter);
    settle(&mut harness);
    assert!(editor(&mut harness).project.has_folder("Assets/Rocks"));
    assert!(tile_exists(&harness, "Rocks"));

    // F2 renames the selected file; its extension stays.
    double_click_tile(&mut harness, "Trees");
    let id = editor(&mut harness)
        .project
        .assets()
        .id("Assets/Trees/oak.png")
        .unwrap();
    harness.get_by_role_and_label(Role::GridCell, "oak").click();
    settle(&mut harness);
    harness.key_press(Key::F2);
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TextInput, "Name of oak")
        .type_text("birch");
    harness.key_press(Key::Enter);
    settle(&mut harness);
    let project = &editor(&mut harness).project;
    assert!(project.contains("Assets/Trees/birch.png") && !project.contains("Assets/Trees/oak.png"));
    assert_eq!(
        project.assets().id("Assets/Trees/birch.png"),
        Some(id),
        "the id is kept"
    );

    // Ctrl+D copies it beside itself, as a new asset.
    harness.get_by_role_and_label(Role::GridCell, "birch").click();
    settle(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::D);
    settle(&mut harness);
    let project = &editor(&mut harness).project;
    let copy = project.assets().id("Assets/Trees/birch 2.png").expect("the copy");
    assert_ne!(copy, id);

    // Dragging a tile onto a folder's breadcrumb moves it there.
    let from = harness.get_by_role_and_label(Role::GridCell, "birch 2").rect().center();
    let to = harness.get_by_role_and_label(Role::Link, "Open Assets").rect().center();
    drag(&mut harness, from, to);
    let project = &editor(&mut harness).project;
    assert!(project.contains("Assets/birch 2.png") && !project.contains("Assets/Trees/birch 2.png"));
    assert_eq!(project.assets().id("Assets/birch 2.png"), Some(copy));

    // And onto a folder's tile.
    harness.get_by_role_and_label(Role::Link, "Open Assets").click();
    settle(&mut harness);
    let from = harness.get_by_role_and_label(Role::GridCell, "birch 2").rect().center();
    let to = harness.get_by_role_and_label(Role::GridCell, "Rocks").rect().center();
    drag(&mut harness, from, to);
    assert!(editor(&mut harness).project.contains("Assets/Rocks/birch 2.png"));
}

#[test]
fn moving_a_script_rewrites_the_source_of_the_scene_being_edited() {
    let mut harness = browsing("assets-script-source");
    let editor_state = editor(&mut harness);
    let workspace = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    let script = editor_state
        .dm
        .create(editor_state.dm.registry().class_id("Script").unwrap())
        .unwrap();
    editor_state.dm.set_parent(script, Some(workspace)).unwrap();
    editor_state
        .dm
        .set_by_name(script, "Source", Value::String("Scripts/Door.server.luau".into()))
        .unwrap();
    // Its text, open in the script editor, goes with it.
    let text = editor_state
        .scripts
        .text(&mut editor_state.project, "Scripts/Door.server.luau")
        .clone();
    assert_eq!(text, "print('door')");

    // Dragged onto the Assets folder in the tree.
    harness.get_by_role_and_label(Role::TreeItem, "Scripts").click();
    settle(&mut harness);
    let from = harness.get_by_role_and_label(Role::GridCell, "Door").rect().center();
    let to = harness.get_by_role_and_label(Role::TreeItem, "Assets").rect().center();
    drag(&mut harness, from, to);
    let editor_state = editor(&mut harness);
    assert!(editor_state.project.contains("Assets/Door.server.luau"));
    assert_eq!(
        editor_state.dm.get_by_name(script, "Source"),
        Ok(Value::String("Assets/Door.server.luau".into()))
    );
    let text = editor_state
        .scripts
        .text(&mut editor_state.project, "Assets/Door.server.luau")
        .clone();
    assert_eq!(text, "print('door')");

    // Saved, the scene keeps the new path.
    editor_state.save().unwrap();
    let path = editor_state.project.path().to_path_buf();
    let mut project = metallic_engine::metallic_project::Project::open(&path).unwrap();
    let scene = project.read_text("Scenes/Main.scene").unwrap();
    assert!(scene.contains("Assets/Door.server.luau"), "{scene}");
}

#[test]
fn delete_asks_first_and_says_what_uses_the_file() {
    let mut harness = browsing("assets-delete");
    let editor_state = editor(&mut harness);
    // A mesh part that uses the picture as its texture, saved so the scene file names it.
    let id = editor_state.project.assets().id("Assets/Trees/oak.png").unwrap();
    let workspace = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    let part = editor_state
        .dm
        .create(editor_state.dm.registry().class_id("MeshPart").unwrap())
        .unwrap();
    editor_state.dm.set_parent(part, Some(workspace)).unwrap();
    editor_state
        .dm
        .set_by_name(part, "TextureId", Value::String(id.to_string().into()))
        .unwrap();
    editor_state.save().unwrap();

    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Trees").click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Delete").click();
    settle(&mut harness);
    harness.get_by_label("Delete Trees?");
    harness.get_by_label("1 file goes. It leaves the project's file when you save.");
    harness.get_by_label("Scenes/Main.scene: TextureId");
    harness.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut harness);
    assert!(editor(&mut harness).project.contains("Assets/Trees/oak.png"));

    harness.get_by_role_and_label(Role::GridCell, "Trees").click();
    settle(&mut harness);
    harness.key_press(Key::Delete);
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Delete").click();
    settle(&mut harness);
    let project = &editor(&mut harness).project;
    assert!(!project.has_folder("Assets/Trees") && !project.contains("Assets/Trees/oak.png"));
    // Still indexed, as missing: the scene refers to it.
    assert_eq!(project.assets().id("Assets/Trees/oak.png"), Some(id));
    assert!(!tile_exists(&harness, "Trees"));

    // The scene being edited cannot be deleted.
    harness.get_by_role_and_label(Role::TreeItem, "Scenes").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Main").click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Delete").click();
    settle(&mut harness);
    assert!(harness.query_by_label("Delete Main.scene?").is_none());
    harness.get_by_label("Scenes/Main.scene is the scene being edited, so it cannot be deleted.");
}

#[test]
#[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
fn screenshot_of_the_assets_browser() {
    use super::offscreen::Offscreen;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let path = config("shot-assets", Some("[general]\nskip_sign_in = true\n"), None);
    let project = make_project_once(path.parent().unwrap(), "Forest");
    let mut studio = studio_after_startup(steps_reading(path));
    let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.0).expect("a GPU");
    studio.open(project);
    for _ in 0..10_000 {
        offscreen.frame(&mut studio);
        if !studio.is_opening() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let editor = studio.editor_mut().unwrap();
    for (member, bytes) in [
        ("Assets/Trees/oak.png", vec![1; 900]),
        ("Assets/Trees/pine.png", vec![2; 900]),
        ("Assets/Trees/Pine bark.jpg", vec![3; 900]),
        ("Assets/Rock.glb", vec![4; 900]),
        ("Assets/Birdsong.ogg", vec![5; 900]),
        ("Scripts/Door.server.luau", b"print('door')".to_vec()),
    ] {
        if !editor.project.contains(member) {
            editor.project.create_member(member, bytes).unwrap();
        }
    }
    let _ = editor.project.create_folder("Assets/Trees/Saplings");
    editor.browser.open_folder("Assets");
    editor.browser.open_folder("Assets/Trees");
    editor.browser.selected.insert("Assets/Trees/oak.png".into());
    studio.dock_mut().open(crate::panels::ids::ASSETS);
    for _ in 0..30 {
        offscreen.frame(&mut studio);
    }
    write_png(&out.join("676-assets-browser.png"), offscreen.read());
}
