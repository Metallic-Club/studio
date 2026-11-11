//! The shading views (#478): Unlit, Lighting only, Light density and Wireframe draw what they say, against the
//! Shaded view of the same scene.

mod support;

use metallic_core::glam::{Vec3, Vec4};
use metallic_core::{CFrame, Color3, Id, Value};
use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, GpuScene, OffscreenTarget, RenderContext, Renderer, RendererSettings, ShadingView, wgpu,
};
use support::scenes::{Game, Look};
use support::{assert_no_wgpu_errors, collect_wgpu_messages, gpu, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Draws `game` in `view` (waiting for every pipeline, the wireframe's too) and reads it back. `None` when the view
/// is the wireframe and the device cannot draw lines.
fn render(context: &RenderContext, game: &mut Game, view: ShadingView) -> Option<Vec<[u8; 4]>> {
    collect_wgpu_messages();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    if view == ShadingView::Wireframe && !renderer.supports_wireframe() {
        eprintln!("skipped: {:?} draws no lines", context.capabilities().backend);
        return None;
    }
    renderer.set_shading_view(view);
    assert_eq!(renderer.shading_view(), view);
    renderer.wait_for_pipelines();
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
        renderer.wait_for_pipelines();
    }
    let pixels = read_back_rgba8(context, target.texture());
    assert_no_wgpu_errors();
    Some(pixels)
}

/// A grey floor with a red block on it in the afternoon sun, seen from above at an angle.
fn day_game() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [170, 168, 160]),
        Vec3::new(120.0, 1.0, 120.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::new(3.0, 6.0, 3.0),
        CFrame::new(Vec3::new(0.0, 3.0, 0.0)),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, 14.0, 16.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        60.0,
    );
    set_clock(&mut game, 16.0);
    game
}

fn set_clock(game: &mut Game, hours: f64) {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    let lighting = game.dm.service(class).expect("a Lighting");
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(hours))
        .expect("ClockTime");
}

/// Adds a white point light at `at`, of range `range`.
fn point_light(game: &mut Game, at: Vec3, range: f64) -> Id {
    let dm = &mut game.dm;
    let id = dm
        .create(dm.registry().class_id("PointLight").expect("PointLight"))
        .expect("a light");
    dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at)))
        .expect("CFrame");
    dm.set_by_name(id, "Color", Value::Color3(Color3::new(1.0, 1.0, 1.0)))
        .expect("Color");
    dm.set_by_name(id, "Range", Value::Number(range)).expect("Range");
    dm.set_parent(id, Some(game.workspace)).expect("parented");
    id
}

/// Where world point `point` lands on the target, in pixels from the top left.
fn project(game: &Game, point: Vec3) -> (u32, u32) {
    let camera = CameraReader::new().read(&game.dm, game.workspace).expect("a camera");
    let clip = camera.view_projection(1.0) * Vec4::from((point - camera.position, 1.0));
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    (
        ((x * 0.5 + 0.5) * SIZE as f32) as u32,
        ((0.5 - y * 0.5) * SIZE as f32) as u32,
    )
}

/// The mean colour of the 3 × 3 pixels around where `point` lands.
fn colour(game: &Game, pixels: &[[u8; 4]], point: Vec3) -> [f32; 3] {
    let (x, y) = project(game, point);
    let mut sum = [0.0f32; 3];
    for dy in 0..3 {
        for dx in 0..3 {
            let p = pixels[((y + dy - 1) * SIZE + x + dx - 1) as usize];
            for c in 0..3 {
                sum[c] += f32::from(p[c]) / 9.0;
            }
        }
    }
    sum
}

fn brightness(c: [f32; 3]) -> f32 {
    (c[0] + c[1] + c[2]) / 3.0
}

/// The floor in the block's shadow and in the sun: 4 m from the block, down-sun and up-sun.
fn floor_points(game: &Game) -> (Vec3, Vec3) {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    let lighting = game.dm.service(class).expect("a Lighting");
    let Ok(Value::Vector3(sun)) = game.dm.get_by_name(lighting, "SunDirection") else {
        panic!("no SunDirection");
    };
    let down_sun = Vec3::new(-sun.x, 0.0, -sun.z).normalize();
    (down_sun * 4.0, -down_sun * 4.0)
}

#[test]
fn unlit_draws_the_parts_own_colours_with_no_light_or_shadow() {
    let Some(context) = gpu() else { return };
    let mut game = day_game();
    let (shaded_at, sunlit_at) = floor_points(&game);
    let shaded = render(&context, &mut game, ShadingView::Shaded).expect("drawn");
    let unlit = render(&context, &mut game, ShadingView::Unlit).expect("drawn");
    let (dark, lit) = (colour(&game, &shaded, shaded_at), colour(&game, &shaded, sunlit_at));
    let (dark_unlit, lit_unlit) = (colour(&game, &unlit, shaded_at), colour(&game, &unlit, sunlit_at));
    eprintln!("shaded: floor in shadow {dark:?}, in sun {lit:?}; unlit {dark_unlit:?}, {lit_unlit:?}");
    assert!(
        brightness(lit) > brightness(dark) + 15.0,
        "the shaded floor has a shadow"
    );
    assert!(
        (brightness(lit_unlit) - brightness(dark_unlit)).abs() < 3.0,
        "the unlit floor has no shadow"
    );
    // The block is its own red.
    let block = colour(&game, &unlit, Vec3::new(0.0, 6.0, 0.0));
    assert!(block[0] > block[1] + 80.0 && block[0] > block[2] + 80.0, "{block:?}");
}

#[test]
fn lighting_only_draws_the_light_on_grey() {
    let Some(context) = gpu() else { return };
    let mut game = day_game();
    let (shaded_at, sunlit_at) = floor_points(&game);
    let shaded = render(&context, &mut game, ShadingView::Shaded).expect("drawn");
    let lighting = render(&context, &mut game, ShadingView::LightingOnly).expect("drawn");
    let top = Vec3::new(0.0, 6.0, 0.0);
    let (red, grey) = (colour(&game, &shaded, top), colour(&game, &lighting, top));
    eprintln!("the block's top shaded {red:?}, lighting only {grey:?}");
    assert!(red[0] > red[1] + 60.0, "the shaded block is red");
    assert!(
        (grey[0] - grey[1]).abs() < 25.0 && (grey[1] - grey[2]).abs() < 25.0,
        "{grey:?}"
    );
    // The shadow stays.
    let (dark, lit) = (colour(&game, &lighting, shaded_at), colour(&game, &lighting, sunlit_at));
    assert!(brightness(lit) > brightness(dark) + 15.0, "{dark:?} against {lit:?}");
}

#[test]
fn light_density_colours_by_how_many_lights_reach() {
    let Some(context) = gpu() else { return };
    let mut game = day_game();
    set_clock(&mut game, 0.0);
    // One light on the left; three over one spot on the right.
    point_light(&mut game, Vec3::new(-6.0, 2.0, 0.0), 6.0);
    for dz in [-0.5, 0.0, 0.5] {
        point_light(&mut game, Vec3::new(6.0, 2.0, dz), 6.0);
    }
    let pixels = render(&context, &mut game, ShadingView::LightDensity).expect("drawn");
    let none = colour(&game, &pixels, Vec3::new(0.0, 0.0, 8.0));
    let one = colour(&game, &pixels, Vec3::new(-6.0, 0.0, 1.0));
    let three = colour(&game, &pixels, Vec3::new(6.0, 0.0, 1.0));
    eprintln!("no light {none:?}, one {one:?}, three {three:?}");
    assert!(brightness(none) < 40.0, "no light is dark: {none:?}");
    assert!(
        one[2] > one[0] + 30.0 && one[2] > one[1] + 30.0,
        "one light is blue: {one:?}"
    );
    assert!(
        three[1] > three[0] + 30.0 && three[1] > three[2] + 30.0,
        "three are green: {three:?}"
    );
}

#[test]
fn wireframe_draws_the_triangles_edges_over_the_sky() {
    let Some(context) = gpu() else { return };
    let mut game = day_game();
    let Some(wire) = render(&context, &mut game, ShadingView::Wireframe) else {
        return;
    };
    let shaded = render(&context, &mut game, ShadingView::Shaded).expect("drawn");
    // A point of the floor off its edges and diagonal: the sky shows through it.
    let at = Vec3::new(5.0, 0.0, 2.0);
    let (floor, through) = (colour(&game, &shaded, at), colour(&game, &wire, at));
    eprintln!("the floor shaded {floor:?}, in wireframe {through:?}");
    assert!((floor[2] - floor[0]).abs() < 20.0, "the shaded floor is grey");
    assert!(through[2] > through[0] + 20.0, "the sky shows through the wireframe");
    // Most of the image is the sky; the edges are lines across it.
    let differ = shaded
        .iter()
        .zip(&wire)
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 24))
        .count();
    assert!(differ > shaded.len() / 3, "{differ} of {} pixels differ", shaded.len());
}
