//! Drawing parts on a real adapter (#77): the demo scene against a committed reference image, the same image from
//! the vertex-buffer fallback and far from the world origin, a missing camera, and the draw count from 1,000 to
//! 100,000 parts. Culling (#79) is on in every image, which therefore also shows it hides nothing in view; culled
//! parts are not drawn on either instance path. Skips when the machine has no adapter (see `support::gpu`).
//!
//! The reference is `tests/reference/parts.png`. Software and hardware rasterisers differ slightly at triangle
//! edges and in the last bits of the shading maths, so images are compared with a tolerance: almost every pixel
//! within a few levels, and the few that are not (edges) limited in number. To make a new reference (after a
//! deliberate change to how parts look), run with `METALLIC_UPDATE_REFERENCES=1` and look at the image.
//!
//! Every render also checks that wgpu reported no error through its log, which carries the graphics API's
//! validation messages where validation layers are installed (debug builds turn validation on).
//!
//! These images are of parts alone: the sky pass is left out ([`PARTS_ONLY`]), so whatever no part covers keeps
//! the clear colour and the tests can tell drawn pixels from empty ones. The sky has its own image test
//! (`tests/sky.rs`). The parts are lit by the renderer's default lighting, `Lighting`'s defaults (#81).

mod support;

use std::path::PathBuf;
use std::sync::{Mutex, Once, PoisonError};

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Value};
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    CameraReader, ContextOptions, CullSettings, DrawStats, GpuScene, LodSettings, OffscreenTarget, PassId, PassSet,
    RenderContext, Renderer, RendererSettings, wgpu,
};
use support::scenes::{self, Game, ImageDiff, Look, SeeThrough};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SKY: wgpu::Color = wgpu::Color {
    r: 0.36,
    g: 0.55,
    b: 0.82,
    a: 1.0,
};

/// The renderer's default passes without the sky: the images show the parts on the clear colour.
const PARTS_ONLY: PassSet = PassSet::EMPTY
    .with(PassId::Opaque)
    .with(PassId::Transparent)
    .with(PassId::Ui);

/// A pixel differing by more than this in a channel counts as different.
const THRESHOLD: u8 = 12;
/// At most this share of pixels may differ (triangle edges rasterised a little differently).
const MAX_OVER: f64 = 0.01;
/// The mean difference over all pixels may not exceed this.
const MAX_MEAN: f64 = 1.0;

fn reference_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference/parts.png")
}

/// wgpu's warnings and errors, which include the graphics API's validation messages in debug builds (Vulkan
/// validation layers, the D3D12 debug layer) where the machine has them installed, as CI's images do.
struct WgpuMessages(Mutex<Vec<(log::Level, String)>>);

impl log::Log for WgpuMessages {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("wgpu")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let message = format!("{}: {}", record.target(), record.args());
            // The Vulkan loader's own messages while instances are made (on a machine without a Vulkan driver,
            // "Found no drivers" is an error) say nothing about the frames drawn; the loader names the instance.
            if message.contains("[Loader Message") || message.contains("type: INSTANCE") {
                return;
            }
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((record.level(), message));
        }
    }

    fn flush(&self) {}
}

static MESSAGES: WgpuMessages = WgpuMessages(Mutex::new(Vec::new()));

/// Starts collecting wgpu's messages (once per test binary).
fn collect_wgpu_messages() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if log::set_logger(&MESSAGES).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }
    });
}

/// Fails on any validation error wgpu reported since the last check (by any test); prints the warnings.
fn assert_no_wgpu_errors() {
    let messages = std::mem::take(&mut *MESSAGES.0.lock().unwrap_or_else(PoisonError::into_inner));
    for (level, message) in &messages {
        eprintln!("wgpu {level}: {message}");
    }
    let errors = messages.iter().filter(|(level, _)| *level == log::Level::Error).count();
    assert_eq!(errors, 0, "wgpu reported {errors} error(s), printed above");
}

/// Renders `game` once (a few frames, so everything is uploaded and sorted) and reads the image back, with the
/// last frame's draw statistics.
fn render(context: &RenderContext, game: &mut Game, passes: PassSet) -> (Vec<[u8; 4]>, DrawStats) {
    render_culled(context, game, passes, CullSettings::default())
}

/// Like [`render`], culling as `cull` says.
fn render_culled(
    context: &RenderContext,
    game: &mut Game,
    passes: PassSet,
    cull: CullSettings,
) -> (Vec<[u8; 4]>, DrawStats) {
    render_with(context, game, passes, cull, LodSettings::default())
}

/// Like [`render_culled`], with the levels of detail `lod`.
fn render_with(
    context: &RenderContext,
    game: &mut Game,
    passes: PassSet,
    cull: CullSettings,
    lod: LodSettings,
) -> (Vec<[u8; 4]>, DrawStats) {
    collect_wgpu_messages();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            clear_color: SKY,
            passes,
            gpu_timing: false,
            cull,
            lod,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
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
    let pixels = read_back_rgba8(context, target.texture());
    assert_no_wgpu_errors();
    (pixels, renderer.draw_stats())
}

fn demo_game(offset: Vec3) -> Game {
    let mut game = Game::new();
    scenes::demo(&mut game, offset);
    game
}

fn check(name: &str, diff: ImageDiff) {
    eprintln!(
        "{name}: max difference {}, mean {:.3}, {:.3}% of pixels over {THRESHOLD}",
        diff.max,
        diff.mean,
        diff.over * 100.0
    );
    assert!(
        diff.over <= MAX_OVER && diff.mean <= MAX_MEAN,
        "{name} differs from the reference beyond the tolerance ({diff:?})"
    );
}

/// The image against the committed reference, writing it instead when asked to.
fn check_reference(name: &str, pixels: &[[u8; 4]]) {
    let path = reference_path();
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
    if diff.over > MAX_OVER || diff.mean > MAX_MEAN {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, SIZE, SIZE, pixels);
        eprintln!("{name}: the image is in {}", out.display());
    }
    check(name, diff);
}

fn describe(context: &RenderContext) {
    let caps = context.capabilities();
    eprintln!(
        "adapter: {} ({:?}, {:?}), instances: {:?}",
        caps.adapter_name,
        caps.backend,
        caps.device_type,
        InstanceBinding::for_context(context)
    );
}

#[test]
fn the_demo_scene_matches_the_reference() {
    let Some(context) = gpu() else { return };
    describe(&context);
    let (pixels, stats) = render(&context, &mut demo_game(Vec3::ZERO), PARTS_ONLY);
    eprintln!("{stats:?}");
    // Four kinds drawn opaque (the floor and solid parts), and the glass block and the ball sorted.
    assert_eq!(stats.opaque_draws, 4);
    assert_eq!(stats.transparent_draws, 2);
    assert_eq!(stats.instances, 9);
    check_reference("parts", &pixels);
}

#[test]
fn the_vertex_buffer_fallback_draws_the_same_image() {
    let Some(primary) = gpu() else { return };
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    describe(&fallback);
    let (pixels, stats) = render(&fallback, &mut demo_game(Vec3::ZERO), PARTS_ONLY);
    assert_eq!((stats.opaque_draws, stats.transparent_draws), (4, 2));
    // Same device and maths, only the instance path differs: the images match exactly, give or take rounding.
    let (storage, _) = render(&primary, &mut demo_game(Vec3::ZERO), PARTS_ONLY);
    let diff = scenes::compare(&pixels, &storage, 1);
    check("fallback against storage", diff);
    assert!(diff.max <= 2, "{diff:?}");
    check_reference("parts-fallback", &pixels);
}

#[test]
fn far_from_the_origin_the_image_is_the_same() {
    let Some(context) = gpu() else { return };
    // 50 km out: the scene origin moves next to the camera, and positions stay precise.
    let (pixels, _) = render(
        &context,
        &mut demo_game(Vec3::new(50_000.0, 200.0, -30_000.0)),
        PARTS_ONLY,
    );
    check_reference("parts-far", &pixels);
}

#[test]
fn a_depth_prepass_draws_the_same_image() {
    let Some(context) = gpu() else { return };
    let (pixels, stats) = render(
        &context,
        &mut demo_game(Vec3::ZERO),
        PARTS_ONLY.with(PassId::DepthPrepass),
    );
    assert_eq!(stats.prepass_draws, 4);
    check_reference("parts-prepass", &pixels);
}

/// The sky colour, sRGB-encoded as the target stores it.
fn sky_pixel() -> [u8; 4] {
    let encode = |linear: f64| {
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    };
    [encode(SKY.r), encode(SKY.g), encode(SKY.b), 255]
}

/// Whether the pixel at `(x, y)` shows the sky (nothing drawn there).
fn is_sky(pixels: &[[u8; 4]], x: u32, y: u32) -> bool {
    let pixel = pixels[(y * SIZE + x) as usize];
    pixel.iter().zip(sky_pixel()).all(|(&a, b)| a.abs_diff(b) <= 2)
}

/// Three parts in front of a camera at the origin looking down −Z (60°, square target) and one behind it: a near
/// block (pixel 61, 128 on the 256-pixel target), a large block 60 m away (183, 128), and a small ball 40 m away
/// (128, 161).
fn culling_game() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::splat(4.0),
        CFrame::new(Vec3::new(-6.0, 0.0, -20.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [40, 160, 60]),
        Vec3::splat(6.0),
        CFrame::new(Vec3::new(15.0, 0.0, -60.0)),
    );
    game.part(
        Look::new("Ball", "Plastic", [30, 60, 200]),
        Vec3::splat(2.0),
        CFrame::new(Vec3::new(0.0, -6.0, -40.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [250, 220, 40]),
        Vec3::splat(4.0),
        CFrame::new(Vec3::new(0.0, 0.0, 20.0)),
    );
    game.set_camera(CFrame::look_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y), 60.0);
    game
}

/// Culled parts are not drawn (#79), on `context`'s instance path: the frustum culls without changing the image,
/// the view distance removes the far block, and the screen-size threshold the small ball.
fn culled_parts_are_not_drawn_on(context: &RenderContext) {
    describe(context);
    let (near, far, small) = ((61, 128), (183, 128), (128, 161));
    let drawn = |pixels: &[[u8; 4]]| [near, far, small].map(|(x, y)| !is_sky(pixels, x, y));
    let mut game = culling_game();
    let (everything, all) = render_culled(context, &mut game, PARTS_ONLY, CullSettings::OFF);
    assert_eq!((all.instances, all.culled_instances), (4, 0));
    assert_eq!(
        drawn(&everything),
        [true; 3],
        "every part in view is drawn without culling"
    );

    // The default culling: only the block behind the camera, and the image is the same (with the levels of detail
    // off: the culled view would draw the far ball at another level, tests/lod.rs).
    let (pixels, stats) = render_with(
        context,
        &mut game,
        PARTS_ONLY,
        CullSettings::default(),
        LodSettings::OFF,
    );
    assert_eq!((stats.instances, stats.culled_instances), (3, 1), "{stats:?}");
    let diff = scenes::compare(&pixels, &everything, 0);
    assert_eq!(diff.max, 0, "frustum culling changed the image: {diff:?}");

    // A 40 m view distance: the far block goes (its nearest point is 55 m away), the ball (37 m) stays.
    let distance = CullSettings {
        max_distance: 40.0,
        ..CullSettings::default()
    };
    let (pixels, stats) = render_culled(context, &mut game, PARTS_ONLY, distance);
    assert_eq!((stats.instances, stats.culled_instances), (2, 2), "{stats:?}");
    assert_eq!(drawn(&pixels), [true, false, true]);

    // At least 30 pixels across: the ball's bounding sphere covers 19, the far block's 38.
    let size = CullSettings {
        min_screen_size: 30.0,
        ..CullSettings::default()
    };
    let (pixels, stats) = render_culled(context, &mut game, PARTS_ONLY, size);
    assert_eq!((stats.instances, stats.culled_instances), (2, 2), "{stats:?}");
    assert_eq!(drawn(&pixels), [true, true, false]);
}

#[test]
fn culled_parts_are_not_drawn() {
    let Some(context) = gpu() else { return };
    culled_parts_are_not_drawn_on(&context);
}

#[test]
fn culled_parts_are_not_drawn_on_the_vertex_buffer_fallback() {
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    culled_parts_are_not_drawn_on(&fallback);
}

#[test]
fn without_a_camera_nothing_is_drawn() {
    let Some(context) = gpu() else { return };
    let mut game = demo_game(Vec3::ZERO);
    game.dm
        .set_by_name(game.workspace, "CurrentCamera", Value::Ref(None))
        .expect("CurrentCamera");
    let (pixels, stats) = render(&context, &mut game, PARTS_ONLY);
    assert_eq!(stats, DrawStats::default());
    // The sky colour, sRGB-encoded.
    let encode = |linear: f64| {
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    };
    let sky = [encode(SKY.r), encode(SKY.g), encode(SKY.b), 255];
    support::assert_all_pixels(&pixels, sky, 1);
}

#[test]
fn the_camera_follows_current_camera() {
    let Some(context) = gpu() else { return };
    let mut game = demo_game(Vec3::ZERO);
    let (before, _) = render(&context, &mut game, PARTS_ONLY);
    // Looking straight down from high up: mostly floor.
    game.set_camera(CFrame::look_at(Vec3::new(0.0, 60.0, 0.1), Vec3::ZERO, Vec3::Y), 30.0);
    let (after, _) = render(&context, &mut game, PARTS_ONLY);
    let diff = scenes::compare(&before, &after, THRESHOLD);
    assert!(diff.over > 0.5, "the view did not change: {diff:?}");
    // The centre is the concrete floor.
    let centre = after[(SIZE / 2 * SIZE + SIZE / 2) as usize];
    assert!(centre[0].abs_diff(centre[2]) < 30 && centre[0] > 60, "{centre:?}");
}

#[test]
fn the_draw_count_does_not_depend_on_the_part_count() {
    let Some(context) = gpu() else { return };
    let mut counts = Vec::new();
    for parts in [1_000, 10_000, 100_000] {
        let mut game = Game::new();
        // One in 100 parts is a ball, a cylinder or a wedge, one in 50 a see-through block.
        scenes::grid(&mut game, parts, 100, SeeThrough::Blocks(50));
        // Every part drawn, then frustum-culled (the grid's far corners are outside the view). Not by screen size:
        // on this small target the 100,000-part grid is seen from so far that every part is under a pixel.
        let (_, stats) = render_culled(&context, &mut game, PARTS_ONLY, CullSettings::OFF);
        let frustum = CullSettings {
            min_screen_size: 0.0,
            ..CullSettings::default()
        };
        let (_, culled) = render_culled(&context, &mut game, PARTS_ONLY, frustum);
        eprintln!(
            "{parts} parts: {} opaque draws, {} transparent draws, {} instances, {} triangles; culled: {} instances, \
             {} triangles",
            stats.opaque_draws,
            stats.transparent_draws,
            stats.instances,
            stats.triangles,
            culled.instances,
            culled.triangles
        );
        assert_eq!(stats.instances as usize, parts);
        assert_eq!(
            (stats.culled_instances, culled.instances + culled.culled_instances),
            (0, parts as u32)
        );
        counts.push((stats.opaque_draws, stats.transparent_draws));
        // Culled, the balls and cylinders also take levels of detail (#80): a draw per kind and level in use, at
        // most nine whatever the part count.
        assert!(
            (4..=9).contains(&culled.opaque_draws) && culled.transparent_draws == 1,
            "{parts} parts culled: {culled:?}"
        );
    }
    assert!(counts.iter().all(|&c| c == (4, 1)), "{counts:?}");
}
