//! #803: an exported game is its client alone. The player renamed after a game, without the game's `game.pak` beside
//! it, says so and closes; it never runs a built-in scene.

#[test]
fn a_renamed_player_without_its_pak_says_so_and_closes() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("exported_client");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let game = dir.join(format!("Rover{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_metallic_player"), &game).unwrap();
    let ran = std::process::Command::new(&game)
        .args(["--scene", "demo"])
        .env("METALLIC_NO_DIALOGS", "1")
        .output()
        .unwrap();
    assert!(!ran.status.success(), "it closes");
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(said.contains("game.pak"), "{said}");
}
