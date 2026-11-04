//! Merged scene passes (#97) offscreen: each preset's anti-aliasing, effects and scale drawn with the scene passes
//! apart (opaque, sky, transparent) and merged into one render pass ([`PassId::Scene`]), on any backend, including
//! Vulkan on machines where it has no adapter for a window. Reports the render passes per frame, the CPU time of
//! `Renderer::render_scene` and the GPU time of the frame and of its scene passes, as p50 over the rounds' frames.
//!
//! ```text
//! cargo bench -p metallic_render --bench scene_passes
//! WGPU_BACKEND=vulkan cargo bench -p metallic_render --bench scene_passes
//! ```
//!
//! The scene is 10,000 parts in a grid with 1% see-through blocks under the sun with its shadows, at the `mobile`
//! stress scene's 1280 × 720. Configurations run in turn, a few rounds each, so drift on the machine falls on all of
//! them. Frames are paced like a swapchain with two frames in flight (the wait is outside the timed part). Without
//! an adapter it prints why and exits.

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::Instant;

use metallic_render::post::PostSettings;
use metallic_render::resolution::ResolutionSettings;
use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, OffscreenTarget, PassId, PassSet, RenderContext, Renderer,
    RendererSettings, wgpu,
};
use support::scenes::{self, Game, SeeThrough};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
const WARM_UP: usize = 60;
const FRAMES: usize = 400;
const ROUNDS: usize = 3;

/// A preset's scene settings: its anti-aliasing, effects and scale.
struct Preset {
    name: &'static str,
    msaa: u32,
    mlaa: bool,
    post: PostSettings,
    scale: f32,
}

const PRESETS: [Preset; 4] = [
    Preset {
        name: "low",
        msaa: 1,
        mlaa: true,
        post: PostSettings::LOW,
        scale: 0.75,
    },
    Preset {
        name: "medium",
        msaa: 1,
        mlaa: true,
        post: PostSettings::MEDIUM,
        scale: 1.0,
    },
    Preset {
        name: "high",
        msaa: 4,
        mlaa: false,
        post: PostSettings::HIGH,
        scale: 1.0,
    },
    Preset {
        name: "ultra",
        msaa: 8,
        mlaa: false,
        post: PostSettings::HIGH,
        scale: 1.0,
    },
];

#[derive(Default)]
struct Samples {
    cpu_ms: Vec<f32>,
    gpu_ms: Vec<f32>,
    scene_ms: Vec<f32>,
    render_passes: u32,
}

fn median(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}

fn run(context: &RenderContext, preset: &Preset, merged: bool, samples: &mut Samples) {
    // A game of its own each run: a data model takes a few scenes' consumers at most.
    let mut game = Game::new();
    scenes::grid(&mut game, 10_000, 10, SeeThrough::Blocks(100));
    let game = &mut game;
    let passes = PassSet::default()
        .with_if(PassId::Mlaa, preset.mlaa)
        .with_if(PassId::Scene, merged);
    let settings = RendererSettings {
        msaa_samples: preset.msaa,
        passes,
        post: preset.post,
        resolution: ResolutionSettings::fixed(preset.scale),
        ..RendererSettings::new(FORMAT, WIDTH, HEIGHT)
    };
    let target = OffscreenTarget::new(context, WIDTH, HEIGHT, FORMAT).expect("a target");
    let mut renderer = Renderer::new(context, settings).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    let mut in_flight: [Option<wgpu::SubmissionIndex>; 2] = [None, None];
    let mut last_timed = None;
    for frame in 0..WARM_UP + FRAMES {
        let camera = reader.read(&game.dm, game.workspace);
        scene.sync(&mut game.dm);
        let start = Instant::now();
        let submission = renderer.render_scene(target.view(), &mut scene, camera.as_ref());
        let elapsed = start.elapsed();
        if let Some(oldest) = in_flight[frame % 2].replace(submission) {
            context
                .device()
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(oldest),
                    timeout: None,
                })
                .expect("wait for an earlier frame");
        }
        if frame < WARM_UP {
            continue;
        }
        samples.cpu_ms.push(elapsed.as_secs_f32() * 1000.0);
        if let Some(timings) = renderer.gpu_timings()
            && last_timed != Some(timings.frame())
        {
            last_timed = Some(timings.frame());
            samples.gpu_ms.push(timings.total_ms());
            let scene_ms = [
                PassId::DepthPrepass,
                PassId::Opaque,
                PassId::Sky,
                PassId::Transparent,
                PassId::Scene,
            ]
            .into_iter()
            .filter_map(|pass| timings.pass(pass))
            .sum();
            samples.scene_ms.push(scene_ms);
        }
    }
    samples.render_passes = renderer.post_stats().render_passes;
}

fn main() {
    // `cargo bench` passes `--bench`; a filter argument is not supported.
    let context = match RenderContext::new_offscreen(&ContextOptions::default()) {
        Ok(context) => context,
        Err(error) => {
            println!("skipped: {error}");
            return;
        }
    };
    let caps = context.capabilities();
    println!(
        "adapter: {} ({:?}, {:?}, {}), timestamps inside passes: {}, {WIDTH}x{HEIGHT}",
        caps.adapter_name, caps.backend, caps.device_type, caps.driver, caps.timestamp_queries_inside_passes
    );
    let mut results: Vec<[Samples; 2]> = PRESETS.iter().map(|_| Default::default()).collect();
    for _ in 0..ROUNDS {
        for (preset, result) in PRESETS.iter().zip(&mut results) {
            if preset.msaa > 1 && !caps.msaa.contains(preset.msaa) {
                continue;
            }
            for (merged, samples) in [false, true].into_iter().zip(result.iter_mut()) {
                run(&context, preset, merged, samples);
            }
        }
    }
    println!("preset  scene   render passes  CPU p50 ms  GPU p50 ms  scene passes GPU p50 ms");
    for (preset, result) in PRESETS.iter().zip(&results) {
        for (name, samples) in ["apart", "merged"].into_iter().zip(result) {
            if samples.cpu_ms.is_empty() {
                continue;
            }
            println!(
                "{:<7} {name:<7} {:>13}  {:>10.3}  {:>10.3}  {:>10.3}",
                preset.name,
                samples.render_passes,
                median(&samples.cpu_ms),
                median(&samples.gpu_ms),
                median(&samples.scene_ms),
            );
        }
    }
}
