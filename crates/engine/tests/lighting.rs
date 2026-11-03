//! `Lighting` driven from Luau reaches the renderer (#81): a script that changes `ClockTime` moves the sun the
//! renderer lights with, and the renderer reads `Lighting` again only in frames after it changed.
//!
//! It runs headless and needs no GPU: it follows the path [`metallic_engine::SceneRenderer`] takes each frame (the
//! scene's sync on its change consumer, which watches the `Lighting` service, then [`LightingReader`] when the sync
//! reports it), on the CPU side of the scene ([`InstanceScene`]).
#![cfg(all(feature = "render", feature = "luau"))]

use metallic_core::glam::Vec3;
use metallic_core::{DataModel, Id, Value};
use metallic_engine::{Engine, EngineConfig, MemorySources, RunContext};
use metallic_render::scene::RenderComponents;
use metallic_render::{InstanceScene, LightingReader, SceneLighting};
use metallic_script::{CollectOutput, OutputLevel};

/// Sets the clock to 6:00 when the game starts, then moves it on by an hour every second of game time, but only
/// while `_G.running` is true; publishes `Lighting:GetSunDirection()` in `_G.sun` after each change.
const DAY: &str = r#"
local Lighting = game:GetService("Lighting")
local RunService = game:GetService("RunService")
Lighting.ClockTime = 6
_G.running = true
_G.sun = Lighting:GetSunDirection()
RunService.Heartbeat:Connect(function(deltaTime)
    if _G.running then
        Lighting.ClockTime += deltaTime
        _G.sun = Lighting:GetSunDirection()
    end
end)
"#;

fn engine() -> Engine<CollectOutput> {
    let sources = MemorySources::new().with("Day.luau", DAY);
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    let dm = engine.data_model_mut();
    let registry = dm.registry().clone();
    let service = dm
        .service(registry.class_id("ServerScriptService").expect("class"))
        .expect("service");
    let script = dm.create(registry.class_id("Script").expect("Script")).expect("script");
    dm.set_by_name(script, "Source", Value::String("Day.luau".into()))
        .expect("source");
    dm.set_parent(script, Some(service)).expect("parent");
    engine
}

fn lighting(dm: &DataModel) -> Id {
    dm.service(dm.registry().class_id("Lighting").expect("Lighting"))
        .expect("a Lighting")
}

/// The scene `SceneRenderer` attaches: the parts in `Workspace`, watching `Lighting`.
fn watching_scene(dm: &mut DataModel) -> InstanceScene {
    let workspace = dm
        .service(dm.registry().class_id("Workspace").expect("Workspace"))
        .expect("a Workspace");
    let mut scene = InstanceScene::attach(dm, Some(workspace), &RenderComponents::builtin()).expect("a scene");
    scene.watch(lighting(dm));
    scene
}

/// What the renderer's side of a frame does: sync the scene, and read `Lighting` again if the sync says it changed.
/// Returns the lighting when it was read.
fn render_side(scene: &mut InstanceScene, reader: &mut LightingReader, dm: &mut DataModel) -> Option<SceneLighting> {
    let stats = scene.sync(dm);
    (stats.watched > 0).then(|| reader.read(dm, lighting(dm)))
}

/// Runs a probe script and returns what it printed.
fn probe(engine: &mut Engine<CollectOutput>, expr: &str) -> String {
    let (dm, host) = engine.parts_mut();
    let script = dm
        .create(dm.registry().class_id("Script").expect("Script"))
        .expect("probe");
    let before = host.output().lines.len();
    host.run_script(dm, script, "Probe.luau", &format!("print(tostring({expr}))"))
        .expect("probe runs");
    let line = host.output().lines.get(before).expect("the probe printed").clone();
    assert_eq!(line.level, OutputLevel::Print, "probe failed: {}", line.message);
    host.output_mut().lines.truncate(before);
    line.message
}

fn assert_no_errors(engine: &Engine<CollectOutput>) {
    let errors: Vec<&str> = engine
        .host()
        .output()
        .of_level(OutputLevel::Error)
        .map(|l| l.message.as_str())
        .collect();
    assert!(errors.is_empty(), "script errors: {errors:#?}");
}

#[test]
fn a_script_changing_clock_time_moves_the_sun() {
    let mut engine = engine();
    let mut scene = watching_scene(engine.data_model_mut());
    let mut reader = LightingReader::new();

    // Before the game starts: the first sync reports Lighting (everything is new to the scene), at its default.
    let first = render_side(&mut scene, &mut reader, engine.data_model_mut()).expect("read on the first sync");
    assert!(
        first
            .sun_direction
            .abs_diff_eq(SceneLighting::default().sun_direction, 1e-5),
        "{first:?}"
    );

    // The script sets the clock to 6:00 as the game starts: the sun is on the eastern horizon.
    engine.start().expect("start");
    assert_no_errors(&engine);
    let dawn = render_side(&mut scene, &mut reader, engine.data_model_mut()).expect("read after the change");
    assert!(dawn.sun_direction.abs_diff_eq(Vec3::X, 1e-4), "{dawn:?}");

    // Six seconds of frames move the clock to 12:00, the sun rising every frame, as the script sees it too.
    let mut last = dawn.sun_direction;
    for frame in 1..=360 {
        engine.tick();
        let lit = render_side(&mut scene, &mut reader, engine.data_model_mut())
            .unwrap_or_else(|| panic!("Lighting changed in frame {frame} but was not read"));
        assert!(lit.sun_direction.y > last.y, "the sun did not rise in frame {frame}");
        last = lit.sun_direction;
    }
    assert_no_errors(&engine);
    let clock: f64 = probe(&mut engine, "game:GetService(\"Lighting\").ClockTime")
        .parse()
        .expect("a number");
    assert!((clock - 12.0).abs() < 1e-6, "{clock}");
    let noon = sun_direction_of(&engine);
    assert!(
        last.abs_diff_eq(noon, 1e-6),
        "the renderer's sun {last} is not Lighting's {noon}"
    );
    assert!(
        noon.y > 0.7 && noon.z > 0.6,
        "at noon the sun is high in the south: {noon}"
    );
    let scripted: Vec<f32> = ["X", "Y", "Z"]
        .iter()
        .map(|axis| probe(&mut engine, &format!("_G.sun.{axis}")).parse().expect("a number"))
        .collect();
    assert!(
        Vec3::from_slice(&scripted).abs_diff_eq(noon, 1e-6),
        "GetSunDirection {scripted:?} disagrees with SunDirection {noon}"
    );

    // While Lighting stays as it is, the renderer does not read it.
    probe(&mut engine, "(function() _G.running = false return true end)()");
    for _ in 0..10 {
        engine.tick();
        assert_eq!(render_side(&mut scene, &mut reader, engine.data_model_mut()), None);
    }
}

#[test]
fn every_lighting_property_reaches_the_renderer() {
    let mut engine = engine();
    engine.start().expect("start");
    let mut scene = watching_scene(engine.data_model_mut());
    let mut reader = LightingReader::new();
    render_side(&mut scene, &mut reader, engine.data_model_mut()).expect("the first read");
    probe(
        &mut engine,
        r#"(function()
            _G.running = false
            local Lighting = game:GetService("Lighting")
            Lighting.Brightness = 3
            Lighting.Ambient = Color3.new(0.1, 0.2, 0.3)
            Lighting.OutdoorAmbient = Color3.new(0.4, 0.4, 0.4)
            Lighting.SkyColor = Color3.new(0.2, 0.3, 0.9)
            Lighting.HorizonColor = Color3.new(0.9, 0.8, 0.7)
            Lighting.GeographicLatitude = 0
            Lighting.ClockTime = 12
            return true
        end)()"#,
    );
    assert_no_errors(&engine);
    let lit = render_side(&mut scene, &mut reader, engine.data_model_mut()).expect("read after the changes");
    assert!(lit.sun_direction.abs_diff_eq(Vec3::Y, 1e-5), "{lit:?}");
    assert_eq!(lit.brightness, 3.0);
    assert_eq!(lit.ambient, metallic_core::Color3::new(0.1, 0.2, 0.3));
    assert_eq!(lit.outdoor_ambient, metallic_core::Color3::new(0.4, 0.4, 0.4));
    assert_eq!(lit.sky_color, metallic_core::Color3::new(0.2, 0.3, 0.9));
    assert_eq!(lit.horizon_color, metallic_core::Color3::new(0.9, 0.8, 0.7));
}

fn sun_direction_of(engine: &Engine<CollectOutput>) -> Vec3 {
    let dm = engine.data_model();
    match dm.get_by_name(lighting(dm), "SunDirection") {
        Ok(Value::Vector3(v)) => v,
        other => panic!("{other:?}"),
    }
}
