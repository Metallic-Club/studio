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

/// Chooses `item` from the New submenu of the right-click menu open now (#702): the pointer over New opens it to the side.
fn choose_new(harness: &mut Harness<'_, Test>, item: &str) {
    let new = harness.get_by_role_and_label(Role::MenuItem, "New").rect();
    harness.event(egui::Event::PointerMoved(new.center()));
    settle(harness);
    harness.get_by_role_and_label(Role::MenuItem, item).click();
    settle(harness);
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
fn the_tree_shows_a_picture_s_thumbnail() {
    let mut harness = browsing("assets-tree-thumbnails");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    double_click_tile(&mut harness, "Trees");
    harness.get_by_role_and_label(Role::Button, "Back").click();
    settle(&mut harness);
    // Assets is open, so no tile shows oak.png; its row in the tree does, and asks for its thumbnail (#756).
    assert!(!tile_exists(&harness, "oak"));
    harness.get_by_role_and_label(Role::TreeItem, "oak.png");
    let editor_state = editor(&mut harness);
    let oak = editor_state.project.assets().id("Assets/Trees/oak.png").unwrap();
    editor_state.thumbnails.clear();
    until_thumbnail(&mut harness, oak);
    assert!(editor(&mut harness).thumbnails.contains_key(&oak));
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
    ground_menu(&mut harness);
    settle(&mut harness);
    choose_new(&mut harness, "Folder");
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

/// #677: a script file's type is in its name; the browser makes each type, and opening one opens the tab of the script
/// that runs it, making one where its type runs when none does yet.
#[test]
fn script_files_of_each_type_are_made_opened_and_dragged_into_the_explorer() {
    let mut harness = browsing("assets-scripts");
    harness.get_by_role_and_label(Role::TreeItem, "Scripts").click();
    settle(&mut harness);
    for (item, file) in [
        ("Script", "Scripts/Script.server.luau"),
        ("LocalScript", "Scripts/LocalScript.client.luau"),
        ("ModuleScript", "Scripts/ModuleScript.luau"),
    ] {
        ground_menu(&mut harness);
        settle(&mut harness);
        choose_new(&mut harness, item);
        // Named in place at once; Escape keeps the name it was given.
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(editor(&mut harness).project.contains(file), "{file}");
    }
    let module = editor(&mut harness)
        .project
        .read_text("Scripts/ModuleScript.luau")
        .unwrap();
    assert_eq!(module, "local module = {}\n\nreturn module\n");

    // Door.server.luau runs nowhere yet: opening it adds a Script for it to ServerScriptService, and opens its tab.
    double_click_tile(&mut harness, "Door");
    harness.get_by_role_and_label(Role::Tab, "Door");
    let editor_state = editor(&mut harness);
    let service = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("ServerScriptService").unwrap())
        .unwrap();
    let door = editor_state
        .dm
        .find_first_child(service, "Door", false)
        .unwrap()
        .expect("a Script for it");
    assert_eq!(editor_state.dm.class_name(door).unwrap(), "Script");
    assert_eq!(crate::code::source(&editor_state.dm, door), "Scripts/Door.server.luau");
    // Opened again, it is the same script.
    double_click_tile(&mut harness, "Door");
    let editor_state = editor(&mut harness);
    let doors = editor_state
        .dm
        .children(service)
        .unwrap()
        .filter(|&id| editor_state.dm.name(id).unwrap() == "Door")
        .count();
    assert_eq!(doors, 1);

    // Renaming keeps its type, and its script follows.
    harness.get_by_role_and_label(Role::GridCell, "Door").click();
    settle(&mut harness);
    editor(&mut harness).browser.renaming = Some("Scripts/Door.server.luau".into());
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TextInput, "Name of Door")
        .type_text("Gate");
    harness.key_press(Key::Enter);
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    assert!(editor_state.project.contains("Scripts/Gate.server.luau"));
    assert_eq!(crate::code::source(&editor_state.dm, door), "Scripts/Gate.server.luau");

    // A LocalScript file dragged onto the Workspace's row is a LocalScript there.
    let from = harness
        .get_by_role_and_label(Role::GridCell, "LocalScript")
        .rect()
        .center();
    let to = harness
        .get_by_role_and_label(Role::TreeItem, "Workspace")
        .rect()
        .center();
    drag(&mut harness, from, to);
    let editor_state = editor(&mut harness);
    let workspace = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    let local = editor_state
        .dm
        .find_first_child(workspace, "LocalScript", false)
        .unwrap()
        .expect("a LocalScript in the Workspace");
    assert_eq!(editor_state.dm.class_name(local).unwrap(), "LocalScript");
    assert_eq!(
        crate::code::source(&editor_state.dm, local),
        "Scripts/LocalScript.client.luau"
    );
    // One undo step takes it out again.
    harness.state_mut().studio.menu_action(MenuAction::Undo);
    settle(&mut harness);
    assert!(!crate::code::in_game(&editor(&mut harness).dm, local));
}

/// #701: a script file takes the type of the script that runs it, so a Script saved as `Name.luau` (before #677) is a
/// Script in the browser, not a ModuleScript; a file no script runs keeps the type its name gives.
#[test]
fn script_files_take_the_type_of_the_script_that_runs_them() {
    use crate::editor::assets::EntryKind;
    use crate::panels::assets::{AssetKind, ScriptType};
    let mut harness = browsing("assets-script-types");
    let editor_state = editor(&mut harness);
    for member in ["Scripts/Old.luau", "Scripts/Hud.luau", "Scripts/Util.luau"] {
        editor_state
            .project
            .create_member(member, b"print(1)".to_vec())
            .unwrap();
    }
    let workspace = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    for (class, member) in [("Script", "Scripts/Old.luau"), ("LocalScript", "Scripts/Hud.luau")] {
        let id = editor_state
            .dm
            .create(editor_state.dm.registry().class_id(class).unwrap())
            .unwrap();
        editor_state
            .dm
            .set_by_name(id, "Source", Value::String(member.into()))
            .unwrap();
        editor_state.dm.set_parent(id, Some(workspace)).unwrap();
    }
    let kind = |entries: &[crate::editor::assets::Entry], path: &str| {
        entries.iter().find(|entry| entry.path == path).map(|entry| entry.kind)
    };
    let shown = editor_state.asset_children("Scripts");
    let script = |script| Some(EntryKind::File(AssetKind::Script(script)));
    assert_eq!(kind(&shown, "Scripts/Old.luau"), script(ScriptType::Server));
    assert_eq!(kind(&shown, "Scripts/Hud.luau"), script(ScriptType::Local));
    assert_eq!(kind(&shown, "Scripts/Util.luau"), script(ScriptType::Module));
    assert_eq!(kind(&shown, "Scripts/Door.server.luau"), script(ScriptType::Server));
    let found = editor_state.asset_search("Old");
    assert_eq!(kind(&found, "Scripts/Old.luau"), script(ScriptType::Server));
    assert_eq!(editor_state.script_type("Scripts/Hud.luau"), Some(ScriptType::Local));
    assert_eq!(editor_state.script_type("Scripts/Util.luau"), Some(ScriptType::Module));
    assert_eq!(editor_state.script_type("Assets/Trees/oak.png"), None);

    // Its tile is there, drawn with that type.
    editor_state.browser.open_folder("Scripts");
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Old");
}

/// #694: the line between the folder tree and the tiles is drawn after both, so nothing in them covers it.
#[test]
fn the_line_beside_the_folder_tree_is_whole() {
    let mut harness = browsing("assets-tree-line");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    let grip = harness
        .get_by_role_and_label(Role::Splitter, "Folder tree width")
        .rect();
    let x = grip.center().x;
    let shapes = &harness.output().shapes;
    let vertical_at_x = |shape: &egui::Shape| match shape {
        egui::Shape::LineSegment { points, .. } => {
            (points[0].x - x).abs() < 1.0 && (points[1].x - x).abs() < 1.0 && points[0].y != points[1].y
        }
        _ => false,
    };
    let line = shapes
        .iter()
        .rposition(|clipped| vertical_at_x(&clipped.shape))
        .expect("the line is drawn");
    let covers = |rect: egui::Rect| rect.x_range().contains(x) && rect.intersects(grip);
    for clipped in &shapes[line + 1..] {
        if let egui::Shape::Rect(rect) = &clipped.shape {
            assert!(!covers(rect.rect), "{:?} is drawn over the line at {x}", rect.rect);
        }
    }
}

/// #695: the search field is wide enough to read a search in, and still at most half the toolbar.
#[test]
fn the_search_field_is_wide() {
    let mut harness = browsing("assets-search-width");
    let field = harness
        .get_by_role_and_label(Role::TextInput, "Search the assets")
        .rect();
    assert!(field.width() >= 260.0, "{field:?}");
    harness.set_size(egui::vec2(700.0, 1000.0));
    settle(&mut harness);
    let narrow = harness
        .get_by_role_and_label(Role::TextInput, "Search the assets")
        .rect();
    assert!(narrow.width() < field.width(), "it shrinks with the panel: {narrow:?}");
}

/// #698: a selected tile's fill has the browser's squarer corners.
#[test]
fn tiles_have_squarer_corners() {
    use crate::theme::tokens::radius;
    let mut harness = browsing("assets-corners");
    harness.get_by_role_and_label(Role::GridCell, "Assets").click();
    settle(&mut harness);
    let tile = harness.get_by_role_and_label(Role::GridCell, "Assets").rect();
    let fill = harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.rect == tile => Some(rect.corner_radius),
            _ => None,
        })
        .expect("the selected tile is filled");
    assert_eq!(fill, egui::CornerRadius::same(radius::TILE));
}

/// #700: the tiles keep some room from the edges of the area they are in.
#[test]
fn tiles_keep_room_from_the_edges() {
    let harness = browsing("assets-padding");
    let grid = harness.get_by_role_and_label(Role::Grid, "Assets").rect();
    let first = harness.get_by_role_and_label(Role::GridCell, "Assets").rect();
    assert!(first.left() - grid.left() >= 6.0, "{first:?} in {grid:?}");
    assert!(first.top() - grid.top() >= 6.0, "{first:?} in {grid:?}");
}

/// #696: Ctrl and the wheel over the tiles make them bigger or smaller, within limits; the wheel alone does not.
#[test]
fn ctrl_and_the_wheel_scale_the_tiles() {
    use crate::editor::assets::{MAX_SCALE, MIN_SCALE};
    let mut harness = browsing("assets-scale");
    let before = harness.get_by_role_and_label(Role::GridCell, "Assets").rect();
    let wheel = |harness: &mut Harness<'_, Test>, y: f32, modifiers: egui::Modifiers| {
        let over = harness.get_by_role_and_label(Role::GridCell, "Assets").rect().center();
        harness.event(egui::Event::PointerMoved(over));
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        settle(harness);
    };
    wheel(&mut harness, 1.0, egui::Modifiers::NONE);
    assert_eq!(editor(&mut harness).browser.scale, 1.0, "the wheel alone scrolls");
    wheel(&mut harness, 3.0, egui::Modifiers::COMMAND);
    let scale = editor(&mut harness).browser.scale;
    assert!(scale > 1.0, "{scale}");
    let after = harness.get_by_role_and_label(Role::GridCell, "Assets").rect();
    assert!(
        after.width() > before.width() && after.height() > before.height(),
        "{before:?} {after:?}"
    );
    for _ in 0..40 {
        wheel(&mut harness, 3.0, egui::Modifiers::COMMAND);
    }
    assert_eq!(editor(&mut harness).browser.scale, MAX_SCALE);
    for _ in 0..80 {
        wheel(&mut harness, -3.0, egui::Modifiers::COMMAND);
    }
    assert_eq!(editor(&mut harness).browser.scale, MIN_SCALE);
    let small = harness.get_by_role_and_label(Role::GridCell, "Assets").rect();
    assert!(small.width() < before.width());
}

/// #702: the menus show one New item; its items open to the side, not in the menu itself.
#[test]
fn new_items_are_in_a_submenu() {
    let mut harness = browsing("assets-new-submenu");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    ground_menu(&mut harness);
    settle(&mut harness);
    let new = harness.get_by_role_and_label(Role::MenuItem, "New").rect();
    assert!(
        harness.query_by_role_and_label(Role::MenuItem, "Material").is_none(),
        "not until New opens"
    );
    harness.event(egui::Event::PointerMoved(new.center()));
    settle(&mut harness);
    let material = harness.get_by_role_and_label(Role::MenuItem, "Material").rect();
    assert!(
        material.left() >= new.right() - 1.0,
        "to the side: {material:?} beside {new:?}"
    );
    harness.get_by_role_and_label(Role::MenuItem, "Material").click();
    settle(&mut harness);
    harness.key_press(Key::Escape);
    settle(&mut harness);
    assert!(editor(&mut harness).project.contains("Assets/Material.material"));
    assert!(
        harness.query_by_role_and_label(Role::MenuItem, "New").is_none(),
        "the menus closed"
    );
    // A folder's row in the tree has the same New.
    harness
        .get_by_role_and_label(Role::TreeItem, "Assets")
        .click_secondary();
    settle(&mut harness);
    choose_new(&mut harness, "Folder");
    harness.key_press(Key::Escape);
    settle(&mut harness);
    assert!(editor(&mut harness).project.has_folder("Assets/New Folder"));
}

/// What the frames run until it is there put on the clipboard (at most a few).
fn copied_text(harness: &mut Harness<'_, Test>) -> Option<String> {
    for _ in 0..4 {
        harness.step();
        let copied = harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        if copied.is_some() {
            return copied;
        }
    }
    None
}

/// #705: Copy path, on a tile and on a tree row, puts the path in the project on the clipboard.
#[test]
fn copy_path_puts_the_path_on_the_clipboard() {
    let mut harness = browsing("assets-copy-path");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Trees").click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Copy path").click();
    assert_eq!(copied_text(&mut harness).as_deref(), Some("Assets/Trees"));
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TreeItem, "Assets")
        .click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Copy path").click();
    assert_eq!(copied_text(&mut harness).as_deref(), Some("Assets"));
}

/// The editor's Baseplate.
fn baseplate(harness: &mut Harness<'_, Test>) -> metallic_engine::metallic_core::Id {
    let editor_state = editor(harness);
    let workspace = editor_state
        .dm
        .service(editor_state.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    editor_state
        .dm
        .find_first_child(workspace, "Baseplate", false)
        .unwrap()
        .unwrap()
}

/// Puts the pointer on the tile `name`.
fn hover_tile(harness: &mut Harness<'_, Test>, name: &str) {
    let tile = harness.get_by_role_and_label(Role::GridCell, name).rect();
    harness.event(egui::Event::PointerMoved(tile.center()));
    settle(harness);
}

/// #703: the pointer on a material's tile shows a button that gives it to the selected parts, as one undo step;
/// greyed out with no part selected.
#[test]
fn a_material_s_tile_has_a_button_that_applies_it() {
    let mut harness = browsing("assets-apply-material");
    let path = editor(&mut harness).new_material_file("Assets").unwrap();
    let id = editor(&mut harness).project.assets().id(&path).unwrap();
    editor(&mut harness).browser.open_folder("Assets");
    settle(&mut harness);
    let label = "Apply Material to the selected parts";
    assert!(
        harness.query_by_label(label).is_none(),
        "only while the pointer is on the tile"
    );
    hover_tile(&mut harness, "Material");
    let button = harness.get_by_role_and_label(Role::Button, label);
    assert!(
        egui_kittest::kittest::NodeT::accesskit_node(&button).is_disabled(),
        "no part is selected"
    );

    let part = baseplate(&mut harness);
    editor(&mut harness).select(Some(part));
    settle(&mut harness);
    hover_tile(&mut harness, "Material");
    harness.get_by_role_and_label(Role::Button, label).click();
    settle(&mut harness);
    let material = |harness: &mut Harness<'_, Test>| editor(harness).dm.get_by_name(part, "Material").unwrap();
    assert_eq!(material(&mut harness), Value::String(id.to_string().into()));
    assert!(
        editor(&mut harness).browser.selected.is_empty(),
        "the button does not select the tile"
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    settle(&mut harness);
    assert_ne!(
        material(&mut harness),
        Value::String(id.to_string().into()),
        "one undo step"
    );
}

/// #704: the pointer on a script file's tile shows a button that adds a script running it inside each selected object,
/// as one undo step, selecting the new scripts; greyed out with nothing selected.
#[test]
fn a_script_file_s_tile_has_a_button_that_adds_it() {
    let mut harness = browsing("assets-add-script");
    editor(&mut harness).browser.open_folder("Scripts");
    settle(&mut harness);
    let label = "Add Door to the selected objects";
    hover_tile(&mut harness, "Door");
    let button = harness.get_by_role_and_label(Role::Button, label);
    assert!(
        egui_kittest::kittest::NodeT::accesskit_node(&button).is_disabled(),
        "nothing is selected"
    );

    let part = baseplate(&mut harness);
    let editor_state = editor(&mut harness);
    let workspace = editor_state.dm.parent(part).unwrap().unwrap();
    let spawn = editor_state
        .dm
        .find_first_child(workspace, "SpawnLocation", false)
        .unwrap()
        .unwrap();
    editor_state.select(Some(part));
    editor_state.selection.add(spawn);
    settle(&mut harness);
    hover_tile(&mut harness, "Door");
    harness.get_by_role_and_label(Role::Button, label).click();
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    let scripts: Vec<_> = [part, spawn]
        .into_iter()
        .map(|parent| {
            let door = editor_state
                .dm
                .find_first_child(parent, "Door", false)
                .unwrap()
                .expect("a script inside each");
            assert_eq!(editor_state.dm.class_name(door).unwrap(), "Script");
            assert_eq!(
                editor_state.dm.get_by_name(door, "Source"),
                Ok(Value::String("Scripts/Door.server.luau".into()))
            );
            door
        })
        .collect();
    assert_eq!(
        editor_state.selection.ids(),
        scripts.as_slice(),
        "the new scripts are selected"
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    for parent in [part, spawn] {
        assert!(
            editor_state
                .dm
                .find_first_child(parent, "Door", false)
                .unwrap()
                .is_none(),
            "one undo step"
        );
    }
}

/// The names of the list's rows (#699), top to bottom, of those named in `among`.
fn list_rows(harness: &Harness<'_, Test>, among: &[&str]) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = harness
        .query_all_by_role(Role::Row)
        .filter_map(|node| {
            let label = egui_kittest::kittest::NodeT::accesskit_node(&node).label()?;
            among.contains(&label.as_str()).then(|| (node.rect().top(), label))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, label)| label).collect()
}

/// #699: the button by the search shows the folder as a list, whose header sorts it by name, type or size (again,
/// the other way), folders first; a row selects as a tile does, and the button goes back to the tiles.
#[test]
fn the_list_view_sorts_by_its_header() {
    use crate::editor::assets::View;
    let mut harness = browsing("assets-list");
    let editor_state = editor(&mut harness);
    editor_state
        .project
        .create_member("Assets/Trees/big.png", vec![1; 5000])
        .unwrap();
    editor_state
        .project
        .create_member("Assets/Trees/apple.glb", vec![1; 10])
        .unwrap();
    let _ = editor_state.project.create_folder("Assets/Trees/Saplings");
    editor_state.browser.open_folder("Assets/Trees");
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Show as a list").click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).browser.view, View::List);
    harness.get_by_role_and_label(Role::List, "Assets");
    let names = ["Saplings", "apple", "big", "oak"];
    assert_eq!(list_rows(&harness, &names), names, "folders first, then by name");
    let sort = |harness: &mut Harness<'_, Test>, column: &str| {
        harness.get_by_role_and_label(Role::ColumnHeader, column).click();
        settle(harness);
    };
    sort(&mut harness, "Size");
    assert_eq!(list_rows(&harness, &names), ["Saplings", "apple", "oak", "big"]);
    sort(&mut harness, "Size");
    assert_eq!(
        list_rows(&harness, &names),
        ["Saplings", "big", "oak", "apple"],
        "the other way"
    );
    sort(&mut harness, "Type");
    assert_eq!(
        list_rows(&harness, &names),
        ["Saplings", "big", "oak", "apple"],
        "Image before Mesh"
    );
    sort(&mut harness, "Name");
    sort(&mut harness, "Name");
    assert_eq!(list_rows(&harness, &names), ["Saplings", "oak", "big", "apple"]);
    sort(&mut harness, "Date created");
    assert_eq!(list_rows(&harness, &names)[0], "Saplings");

    harness.get_by_role_and_label(Role::Row, "big").click();
    settle(&mut harness);
    assert!(editor(&mut harness).browser.selected.contains("Assets/Trees/big.png"));
    harness.get_by_role_and_label(Role::Button, "Show as tiles").click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).browser.view, View::Grid);
    assert!(tile_exists(&harness, "big"));
}

/// A fresh folder on the computer for `test` to import from.
fn import_source(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("metallic-import-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Pack/Sounds")).unwrap();
    std::fs::write(dir.join("bark.png"), vec![9; 64]).unwrap();
    std::fs::write(dir.join("Pack/rock.glb"), vec![8; 32]).unwrap();
    std::fs::write(dir.join("Pack/Sounds/hit.ogg"), vec![7; 16]).unwrap();
    dir
}

/// #706: files and folders from the computer come into a project folder as new assets, a folder with what is in it;
/// a name already taken is made unique; what cannot be read is said, and the rest still comes in.
#[test]
fn files_and_folders_are_imported_as_assets() {
    let mut harness = browsing("assets-import-files");
    let source = import_source("files");
    let editor_state = editor(&mut harness);
    editor_state
        .project
        .create_member("Assets/bark.png", vec![1; 4])
        .unwrap();
    let imported = editor_state.import_files(
        "Assets",
        &[
            source.join("bark.png"),
            source.join("Pack"),
            source.join("nothing-here.png"),
        ],
    );
    assert_eq!(imported.paths, ["Assets/bark 2.png", "Assets/Pack"]);
    assert_eq!(imported.failed.len(), 1);
    assert!(
        imported.failed[0].starts_with("nothing-here.png"),
        "{:?}",
        imported.failed
    );
    assert!(imported.error().unwrap().contains("nothing-here.png"));
    for (member, bytes) in [
        ("Assets/bark 2.png", vec![9; 64]),
        ("Assets/Pack/rock.glb", vec![8; 32]),
        ("Assets/Pack/Sounds/hit.ogg", vec![7; 16]),
    ] {
        assert_eq!(editor_state.project.read(member).unwrap(), bytes, "{member}");
        assert!(
            editor_state.project.assets().id(member).is_some(),
            "{member} is an asset"
        );
    }
    assert_eq!(editor_state.browser.open, "Assets");
    assert_eq!(
        editor_state.browser.selected.iter().collect::<Vec<_>>(),
        ["Assets/Pack", "Assets/bark 2.png"]
    );
    let _ = std::fs::remove_dir_all(source);
}

/// #706: File, Import asks for files and brings them into the Assets browser's open folder, showing the browser.
#[test]
fn file_import_asks_for_files_and_brings_them_in() {
    let mut harness = browsing("assets-file-import");
    let source = import_source("menu");
    let dialog = crate::projects::dialog::ScriptedDialog::new([Some(source.join("bark.png"))]);
    harness.state_mut().studio.set_file_dialog(Box::new(dialog.clone()));
    editor(&mut harness).browser.open_folder("Assets/Trees");
    settle(&mut harness);
    harness
        .get_all_by_label("File")
        .find(|node| egui_kittest::kittest::NodeT::accesskit_node(node).role() != Role::Tab)
        .unwrap()
        .click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Import\u{2026}").click();
    settle(&mut harness);
    assert_eq!(dialog.asked(), ["files"]);
    assert!(editor(&mut harness).project.contains("Assets/Trees/bark.png"));
    assert!(tile_exists(&harness, "bark"), "shown in the browser");
    let _ = std::fs::remove_dir_all(source);
}

/// #707: the toolbar's Import button imports into the open folder; the empty space's menu does too, and a folder's
/// menu in the tree into that folder.
#[test]
fn the_browser_imports_from_its_toolbar_and_menus() {
    let mut harness = browsing("assets-browser-import");
    let source = import_source("browser");
    let dialog = crate::projects::dialog::ScriptedDialog::new([
        Some(source.join("bark.png")),
        Some(source.join("Pack")),
        Some(source.join("Pack/rock.glb")),
    ]);
    harness.state_mut().studio.set_file_dialog(Box::new(dialog.clone()));
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);

    harness
        .get_by_role_and_label(Role::Button, "Import into this folder")
        .click();
    settle(&mut harness);
    assert!(editor(&mut harness).project.contains("Assets/bark.png"));

    ground_menu(&mut harness);
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Import\u{2026}").click();
    settle(&mut harness);
    assert!(editor(&mut harness).project.contains("Assets/Pack/Sounds/hit.ogg"));

    harness
        .get_by_role_and_label(Role::TreeItem, "Assets")
        .click_secondary();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::MenuItem, "Import\u{2026}").click();
    settle(&mut harness);
    assert!(editor(&mut harness).project.contains("Assets/rock.glb"));
    assert_eq!(dialog.asked(), ["files", "files", "files"]);
    let _ = std::fs::remove_dir_all(source);
}

/// A file dropped from the computer, as the window hands it over (#708).
#[derive(Debug)]
struct OnDisk(std::path::PathBuf);

impl egui::DroppedFile for OnDisk {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

/// #708: while files from the computer are dragged over Studio, the browser says which folder they will go into;
/// dropped, files and folders are imported there.
#[test]
fn files_dropped_from_the_computer_are_imported() {
    let mut harness = browsing("assets-drop-files");
    let source = import_source("drop");
    editor(&mut harness).browser.open_folder("Assets/Trees");
    settle(&mut harness);
    harness.input_mut().hovered_files.push(egui::HoveredFile {
        path: Some(source.join("bark.png")),
        ..Default::default()
    });
    harness.step();
    harness.step();
    harness.get_by_label("Drop to import into Trees");
    harness.input_mut().hovered_files.clear();
    for path in [source.join("bark.png"), source.join("Pack")] {
        harness
            .input_mut()
            .dropped_files
            .push(std::sync::Arc::new(OnDisk(path)));
    }
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    assert!(editor_state.project.contains("Assets/Trees/bark.png"));
    assert!(editor_state.project.contains("Assets/Trees/Pack/Sounds/hit.ogg"));
    assert!(
        harness.query_by_label("Drop to import into Trees").is_none(),
        "gone once dropped"
    );
    assert!(tile_exists(&harness, "bark") && tile_exists(&harness, "Pack"));
    let _ = std::fs::remove_dir_all(source);
}

/// #734: the button on a material's tile grows and shrinks with the tiles.
#[test]
fn a_tile_s_button_scales_with_the_tiles() {
    let mut harness = browsing("assets-corner-scale");
    editor(&mut harness).new_material_file("Assets").unwrap();
    editor(&mut harness).browser.open_folder("Assets");
    settle(&mut harness);
    let label = "Apply Material to the selected parts";
    hover_tile(&mut harness, "Material");
    let small = harness.get_by_role_and_label(Role::Button, label).rect();
    editor(&mut harness).browser.zoom(2.0);
    settle(&mut harness);
    hover_tile(&mut harness, "Material");
    let big = harness.get_by_role_and_label(Role::Button, label).rect();
    assert!(big.width() > small.width() * 1.5, "{small:?} {big:?}");
}

/// Where the text `label` is drawn in the row `row`, left edge.
fn text_left(harness: &Harness<'_, Test>, label: &str, row: egui::Rect) -> f32 {
    harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == label && row.y_range().contains(text.pos.y + 1.0) => {
                Some(text.pos.x)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label}"))
}

/// #736: the folder tree starts near its left edge: the root has no empty caret column, and the first level's carets
/// sit under the root's icon.
#[test]
fn the_folder_tree_starts_near_its_left_edge() {
    let harness = browsing("assets-tree-left");
    let root = harness.get_by_role_and_label(Role::TreeItem, "Project").rect();
    let name = text_left(&harness, "Project", root) - root.left();
    assert!(name <= 24.0, "the root's name {name} from the edge");
    let assets = harness.get_by_role_and_label(Role::TreeItem, "Assets").rect();
    let child = text_left(&harness, "Assets", assets) - root.left();
    assert!(child <= 42.0, "a first-level name {child} from the edge");
    assert!(child > name, "indented under the root");
}

/// #737: a press on a folder's caret opens it, and dragging on over other carets opens them too; starting on an open
/// folder's caret closes the ones passed over instead.
#[test]
fn a_drag_across_the_carets_opens_or_closes_each() {
    let mut harness = browsing("assets-caret-sweep");
    let expanded = |harness: &mut Harness<'_, Test>, folder: &str| editor(harness).browser.expanded.contains(folder);
    assert!(!expanded(&mut harness, "Assets") && !expanded(&mut harness, "Scripts"));
    let caret = |harness: &Harness<'_, Test>, label: &str| harness.get_by_label(label).rect().center();
    let from = caret(&harness, "Expand Assets");
    harness.drag_at(from);
    settle(&mut harness);
    assert!(expanded(&mut harness, "Assets"), "opened when pressed");
    // Assets' folders are in the tree now, and Scripts lower down.
    let to = caret(&harness, "Expand Scripts");
    harness.event(egui::Event::PointerMoved(to));
    settle(&mut harness);
    harness.drop_at(to);
    settle(&mut harness);
    assert!(expanded(&mut harness, "Scripts"), "opened when passed over");
    assert!(expanded(&mut harness, "Assets"), "still open");

    // From an open folder's caret, the others passed over close.
    let from = caret(&harness, "Collapse Assets");
    harness.drag_at(from);
    settle(&mut harness);
    assert!(!expanded(&mut harness, "Assets"));
    let to = caret(&harness, "Collapse Scripts");
    harness.event(egui::Event::PointerMoved(to));
    settle(&mut harness);
    harness.drop_at(to);
    settle(&mut harness);
    assert!(!expanded(&mut harness, "Scripts"));
    // A plain click still toggles one.
    harness.get_by_label("Expand Scripts").click();
    settle(&mut harness);
    assert!(expanded(&mut harness, "Scripts") && !expanded(&mut harness, "Assets"));
}

/// #738: a column's right edge in the list's header is dragged to make it wider; the rows follow.
#[test]
fn the_list_s_columns_are_resized_from_its_header() {
    let mut harness = browsing("assets-column-widths");
    editor(&mut harness).browser.view = crate::editor::assets::View::List;
    settle(&mut harness);
    let header =
        |harness: &Harness<'_, Test>, label: &str| harness.get_by_role_and_label(Role::ColumnHeader, label).rect();
    let before = header(&harness, "Type");
    let grip = harness
        .get_by_role_and_label(Role::Splitter, "Type width")
        .rect()
        .center();
    drag(&mut harness, grip, grip + egui::vec2(40.0, 0.0));
    let after = header(&harness, "Type");
    assert!(
        (after.width() - before.width() - 40.0).abs() < 2.0,
        "{before:?} {after:?}"
    );
    assert!((header(&harness, "Date created").left() - after.right()).abs() < 0.5);
    // Name keeps the width it is dragged to.
    let grip = harness
        .get_by_role_and_label(Role::Splitter, "Name width")
        .rect()
        .center();
    drag(&mut harness, grip, grip - egui::vec2(60.0, 0.0));
    let name = editor(&mut harness).browser.columns.name;
    assert!(name.is_some(), "Name has a width of its own now");
}

/// #740, #779: one file or folder selected in the browser shows in the Properties panel: its name (renamed in place),
/// its path and its details, in the panel's rows; the browser has no pane of its own for it. Selecting an instance
/// shows the instance; a click on the file shows it again; with two selected there is none.
#[test]
fn the_properties_panel_shows_the_one_asset_selected() {
    let mut harness = browsing("assets-properties");
    harness.set_size(egui::vec2(1280.0, 1600.0));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    assert!(harness.query_by_label("Asset properties").is_none(), "nothing selected");
    harness.get_by_role_and_label(Role::GridCell, "Trees").click();
    settle(&mut harness);
    let pane = harness.get_by_label("Asset properties").rect();
    let grid = harness.get_by_role_and_label(Role::Grid, "Assets").rect();
    assert!(!pane.intersects(grid), "in the Properties panel: {pane:?} {grid:?}");
    let name = |harness: &Harness<'_, Test>| {
        harness
            .get_by_role_and_label(Role::TextInput, "Name")
            .value()
            .unwrap_or_default()
    };
    assert_eq!(name(&harness), "Trees");
    harness.get_by_label("Path: Assets/Trees");
    harness.get_by_label("Type: Folder");
    harness.get_by_label("Contents: 1 item");
    harness.get_by_label("FILE");

    // Renamed from its name.
    harness.get_by_role_and_label(Role::TextInput, "Name").click();
    settle(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, "Name")
        .type_text("Pines");
    harness.key_press(Key::Enter);
    settle(&mut harness);
    assert!(editor(&mut harness).project.has_folder("Assets/Pines"));
    assert_eq!(name(&harness), "Pines", "it follows the rename");

    // An instance selected shows the instance; a click on the folder shows it again.
    harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
    settle(&mut harness);
    assert!(harness.query_by_label("Asset properties").is_none());
    assert_eq!(name(&harness), "Baseplate");
    harness.get_by_role_and_label(Role::GridCell, "Pines").click();
    settle(&mut harness);
    assert_eq!(name(&harness), "Pines");
    // Two selected: none.
    harness
        .get_by_role_and_label(Role::GridCell, "Materials")
        .click_modifiers(egui::Modifiers::COMMAND);
    settle(&mut harness);
    assert!(harness.query_by_label("Asset properties").is_none());
}

/// #781: a picture's properties set its filtering, kept in the project's asset index.
#[test]
fn a_picture_is_set_pixelated_in_its_properties() {
    let mut harness = browsing("assets-pixelated");
    harness.set_size(egui::vec2(1280.0, 1600.0));
    settle(&mut harness);
    editor(&mut harness).browser.open_folder("Assets/Trees");
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "oak").click();
    settle(&mut harness);
    harness.get_by_label("PICTURE");
    harness.get_by_role_and_label(Role::ComboBox, "Filtering").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Pixelated").click();
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    assert!(editor_state.project.assets().pixelated("Assets/Trees/oak.png"));
    let index = editor_state.project.read_text("assets.json").unwrap();
    assert!(index.contains("\"filter\": \"pixelated\""), "{index}");
}

/// Right-clicks the empty space behind the tiles, at its bottom right, where no tile is (the inspector, #740, can make
/// the area narrow enough for a tile to sit in its middle).
fn ground_menu(harness: &mut Harness<'_, Test>) {
    let grid = harness.get_by_role_and_label(Role::Grid, "Assets").rect();
    let at = grid.right_bottom() - egui::vec2(12.0, 12.0);
    right_click(harness, egui::Rect::from_center_size(at, egui::vec2(2.0, 2.0)));
    settle(harness);
}

/// #741: a folder's properties give it a colour, kept in the project's editor data; it follows a rename, and the
/// usual colour comes back from its button.
#[test]
fn a_folder_s_colour_is_set_in_its_properties_and_follows_it() {
    let mut harness = browsing("assets-folder-colour");
    harness.get_by_role_and_label(Role::TreeItem, "Assets").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Trees").click();
    settle(&mut harness);
    // In a short panel, the Properties panel is scrolled to its colour.
    let pane = harness.get_by_label("Asset properties").rect();
    harness.event(egui::Event::PointerMoved(pane.center()));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -20.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    });
    settle(&mut harness);
    // The swatch opens Studio's colour picker; a hex typed there is the folder's colour.
    harness.get_by_role_and_label(Role::ColorWell, "Folder colour").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::TextInput, "Hex").click();
    settle(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, "Hex")
        .type_text("#ff8000");
    harness.key_press(Key::Enter);
    settle(&mut harness);
    harness.key_press(Key::Escape);
    settle(&mut harness);
    assert_eq!(editor(&mut harness).folder_color("Assets/Trees"), Some([255, 128, 0]));
    let member = crate::editor::folder_colors::member();
    let kept = editor(&mut harness).project.read_text(&member).unwrap();
    assert!(kept.contains("\"Assets/Trees\": \"#ff8000\""), "{kept}");

    // It follows a rename.
    editor(&mut harness).rename_asset("Assets/Trees", "Pines").unwrap();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).folder_color("Assets/Pines"), Some([255, 128, 0]));
    assert_eq!(editor(&mut harness).folder_color("Assets/Trees"), None);

    // The usual colour back: the editor data goes with the last colour.
    harness
        .get_by_role_and_label(Role::Button, "Use the usual colour")
        .click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).folder_color("Assets/Pines"), None);
    assert!(!editor(&mut harness).project.contains(&member));
}
