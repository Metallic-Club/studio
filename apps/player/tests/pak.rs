//! #783: the Player opens a game's build, its `game.pak`, as it opens a `.mtproj`: the start scene loads from the pak
//! alone.

use std::path::Path;

use metallic_engine::metallic_classes;
use metallic_engine::metallic_project::{Pak, Project, Template};

#[test]
fn a_game_s_pak_opens_as_its_project_does() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("player_pak");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let template =
        Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")).unwrap();
    let mut project = Project::create(dir.join("Rover.mtproj"), &template, "Rover").unwrap();
    project.save().unwrap();
    let pak = dir.join("game.pak");
    Pak::pack(&mut project, &pak).unwrap();

    let mut game = metallic_player::open_game(&pak).unwrap();
    assert_eq!(game.manifest().name, "Rover");
    let mut dm = metallic_classes::new_game().unwrap();
    game.load_start_scene(&mut dm).unwrap();
    let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
    assert!(dm.find_first_child(workspace, "Baseplate", false).unwrap().is_some());
    // A project opens too, by its extension.
    let opened = metallic_player::open_game(&dir.join("Rover.mtproj")).unwrap();
    assert_eq!(opened.manifest().name, "Rover");
}
