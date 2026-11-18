//! `Sound` and `SoundService` (#811) driven through the engine: a WAV in a project loaded off the main thread, played
//! through the mixer as the properties say, `TimePosition` and the events written back, looping, the end, seeking,
//! pausing and `PlayLocalSound`. No device is opened: the test renders the mixer itself.
#![cfg(feature = "luau")]

use std::path::Path;
use std::time::{Duration, Instant};

use metallic_engine::metallic_project::{Project, Template};
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};
use metallic_script::{CollectOutput, OutputLevel};

const SOUND: &str = "Assets/beep.wav";

/// A WAV of `seconds` of a constant half-loud tone at 8 kHz, mono.
fn wav(seconds: f64) -> Vec<u8> {
    let rate = 8000u32;
    let frames = (seconds * f64::from(rate)) as u32;
    let data = frames * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for _ in 0..frames {
        out.extend_from_slice(&16384i16.to_le_bytes());
    }
    out
}

/// A project holding a one-second sound, and the sound's asset id.
fn project(test: &str) -> (Project, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sounds").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("setup");
    let template =
        Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")).expect("setup");
    let mut project = Project::create(dir.join("Game.mtproj"), &template, "Sounds").expect("setup");
    project.write(SOUND, wav(1.0)).expect("setup");
    project.refresh_assets().expect("setup");
    let id = project.assets().id(SOUND).expect("setup").to_string();
    (project, id)
}

/// A started game that reads `project`'s sounds.
fn started(project: &Project) -> Engine<CollectOutput> {
    let config = EngineConfig::new(RunContext::Local).with_sources(MemorySources::new());
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    engine.set_sound_assets(project.asset_reader());
    engine.start().expect("start");
    engine
}

/// Runs `code` in a script that stays (its connections too), and returns what it prints.
fn run(engine: &mut Engine<CollectOutput>, code: &str) -> String {
    let (dm, host) = engine.parts_mut();
    let class = dm.registry().class_id("Script").expect("Script");
    let script = dm.create(class).expect("runner");
    let service = dm.registry().class_id("ServerScriptService").expect("service class");
    let service = dm.service(service).expect("ServerScriptService");
    dm.set_parent(script, Some(service)).expect("parent");
    let before = host.output().lines.len();
    host.run_script(dm, script, "Run.luau", code).expect("runs");
    let printed: Vec<String> = host.output().lines[before..]
        .iter()
        .map(|line| {
            assert_eq!(line.level, OutputLevel::Print, "failed: {}", line.message);
            line.message.clone()
        })
        .collect();
    host.output_mut().lines.truncate(before);
    printed.join("\n")
}

/// Frames until the sound files are loaded, then one more so the sounds see them.
fn load(engine: &mut Engine<CollectOutput>) {
    let until = Instant::now() + Duration::from_secs(10);
    engine.tick();
    while !engine.sounds().idle() {
        assert!(Instant::now() < until, "the sound never loaded");
        std::thread::sleep(Duration::from_millis(5));
        engine.tick();
    }
    engine.tick();
}

/// Plays `seconds` of the mixer at 8 kHz (as a device would) and runs a frame, so the engine sees where it is.
fn listen(engine: &mut Engine<CollectOutput>, seconds: f64) -> Vec<f32> {
    let mut out = vec![0.0; (seconds * 8000.0) as usize * 2];
    engine.sounds().mixer().render(&mut out, 2, 8000);
    engine.tick();
    out
}

#[test]
fn a_sound_plays_loops_ends_and_seeks() {
    let (project, id) = project("plays");
    let mut engine = started(&project);
    run(
        &mut engine,
        &format!(
            "local s = Instance.new(\"Sound\")
             s.Name = \"Beep\"
             s.SoundId = \"{id}\"
             s.Volume = 1
             s.Parent = workspace
             _G.log = {{}}
             for _, name in {{\"Loaded\", \"Played\", \"Ended\", \"Paused\", \"Resumed\", \"Stopped\"}} do
                 s[name]:Connect(function(soundId) table.insert(_G.log, name) end)
             end
             s.DidLoop:Connect(function(soundId, n) table.insert(_G.log, \"DidLoop \" .. n) end)
             s:Play()"
        ),
    );
    load(&mut engine);
    assert_eq!(
        run(
            &mut engine,
            "print(workspace.Beep.TimeLength, workspace.Beep.IsLoaded, workspace.Beep.IsPlaying)"
        ),
        "1 true true"
    );
    // Half a second of it: at half volume, and TimePosition follows.
    let heard = listen(&mut engine, 0.5);
    assert!(heard.iter().all(|&s| (s - 0.5).abs() < 1e-3), "{:?}", &heard[..4]);
    assert_eq!(run(&mut engine, "print(workspace.Beep.TimePosition)"), "0.5");

    // Paused, nothing plays and the place is kept; resumed, it goes on.
    run(&mut engine, "workspace.Beep:Pause()");
    engine.tick();
    assert!(listen(&mut engine, 0.25).iter().all(|&s| s == 0.0));
    run(&mut engine, "workspace.Beep:Resume()");
    engine.tick();
    listen(&mut engine, 0.25);
    assert_eq!(run(&mut engine, "print(workspace.Beep.TimePosition)"), "0.75");

    // To the end: it stops and goes back to the start.
    listen(&mut engine, 0.5);
    assert_eq!(
        run(
            &mut engine,
            "print(workspace.Beep.Playing, workspace.Beep.TimePosition, workspace.Beep.IsPlaying)"
        ),
        "false 0 false"
    );

    // Looped, from a place a script chose, at double speed.
    run(
        &mut engine,
        "workspace.Beep.Looped = true
         workspace.Beep.PlaybackSpeed = 2
         workspace.Beep.TimePosition = 0.5
         workspace.Beep:Play()",
    );
    engine.tick();
    listen(&mut engine, 0.5);
    assert_eq!(run(&mut engine, "print(workspace.Beep.TimePosition)"), "0.5");
    run(&mut engine, "workspace.Beep:Stop()");
    engine.tick();
    assert_eq!(engine.sounds().mixer().voice_count(), 0);
    assert_eq!(
        run(&mut engine, "print(table.concat(_G.log, \",\"))"),
        "Played,Loaded,Paused,Resumed,Ended,Played,DidLoop 1,Stopped"
    );
}

#[test]
fn sounds_play_only_in_the_game_or_for_play_local_sound() {
    let (project, id) = project("where");
    let mut engine = started(&project);
    run(
        &mut engine,
        &format!(
            "_G.loose = Instance.new(\"Sound\")
             _G.loose.SoundId = \"{id}\"
             _G.loose:Play()
             local kept = Instance.new(\"Sound\")
             kept.SoundId = \"{id}\"
             kept.Name = \"Kept\"
             kept.Parent = game:GetService(\"SoundService\")"
        ),
    );
    load(&mut engine);
    assert_eq!(
        engine.sounds().mixer().voice_count(),
        0,
        "a sound with no parent is silent"
    );
    run(
        &mut engine,
        "game:GetService(\"SoundService\"):PlayLocalSound(_G.loose)",
    );
    engine.tick();
    assert_eq!(engine.sounds().mixer().voice_count(), 1);
    run(&mut engine, "game:GetService(\"SoundService\").Kept.Playing = true");
    engine.tick();
    assert_eq!(engine.sounds().mixer().voice_count(), 2);
    // Gone with the game.
    engine.stop();
    assert_eq!(engine.sounds().mixer().voice_count(), 0);
    // A file that is not there is reported once.
    let mut engine = started(&project);
    run(
        &mut engine,
        "local s = Instance.new(\"Sound\")
         s.SoundId = \"0123456789abcdef0123456789abcdef\"
         s.Parent = workspace
         s:Play()",
    );
    load(&mut engine);
    assert_eq!(engine.sounds().warnings().len(), 1, "{:?}", engine.sounds().warnings());
}
