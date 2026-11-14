//! Lighting and the sky on a real adapter (#81): a late-afternoon scene against a committed reference image, the
//! sun disc where `ClockTime` puts it, the sky only behind what parts left uncovered (ADR 0008 §7), the same sky
//! with MSAA, and a night that is darker but still readable. The lighting is read from the game's `Lighting`
//! service with [`LightingReader`], as the engine does. Skips when the machine has no adapter (see
//! `support::gpu`).
//!
//! The reference is `tests/reference/sky.png`, compared with the tolerance of `tests/parts.rs`. To make a new one
//! (after a deliberate change to the lighting or the sky), run with `METALLIC_UPDATE_REFERENCES=1` and look at it.

mod support;

use std::path::PathBuf;

use metallic_core::glam::{Vec3, Vec4};
use metallic_core::{CFrame, Id, Value};
use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, GpuScene, LightingReader, OffscreenTarget, PassId, PassSet, RenderContext, Renderer,
    RendererSettings, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, non_finite, read_back_rgba8, reference_max_mean, render_raw};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The clear colour: what the sky replaces.
const CLEAR: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const THRESHOLD: u8 = 12;
const MAX_OVER: f64 = 0.01;

/// A floor running to the horizon and a few parts, seen from head height looking west towards the late-afternoon
/// sun. No see-through parts, so every drawn pixel is final before the sky pass.
fn sunset_game() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [99, 95, 98]),
        Vec3::new(2048.0, 2.0, 2048.0),
        CFrame::new(Vec3::new(0.0, -1.0, 0.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::splat(4.0),
        CFrame::new(Vec3::new(-24.0, 2.0, -4.0)),
    );
    game.part(
        Look::new("Ball", "SmoothPlastic", [13, 105, 172]),
        Vec3::splat(5.0),
        CFrame::new(Vec3::new(-30.0, 2.5, 7.0)),
    );
    game.part(
        Look::new("Cylinder", "Metal", [163, 162, 165]),
        Vec3::new(3.0, 3.0, 3.0),
        CFrame::new(Vec3::new(-16.0, 1.5, 4.0)),
    );
    let eye = Vec3::new(0.0, 3.0, 0.0);
    game.set_camera(
        CFrame::look_at(eye, eye + Vec3::new(-0.966, 0.12, 0.172), Vec3::Y),
        70.0,
    );
    game
}

fn lighting_service(game: &Game) -> Id {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    game.dm.service(class).expect("a Lighting")
}

fn set_clock_time(game: &mut Game, hours: f64) {
    let lighting = lighting_service(game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(hours))
        .expect("ClockTime");
}

/// Renders `game` lit by its `Lighting` (two frames, so everything is uploaded) and reads the image back.
fn render(context: &RenderContext, game: &mut Game, passes: PassSet, samples: u32) -> Vec<[u8; 4]> {
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            clear_color: CLEAR,
            passes,
            gpu_timing: false,
            msaa_samples: samples,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    let lighting = lighting_service(game);
    renderer.set_lighting(LightingReader::new().read(&game.dm, lighting));
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    for _ in 0..2 {
        let camera = reader.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&game.dm, camera.position);
        }
        scene.sync(&mut game.dm);
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
    }
    let stats = renderer.draw_stats();
    assert_eq!(stats.sky_draws, u32::from(passes.contains(PassId::Sky)), "{stats:?}");
    read_back_rgba8(context, target.texture())
}

/// Where `direction` (from the camera) lands on the target, in pixels from the top left.
fn project(game: &Game, direction: Vec3) -> (f32, f32) {
    let camera = CameraReader::new().read(&game.dm, game.workspace).expect("a camera");
    let clip = camera.view_projection(1.0) * Vec4::from((direction, 0.0));
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    ((x * 0.5 + 0.5) * SIZE as f32, (0.5 - y * 0.5) * SIZE as f32)
}

/// The centre of the white-hot pixels (every channel at 250 or more) within `radius` pixels of `around`, and how
/// many there are. (The sun's glare off the floor below it is white-hot too.)
fn white_hot(pixels: &[[u8; 4]], around: (f32, f32), radius: f32) -> (f32, f32, usize) {
    let (mut x, mut y, mut n) = (0.0, 0.0, 0);
    for (i, p) in pixels.iter().enumerate() {
        let (px, py) = ((i as u32 % SIZE) as f32 + 0.5, (i as u32 / SIZE) as f32 + 0.5);
        if (px - around.0).hypot(py - around.1) <= radius && p[..3].iter().all(|&c| c >= 250) {
            x += px;
            y += py;
            n += 1;
        }
    }
    let n_f = (n as f32).max(1.0);
    (x / n_f, y / n_f, n)
}

fn mean_level(pixels: &[[u8; 4]]) -> f64 {
    let total: u64 = pixels
        .iter()
        .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
        .sum();
    total as f64 / (pixels.len() * 3) as f64
}

fn pixel(pixels: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    pixels[(y * SIZE + x) as usize]
}

fn check_reference(context: &RenderContext, name: &str, pixels: &[[u8; 4]]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/reference/{name}.png"));
    if std::env::var("METALLIC_UPDATE_REFERENCES").is_ok_and(|v| v == "1") {
        scenes::write_png(&path, SIZE, SIZE, pixels);
        eprintln!("wrote {}", path.display());
        return;
    }
    let Some((width, height, reference)) = scenes::read_png(&path) else {
        panic!(
            "{} is missing; run with METALLIC_UPDATE_REFERENCES=1 to make it",
            path.display()
        );
    };
    assert_eq!((width, height), (SIZE, SIZE));
    let diff = scenes::compare(pixels, &reference, THRESHOLD);
    eprintln!(
        "{name}: max difference {}, mean {:.3}, {:.3}% of pixels over {THRESHOLD}",
        diff.max,
        diff.mean,
        diff.over * 100.0
    );
    if diff.over > MAX_OVER || diff.mean > reference_max_mean(context) {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, SIZE, SIZE, pixels);
        panic!(
            "{name} differs from the reference beyond the tolerance ({diff:?}); the image is in {}",
            out.display()
        );
    }
}

#[test]
fn the_late_afternoon_matches_the_reference() {
    let Some(context) = gpu() else { return };
    let mut game = sunset_game();
    set_clock_time(&mut game, 17.0);
    let pixels = render(&context, &mut game, PassSet::default(), 1);
    check_reference(&context, "sky", &pixels);
    // No pixel kept the clear colour: the sky covers everything the parts left.
    assert!(pixels.iter().all(|p| p[..3] != [255, 0, 255]), "the clear colour shows");
}

#[test]
fn clock_time_moves_the_sun_disc() {
    let Some(context) = gpu() else { return };
    let mut game = sunset_game();
    let lighting = lighting_service(&game);
    // A black sky, so only the sun disc (and the inside of its halo, which is round) is white-hot near the sun: a
    // bright horizon next to it would pull the centre down.
    for name in ["SkyColor", "HorizonColor"] {
        game.dm
            .set_by_name(lighting, name, Value::Color3(metallic_core::Color3::new(0.0, 0.0, 0.0)))
            .expect("sky colour");
    }
    let mut positions = Vec::new();
    for hours in [16.6, 17.0] {
        set_clock_time(&mut game, hours);
        let Ok(Value::Vector3(sun)) = game.dm.get_by_name(lighting, "SunDirection") else {
            panic!("no SunDirection");
        };
        let pixels = render(&context, &mut game, PassSet::default(), 1);
        let (px, py) = project(&game, sun);
        let (x, y, count) = white_hot(&pixels, (px, py), 20.0);
        eprintln!("{hours}: {count} white-hot pixels around ({x:.1}, {y:.1}), the sun projected to ({px:.1}, {py:.1})");
        // The disc (1.4° across a 70° view of 256 pixels: about 10 pixels wide) and a little of its halo.
        assert!((40..400).contains(&count), "{count} white-hot pixels");
        assert!(
            (x - px).abs() <= 1.5 && (y - py).abs() <= 1.5,
            "the sun is drawn at ({x:.1}, {y:.1}), not where it is ({px:.1}, {py:.1})"
        );
        positions.push((x, y));
    }
    // Later in the afternoon the sun is lower: further down the screen.
    assert!(positions[1].1 > positions[0].1 + 10.0, "{positions:?}");
}

#[test]
fn the_sky_fills_only_what_the_parts_left() {
    let Some(context) = gpu() else { return };
    let mut game = sunset_game();
    set_clock_time(&mut game, 17.0);
    // Without bloom, which would carry the sky's light onto the parts' pixels.
    let passes = PassSet::default().without(PassId::Bloom);
    let without = render(&context, &mut game, passes.without(PassId::Sky), 1);
    let with = render(&context, &mut game, passes, 1);
    let (mut parts, mut sky) = (0, 0);
    // The magenta clear colour, through the tone curve.
    let magenta = |p: &[u8; 4]| p[0] > 240 && p[1] < 4 && p[2] > 240;
    for (a, b) in without.iter().zip(&with) {
        if magenta(a) {
            // Empty without the sky pass: the sky drew there.
            assert_ne!(a, b);
            sky += 1;
        } else {
            // A part was there: the sky pass left it exactly as it was (the far-plane depth test failed).
            assert_eq!(a, b);
            parts += 1;
        }
    }
    eprintln!("{parts} part pixels kept, {sky} sky pixels drawn");
    assert!(parts > 10_000 && sky > 10_000, "{parts} part pixels, {sky} sky pixels");
}

#[test]
fn the_sky_is_the_same_with_msaa() {
    let Some(context) = gpu() else { return };
    let counts = context
        .msaa_sample_counts(FORMAT)
        .intersect(context.msaa_sample_counts(metallic_render::depth::DEPTH_FORMAT));
    if !counts.contains(4) {
        eprintln!("skipped: no 4x MSAA for {FORMAT:?} with depth");
        return;
    }
    let mut game = sunset_game();
    set_clock_time(&mut game, 17.0);
    let single = render(&context, &mut game, PassSet::default(), 1);
    let msaa = render(&context, &mut game, PassSet::default(), 4);
    // Only edges change (they are antialiased).
    let diff = scenes::compare(&single, &msaa, THRESHOLD);
    eprintln!("MSAA against none: {diff:?}");
    assert!(diff.over < 0.03 && diff.mean < 1.5, "{diff:?}");
}

#[test]
fn night_is_darker_but_readable() {
    let Some(context) = gpu() else { return };
    let mut game = sunset_game();
    // Looking along the floor towards the parts, with the moon (opposite the sun) high in the south, to the left.
    set_clock_time(&mut game, 12.0);
    let noon = render(&context, &mut game, PassSet::default(), 1);
    set_clock_time(&mut game, 0.0);
    let night = render(&context, &mut game, PassSet::default(), 1);
    // To look at: how dark the night is a judgement, so keep the image.
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sky-night.png");
    scenes::write_png(&out, SIZE, SIZE, &night);
    eprintln!("the night is in {}", out.display());
    let (noon_level, night_level) = (mean_level(&noon), mean_level(&night));
    eprintln!("mean level: noon {noon_level:.1}, midnight {night_level:.1}");
    assert!(night_level < noon_level * 0.6, "night is not darker");
    // The red block (centre-left, low) is still told apart from the floor at night.
    let (bx, by) = project(&game, Vec3::new(-24.0, 2.0, -4.0) - Vec3::new(0.0, 3.0, 0.0));
    let block = pixel(&night, bx as u32, by as u32);
    let floor = pixel(&night, bx as u32, SIZE - 8);
    eprintln!("night: block {block:?}, floor {floor:?}");
    assert!(block[0] > 30, "the block is black at night: {block:?}");
    assert!(block[0] > block[2], "the block is not red at night: {block:?}");
    assert!(
        floor.iter().take(3).any(|&c| c > 20),
        "the floor is black at night: {floor:?}"
    );
    // The night sky is dark blue.
    let top = pixel(&night, SIZE / 2, 2);
    assert!(top[2] >= top[0] && top[2] < 80, "{top:?}");
}

/// The sky and the parts under it write finite values at every time of day, read before the post chain (which
/// would make the others 0): no pixel is not a number or infinite. (Lavapipe wrote 9 at midnight, #149.)
#[test]
fn the_sky_writes_only_finite_values() {
    let Some(context) = gpu() else { return };
    let mut game = sunset_game();
    for hours in [0.0, 6.0, 12.0, 17.0, 18.0] {
        set_clock_time(&mut game, hours);
        let pixels = render_raw(&context, &mut game, SIZE, |s| s.clear_color = CLEAR);
        let bad = non_finite(&pixels, SIZE);
        eprintln!("{hours}:00: {} pixels not finite {bad:?}", bad.len());
        assert!(bad.is_empty(), "{hours}:00: pixels not finite {bad:?}");
    }
}

/// #600: Roblox's Lighting properties reach the image. Fog turns the far floor and the sky into `FogColor` while the
/// near floor keeps its own; `ExposureCompensation` darkens the whole frame; `EnvironmentDiffuseScale` 0 leaves the
/// shaded side of a part to the sun alone, darker.
#[test]
fn fog_exposure_and_the_environment_change_the_image() {
    let Some(context) = gpu() else { return };
    let set = |game: &mut Game, name: &str, value: Value| {
        let lighting = lighting_service(game);
        game.dm.set_by_name(lighting, name, value).expect(name);
    };
    let mut game = sunset_game();
    set_clock_time(&mut game, 12.0);
    let plain = render(&context, &mut game, PassSet::default(), 1);

    // Fog: pure red from 20 studs, whole at 60.
    set(
        &mut game,
        "FogColor",
        Value::Color3(metallic_core::Color3::new(1.0, 0.0, 0.0)),
    );
    set(&mut game, "FogStart", Value::Number(20.0));
    set(&mut game, "FogEnd", Value::Number(60.0));
    let fogged = render(&context, &mut game, PassSet::default(), 1);
    let sky = pixel(&fogged, SIZE / 2, 2);
    let near = pixel(&fogged, SIZE / 2, SIZE - 4);
    eprintln!(
        "fog: sky {sky:?}, near floor {near:?} (was {:?})",
        pixel(&plain, SIZE / 2, SIZE - 4)
    );
    assert!(
        sky[0] > 200 && sky[1] < 40 && sky[2] < 40,
        "the sky is the fog: {sky:?}"
    );
    // Within rounding: adapters differ by a level here and there (WARP on CI did).
    let close = |a: &[u8; 4], b: &[u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 2);
    assert!(
        close(&near, &pixel(&plain, SIZE / 2, SIZE - 4)),
        "nearer than FogStart, nothing changes: {near:?}"
    );
    set(&mut game, "FogEnd", Value::Number(100_000.0));
    let unfogged = render(&context, &mut game, PassSet::default(), 1);
    assert!(
        unfogged.iter().zip(&plain).all(|(a, b)| close(a, b)),
        "no fog at FogEnd 100000"
    );

    // Exposure: two stops down is darker everywhere.
    set(&mut game, "ExposureCompensation", Value::Number(-2.0));
    let dark = render(&context, &mut game, PassSet::default(), 1);
    eprintln!(
        "mean level: plain {:.1}, -2 stops {:.1}",
        mean_level(&plain),
        mean_level(&dark)
    );
    assert!(mean_level(&dark) < mean_level(&plain) * 0.7);
    set(&mut game, "ExposureCompensation", Value::Number(0.0));

    // No environment light: the frame loses the sky's light on every surface.
    set(&mut game, "EnvironmentDiffuseScale", Value::Number(0.0));
    set(&mut game, "EnvironmentSpecularScale", Value::Number(0.0));
    let unlit = render(&context, &mut game, PassSet::default(), 1);
    let floor = |pixels: &[[u8; 4]]| {
        pixel(pixels, SIZE / 2, SIZE - 4)
            .iter()
            .take(3)
            .map(|&c| u32::from(c))
            .sum::<u32>()
    };
    eprintln!("floor: plain {}, no environment {}", floor(&plain), floor(&unlit));
    assert!(floor(&unlit) < floor(&plain), "the floor lost the sky's light");
}

/// #615: OutdoorAmbient lights only what the sky reaches. Under a roof (the sun off, so only the ambient lights the
/// floor) the floor gets Ambient alone, darker than the open floor beside it; with GlobalShadows off nothing is kept
/// from the sky, as in Roblox.
#[test]
fn outdoor_ambient_lights_only_what_the_sky_reaches() {
    let Some(context) = gpu() else { return };
    let set = |game: &mut Game, name: &str, value: Value| {
        let lighting = lighting_service(game);
        game.dm.set_by_name(lighting, name, value).expect(name);
    };
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [160, 160, 160]),
        Vec3::new(400.0, 2.0, 400.0),
        CFrame::new(Vec3::new(0.0, -1.0, 0.0)),
    );
    // A roof over the left half of the view, 6 studs up.
    game.part(
        Look::new("Block", "Plastic", [160, 160, 160]),
        Vec3::new(20.0, 1.0, 40.0),
        CFrame::new(Vec3::new(-10.0, 6.5, -20.0)),
    );
    let eye = Vec3::new(0.0, 3.0, 4.0);
    game.set_camera(CFrame::look_at(eye, Vec3::new(0.0, 0.0, -12.0), Vec3::Y), 70.0);
    set(&mut game, "Brightness", Value::Number(0.0));
    set(
        &mut game,
        "Ambient",
        Value::Color3(metallic_core::Color3::new(0.15, 0.15, 0.15)),
    );
    let spots = [-8.0f32, 8.0].map(|x| project(&game, Vec3::new(x, 0.0, -12.0) - eye));
    let floor_at = |pixels: &[[u8; 4]], x: f32| {
        let (px, py) = if x < 0.0 { spots[0] } else { spots[1] };
        let p = pixel(pixels, px as u32, py as u32);
        f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])
    };
    let lit = render(&context, &mut game, PassSet::default(), 1);
    let (under, open) = (floor_at(&lit, -8.0), floor_at(&lit, 8.0));
    eprintln!("floor under the roof {under}, in the open {open}");
    assert!(
        under < open * 0.75,
        "the roof keeps the sky's light off the floor under it"
    );
    assert!(under > 30.0, "Ambient still lights it");
    set(&mut game, "GlobalShadows", Value::Bool(false));
    let flat = render(&context, &mut game, PassSet::default(), 1);
    let (under, open) = (floor_at(&flat, -8.0), floor_at(&flat, 8.0));
    eprintln!("without GlobalShadows: under {under}, open {open}");
    assert!(
        (under - open).abs() < 8.0,
        "without GlobalShadows the sky reaches everywhere"
    );
}

/// #625: the floor goes from open to covered under a roof's edge in a smooth ramp several studs wide, not a step;
/// and a block's outside wall in the open is not darkened by the block itself.
#[test]
fn the_sky_fades_out_smoothly_under_a_roof() {
    let Some(context) = gpu() else { return };
    let set = |game: &mut Game, name: &str, value: Value| {
        let lighting = lighting_service(game);
        game.dm.set_by_name(lighting, name, value).expect(name);
    };
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [160, 160, 160]),
        Vec3::new(400.0, 2.0, 400.0),
        CFrame::new(Vec3::new(0.0, -1.0, 0.0)),
    );
    // A roof over everything left of x = 0, 6 studs up.
    game.part(
        Look::new("Block", "Plastic", [160, 160, 160]),
        Vec3::new(40.0, 1.0, 60.0),
        CFrame::new(Vec3::new(-20.0, 6.5, -20.0)),
    );
    // A block in the open on the right, its front wall facing the camera.
    game.part(
        Look::new("Block", "Concrete", [160, 160, 160]),
        Vec3::new(6.0, 6.0, 6.0),
        CFrame::new(Vec3::new(9.0, 3.0, -20.0)),
    );
    let eye = Vec3::new(0.0, 4.0, 4.0);
    game.set_camera(CFrame::look_at(eye, Vec3::new(0.0, 0.0, -12.0), Vec3::Y), 70.0);
    set(&mut game, "Brightness", Value::Number(0.0));
    set(
        &mut game,
        "Ambient",
        Value::Color3(metallic_core::Color3::new(0.1, 0.1, 0.1)),
    );
    // Where the floor across the roof's edge (every fifth of a stud from x = -8 to 8) and the block's front wall (2
    // studs up) land on the target.
    let floor_spots: Vec<(f32, f32)> = (-40..=40)
        .map(|i| project(&game, Vec3::new(i as f32 * 0.2, 0.0, -12.0) - eye))
        .collect();
    let wall_spot = project(&game, Vec3::new(9.0, 2.0, -17.0) - eye);
    let level = |pixels: &[[u8; 4]], (px, py): (f32, f32)| {
        let p = pixel(pixels, px as u32, py as u32);
        f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])
    };
    let passes = PassSet::default().without(PassId::Bloom);
    let lit = render(&context, &mut game, passes, 1);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sky-edge.png");
    scenes::write_png(&out, SIZE, SIZE, &lit);
    eprintln!("the roof's edge is in {}", out.display());
    let profile: Vec<f32> = floor_spots.iter().map(|&at| level(&lit, at)).collect();
    eprintln!("floor across the edge, every 0.2 studs from x = -8: {profile:?}");
    let (under, open) = (profile[0], profile[80]);
    assert!(under < open * 0.75, "under {under}, open {open}");
    // Smooth: it never steps by more than a tenth of the whole change between neighbours a fifth of a stud apart
    // (dithering moves a pixel by a level or two either way).
    let change = open - under;
    for (i, pair) in profile.windows(2).enumerate() {
        assert!(
            (pair[1] - pair[0]).abs() <= change * 0.1 + 6.0,
            "a step of {} at x = {:.1}",
            pair[1] - pair[0],
            -8.0 + i as f32 * 0.2
        );
    }
    // Wide: the floor is between 15% and 85% of the way from covered to open over at least 2 studs (it was under
    // one before #625), all of it out in the open (#642): under the roof's edge the floor is as dark as deep under it.
    assert!(
        profile[38] <= under + 6.0,
        "the sky reaches under the roof: {} at x = -0.4",
        profile[38]
    );
    let ramp = profile
        .iter()
        .filter(|&&v| v > under + change * 0.15 && v < open - change * 0.15)
        .count();
    eprintln!("{ramp} samples on the ramp");
    assert!(ramp >= 10, "the ramp is {:.1} studs wide", ramp as f32 * 0.2);
    // The block's front wall, against the same wall with GlobalShadows off (nothing kept from the sky).
    let wall = level(&lit, wall_spot);
    set(&mut game, "GlobalShadows", Value::Bool(false));
    let flat = render(&context, &mut game, passes, 1);
    let open_wall = level(&flat, wall_spot);
    eprintln!("the block's wall {wall}, without GlobalShadows {open_wall}");
    assert!(wall > open_wall * 0.9, "the block darkens its own outside wall");
}

/// #642: a ball resting in the open shades smoothly down to the floor. Its lower half once dropped to black across
/// a hard horizontal line, where the sky map's lookup, moved down along the normal, went under the floor.
#[test]
fn a_ball_in_the_open_shades_smoothly_to_the_floor() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [99, 95, 98]),
        Vec3::new(400.0, 2.0, 400.0),
        CFrame::new(Vec3::new(0.0, -1.0, 0.0)),
    );
    let centre = Vec3::new(0.0, 5.0, -14.0);
    game.part(
        Look::new("Ball", "SmoothPlastic", [255, 0, 0]),
        Vec3::splat(10.0),
        CFrame::new(centre),
    );
    let eye = Vec3::new(0.0, 4.0, 2.0);
    game.set_camera(CFrame::look_at(eye, Vec3::new(0.0, 4.0, -14.0), Vec3::Y), 70.0);
    // Late afternoon: the sun low in the west, lighting the ball's left side.
    set_clock_time(&mut game, 17.6);
    let lit = render(&context, &mut game, PassSet::default().without(PassId::Bloom), 1);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sky-ball.png");
    scenes::write_png(&out, SIZE, SIZE, &lit);
    eprintln!("the ball is in {}", out.display());
    // Down the ball's right side, from its middle to near its outline, a pixel at a time.
    let (_, top) = project(&game, centre - eye);
    let (_, bottom) = project(&game, centre - Vec3::new(0.0, 4.0, 0.0) - eye);
    let (x, _) = project(&game, centre + Vec3::new(2.5, 0.0, 0.0) - eye);
    let column: Vec<u8> = (top as u32..bottom as u32)
        .map(|y| pixel(&lit, x as u32, y)[0])
        .collect();
    eprintln!("red down the ball: {column:?}");
    for (i, pair) in column.windows(2).enumerate() {
        let step = i32::from(pair[1]) - i32::from(pair[0]);
        assert!(step.abs() <= 8, "a step of {step} {i} pixels down the ball");
    }
}

/// #642: a closed room's floor, walls and ceiling get no OutdoorAmbient, right up to the walls. It once leaked through
/// the walls along every edge, where the sky map's filter reached past them to the open ground outside.
#[test]
fn outdoor_ambient_does_not_leak_into_a_closed_room() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    let grey = |game: &mut Game, size: Vec3, at: Vec3| {
        game.part(
            Look::new("Block", "SmoothPlastic", [160, 160, 160]),
            size,
            CFrame::new(at),
        );
    };
    grey(&mut game, Vec3::new(400.0, 2.0, 400.0), Vec3::new(0.0, -1.0, 0.0));
    // A 20-stud room: walls a stud thick and 8 high, and a roof.
    grey(&mut game, Vec3::new(1.0, 8.0, 22.0), Vec3::new(-10.5, 4.0, 0.0));
    grey(&mut game, Vec3::new(1.0, 8.0, 22.0), Vec3::new(10.5, 4.0, 0.0));
    grey(&mut game, Vec3::new(22.0, 8.0, 1.0), Vec3::new(0.0, 4.0, -10.5));
    grey(&mut game, Vec3::new(22.0, 8.0, 1.0), Vec3::new(0.0, 4.0, 10.5));
    grey(&mut game, Vec3::new(22.0, 1.0, 22.0), Vec3::new(0.0, 8.5, 0.0));
    let eye = Vec3::new(0.0, 4.0, 8.0);
    game.set_camera(CFrame::look_at(eye, Vec3::new(0.0, 1.0, -10.0), Vec3::Y), 90.0);
    let set = |game: &mut Game, name: &str, value: Value| {
        let lighting = lighting_service(game);
        game.dm.set_by_name(lighting, name, value).expect(name);
    };
    let grey_light = |level: f32| Value::Color3(metallic_core::Color3::new(level, level, level));
    set(&mut game, "Brightness", Value::Number(0.0));
    set(&mut game, "Ambient", grey_light(0.1));
    let passes = PassSet::default().without(PassId::Bloom);
    let lit = render(&context, &mut game, passes, 1);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sky-room.png");
    scenes::write_png(&out, SIZE, SIZE, &lit);
    eprintln!("the room is in {}", out.display());
    // The same room with no OutdoorAmbient at all: whatever the first has more of came from outside.
    set(&mut game, "OutdoorAmbient", grey_light(0.0));
    let dark = render(&context, &mut game, passes, 1);
    let level = |pixels: &[[u8; 4]], at: Vec3| {
        let (x, y) = project(&game, at - eye);
        let p = pixel(pixels, x as u32, y as u32);
        i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2])
    };
    // The middle of the floor, and the floor, walls and ceiling half a stud from the edges and corners.
    for (what, at) in [
        ("the middle of the floor", Vec3::new(0.0, 0.0, -2.0)),
        ("the floor by the back wall", Vec3::new(0.0, 0.0, -9.5)),
        ("the floor by the left wall", Vec3::new(-9.5, 0.0, -4.0)),
        ("the floor in the back corner", Vec3::new(9.5, 0.0, -9.5)),
        ("the back wall by the corner", Vec3::new(9.5, 3.0, -10.0)),
        ("the back wall at the floor", Vec3::new(0.0, 0.5, -10.0)),
        ("the ceiling by the back wall", Vec3::new(0.0, 8.0, -9.5)),
    ] {
        let (here, without) = (level(&lit, at), level(&dark, at));
        eprintln!("{what}: {here}, without OutdoorAmbient {without}");
        assert!(
            here <= without + 4,
            "{what} is lit from outside: {here} against {without}"
        );
    }
}
