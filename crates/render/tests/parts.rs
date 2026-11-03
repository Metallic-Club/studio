//! Drawing parts on a real adapter (#77): the demo scene against a committed reference image, the same image from
//! the vertex-buffer fallback and far from the world origin, a missing camera, and the draw count from 1,000 to
//! 100,000 parts. Skips when the machine has no adapter (see `support::gpu`).
//!
//! The reference is `tests/reference/parts.png`. Software and hardware rasterisers differ slightly at triangle
//! edges and in the last bits of the shading maths, so images are compared with a tolerance: almost every pixel
//! within a few levels, and the few that are not (edges) limited in number. To make a new reference (after a
//! deliberate change to how parts look), run with `METALLIC_UPDATE_REFERENCES=1` and look at the image.
//!
//! Every render also checks that wgpu reported no error through its log, which carries the graphics API's
//! validation messages where validation layers are installed (debug builds turn validation on).

mod support;

use std::path::PathBuf;
use std::sync::{Mutex, Once, PoisonError};

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Value};
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    CameraReader, ContextOptions, DrawStats, GpuScene, OffscreenTarget, PassId, PassSet, RenderContext, Renderer,
    RendererSettings, wgpu,
};
use support::scenes::{self, Game, ImageDiff, SeeThrough};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SKY: wgpu::Color = wgpu::Color {
    r: 0.36,
    g: 0.55,
    b: 0.82,
    a: 1.0,
};

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
    collect_wgpu_messages();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            clear_color: SKY,
            passes,
            gpu_timing: false,
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
    let (pixels, stats) = render(&context, &mut demo_game(Vec3::ZERO), PassSet::default());
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
    let (pixels, stats) = render(&fallback, &mut demo_game(Vec3::ZERO), PassSet::default());
    assert_eq!((stats.opaque_draws, stats.transparent_draws), (4, 2));
    // Same device and maths, only the instance path differs: the images match exactly, give or take rounding.
    let (storage, _) = render(&primary, &mut demo_game(Vec3::ZERO), PassSet::default());
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
        PassSet::default(),
    );
    check_reference("parts-far", &pixels);
}

#[test]
fn a_depth_prepass_draws_the_same_image() {
    let Some(context) = gpu() else { return };
    let (pixels, stats) = render(
        &context,
        &mut demo_game(Vec3::ZERO),
        PassSet::default().with(PassId::DepthPrepass),
    );
    assert_eq!(stats.prepass_draws, 4);
    check_reference("parts-prepass", &pixels);
}

#[test]
fn without_a_camera_nothing_is_drawn() {
    let Some(context) = gpu() else { return };
    let mut game = demo_game(Vec3::ZERO);
    game.dm
        .set_by_name(game.workspace, "CurrentCamera", Value::Ref(None))
        .expect("CurrentCamera");
    let (pixels, stats) = render(&context, &mut game, PassSet::default());
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
    let (before, _) = render(&context, &mut game, PassSet::default());
    // Looking straight down from high up: mostly floor.
    game.set_camera(CFrame::look_at(Vec3::new(0.0, 60.0, 0.1), Vec3::ZERO, Vec3::Y), 30.0);
    let (after, _) = render(&context, &mut game, PassSet::default());
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
        let (_, stats) = render(&context, &mut game, PassSet::default());
        eprintln!(
            "{parts} parts: {} opaque draws, {} transparent draws, {} instances, {} triangles",
            stats.opaque_draws, stats.transparent_draws, stats.instances, stats.triangles
        );
        assert_eq!(stats.instances as usize, parts);
        counts.push((stats.opaque_draws, stats.transparent_draws));
    }
    assert!(counts.iter().all(|&c| c == (4, 1)), "{counts:?}");
}
