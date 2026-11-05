//! Pipelines compiled off the frame (#86, ADR 0008 §9): the renderer returns once its first frame's pipelines are
//! compiled and compiles every other permutation in the background; a settings change whose pipelines are still
//! compiling waits for them while frames draw as before; and Vulkan's pipeline cache is kept on disk.

mod support;

use std::time::{Duration, Instant};

use metallic_render::{
    AntiAliasing, ContextOptions, OffscreenTarget, PassId, PassSet, RenderContext, Renderer, RendererSettings, wgpu,
};
use support::{gpu, gpu_with, read_back_rgba8};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SIZE: u32 = 32;

fn settings() -> RendererSettings {
    RendererSettings {
        clear_color: wgpu::Color {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 1.0,
        },
        ..RendererSettings::new(FORMAT, SIZE, SIZE)
    }
}

/// Renders one frame into `target` and reads it back (waiting for the GPU, which only the test does).
fn frame(context: &RenderContext, renderer: &mut Renderer, target: &OffscreenTarget) -> Vec<[u8; 4]> {
    renderer.render(target.view());
    read_back_rgba8(context, target.texture())
}

#[test]
fn the_first_frame_is_compiled_first_and_the_rest_behind_it() {
    let Some(context) = gpu() else { return };
    let mut renderer = Renderer::new(&context, settings()).expect("a renderer");
    let first = renderer.pipeline_stats();
    // The scene without local lights (5), the finish pass and bloom's three: what the default frame draws with.
    assert!(first.compiled >= 9, "{first:?}");
    assert!(first.first_frame_ms > 0.0, "{first:?}");
    assert_eq!(first.failed, 0);
    renderer.wait_for_pipelines();
    let all = renderer.pipeline_stats();
    // Every permutation without a new colour format: the scene's 11 at each MSAA count up to 8, the post chain's 11,
    // the upscale pass and the overlay. The scene's 9 with extra suns follow only once a scene has a
    // `DirectionalLight` (#123), but inline (OpenGL) they are compiled up front too.
    let counts = renderer.msaa_sample_counts().iter().filter(|&n| n <= 8).count() as u32;
    let scene = if all.background { 11 } else { 20 };
    assert_eq!(all.compiled, scene * counts + 13, "{all:?}");
    assert_eq!((all.pending, all.failed), (0, 0), "{all:?}");
    assert!(all.all_ms.is_some_and(|ms| ms >= first.first_frame_ms), "{all:?}");
    assert!(!all.switch_waiting);
    if all.background {
        assert!(all.workers > 0);
    } else {
        // Inline (OpenGL): everything was compiled before the renderer returned.
        assert_eq!(first.pending, 0);
        assert_eq!(first.compiled, all.compiled);
    }
}

#[test]
fn a_switch_waits_for_its_pipelines_while_frames_draw_as_before() {
    let Some(context) = gpu() else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(&context, settings()).expect("a renderer");
    renderer.wait_for_pipelines();
    if !renderer.pipeline_stats().background {
        // OpenGL compiles inline: a switch never waits.
        return;
    }
    let before = frame(&context, &mut renderer, &target);
    // Dropping the post chain draws the scene straight into the target: pipelines for the target's own format,
    // which nothing compiled yet. With compiles paused, the switch has to wait.
    renderer.pause_pipeline_compiles(true);
    let direct = PassSet::default().without(PassId::Post).without(PassId::Bloom);
    renderer.set_passes(direct);
    let stats = renderer.pipeline_stats();
    assert!(stats.switch_waiting && stats.pending > 0, "{stats:?}");
    assert_eq!(renderer.settings().passes, PassSet::default());
    for _ in 0..3 {
        assert_eq!(frame(&context, &mut renderer, &target), before);
        assert!(renderer.plan().post());
    }
    renderer.wait_for_pipelines();
    let stats = renderer.pipeline_stats();
    assert!(!stats.switch_waiting && stats.pending == 0, "{stats:?}");
    assert_eq!(renderer.settings().passes, direct);
    assert!(!renderer.plan().post());
    frame(&context, &mut renderer, &target);
}

#[test]
fn switching_back_before_the_pipelines_arrive_cancels_the_switch() {
    let Some(context) = gpu() else { return };
    let mut renderer = Renderer::new(&context, settings()).expect("a renderer");
    renderer.wait_for_pipelines();
    if !renderer.pipeline_stats().background {
        return;
    }
    renderer.pause_pipeline_compiles(true);
    renderer.set_passes(PassSet::default().without(PassId::Post).without(PassId::Bloom));
    assert!(renderer.pipeline_stats().switch_waiting);
    renderer.set_passes(PassSet::default());
    assert!(!renderer.pipeline_stats().switch_waiting);
    renderer.wait_for_pipelines();
    assert!(renderer.plan().post());
    assert_eq!(renderer.settings().passes, PassSet::default());
}

/// Right after the renderer is made, the other MSAA counts are still compiling: asking for one takes effect in a later
/// frame by itself, with nothing but frames drawn in between.
#[test]
fn anti_aliasing_switches_in_a_frame_once_its_pipelines_are_compiled() {
    let Some(context) = gpu() else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(&context, settings()).expect("a renderer");
    let aa = AntiAliasing::Msaa(8).supported(renderer.msaa_sample_counts());
    renderer.set_anti_aliasing(aa).expect("a supported anti-aliasing");
    let start = Instant::now();
    while renderer.anti_aliasing() != aa {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "{aa} never switched: {:?}",
            renderer.pipeline_stats()
        );
        frame(&context, &mut renderer, &target);
    }
    assert!(!renderer.pipeline_stats().switch_waiting);
    assert_eq!(renderer.plan().msaa_samples(), aa.msaa_samples());
}

/// A switch whose attachments are large (#152): once its pipelines exist, its attachments are made off the frame, one
/// a frame, while frames draw with the settings before it; then it takes effect at once. Before the first frame they
/// are made at once, like the renderer's own.
#[test]
fn a_switch_waits_for_its_large_attachments_while_frames_draw_as_before() {
    let Some(context) = gpu() else { return };
    // 1024² at 4 bytes and 4 samples: 16 MiB a texture, at the size made off the frame.
    let size = 1024;
    let target = OffscreenTarget::new(&context, size, size, FORMAT).expect("a target");
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, size, size)).expect("a renderer");
    renderer.wait_for_pipelines();
    let Some(samples) = renderer.msaa_sample_counts().iter().filter(|&n| n >= 4).min() else {
        return;
    };
    renderer.render(target.view());
    renderer.set_msaa_samples(samples).expect("a supported count");
    let stats = renderer.texture_stats();
    // The MSAA colour and depth.
    assert_eq!(stats.pending, 2, "{stats:?}");
    assert!(renderer.pipeline_stats().switch_waiting);
    assert_eq!(renderer.plan().msaa_samples(), 1);
    // One a frame, each made on a worker while the frame before is presented.
    let (mut frames, deadline) = (0, Instant::now() + Duration::from_secs(10));
    while renderer.plan().msaa_samples() == 1 {
        assert!(
            Instant::now() < deadline,
            "the switch never took effect: {:?}",
            renderer.texture_stats()
        );
        frame(&context, &mut renderer, &target);
        frames += 1;
    }
    assert!(frames >= 2, "both attachments arrived in one frame");
    assert_eq!(renderer.plan().msaa_samples(), samples);
    assert_eq!(renderer.settings().msaa_samples, samples);
    assert!(!renderer.pipeline_stats().switch_waiting);
    assert_eq!(renderer.texture_stats().pending, 0);
    let msaa = read_back_rgba8(&context, target.texture());
    // `wait_for_pipelines` makes them at once (tools and tests); switching back makes none (one sample needs no MSAA
    // colour, and the depth is 4 MiB).
    renderer.set_msaa_samples(1).expect("one sample");
    assert_eq!(renderer.plan().msaa_samples(), 1);
    renderer.set_msaa_samples(samples).expect("a supported count");
    renderer.wait_for_pipelines();
    assert_eq!(renderer.plan().msaa_samples(), samples);
    assert_eq!(frame(&context, &mut renderer, &target), msaa);
}

#[test]
fn the_pipeline_cache_is_kept_between_runs() {
    let dir = std::env::temp_dir().join(format!("metallic-pipeline-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let options = ContextOptions {
        pipeline_cache_dir: Some(dir.clone()),
        ..ContextOptions::default()
    };
    let Some(context) = gpu_with(&options) else { return };
    let Some(loaded) = context.pipeline_cache_loaded() else {
        // No pipeline cache on this backend (every one but Vulkan).
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    assert_eq!(loaded, 0, "a cold start finds no file");
    let mut renderer = Renderer::new(&context, settings()).expect("a renderer");
    renderer.wait_for_pipelines();
    // A worker writes the file once its queue runs dry, just after the last compile is handed over.
    let start = Instant::now();
    let written = || {
        std::fs::read_dir(&dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "bin"))
        })
    };
    while !written() && start.elapsed() < Duration::from_secs(10) {
        std::thread::yield_now();
    }
    assert!(written(), "no pipeline cache written in {}", dir.display());
    drop(renderer);
    drop(context);
    let warm = gpu_with(&options).expect("the same adapter again");
    assert!(warm.pipeline_cache_loaded().is_some_and(|bytes| bytes > 0));
    let mut renderer = Renderer::new(&warm, settings()).expect("a renderer from the cache");
    renderer.wait_for_pipelines();
    assert_eq!(renderer.pipeline_stats().failed, 0);
    drop(renderer);
    drop(warm);
    let _ = std::fs::remove_dir_all(&dir);
}
