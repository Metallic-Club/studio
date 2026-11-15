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

    // The mesh's picker offers the model alone; choosing it stores its id.
    let mesh = harness.get_by_role_and_label(Role::ComboBox, "MeshId");
    mesh.click();
    settle(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Assets/bark.png")
            .is_none()
    );
    harness.get_by_role_and_label(Role::Button, "None");
    harness.get_by_role_and_label(Role::Button, "Assets/rock.glb").click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "MeshId"),
        Ok(Value::String(rock.as_str().into()))
    );

    // The texture's picker offers the picture; its row then shows the file's name.
    harness.get_by_role_and_label(Role::ComboBox, "TextureId").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Assets/bark.png").click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "TextureId"),
        Ok(Value::String(bark.as_str().into()))
    );
    let shown = harness.get_by_role_and_label(Role::ComboBox, "TextureId").value();
    assert_eq!(shown.as_deref(), Some("bark.png"));
    // A separate step: edits of one property less than 600 ms apart are one.
    std::thread::sleep(std::time::Duration::from_millis(700));
    // None empties it, as one undo step.
    harness.get_by_role_and_label(Role::ComboBox, "TextureId").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "None").click();
    settle(&mut harness);
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
    let to = harness.get_by_role_and_label(Role::ComboBox, "MeshId").rect().center();
    drag(&mut harness, from, to);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(
        editor.dm.get_by_name(part, "MeshId"),
        Ok(Value::String(rock.as_str().into()))
    );
    // Only the picker it was let go on changed.
    assert_eq!(editor.dm.get_by_name(part, "TextureId"), Ok(Value::String("".into())));
}
