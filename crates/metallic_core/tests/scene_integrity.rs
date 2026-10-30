use metallic_core::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("metallic_scene_integrity_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn load_text(name: &str, text: &str) -> Result<World, CoreError> {
    let path = tmp(name);
    std::fs::write(&path, text).unwrap();
    load_scene(&path)
}

#[test]
fn duplicate_properties_are_rejected() {
    let result = load_text(
        "duplicate.ron",
        r#"(version: 1, roots: [(class: "Part", props: [("Name", String("first")), ("Name", String("second"))])])"#,
    );
    assert!(
        matches!(result, Err(CoreError::Scene(ref message)) if message.contains("duplicate property `Name`"))
    );
}

#[test]
fn invalid_builtin_markers_are_rejected() {
    for (name, text) in [
        (
            "builtin_part",
            r#"(version: 1, roots: [(class: "Part", builtin: true)])"#,
        ),
        (
            "duplicate_service",
            r#"(version: 1, roots: [(class: "Workspace", builtin: true), (class: "Workspace", builtin: true)])"#,
        ),
        (
            "misplaced_service",
            r#"(version: 1, roots: [(class: "Folder", children: [(class: "Workspace", builtin: true)])])"#,
        ),
    ] {
        assert!(
            matches!(load_text(name, text), Err(CoreError::Scene(_))),
            "{name}"
        );
    }
}

#[test]
fn unknown_fields_are_rejected_instead_of_losing_data() {
    for (name, text) in [
        ("root_typo", r#"(version: 1, roots: [], rotos: [])"#),
        (
            "node_typo",
            r#"(version: 1, roots: [(class: "Part", properties: [("Anchored", Bool(true))])])"#,
        ),
    ] {
        assert!(
            matches!(load_text(name, text), Err(CoreError::Scene(_))),
            "{name}"
        );
    }
}

#[test]
fn dangling_references_are_rejected() {
    let result = load_text(
        "dangling.ron",
        r#"(version: 1, roots: [(class: "Player", props: [("Character", Ref(Some(999)))])])"#,
    );
    assert!(
        matches!(result, Err(CoreError::Scene(ref message)) if message.contains("missing instance #999"))
    );
}

fn nested_world(depth: usize) -> World {
    let mut world = World::new_empty();
    let mut parent = None;
    for index in 0..depth {
        let node = world.create("Folder", parent).unwrap();
        world
            .set(node, "Name", Value::String(format!("Folder{index}")))
            .unwrap();
        parent = Some(node);
    }
    world
}

#[test]
fn deepest_supported_scene_round_trips() {
    let path = tmp("depth_limit.ron");
    save_scene(&nested_world(MAX_SCENE_DEPTH), &path).unwrap();
    let loaded = load_scene(&path).unwrap();
    let mut current = loaded
        .roots()
        .iter()
        .copied()
        .find(|&id| !loaded.is_builtin(id))
        .unwrap();
    for _ in 1..MAX_SCENE_DEPTH {
        current = loaded.children(current)[0];
    }
    assert!(loaded.children(current).is_empty());
}

#[test]
fn too_deep_save_keeps_previous_file_unchanged() {
    let path = tmp("too_deep.ron");
    std::fs::write(&path, "previous scene").unwrap();
    let result = save_scene(&nested_world(MAX_SCENE_DEPTH + 1), &path);
    assert!(matches!(result, Err(CoreError::Scene(ref message)) if message.contains("depth")));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous scene");
}

#[test]
fn excessive_depth_is_rejected_when_loading() {
    for depth in [MAX_SCENE_DEPTH + 1, 1_024] {
        let text = format!(
            "(version: 1, roots: [{}{}])",
            "(class: \"Folder\", children: [".repeat(depth),
            "]),".repeat(depth)
        );
        assert!(matches!(
            load_text(&format!("depth_{depth}.ron"), &text),
            Err(CoreError::Scene(_))
        ));
    }
}

#[test]
fn moved_builtin_save_is_rejected_before_replacing_previous_file() {
    let path = tmp("moved_builtin.ron");
    std::fs::write(&path, "previous complete scene").unwrap();
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    let folder = world.create("Folder", None).unwrap();
    world.set_parent(workspace, Some(folder)).unwrap();
    assert!(
        matches!(save_scene(&world, &path), Err(CoreError::Scene(ref message)) if message.contains("builtin"))
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "previous complete scene"
    );
}

#[test]
fn unicode_names_and_many_cross_references_round_trip() {
    let path = tmp("世界 with spaces.mscene");
    let mut world = World::new_with_services();
    let players = world.root("Players").unwrap();
    let workspace = world.root("Workspace").unwrap();
    let mut expected = Vec::new();
    for index in 0..512 {
        let spawn = world.create("SpawnLocation", Some(workspace)).unwrap();
        let name = format!("出生点 {index}");
        world
            .set(spawn, "Name", Value::String(name.clone()))
            .unwrap();
        let player = world.create("Player", Some(players)).unwrap();
        world
            .set(player, "RespawnLocation", Value::Ref(Some(spawn)))
            .unwrap();
        expected.push(name);
    }
    save_scene(&world, &path).unwrap();
    let loaded = load_scene(&path).unwrap();
    for (&player, expected_name) in loaded
        .children(loaded.root("Players").unwrap())
        .iter()
        .zip(expected)
    {
        let Value::Ref(Some(spawn)) = loaded.get(player, "RespawnLocation").unwrap() else {
            panic!("reference disappeared");
        };
        assert_eq!(
            loaded.get(spawn, "Name").unwrap(),
            Value::String(expected_name)
        );
    }
}
