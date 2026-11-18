//! #55: properties that name a project file pick it from the project's files: a script's `Source` by path, a mesh
//! part's `MeshId` and `TextureId` by asset id, each offering only the files of its kinds, and None.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::Value;

use super::*;

#[test]
fn a_mesh_part_s_ids_and_a_script_s_source_are_picked_from_the_project_s_files() {
    let mut harness = editing(settings_file("editor-file-pickers", None), "Pickers Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, script, rock, bark) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        for (member, bytes) in [
            ("Assets/rock.glb", vec![1; 64]),
            ("Assets/bark.png", vec![2; 64]),
            ("Scripts/Door.server.luau", b"print(1)".to_vec()),
        ] {
            if !editor.project.contains(member) {
                editor.project.create_member(member, bytes).unwrap();
            }
        }
        let rock = editor.project.assets().id("Assets/rock.glb").unwrap().to_string();
        let bark = editor.project.assets().id("Assets/bark.png").unwrap().to_string();
        let dm = &mut editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.create(dm.registry().class_id("MeshPart").unwrap()).unwrap();
        dm.set_name(part, "Statue".into()).unwrap();
        dm.set_parent(part, Some(workspace)).unwrap();
        let service = dm
            .service(dm.registry().class_id("ServerScriptService").unwrap())
            .unwrap();
        let script = dm.create(dm.registry().class_id("Script").unwrap()).unwrap();
        dm.set_name(script, "Door".into()).unwrap();
        dm.set_parent(script, Some(service)).unwrap();
        editor.select(Some(part));
        (part, script, rock, bark)
    };
    settle(&mut harness);

    // The mesh's window (#806) offers the model alone; choosing it stores its id.
    harness.get_by_role_and_label(Role::Button, "Choose MeshId").click();
    settle(&mut harness);
    harness.get_by_label("Search the meshes");
    assert!(harness.query_by_role_and_label(Role::Row, "bark").is_none());
    harness.get_by_role_and_label(Role::Row, "None");
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::Row, "rock").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "MeshId")
            .value()
            .as_deref(),
        Some("rock"),
        "the mesh's name"
    );
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "MeshId"),
        Ok(Value::String(rock.as_str().into()))
    );

    // The texture's window (#852) offers the picture alone, as a material's texture rows' does; its row then shows
    // the picture's name.
    let choose = |harness: &mut Harness<'_, Test>, row: &str| {
        harness.get_by_role_and_label(Role::Button, "Choose TextureId").click();
        settle(harness);
        harness.get_by_label("Search the pictures");
        assert!(harness.query_by_role_and_label(Role::Row, "rock").is_none());
        let time = harness.ctx.input(|input| input.time) + 1.0;
        harness.input_mut().time = Some(time);
        harness.get_by_role_and_label(Role::Row, row).click();
        settle(harness);
        harness.get_by_role_and_label(Role::Button, "Select").click();
        settle(harness);
        harness.input_mut().time = None;
    };
    choose(&mut harness, "bark");
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "TextureId"),
        Ok(Value::String(bark.as_str().into()))
    );
    let shown = harness.get_by_role_and_label(Role::TextInput, "TextureId").value();
    assert_eq!(shown.as_deref(), Some("bark"));
    // A separate step: edits of one property less than 600 ms apart are one.
    std::thread::sleep(std::time::Duration::from_millis(700));
    // None empties it, as one undo step.
    choose(&mut harness, "None");
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(editor.dm.get_by_name(part, "TextureId"), Ok(Value::String("".into())));
    harness.state_mut().studio.menu_action(MenuAction::Undo);
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "TextureId"),
        Ok(Value::String(bark.as_str().into()))
    );

    // A script's Source is a path, and only scripts are offered.
    editor.select(Some(script));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::ComboBox, "Source").click();
    settle(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Assets/rock.glb")
            .is_none()
    );
    harness
        .get_by_role_and_label(Role::Button, "Scripts/Door.server.luau")
        .click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(script, "Source"),
        Ok(Value::String("Scripts/Door.server.luau".into()))
    );
}

/// #55 with #676: a file dragged from the Assets panel onto a picker that takes it is chosen.
#[test]
fn a_file_dragged_from_the_assets_panel_onto_a_picker_is_chosen() {
    let mut harness = editing(settings_file("editor-file-picker-drop", None), "Picker Drop Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, rock) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        if !editor.project.contains("Assets/rock.glb") {
            editor.project.create_member("Assets/rock.glb", vec![1; 64]).unwrap();
        }
        let rock = editor.project.assets().id("Assets/rock.glb").unwrap().to_string();
        let dm = &mut editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.create(dm.registry().class_id("MeshPart").unwrap()).unwrap();
        dm.set_parent(part, Some(workspace)).unwrap();
        editor.select(Some(part));
        editor.browser.open_folder("Assets");
        (part, rock)
    };
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);
    let from = harness.get_by_role_and_label(Role::GridCell, "rock").rect().center();
    let to = harness.get_by_role_and_label(Role::TextInput, "MeshId").rect().center();
    drag(&mut harness, from, to);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "MeshId"),
        Ok(Value::String(rock.as_str().into()))
    );
    // Only the picker it was let go on changed.
    assert_eq!(editor.dm.get_by_name(part, "TextureId"), Ok(Value::String("".into())));
}

/// #739: a part's Material is a field showing the material's name, its path while it is edited; a path or a name
/// typed is taken on Enter, and the button beside it opens a window to choose one.
#[test]
fn a_part_s_material_is_typed_or_chosen_in_a_window() {
    let mut harness = editing(settings_file("editor-material-field", None), "Material Field Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, neon, wood) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let id = |path: &str| editor.project.assets().id(path).unwrap().to_string();
        let (neon, wood) = (
            id("Assets/Materials/Neon.material"),
            id("Assets/Materials/Wood.material"),
        );
        let dm = &editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        editor.select(Some(part));
        (part, neon, wood)
    };
    settle(&mut harness);
    let material = |harness: &mut Harness<'_, Test>| {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        editor.dm.get_by_name(part, "Material").unwrap()
    };
    let field = |harness: &Harness<'_, Test>| harness.get_by_role_and_label(Role::TextInput, "Material").value();
    assert_eq!(field(&harness).as_deref(), Some("Concrete"));

    // Focused, it shows the file's path; a path typed is the material's id.
    harness.get_by_role_and_label(Role::TextInput, "Material").click();
    settle(&mut harness);
    assert_eq!(field(&harness).as_deref(), Some("Assets/Materials/Concrete.material"));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, "Material")
        .type_text("Assets/Materials/Neon.material");
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(neon.as_str().into()));
    assert_eq!(field(&harness).as_deref(), Some("Neon"));

    // The window: a row picked out, then Select.
    // Clicks a second apart, so none is a double click (which chooses at once).
    let later = |harness: &mut Harness<'_, Test>| {
        let time = harness.ctx.input(|input| input.time) + 1.0;
        harness.input_mut().time = Some(time);
        settle(harness);
    };
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    // The search narrows the list to what it names.
    harness.get_by_label("Search the materials").click();
    settle(&mut harness);
    harness.get_by_label("Search the materials").type_text("wood");
    settle(&mut harness);
    assert!(harness.query_by_role_and_label(Role::Row, "Neon").is_none());
    harness.get_by_role_and_label(Role::Row, "WoodPlanks");
    later(&mut harness);
    harness.get_by_role_and_label(Role::Row, "Wood").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(wood.as_str().into()));
    assert!(harness.query_by_label("Search the materials").is_none(), "closed");

    // Escape closes it without a change; None empties the value.
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(wood.as_str().into()));
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    later(&mut harness);
    harness.get_by_role_and_label(Role::Row, "None").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String("".into()));
}

/// #761: the Choose a material window shows the materials as a grid of their balls too, and the next window opens
/// the way the last was left.
#[test]
fn the_material_window_shows_a_grid() {
    let mut harness = editing(settings_file("editor-material-grid", None), "Material Grid Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, wood, ball) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let dm = &editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        editor.select(Some(part));
        let path = "Assets/Materials/Wood.material";
        let wood = editor.project.assets().id(path).unwrap();
        // Its ball, as the thumbnails' renderer would have drawn it.
        let fingerprint = editor.project.fingerprint(path).unwrap();
        let ball = egui::TextureId::User(41);
        editor.pictures.done(wood, fingerprint, ball);
        (part, wood, ball)
    };
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Row, "Concrete");
    harness.get_by_role_and_label(Role::Button, "Show as a grid").click();
    settle(&mut harness);
    assert!(harness.query_by_role_and_label(Role::Row, "Concrete").is_none());
    harness.get_by_label("Search the materials").click();
    settle(&mut harness);
    harness.get_by_label("Search the materials").type_text("wood");
    settle(&mut harness);
    settle(&mut harness);
    // Its tile shows its ball.
    let tile = harness.get_by_role_and_label(Role::GridCell, "Wood").rect();
    let output = harness.output();
    let drawn = output.shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Mesh(mesh) => mesh.texture_id == ball && tile.contains(mesh.vertices[0].pos),
        _ => false,
    });
    assert!(drawn, "the ball is drawn on its tile");
    // A click picks it out, then Select.
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::GridCell, "Wood").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    let material = |harness: &mut Harness<'_, Test>| {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        editor.dm.get_by_name(part, "Material").unwrap()
    };
    assert_eq!(material(&mut harness), Value::String(wood.to_string().into()));
    // The next window is a grid too, until it is switched back.
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Concrete");
    harness.get_by_role_and_label(Role::Button, "Show as a list").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Row, "Concrete");
}

/// #777: the window's list rows show the materials' balls, in place of their icon, once drawn.
#[test]
fn the_material_window_s_rows_show_balls() {
    let mut harness = editing(settings_file("editor-material-rows", None), "Material Rows Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let ball = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let dm = &editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        editor.select(Some(part));
        let path = "Assets/Materials/Wood.material";
        let wood = editor.project.assets().id(path).unwrap();
        let fingerprint = editor.project.fingerprint(path).unwrap();
        let ball = egui::TextureId::User(43);
        editor.pictures.done(wood, fingerprint, ball);
        ball
    };
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Choose Material").click();
    settle(&mut harness);
    harness.get_by_label("Search the materials").click();
    settle(&mut harness);
    harness.get_by_label("Search the materials").type_text("wood");
    settle(&mut harness);
    settle(&mut harness);
    let row = harness.get_by_role_and_label(Role::Row, "Wood").rect();
    let output = harness.output();
    let drawn = output.shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Mesh(mesh) => mesh.texture_id == ball && row.contains(mesh.vertices[0].pos),
        _ => false,
    });
    assert!(drawn, "the ball is drawn on its row");
}

/// #760: what is typed in the Material field is completed from the project's materials: Up and Down and Enter, Tab,
/// or a click.
#[test]
fn the_material_field_completes_what_is_typed() {
    let mut harness = editing(settings_file("editor-material-completions", None), "Completions Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, id) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let dm = &editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        editor.select(Some(part));
        let assets = editor.project.assets();
        let id = |name: &str| {
            assets
                .id(&format!("Assets/Materials/{name}.material"))
                .unwrap()
                .to_string()
        };
        (part, [id("Wood"), id("WoodPlanks"), id("Neon"), id("Concrete")])
    };
    let [wood, planks, neon, _] = id;
    settle(&mut harness);
    let material = |harness: &mut Harness<'_, Test>| {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        editor.dm.get_by_name(part, "Material").unwrap()
    };
    let type_in = |harness: &mut Harness<'_, Test>, text: &str| {
        harness.get_by_role_and_label(Role::TextInput, "Material").click();
        settle(harness);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness
            .get_by_role_and_label(Role::TextInput, "Material")
            .type_text(text);
        settle(harness);
        // The list of completions is laid out unseen the frame it first shows.
        settle(harness);
    };

    // Nothing is offered until something is typed.
    harness.get_by_role_and_label(Role::TextInput, "Material").click();
    settle(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::ListBoxOption, "Concrete")
            .is_none()
    );
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);

    // Names that start with it first; Down twice and Enter takes the second.
    type_in(&mut harness, "woo");
    harness.get_by_role_and_label(Role::ListBoxOption, "Wood");
    harness.get_by_role_and_label(Role::ListBoxOption, "WoodPlanks");
    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Neon").is_none());
    harness.key_press(egui::Key::ArrowDown);
    harness.key_press(egui::Key::ArrowDown);
    settle(&mut harness);
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(planks.as_str().into()));
    assert!(
        harness.query_by_role_and_label(Role::ListBoxOption, "Wood").is_none(),
        "closed"
    );

    // Enter on text naming nothing takes the first; Tab does too.
    type_in(&mut harness, "neo");
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(neon.as_str().into()));
    type_in(&mut harness, "wood");
    harness.key_press(egui::Key::Tab);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(wood.as_str().into()));

    // A click takes the one clicked.
    type_in(&mut harness, "planks");
    harness.get_by_role_and_label(Role::ListBoxOption, "WoodPlanks").click();
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(planks.as_str().into()));

    // Escape keeps the value.
    type_in(&mut harness, "neo");
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
    assert_eq!(material(&mut harness), Value::String(planks.as_str().into()));
}

/// #849: a MeshPart given a mesh takes the mesh's own size, so the model (and its texture) keep their shape.
#[test]
fn a_mesh_part_takes_its_new_mesh_s_size() {
    let mut harness = editing(settings_file("editor-mesh-size", None), "Mesh Size Game");
    harness.set_size(egui::vec2(1280.0, 1400.0));
    let (part, expected) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/project/tests/fixtures/textured_cube.glb");
        let bytes = std::fs::read(model).unwrap();
        let expected = metallic_engine::metallic_project::decode_mesh(&bytes, |_| Err(String::new()))
            .unwrap()
            .size;
        if !editor.project.contains("Assets/cube.glb") {
            editor.project.create_member("Assets/cube.glb", bytes).unwrap();
        }
        let dm = &mut editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.create(dm.registry().class_id("MeshPart").unwrap()).unwrap();
        dm.set_parent(part, Some(workspace)).unwrap();
        editor.select(Some(part));
        (part, expected)
    };
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Choose MeshId").click();
    settle(&mut harness);
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::Row, "cube").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    let Ok(Value::Vector3(size)) = editor.dm.get_by_name(part, "Size") else {
        panic!("a size");
    };
    assert!(
        (size - metallic_engine::metallic_core::Vector3::from(expected)).length() < 1e-4,
        "{size:?} {expected:?}"
    );
    // One undo step puts the mesh and the size back.
    editor.scene().undo(std::time::Instant::now());
    assert_eq!(editor.dm.get_by_name(part, "MeshId"), Ok(Value::String("".into())));
}
