//! Render scale and dynamic resolution on a real adapter (#84): the scene drawn through the upscale pass at a scale
//! of 1 is the image drawn straight into the target; at half and double scale it is the same image, softer, and the
//! local lights light the same places (their clusters follow the size drawn); a frame drawn into the corner of
//! larger textures (dynamic resolution below its highest scale) is the image of textures of its own size; and the UI
//! stays at the target's resolution. Both instance paths. Skips when the machine has no adapter (see
//! `support::gpu`).

mod support;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, Id, Value};
use metallic_render::resolution::ResolutionSettings;
use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, LightingReader, OffscreenTarget, RenderContext, Renderer, RendererSettings,
    wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Dynamic resolution between `min` and `max` that never moves: without GPU timing it has nothing to go by, so it
/// stays at `start`.
fn held(start: f32, min: f32, max: f32) -> ResolutionSettings {
    ResolutionSettings {
        scale: start,
        ..ResolutionSettings::dynamic(1000.0, min, max)
    }
}

/// Makes it night and adds four point lights over the ground, one in each quarter of the demo's view.
fn light(game: &mut Game) {
    let lighting = lighting_service(game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(0.0))
        .expect("ClockTime");
    let class = game.dm.registry().class_id("PointLight").expect("PointLight");
    for (x, z, color) in [
        (-7.0, -6.0, [255, 120, 90]),
        (7.0, -6.0, [120, 255, 120]),
        (-5.0, 5.0, [120, 160, 255]),
        (5.0, 5.0, [255, 230, 160]),
    ] {
        let dm = &mut game.dm;
        let id = dm.create(class).expect("a light");
        dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(Vec3::new(x, 2.0, z))))
            .expect("CFrame");
        let [r, g, b] = color;
        dm.set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(r, g, b)))
            .expect("Color");
        dm.set_by_name(id, "Brightness", Value::Number(3.0))
            .expect("Brightness");
        dm.set_by_name(id, "Range", Value::Number(7.0)).expect("Range");
        dm.set_parent(id, Some(game.workspace)).expect("parented");
    }
}

/// A grey floor at night lit by the four lights, seen from above: smooth pools of light and little else.
fn lit_floor() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [170, 168, 160]),
        Vec3::new(120.0, 1.0, 120.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, 14.0, 16.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        60.0,
    );
    light(&mut game);
    game
}

/// The image tests' demo scene (every shape and material, see-through parts) at night under the four lights.
fn lit_demo() -> Game {
    let mut game = Game::new();
    scenes::demo(&mut game, Vec3::ZERO);
    light(&mut game);
    game
}

fn lighting_service(game: &Game) -> Id {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    game.dm.service(class).expect("a Lighting")
}

/// Draws `game` at `resolution` for a few frames (the lights fully faded in) and reads the target back.
fn render(context: &RenderContext, game: &mut Game, resolution: ResolutionSettings) -> Vec<[u8; 4]> {
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            gpu_timing: false,
            resolution,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.set_light_fade_step(Some(1.0));
    renderer.set_lighting(LightingReader::new().read(&game.dm, lighting_service(game)));
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    for _ in 0..3 {
        let camera = reader.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&game.dm, camera.position);
        }
        scene.sync(&mut game.dm);
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
    }
    if resolution.scaled() {
        let (min, max) = resolution.range();
        let stats = renderer.resolution_stats();
        assert_eq!(stats.scale, resolution.scale.clamp(min, max), "drawn at another scale");
        let drawn = metallic_render::resolution::scaled_size(SIZE, SIZE, stats.scale);
        assert_eq!((stats.width, stats.height), drawn);
    }
    read_back_rgba8(context, target.texture())
}

fn contexts() -> Vec<(&'static str, RenderContext)> {
    let mut contexts = Vec::new();
    if let Some(context) = gpu() {
        contexts.push(("storage", context));
    }
    let options = ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    };
    if let Some(context) = gpu_with(&options) {
        contexts.push(("vertex", context));
    }
    contexts
}

#[test]
fn a_scale_of_one_through_the_upscale_is_the_native_image() {
    for (path, context) in contexts() {
        let mut game = lit_demo();
        let native = render(&context, &mut game, ResolutionSettings::NATIVE);
        let upscaled = render(&context, &mut game, held(1.0, 1.0, 1.0));
        let diff = scenes::compare(&native, &upscaled, 1);
        eprintln!("{path}: scale 1 through the upscale: {diff:?}");
        // Each output pixel reads its own texel's centre: the same colour, give or take the sRGB round trip.
        assert!(diff.max <= 1, "{path}: {diff:?}");
    }
}

#[test]
fn at_half_and_double_scale_it_is_the_same_image() {
    for (path, context) in contexts() {
        for (name, mut game) in [("lit floor", lit_floor()), ("lit demo", lit_demo())] {
            let native = render(&context, &mut game, ResolutionSettings::NATIVE);
            for scale in [0.5, 2.0] {
                let scaled = render(&context, &mut game, ResolutionSettings::fixed(scale));
                let diff = scenes::compare(&native, &scaled, 12);
                eprintln!("{path}, {name}: scale {scale} against native: {diff:?}");
                // Softer at half scale, so edges differ; a cluster lookup at the wrong size would leave whole pools
                // of light out (the clusters of half the screen holding the lights of the other half).
                assert!(
                    diff.over <= 0.04 && diff.mean <= 2.0,
                    "{path}, {name}, {scale}: {diff:?}"
                );
                let lit = |pixels: &[[u8; 4]]| pixels.iter().filter(|p| p[0].max(p[1]).max(p[2]) > 60).count();
                let (native_lit, scaled_lit) = (lit(&native), lit(&scaled));
                assert!(
                    native_lit > 2_000,
                    "{path}, {name}: the lights light {native_lit} pixels"
                );
                let ratio = scaled_lit as f64 / native_lit as f64;
                assert!(
                    (0.95..=1.05).contains(&ratio),
                    "{path}, {name}, {scale}: {scaled_lit} lit pixels against {native_lit}"
                );
            }
        }
    }
}

#[test]
fn a_frame_in_the_corner_of_larger_textures_is_the_image_of_its_own_size() {
    for (path, context) in contexts() {
        let mut game = lit_demo();
        let fixed = render(&context, &mut game, ResolutionSettings::fixed(0.5));
        // Textures made for a scale of 1, the frame drawn at 0.5 into their top-left corner.
        let corner = render(&context, &mut game, held(0.5, 0.5, 1.0));
        let diff = scenes::compare(&fixed, &corner, 1);
        eprintln!("{path}: half scale in the corner against its own textures: {diff:?}");
        assert!(diff.max <= 1, "{path}: {diff:?}");
    }
}

#[test]
fn the_ui_stays_at_the_targets_resolution() {
    let Some(context) = gpu() else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            resolution: ResolutionSettings::fixed(0.5),
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    assert!(renderer.plan().scaled());
    renderer.set_overlay_enabled(true);
    // A one-pixel line: drawn at half scale and upscaled it would be two pixels of half its colour.
    renderer
        .overlay_mut()
        .rect(101.0, 20.0, 1.0, 50.0, [255, 255, 255, 255]);
    renderer.render(target.view());
    let pixels = read_back_rgba8(&context, target.texture());
    let at = |x: u32, y: u32| pixels[(y * SIZE + x) as usize];
    assert_eq!(at(101, 40), [255, 255, 255, 255]);
    assert_eq!(at(100, 40), [0, 0, 0, 255]);
    assert_eq!(at(102, 40), [0, 0, 0, 255]);
}

#[test]
fn changing_the_scale_makes_textures_only_when_their_size_changes() {
    let Some(context) = gpu() else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    assert!(
        !renderer.plan().scaled(),
        "no upscale pass at the target's own resolution"
    );
    renderer.render(target.view());
    renderer.set_resolution(held(0.7, 0.5, 1.0));
    assert!(renderer.plan().scaled());
    renderer.render(target.view());
    assert_eq!(renderer.resolution_stats().width, 179);
    // Within the range nothing is planned again; out of it (a fixed scale) the textures are made again.
    renderer.set_resolution(held(0.9, 0.5, 1.0));
    renderer.render(target.view());
    assert_eq!(
        renderer.resolution_stats().scale,
        0.7,
        "dynamic resolution keeps its scale"
    );
    renderer.set_resolution(ResolutionSettings::fixed(0.25));
    renderer.render(target.view());
    let stats = renderer.resolution_stats();
    assert_eq!((stats.scale, stats.width), (0.5, 128), "clamped to the lowest scale");
    renderer.set_resolution(ResolutionSettings::NATIVE);
    assert!(!renderer.plan().scaled());
    renderer.render(target.view());
}
