//! Saving and loading real games, built from the built-in classes.

use std::sync::Arc;

use metallic_core::glam::Quat;
use metallic_core::{CFrame, Color3, DataModel, Id, Value, Vector3};
use serde_json::{Value as Json, json};

use super::*;

fn game() -> DataModel {
    metallic_classes::new_game().unwrap()
}

fn service(dm: &DataModel, class: &str) -> Id {
    dm.service(dm.registry().class_id(class).unwrap()).unwrap()
}

fn new_in(dm: &mut DataModel, class: &str, parent: Id, name: &str) -> Id {
    let id = dm.create(dm.registry().class_id(class).unwrap()).unwrap();
    dm.set_name(id, name.into()).unwrap();
    dm.set_parent(id, Some(parent)).unwrap();
    id
}

fn enum_item(dm: &DataModel, enum_type: &str, name: &str) -> Value {
    Value::Enum(dm.registry().enum_item(enum_type, name).unwrap())
}

fn save(dm: &DataModel) -> String {
    let saved = save_scene(dm, &mut SceneIds::new()).unwrap();
    assert_eq!(saved.warnings, [], "save warnings");
    saved.text
}

/// Loads `text` into a new game and returns it with what the load reported.
fn load(text: &str) -> (DataModel, LoadedScene) {
    let mut dm = game();
    let loaded = load_scene(&mut dm, text).unwrap_or_else(|e| panic!("{e}"));
    (dm, loaded)
}

/// A game using every property value type, attributes, nesting, references between instances and changed service
/// properties. Returns it with the `Part` and the `ObjectValue`.
fn rich_game() -> (DataModel, Id, Id) {
    let mut dm = game();
    let workspace = service(&dm, "Workspace");
    let model = new_in(&mut dm, "Model", workspace, "House");
    let part = new_in(&mut dm, "Part", model, "Door");
    let cframe = CFrame::from_parts(Vector3::new(1.5, 2.0, -3.25), Quat::from_rotation_y(0.7));
    dm.set_by_name(part, "CFrame", Value::CFrame(cframe)).unwrap();
    dm.set_by_name(part, "Size", Value::Vector3(Vector3::new(4.0, 7.0, 0.2)))
        .unwrap();
    dm.set_by_name(part, "Color", Value::Color3(Color3::new(0.1, 0.2, 0.3)))
        .unwrap();
    dm.set_by_name(part, "Material", enum_item(&dm, "Material", "Wood"))
        .unwrap();
    dm.set_by_name(part, "Shape", enum_item(&dm, "PartShape", "Wedge"))
        .unwrap();
    dm.set_by_name(part, "Transparency", Value::Number(0.1)).unwrap();
    dm.set_by_name(part, "Anchored", Value::Bool(true)).unwrap();
    dm.set_by_name(model, "PrimaryPart", Value::Ref(Some(part))).unwrap();
    let light = new_in(&mut dm, "PointLight", part, "Lamp");
    dm.set_by_name(light, "Range", Value::Number(30.0)).unwrap();
    dm.set_attribute(part, "Speed", Some(Value::Number(3.5))).unwrap();
    dm.set_attribute(part, "Label", Some(Value::String("front".into())))
        .unwrap();
    dm.set_attribute(part, "Offset", Some(Value::Vector3(Vector3::ONE)))
        .unwrap();
    dm.set_attribute(part, "Kind", Some(enum_item(&dm, "Material", "Metal")))
        .unwrap();

    let storage = service(&dm, "ReplicatedStorage");
    let pointer = new_in(&mut dm, "ObjectValue", storage, "Target");
    dm.set_by_name(pointer, "Value", Value::Ref(Some(part))).unwrap();
    let count = new_in(&mut dm, "IntValue", storage, "Count");
    dm.set_by_name(count, "Value", Value::Int(-42)).unwrap();
    let text = new_in(&mut dm, "StringValue", storage, "Text");
    dm.set_by_name(text, "Value", Value::String("a \"quoted\" line\nand another".into()))
        .unwrap();

    let scripts = service(&dm, "ServerScriptService");
    let script = new_in(&mut dm, "Script", scripts, "Door");
    dm.set_by_name(script, "Source", Value::String("Scripts/Door.luau".into()))
        .unwrap();
    dm.set_by_name(script, "Enabled", Value::Bool(false)).unwrap();

    let lighting = service(&dm, "Lighting");
    dm.set_by_name(lighting, "ClockTime", Value::Number(6.5)).unwrap();
    dm.set_by_name(lighting, "Ambient", Value::Color3(Color3::new(0.1, 0.1, 0.2)))
        .unwrap();
    (dm, part, pointer)
}

fn doc(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

/// The entry with the scene id `id`.
fn entry(doc: &Json, id: u64) -> &Json {
    doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap()
}

/// The entry of the instance named `name`.
fn named<'a>(doc: &'a Json, name: &str) -> &'a Json {
    doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn a_new_game_saves_its_services_and_camera() {
    let text = save(&game());
    let doc = doc(&text);
    assert_eq!(doc["format"], SCENE_FORMAT);
    let classes: Vec<&str> = doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["class"].as_str().unwrap())
        .collect();
    // The persisted services in service order, the camera under Workspace; nothing else.
    assert_eq!(
        classes,
        [
            "Workspace",
            "Camera",
            "Lighting",
            "ReplicatedStorage",
            "ServerScriptService",
            "ServerStorage"
        ]
    );
    let workspace = entry(&doc, 1);
    assert_eq!(workspace["properties"]["CurrentCamera"], 2);
    assert!(workspace.get("parent").is_none() && workspace.get("name").is_none());
    let camera = entry(&doc, 2);
    assert_eq!(camera["parent"], 1);
    assert_eq!(camera["name"], Json::Null, "named like its class");
    assert_eq!(
        camera["properties"].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["CFrame"]
    );
    assert!(
        entry(&doc, 3).get("properties").is_none(),
        "Lighting is at its defaults"
    );
}

#[test]
fn defaults_are_left_out_and_keys_are_sorted() {
    let mut dm = game();
    let workspace = service(&dm, "Workspace");
    let part = new_in(&mut dm, "Part", workspace, "Part");
    dm.set_by_name(part, "Size", Value::Vector3(Vector3::new(4.0, 1.0, 2.0)))
        .unwrap();
    let text = save(&dm);
    let parsed = doc(&text);
    let saved = named_or_class(&parsed, "Part");
    assert!(
        saved.get("properties").is_none(),
        "every property is at its default: {saved}"
    );
    assert!(saved.get("name").is_none());

    dm.set_by_name(part, "Anchored", Value::Bool(true)).unwrap();
    dm.set_by_name(part, "Size", Value::Vector3(Vector3::new(1.0, 2.0, 3.0)))
        .unwrap();
    let text = save(&dm);
    assert!(text.contains("\"properties\": {\n      \"Anchored\": true,\n      \"Size\": [1.0, 2.0, 3.0]\n    }"));
}

fn named_or_class<'a>(doc: &'a Json, class: &str) -> &'a Json {
    doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["class"] == class)
        .unwrap()
}

#[test]
fn every_value_type_survives_save_load_save() {
    let (dm, part, pointer) = rich_game();
    let text = save(&dm);
    let (loaded_dm, loaded) = load(&text);
    assert_eq!(loaded.warnings, []);
    assert_eq!(loaded.format, SCENE_FORMAT);
    assert_eq!(save(&loaded_dm), text, "a fresh numbering gives the same file");
    let mut ids = loaded.ids.clone();
    assert_eq!(
        save_scene(&loaded_dm, &mut ids).unwrap().text,
        text,
        "so do the loaded ids"
    );

    // The values themselves, not just the text.
    let doc = doc(&text);
    let part_id = named(&doc, "Door")["id"].as_u64().unwrap();
    let loaded_part = loaded
        .ids
        .ids
        .iter()
        .find(|&(_, &scene_id)| scene_id == part_id)
        .map(|(&id, _)| id)
        .unwrap();
    for prop in [
        "CFrame",
        "Size",
        "Color",
        "Material",
        "Shape",
        "Transparency",
        "Anchored",
    ] {
        assert_eq!(
            loaded_dm.get_by_name(loaded_part, prop),
            dm.get_by_name(part, prop),
            "{prop}"
        );
    }
    // Attributes come back sorted by name, which is the order the file lists them in.
    let sorted = |dm: &DataModel, id| {
        let mut attributes = dm.attributes(id).unwrap().to_vec();
        attributes.sort_by(|a, b| a.0.cmp(&b.0));
        attributes
    };
    assert_eq!(sorted(&loaded_dm, loaded_part), sorted(&dm, part));
    assert_eq!(sorted(&dm, part).len(), 4);
    let storage = service(&loaded_dm, "ReplicatedStorage");
    let loaded_pointer = loaded_dm.find_first_child(storage, "Target", false).unwrap().unwrap();
    assert_eq!(
        loaded_dm.get_by_name(loaded_pointer, "Value"),
        Ok(Value::Ref(Some(loaded_part)))
    );
    assert_eq!(dm.get_by_name(pointer, "Value"), Ok(Value::Ref(Some(part))));
    let lighting = service(&loaded_dm, "Lighting");
    assert_eq!(loaded_dm.get_by_name(lighting, "ClockTime"), Ok(Value::Number(6.5)));

    // Scripts point at their source member.
    let scripts = loaded_dm
        .find_first_child(service(&loaded_dm, "ServerScriptService"), "Door", false)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded_dm.get_by_name(scripts, "Source"),
        Ok(Value::String("Scripts/Door.luau".into()))
    );
}

#[test]
fn values_are_written_by_their_declared_type() {
    let (dm, ..) = rich_game();
    let doc = doc(&save(&dm));
    let part = named(&doc, "Door");
    let props = &part["properties"];
    assert_eq!(props["Material"], "Wood");
    assert_eq!(props["Shape"], "Wedge");
    assert_eq!(props["Anchored"], true);
    assert_eq!(props["Size"], json!([4.0, 7.0, 0.2]));
    assert_eq!(props["CFrame"].as_array().unwrap().len(), 7);
    assert_eq!(part["attributes"]["Speed"], json!({"Number": 3.5}));
    assert_eq!(
        part["attributes"]["Kind"],
        json!({"Enum": {"enum_type": "Material", "name": "Metal"}})
    );
    let model = named(&doc, "House");
    assert_eq!(model["properties"]["PrimaryPart"], part["id"]);
    assert_eq!(named(&doc, "Count")["properties"]["Value"], -42);
}

#[test]
fn loading_replaces_the_saved_services() {
    let (dm, ..) = rich_game();
    let text = save(&dm);

    let mut target = game();
    let workspace = service(&target, "Workspace");
    new_in(&mut target, "Part", workspace, "Leftover");
    let storage = service(&target, "ServerStorage");
    new_in(&mut target, "Folder", storage, "Old");
    let lighting = service(&target, "Lighting");
    target.set_by_name(lighting, "Brightness", Value::Number(9.0)).unwrap();
    target
        .set_attribute(lighting, "Stale", Some(Value::Bool(true)))
        .unwrap();

    load_scene(&mut target, &text).unwrap();
    assert_eq!(target.find_first_child(workspace, "Leftover", false), Ok(None));
    assert_eq!(target.child_count(storage), Ok(0));
    assert_eq!(target.get_by_name(lighting, "Brightness"), Ok(Value::Number(2.0)));
    assert_eq!(target.attribute(lighting, "Stale"), Ok(None));
    assert_eq!(save(&target), text);
}

#[test]
fn ids_stay_stable_across_edits() {
    let (dm, ..) = rich_game();
    let text = save(&dm);
    let (mut dm, loaded) = load(&text);
    let mut ids = loaded.ids;
    let before = doc(&text);
    let door = named(&before, "Door")["id"].clone();
    let highest = before["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .max()
        .unwrap();

    // A new part ahead of everything else in Workspace's subtree, and the model's door destroyed.
    let workspace = service(&dm, "Workspace");
    let house = dm.find_first_child(workspace, "House", false).unwrap().unwrap();
    let new_part = new_in(&mut dm, "Part", house, "Window");
    let saved = save_scene(&dm, &mut ids).unwrap();
    let after = doc(&saved.text);
    assert_eq!(named(&after, "Door")["id"], door, "kept its id");
    assert_eq!(named(&after, "Window")["id"], highest + 1, "the next free id");
    assert_eq!(ids.get(new_part), Some(highest + 1));

    let door_part = dm.find_first_child(house, "Door", false).unwrap().unwrap();
    dm.destroy(door_part).unwrap();
    let saved = save_scene(&dm, &mut ids).unwrap();
    let after = doc(&saved.text);
    assert_eq!(named(&after, "Window")["id"], highest + 1);
    assert_eq!(
        named(&after, "House").get("properties"),
        None,
        "PrimaryPart reads nil once the door is gone"
    );
    // The pointer to the destroyed door is nil too, so nothing refers outside the scene.
    assert_eq!(saved.warnings, []);
    let window = new_in(&mut dm, "Part", house, "Window2");
    save_scene(&dm, &mut ids).unwrap();
    assert_eq!(ids.get(window), Some(highest + 2), "a freed id is not reused");
}

#[test]
fn instances_that_are_not_archivable_are_left_out() {
    let (mut dm, part, _) = rich_game();
    dm.set_by_name(part, "Archivable", Value::Bool(false)).unwrap();
    let saved = save_scene(&dm, &mut SceneIds::new()).unwrap();
    let doc = doc(&saved.text);
    let names: Vec<&str> = doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(!names.contains(&"Lamp"), "its children go with it: {names:?}");
    assert!(names.contains(&"House"));
    // PrimaryPart and the ObjectValue pointed at it.
    assert_eq!(saved.warnings.len(), 2, "{:?}", saved.warnings);
    assert!(
        saved
            .warnings
            .iter()
            .all(|w| matches!(w, SceneWarning::ReferenceNotSaved { .. }))
    );
    assert!(named(&doc, "Target").get("properties").is_none());
}

#[test]
fn unknown_and_unfit_content_is_skipped_with_warnings() {
    let text = json!({
        "format": SCENE_FORMAT,
        "instances": [
            {"id": 1, "class": "Workspace", "properties": {"CurrentCamera": 9, "Gravity": 196.2}},
            {"id": 2, "parent": 1, "class": "Hologram", "name": "FromTheFuture"},
            {"id": 3, "parent": 2, "class": "Part", "name": "UnderTheHologram"},
            {"id": 4, "parent": 1, "class": "Part", "name": "Kept",
             "properties": {"Material": "Cheese", "Size": "big", "Position": [1.0, 2.0, 3.0], "Anchored": true},
             "attributes": {"Bad": {"Instance": 1}, "Good": {"Bool": true}}},
            {"id": 5, "parent": 77, "class": "Part"},
            {"id": 6, "parent": 1, "class": "BasePart"},
            {"id": 7, "class": "RunService"},
            {"id": 8, "parent": 1, "class": "ObjectValue", "properties": {"Value": 4}},
            {"id": 9, "parent": 4, "class": "Camera"},
        ],
    })
    .to_string();
    let (dm, loaded) = load(&text);
    let warnings = &loaded.warnings;
    let expected = [
        SceneWarning::UnknownProperty {
            id: 1,
            class: "Workspace",
            property: "Gravity".to_owned(),
        },
        SceneWarning::UnknownClass {
            id: 2,
            class: "Hologram".to_owned(),
        },
        SceneWarning::BadValue {
            id: 4,
            class: "Part",
            property: "Material",
            expected: "Enum.Material".to_owned(),
        },
        SceneWarning::UnknownProperty {
            id: 4,
            class: "Part",
            property: "Position".to_owned(),
        },
        SceneWarning::BadValue {
            id: 4,
            class: "Part",
            property: "Size",
            expected: "Vector3".to_owned(),
        },
        SceneWarning::BadAttribute {
            id: 4,
            name: "Bad".to_owned(),
        },
        SceneWarning::Orphan { id: 5, parent: 77 },
        SceneWarning::Misplaced {
            id: 6,
            class: "BasePart".to_owned(),
        },
        SceneWarning::Misplaced {
            id: 7,
            class: "RunService".to_owned(),
        },
    ];
    assert_eq!(warnings.as_slice(), expected);

    let workspace = service(&dm, "Workspace");
    let kept = dm.find_first_child(workspace, "Kept", false).unwrap().unwrap();
    assert_eq!(dm.get_by_name(kept, "Anchored"), Ok(Value::Bool(true)));
    assert_eq!(dm.attribute(kept, "Good"), Ok(Some(&Value::Bool(true))));
    assert_eq!(dm.find_first_child(workspace, "UnderTheHologram", true), Ok(None));
    // CurrentCamera points at the camera that came later in the file, under the part.
    let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace, "CurrentCamera") else {
        panic!("CurrentCamera is not set");
    };
    assert_eq!(dm.parent(camera), Ok(Some(kept)));
    let pointer = dm.find_first_child(workspace, "ObjectValue", false).unwrap().unwrap();
    assert_eq!(dm.get_by_name(pointer, "Value"), Ok(Value::Ref(Some(kept))));
}

#[test]
fn references_to_missing_or_wrong_instances_stay_nil() {
    let text = json!({
        "format": SCENE_FORMAT,
        "instances": [
            {"id": 1, "class": "Workspace", "properties": {"CurrentCamera": 2}},
            {"id": 2, "parent": 1, "class": "Part"},
            {"id": 3, "parent": 1, "class": "ObjectValue", "properties": {"Value": 40}},
        ],
    })
    .to_string();
    let (dm, loaded) = load(&text);
    assert_eq!(
        loaded.warnings,
        [
            SceneWarning::BadReference {
                id: 1,
                class: "Workspace",
                property: "CurrentCamera",
                target: 2,
            },
            SceneWarning::BadReference {
                id: 3,
                class: "ObjectValue",
                property: "Value",
                target: 40,
            },
        ]
    );
    let workspace = service(&dm, "Workspace");
    assert_eq!(dm.get_by_name(workspace, "CurrentCamera"), Ok(Value::Ref(None)));
}

#[test]
fn a_failed_load_leaves_the_game_untouched() {
    let (mut dm, ..) = rich_game();
    let before = save(&dm);
    let count = dm.len();
    let bad = [
        ("{ not json", "Syntax"),
        (r#"{"instances": []}"#, "MissingFormat"),
        (r#"{"format": 99, "instances": []}"#, "UnsupportedFormat"),
        (r#"{"format": 1, "instances": [{"id": 1}]}"#, "Malformed"),
        (
            r#"{"format": 1, "instances": [{"id": 1, "class": "Workspace"}, {"id": 1, "parent": 1, "class": "Part"}]}"#,
            "DuplicateId",
        ),
    ];
    for (text, kind) in bad {
        let error = load_scene(&mut dm, text).unwrap_err();
        assert!(format!("{error:?}").starts_with(kind), "{text}: {error:?}");
        assert_eq!(dm.len(), count, "{text}");
        assert_eq!(save(&dm), before, "{text}");
    }
}

#[test]
fn a_game_without_its_services_is_refused() {
    let registry = metallic_classes::shared_registry().unwrap();
    let mut dm = DataModel::new(Arc::clone(&registry));
    dm.create_root(registry.class_id("DataModel").unwrap()).unwrap();
    let error = load_scene(&mut dm, &save(&game())).unwrap_err();
    assert_eq!(error, SceneError::MissingService("Workspace"));
    assert_eq!(dm.len(), 1);
}

#[test]
fn bytes_load_like_text() {
    let (dm, ..) = rich_game();
    let text = save(&dm);
    let mut target = game();
    let loaded = load_scene_bytes(&mut target, text.as_bytes()).unwrap();
    assert_eq!(loaded.warnings, []);
    assert_eq!(save(&target), text);
    assert!(matches!(
        load_scene_bytes(&mut target, b"\xff\xfe"),
        Err(SceneError::Syntax(_))
    ));
}

#[test]
fn mesh_parts_keep_their_asset_ids() {
    let mut dm = game();
    let workspace = service(&dm, "Workspace");
    let statue = new_in(&mut dm, "MeshPart", workspace, "Statue");
    let mesh = "00112233445566778899aabbccddeeff";
    let texture = "ffeeddccbbaa99887766554433221100";
    dm.set_by_name(statue, "MeshId", Value::String(mesh.into())).unwrap();
    dm.set_by_name(statue, "TextureId", Value::String(texture.into()))
        .unwrap();
    dm.set_by_name(statue, "Size", Value::Vector3(Vector3::new(2.0, 3.0, 4.0)))
        .unwrap();
    // A mesh part without a texture leaves `TextureId` out of the file.
    let plain = new_in(&mut dm, "MeshPart", workspace, "Plain");
    dm.set_by_name(plain, "MeshId", Value::String(mesh.into())).unwrap();

    let text = save(&dm);
    let parsed = doc(&text);
    assert_eq!(named(&parsed, "Statue")["properties"]["MeshId"], json!(mesh));
    assert_eq!(named(&parsed, "Statue")["properties"]["TextureId"], json!(texture));
    assert!(named(&parsed, "Plain")["properties"].get("TextureId").is_none());

    let (loaded_dm, loaded) = load(&text);
    assert_eq!(loaded.warnings, []);
    assert_eq!(save(&loaded_dm), text, "save, load, save gives the same file");
    let workspace = service(&loaded_dm, "Workspace");
    let statue = loaded_dm.find_first_child(workspace, "Statue", false).unwrap().unwrap();
    assert_eq!(loaded_dm.class_name(statue), Ok("MeshPart"));
    assert_eq!(loaded_dm.get_by_name(statue, "MeshId"), Ok(Value::String(mesh.into())));
    assert_eq!(
        loaded_dm.get_by_name(statue, "TextureId"),
        Ok(Value::String(texture.into()))
    );
    assert_eq!(
        loaded_dm.get_by_name(statue, "Size"),
        Ok(Value::Vector3(Vector3::new(2.0, 3.0, 4.0)))
    );
}
