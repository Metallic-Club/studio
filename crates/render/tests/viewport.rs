//! Drawing into a viewport target on a real adapter (#213): the texture an editor shows inside its UI (Studio's
//! viewport, sampled by egui-wgpu on the same device, ADR 0009). The demo scene drawn into one matches the parts
//! image test's reference (`tests/reference/parts.png`, the same settings); resized small, the next frame switches to a
//! texture of the new size holding the image a window's renderer resized the same way draws; resized large, its textures are
//! made off the frame (#152) while frames keep drawing into the old texture at the old size, so the caller never shows
//! a black frame, and a settings change made meanwhile still lands at the new size; resized back, it matches the
//! reference again. A target that does not match its renderer is refused. Skips when the machine has no adapter (see
//! `support::gpu`).

mod support;

use std::path::PathBuf;

use metallic_core::glam::Vec3;
use metallic_render::scene::RenderComponents;
use metallic_render::{
    AntiAliasing, CameraReader, GpuScene, OffscreenTarget, PassId, PassSet, RenderContext, RenderError, Renderer,
    RendererSettings, ViewportTarget, wgpu,
};
use support::scenes::{self, Game};
use support::{assert_no_wgpu_errors, collect_wgpu_messages, gpu, read_back_rgba8, reference_max_mean};

/// The reference's size.
const SIZE: u32 = 256;
/// A panel's size after a small resize: made at once, so the next frame switches.
const SMALL: (u32, u32) = (192, 128);
/// A size whose textures are made off the frame: 2048 x 2048 holds 16 MiB in each of the target, the depth and the
/// scene colour, the smallest made in the background (`textures::BACKGROUND_BYTES`).
const LARGE: u32 = 2048;
/// The parts image test's clear colour and passes, so the image is its reference.
const SKY: wgpu::Color = wgpu::Color {
    r: 0.36,
    g: 0.55,
    b: 0.82,
    a: 1.0,
};
const PARTS_ONLY: PassSet = PassSet::EMPTY
    .with(PassId::Opaque)
    .with(PassId::Transparent)
    .with(PassId::Bloom)
    .with(PassId::Post)
    .with(PassId::Ui);

/// A pixel differing by more than this in a channel counts as different (as in `tests/parts.rs`).
const THRESHOLD: u8 = 12;
/// At most this share of pixels may differ from the reference.
const MAX_OVER: f64 = 0.01;
/// Frames a large resize may take to switch: one texture is made a frame, and the worker may need a few.
const MAX_SWITCH_FRAMES: usize = 200;

fn settings(width: u32, height: u32) -> RendererSettings {
    RendererSettings {
        clear_color: SKY,
        passes: PARTS_ONLY,
        gpu_timing: false,
        ..RendererSettings::new(ViewportTarget::FORMAT, width, height)
    }
}

/// The demo scene drawn by one renderer into one viewport target.
struct Viewport {
    game: Game,
    scene: GpuScene,
    reader: CameraReader,
    renderer: Renderer,
    target: ViewportTarget,
}

impl Viewport {
    fn new(context: &RenderContext) -> Self {
        let mut game = Game::new();
        scenes::demo(&mut game, Vec3::ZERO);
        let target = ViewportTarget::new(context, SIZE, SIZE).expect("a viewport target");
        let mut renderer = Renderer::new(context, settings(SIZE, SIZE)).expect("a renderer");
        renderer.wait_for_pipelines();
        let scene = GpuScene::new(
            context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .expect("a scene");
        Self {
            game,
            scene,
            reader: CameraReader::new(),
            renderer,
            target,
        }
    }

    /// Draws a frame into the target and waits for the GPU (the test reads it back, and paces itself so).
    fn frame(&mut self, context: &RenderContext) {
        let camera = self.reader.read(&self.game.dm, self.game.workspace);
        if let Some(camera) = camera {
            self.scene.set_camera_position(&self.game.dm, camera.position);
        }
        self.scene.sync(&mut self.game.dm);
        self.renderer
            .render_viewport(&mut self.target, &mut self.scene, camera.as_ref())
            .expect("the target matches its renderer");
        context
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the frame finishes");
    }

    fn pixels(&self, context: &RenderContext) -> Vec<[u8; 4]> {
        read_back_rgba8(context, self.target.texture())
    }
}

/// The demo scene drawn the usual way, two frames at each of `sizes` (the window's path: [`Renderer::resize`] and an
/// offscreen target of each size), read back at the last. The same history matters: the round parts' levels of
/// detail keep what they were drawn at until their size on screen moves past a margin (`lod`'s hysteresis), so a
/// renderer resized to a size draws them as it drew them before, not always as a new one at that size would.
fn offscreen(context: &RenderContext, sizes: &[(u32, u32)]) -> Vec<[u8; 4]> {
    let mut game = Game::new();
    scenes::demo(&mut game, Vec3::ZERO);
    let (width, height) = sizes[0];
    let mut renderer = Renderer::new(context, settings(width, height)).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    let mut pixels = Vec::new();
    for &(width, height) in sizes {
        renderer.resize(width, height).expect("a size");
        let target = OffscreenTarget::new(context, width, height, ViewportTarget::FORMAT).expect("a target");
        for _ in 0..2 {
            let camera = reader.read(&game.dm, game.workspace);
            if let Some(camera) = camera {
                scene.set_camera_position(&game.dm, camera.position);
            }
            scene.sync(&mut game.dm);
            renderer.render_scene(target.view(), &mut scene, camera.as_ref());
        }
        pixels = read_back_rgba8(context, target.texture());
    }
    pixels
}

/// The image against the parts test's committed reference, with its tolerance.
fn check_reference(context: &RenderContext, name: &str, pixels: &[[u8; 4]]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference/parts.png");
    let (width, height, reference) = scenes::read_png(&path).expect("tests/reference/parts.png");
    assert_eq!((width, height), (SIZE, SIZE));
    let diff = scenes::compare(pixels, &reference, THRESHOLD);
    let max_mean = reference_max_mean(context);
    eprintln!(
        "{name}: max difference {}, mean {:.3}, {:.3}% of pixels over {THRESHOLD}",
        diff.max,
        diff.mean,
        diff.over * 100.0
    );
    if diff.over > MAX_OVER || diff.mean > max_mean {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, SIZE, SIZE, pixels);
        panic!(
            "{name} differs from the reference beyond the tolerance ({diff:?}); the image is in {}",
            out.display()
        );
    }
}

#[test]
fn a_viewport_target_draws_the_scene_and_resizes_without_a_black_frame() {
    let Some(context) = gpu() else { return };
    collect_wgpu_messages();
    let mut viewport = Viewport::new(&context);
    // What egui-wgpu samples: an 8-bit texture it can bind, holding gamma-encoded colour (not an sRGB format).
    assert_eq!(viewport.target.format(), wgpu::TextureFormat::Rgba8Unorm);
    assert!(
        viewport
            .target
            .texture()
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT)
    );

    // The scene, through the usual frame (the HDR scene colour, bloom, the finish pass), into the target.
    for _ in 0..2 {
        viewport.frame(&context);
    }
    check_reference(&context, "viewport", &viewport.pixels(&context));
    assert_eq!(viewport.target.generation(), 0);

    // A small resize: its textures are made at once, and the next frame switches and draws into the new texture.
    viewport.renderer.resize_viewport(SMALL.0, SMALL.1).expect("a size");
    assert_eq!(viewport.renderer.pending_viewport_size(), Some(SMALL));
    assert_eq!(viewport.target.size(), (SIZE, SIZE), "nothing changes before a frame");
    viewport.frame(&context);
    assert_eq!(viewport.target.size(), SMALL);
    assert_eq!(viewport.target.generation(), 1);
    assert_eq!(viewport.renderer.pending_viewport_size(), None);
    let small = viewport.pixels(&context);
    // The image a window's renderer resized the same way draws into a target of that size.
    let expected = offscreen(&context, &[(SIZE, SIZE), SMALL]);
    let diff = scenes::compare(&small, &expected, 1);
    assert!(
        diff.over == 0.0,
        "the resized viewport differs from an offscreen target of its size: {diff:?}"
    );

    // A large resize, made off the frame: frames keep drawing the old size into the old texture until all of the new
    // textures exist. A settings change taking effect meanwhile (MLAA, which plans other attachments) is drawn at the
    // new size once it switches.
    viewport.renderer.resize_viewport(LARGE, LARGE).expect("a size");
    viewport.frame(&context);
    assert_eq!(
        (viewport.target.size(), viewport.target.generation()),
        (SMALL, 1),
        "a large target is not made in the frame after it was asked for"
    );
    let waiting = viewport.pixels(&context);
    let diff = scenes::compare(&waiting, &small, 1);
    assert!(
        diff.over == 0.0,
        "while the large textures are made, the old texture keeps the image: {diff:?}"
    );
    viewport.renderer.set_anti_aliasing(AntiAliasing::Mlaa).expect("MLAA");
    let mut frames = 1;
    while viewport.target.generation() == 1 {
        assert!(
            frames < MAX_SWITCH_FRAMES,
            "the large resize did not switch in {frames} frames ({:?})",
            viewport.renderer.texture_stats()
        );
        assert_eq!(viewport.target.size(), SMALL);
        viewport.frame(&context);
        frames += 1;
    }
    eprintln!(
        "the large resize switched after {frames} frames ({:?})",
        viewport.renderer.texture_stats()
    );
    assert_eq!(viewport.target.size(), (LARGE, LARGE));
    assert_eq!(viewport.target.generation(), 2);
    assert_eq!(viewport.renderer.anti_aliasing(), AntiAliasing::Mlaa);
    let stats = viewport.renderer.settings();
    assert_eq!((stats.width, stats.height), (LARGE, LARGE));
    viewport.frame(&context);

    // Back to the reference's size (and settings): the image is the reference's again.
    viewport.renderer.set_anti_aliasing(AntiAliasing::Off).expect("no AA");
    viewport.renderer.resize_viewport(SIZE, SIZE).expect("a size");
    for _ in 0..2 {
        viewport.frame(&context);
    }
    assert_eq!(viewport.target.size(), (SIZE, SIZE));
    assert_eq!(viewport.target.generation(), 3);
    assert_eq!(viewport.renderer.anti_aliasing(), AntiAliasing::Off);
    check_reference(&context, "viewport-resized", &viewport.pixels(&context));
    assert_no_wgpu_errors();
}

#[test]
fn a_resize_back_to_the_size_drawn_cancels_the_one_coming() {
    let Some(context) = gpu() else { return };
    let mut viewport = Viewport::new(&context);
    viewport.frame(&context);
    viewport.renderer.resize_viewport(SMALL.0, SMALL.1).expect("a size");
    // The same size again changes nothing; the size drawn cancels it.
    viewport.renderer.resize_viewport(SMALL.0, SMALL.1).expect("a size");
    assert_eq!(viewport.renderer.pending_viewport_size(), Some(SMALL));
    viewport.renderer.resize_viewport(SIZE, SIZE).expect("a size");
    assert_eq!(viewport.renderer.pending_viewport_size(), None);
    viewport.frame(&context);
    assert_eq!(
        (viewport.target.size(), viewport.target.generation()),
        ((SIZE, SIZE), 0)
    );
    assert!(matches!(
        viewport.renderer.resize_viewport(0, 10),
        Err(RenderError::InvalidSize { .. })
    ));
}

#[test]
fn a_target_that_does_not_match_its_renderer_is_refused() {
    let Some(context) = gpu() else { return };
    let mut viewport = Viewport::new(&context);
    let mut other = ViewportTarget::new(&context, SMALL.0, SMALL.1).expect("a target");
    let camera = viewport.reader.read(&viewport.game.dm, viewport.game.workspace);
    viewport.scene.sync(&mut viewport.game.dm);
    let drawn = viewport
        .renderer
        .render_viewport(&mut other, &mut viewport.scene, camera.as_ref());
    assert!(matches!(
        drawn,
        Err(RenderError::InvalidSize {
            width: 192,
            height: 128
        })
    ));
    // A renderer drawing another format.
    let mut srgb = Renderer::new(
        &context,
        RendererSettings::new(wgpu::TextureFormat::Rgba8UnormSrgb, SIZE, SIZE),
    )
    .expect("a renderer");
    let drawn = srgb.render_viewport(&mut viewport.target, &mut viewport.scene, camera.as_ref());
    assert!(matches!(drawn, Err(RenderError::UnsupportedTarget { .. })));
}

#[test]
fn a_frame_drawn_into_the_corner_is_the_frame_of_that_size() {
    let Some(context) = gpu() else { return };
    collect_wgpu_messages();
    let mut viewport = Viewport::new(&context);
    viewport.frame(&context);
    // Only the top-left SMALL of the target (#330): no texture is made, and the corner holds what a target of that
    // size would.
    viewport.renderer.set_drawn_size(SMALL.0, SMALL.1);
    assert_eq!(viewport.renderer.drawn_size(), SMALL);
    for _ in 0..2 {
        viewport.frame(&context);
    }
    assert_eq!(
        (viewport.target.size(), viewport.target.generation()),
        ((SIZE, SIZE), 0)
    );
    assert_eq!(viewport.renderer.resolution_stats().width, SMALL.0);
    let pixels = viewport.pixels(&context);
    let corner: Vec<[u8; 4]> = (0..SMALL.1)
        .flat_map(|y| {
            let row = (y * SIZE) as usize;
            pixels[row..row + SMALL.0 as usize].iter().copied()
        })
        .collect();
    let expected = offscreen(&context, &[(SIZE, SIZE), SMALL]);
    // Bloom's blur reaches a few texels past the corner's edges, where a target of the corner's size clamps to them,
    // as with a render scale: a faint difference there, and none elsewhere. The parts differ a little too (up to 9
    // levels, #626), which is why the threshold is 10 and not 8.
    let diff = scenes::compare(&corner, &expected, 10);
    assert!(
        diff.over == 0.0,
        "the corner differs from an offscreen target of its size: {diff:?}"
    );
    // A size past the target's is cut to it; the whole target again is the reference.
    viewport.renderer.set_drawn_size(SIZE * 2, SIZE / 2);
    assert_eq!(viewport.renderer.drawn_size(), (SIZE, SIZE / 2));
    viewport.renderer.set_drawn_size(SIZE, SIZE);
    for _ in 0..2 {
        viewport.frame(&context);
    }
    check_reference(&context, "viewport-corner-back", &viewport.pixels(&context));
    assert_no_wgpu_errors();
}
