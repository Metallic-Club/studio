//! End-to-end checks of `metallic-player` and `Game::load`.
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Plays the Parkour template headless with the smoke scene: its test script teleports the
/// character into Checkpoint 2, kills it, checks the respawn at Checkpoint 2, touches the Finish
/// twice and checks exactly one `Finished` event arrived.
#[test]
fn parkour_smoke_scene_passes_headless() {
    let out = Command::new(env!("CARGO_BIN_EXE_metallic-player"))
        .arg(repo_root().join("templates/parkour"))
        .args([
            "--scene",
            "tests/smoke.mscene",
            "--headless",
            "--frames",
            "1200",
        ])
        .output()
        .expect("run metallic-player");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    assert!(
        stdout.contains("PARKOUR_SMOKE_OK"),
        "no PARKOUR_SMOKE_OK\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    for line in stdout.lines().chain(stderr.lines()) {
        assert!(
            !line.to_lowercase().contains("error"),
            "error line: {line}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// A project whose `main_scene` file does not exist fails to load with a clear error, and the
/// player exits with code 1.
#[test]
fn game_load_rejects_missing_main_scene() {
    let dir = std::env::temp_dir().join(format!("metallic_missing_scene_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(
        dir.join("metallic.toml"),
        "name = \"Broken\"\nmain_scene = \"nowhere.mscene\"\n",
    )
    .expect("write metallic.toml");

    let err = match metallic_runtime::Game::load(&dir) {
        Ok(_) => panic!("loading a project without its main scene must fail"),
        Err(e) => e,
    };
    assert!(
        matches!(err, metallic_runtime::RuntimeError::MissingScene(_)),
        "unexpected error: {err}"
    );
    assert!(err.to_string().contains("nowhere.mscene"), "{err}");

    let out = Command::new(env!("CARGO_BIN_EXE_metallic-player"))
        .arg(&dir)
        .args(["--headless", "--frames", "1"])
        .output()
        .expect("run metallic-player");
    assert_eq!(out.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}
