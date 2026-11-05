//! Metallic Player: runs a game, standalone or on the platform.
//!
//! It is a library so that Studio (Play in Studio) and the Launcher run games through exactly the code that ships.
//! The `metallic_player` binary is a thin wrapper around it: `cargo run -p metallic_player` plays the built-in demo
//! (see the README for its flags).
//!
//! # Running a game
//!
//! A [`Player`] is a game in the engine ([`metallic_engine::Engine`]), made from a [`PlayerConfig`]: which game
//! (a built-in [`Scene`], or a `.mtproj` project: [`PlayerConfig::project`]), the window, the graphics [`Preset`] and
//! when to stop. [`Player::new`] loads the scene (a project's start scene, with its scripts), installs the renderer
//! and starts the game. Then one of three things drives
//! its frames, depending on who owns the event loop and the window:
//!
//! | Host | Owns | Drives the Player with |
//! |---|---|---|
//! | the `metallic_player` binary | nothing: the Player opens its window and runs the platform's event loop | [`Player::run`] |
//! | the Launcher (M8), whose web view runs its own loop | the event loop; the Player still opens its own window | [`Player::into_app`]: forward the loop's callbacks to the [`PlayerApp`] (an [`AppHandler`](metallic_engine::metallic_platform::AppHandler)) |
//! | Studio's game view (M6), a panel in Studio's dock | the event loop and the window | [`Player::engine_mut`]: [`attach_window`](metallic_engine::Engine::attach_window), [`resize`](metallic_engine::Engine::resize) and [`push_input`](metallic_engine::Engine::push_input), then [`Player::frame`] once per loop iteration |
//!
//! ```no_run
//! use metallic_player::{Player, PlayerConfig, Scene};
//!
//! let report = Player::new(PlayerConfig::new(Scene::Demo))?.run()?;
//! println!("{} frames", report.total_frames);
//! # Ok::<(), metallic_player::PlayerError>(())
//! ```
//!
//! Scripts print to the standard streams; [`Player::with_output`] sends their output anywhere else (Studio's
//! Output panel).
//!
//! # Measuring
//!
//! The renderer the Player installs keeps the engine's frame statistics ([`metallic_engine::perf`]): frame times
//! with p50 / p99 / 1% low, script, physics and render CPU time, GPU time per pass and the draws per pass
//! ([`Player::summary`]). F3 (or `--overlay`) shows them on screen. `--bench <frames>` runs that many frames after
//! a short warm-up and prints the [`Report`] ([`stats`]) as JSON. `--scene` also runs the engine's stress scenes
//! ([`metallic_engine::stress`]), whose moving parts the Player moves every frame.

mod config;
mod player;
mod scene;
pub mod stats;

pub use config::{
    DEFAULT_FRAME_CAP, DEFAULT_WARMUP_FRAMES, GraphicsSettings, PLAYER_APP_ID, PlayerConfig, Preset, UnknownPreset,
};
pub use player::{Player, PlayerApp, PlayerError};
pub use scene::{STRESS_SEED, Scene, UnknownScene};
pub use stats::Report;

/// The player's version, as shown in logs and in the platform UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use metallic_engine::metallic_core::Value;
    use metallic_engine::metallic_platform::{ButtonState, Key, KeyCode, KeyEvent, WindowEvent, WindowId};
    use metallic_engine::metallic_script::{CollectOutput, OutputLevel};
    use metallic_engine::{InputEvent, RunContext, RunState};

    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    fn demo() -> Player<CollectOutput> {
        Player::with_output(PlayerConfig::new(Scene::Demo), CollectOutput::new()).expect("the demo starts")
    }

    fn assert_no_errors(player: &Player<CollectOutput>) {
        let output = player.engine().host().output();
        let problems: Vec<&str> = output
            .of_level(OutputLevel::Error)
            .chain(output.of_level(OutputLevel::Warn))
            .map(|line| line.message.as_str())
            .collect();
        assert!(problems.is_empty(), "script errors: {problems:#?}");
    }

    /// The value of `prop` on `Workspace`'s child called `name`.
    fn read(player: &Player<CollectOutput>, name: &str, prop: &str) -> Value {
        let dm = player.engine().data_model();
        let registry = dm.registry();
        let workspace = dm
            .service(registry.class_id("Workspace").expect("Workspace class"))
            .expect("Workspace");
        let child = dm
            .find_first_child(workspace, name, false)
            .expect("Workspace")
            .unwrap_or_else(|| panic!("no {name} in Workspace"));
        dm.get_by_name(child, prop).expect("property")
    }

    fn key(code: KeyCode, state: ButtonState) -> InputEvent {
        InputEvent::Window {
            window: WindowId::from_raw(1),
            event: WindowEvent::Key(KeyEvent {
                key_code: code,
                key: Key::Character("w".into()),
                text: None,
                state,
                repeat: false,
            }),
        }
    }

    #[test]
    fn the_demo_builds_its_scene_and_runs_without_script_errors() {
        let mut player = demo();
        assert_eq!(player.engine().state(), RunState::Running);
        assert_no_errors(&player);
        let printed: Vec<&str> = player
            .engine()
            .host()
            .output()
            .of_level(OutputLevel::Print)
            .map(|line| line.message.as_str())
            .collect();
        assert!(
            printed.iter().any(|line| line.starts_with("Metallic demo:")),
            "{printed:?}"
        );
        for name in ["Baseplate", "NeonWedge", "GlassBall", "Sun", "Spot", "LampBulb"] {
            read(&player, name, "Name");
        }

        let before = read(&player, "Car1", "CFrame");
        for _ in 0..30 {
            player.frame(FRAME);
        }
        assert_no_errors(&player);
        assert_ne!(read(&player, "Car1", "CFrame"), before, "Heartbeat moves the carousel");
    }

    #[test]
    fn the_camera_orbits_until_wasd_moves_it() {
        let mut player = demo();
        player.frame(FRAME);
        let orbit = read(&player, "Camera", "CFrame");
        player.frame(FRAME);
        assert_ne!(read(&player, "Camera", "CFrame"), orbit, "the camera orbits on its own");

        // W leaves the orbit and flies forward; letting go stops the camera.
        player.engine_mut().push_input(key(KeyCode::KeyW, ButtonState::Pressed));
        for _ in 0..10 {
            player.frame(FRAME);
        }
        let flying = read(&player, "Camera", "CFrame");
        player.frame(FRAME);
        assert_ne!(
            read(&player, "Camera", "CFrame"),
            flying,
            "held W keeps moving the camera"
        );
        player
            .engine_mut()
            .push_input(key(KeyCode::KeyW, ButtonState::Released));
        player.frame(FRAME);
        let stopped = read(&player, "Camera", "CFrame");
        player.frame(FRAME);
        assert_eq!(
            read(&player, "Camera", "CFrame"),
            stopped,
            "no keys, no orbit: the camera holds still"
        );
        assert_no_errors(&player);
    }

    #[test]
    fn the_preset_reaches_scripts_before_they_run() {
        use metallic_engine::metallic_core::EnumItem;

        let graphics = GraphicsSettings {
            lod_bias: Some(3.0),
            ..GraphicsSettings::new(Preset::Auto)
        };
        let config = PlayerConfig::new(Scene::Demo)
            .with_graphics(graphics)
            .with_preset(Preset::Low);
        let player = Player::with_output(config, CollectOutput::new()).expect("the demo starts");
        let dm = player.engine().data_model();
        let service = dm
            .service(dm.registry().class_id("UserGameSettings").expect("the class"))
            .expect("the service");
        let read = |name: &str| dm.get_by_name(service, name).expect("a property");
        assert_eq!(
            read("GraphicsPreset"),
            Value::Enum(EnumItem::new("GraphicsPreset", "Low")),
            "--preset in place of the file's"
        );
        assert_eq!(read("QualityLevel"), Value::Int(1));
        assert_eq!(read("RenderScale"), Value::Number(0.75));
        assert_eq!(read("LodBias"), Value::Number(3.0), "the file's overrides stay");
        assert_eq!(read("FrameRateCap"), Value::Int(i64::from(DEFAULT_FRAME_CAP)));
    }

    #[test]
    fn a_project_made_from_the_baseplate_template_plays() {
        use metallic_engine::metallic_project::{Project, Template};

        let dir = std::env::temp_dir().join(format!("metallic-player-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let path = dir.join("Baseplate.mtproj");
        let template = Template::from_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../templates/baseplate-3d"))
            .expect("the template");
        let mut project = Project::create(&path, &template, "Baseplate").expect("a new project");

        // A script in the start scene whose source is a member of the project.
        // Built in a headless engine's game that never starts.
        let mut editor =
            metallic_engine::Engine::new(metallic_engine::EngineConfig::new(RunContext::Local)).expect("an engine");
        let dm = editor.data_model_mut();
        project.load_start_scene(dm).expect("the start scene");
        let registry = dm.registry().clone();
        let service = dm
            .service(registry.class_id("ServerScriptService").expect("the class"))
            .expect("the service");
        let script = dm
            .create(registry.class_id("Script").expect("the class"))
            .expect("a script");
        dm.set_name(script, "Hello".into()).expect("named");
        dm.set_by_name(script, "Source", Value::String("Scripts/Hello.luau".into()))
            .expect("its source");
        dm.set_parent(script, Some(service)).expect("parented");
        let scene = metallic_engine::metallic_project::save_scene(dm, &mut Default::default()).expect("saved");
        let start = project.manifest().start_scene.clone();
        project.write(&start, scene.text).expect("the scene");
        project
            .write("Scripts/Hello.luau", "print('hello from the project')")
            .expect("the script");
        project.save().expect("saved");
        drop(project);

        let config = PlayerConfig::new(Scene::Demo).with_project(&path);
        let mut player = Player::with_output(config, CollectOutput::new()).expect("the project plays");
        for name in ["Baseplate", "SpawnLocation"] {
            read(&player, name, "Name");
        }
        player.frame(FRAME);
        assert_no_errors(&player);
        let printed: Vec<&str> = player
            .engine()
            .host()
            .output()
            .of_level(OutputLevel::Print)
            .map(|line| line.message.as_str())
            .collect();
        assert_eq!(
            printed,
            ["hello from the project"],
            "the project's script ran, not the demo's"
        );

        let error = Player::with_output(
            PlayerConfig::new(Scene::Demo).with_project(dir.join("Missing.mtproj")),
            CollectOutput::new(),
        )
        .expect_err("no such file");
        assert!(
            error.to_string().ends_with("Missing.mtproj: no such project file"),
            "{error}"
        );
        drop(player);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_player_records_nothing_until_it_draws() {
        let mut player = demo();
        player.frame(FRAME);
        let summary = player.summary();
        assert_eq!((summary.frames, summary.gpu_frames), (0, 0), "no window, no renderer");
        assert!(player.renderer().borrow().renderer().is_none());
    }
}
