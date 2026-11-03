//! Drawing parts (#77) at 1080p offscreen: the CPU time of a frame (`Renderer::render_scene`: the scene upload,
//! the frame uniforms, the transparent sort, recording every pass and submitting) and the GPU time per pass from
//! timestamps, for 10,000 and 100,000 parts with nothing culled, with the camera still and with it moving every
//! frame (which writes the frame uniform and sorts the see-through parts again). The baseline culling (#79) is
//! measured against.
//!
//! ```text
//! cargo bench -p metallic_render --bench draw_parts
//! ```
//!
//! The grid is 70% blocks and 10% each of balls, cylinders and wedges, with 1% see-through blocks, all in view.
//! Frames are paced like a swapchain with two frames in flight (the wait is outside the timed part). Without an
//! adapter it prints why and exits.

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use metallic_core::CFrame;
use metallic_core::glam::Vec3;
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    Camera, CameraReader, ContextOptions, GpuScene, OffscreenTarget, PassId, RenderContext, Renderer, RendererSettings,
    wgpu,
};
use support::scenes::{self, Game, SeeThrough};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const WARM_UP: usize = 30;
const FRAMES: usize = 300;

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index].as_secs_f64() * 1000.0
}

fn run(context: &RenderContext, parts: usize, orbit: bool) {
    let mut game = Game::new();
    scenes::grid(&mut game, parts, 10, SeeThrough::Blocks(100));
    let extent = (parts as f32).sqrt() * 2.0;
    let target = OffscreenTarget::new(context, WIDTH, HEIGHT, FORMAT).expect("a target");
    let mut renderer = Renderer::new(context, RendererSettings::new(FORMAT, WIDTH, HEIGHT)).expect("a renderer");
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    let mut in_flight: [Option<wgpu::SubmissionIndex>; 2] = [None, None];
    let mut cpu = Vec::with_capacity(FRAMES);
    let mut gpu_total = Vec::new();
    let mut gpu_opaque = Vec::new();
    let mut gpu_transparent = Vec::new();
    let mut last_timed = None;
    for frame in 0..WARM_UP + FRAMES {
        let camera = if orbit {
            // A new view every frame: the frame uniform is written and the see-through parts sorted again.
            let angle = frame as f32 * 0.01;
            let eye = Vec3::new(angle.sin(), 1.0, angle.cos()) * extent * 1.2;
            Some(Camera::from_cframe(CFrame::look_at(eye, Vec3::ZERO, Vec3::Y), 70.0))
        } else {
            reader.read(&game.dm, game.workspace)
        };
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
        cpu.push(elapsed);
        if let Some(timings) = renderer.gpu_timings()
            && last_timed != Some(timings.frame())
        {
            last_timed = Some(timings.frame());
            gpu_total.push(timings.total_ms());
            gpu_opaque.push(timings.pass(PassId::Opaque).unwrap_or(0.0));
            gpu_transparent.push(timings.pass(PassId::Transparent).unwrap_or(0.0));
        }
    }
    cpu.sort_unstable();
    let stats = renderer.draw_stats();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    println!(
        "{parts} parts, {}: {} opaque + {} transparent draws, {} instances, {:.2} M triangles",
        if orbit { "camera moving" } else { "camera still" },
        stats.opaque_draws,
        stats.transparent_draws,
        stats.instances,
        stats.triangles as f64 / 1e6
    );
    println!(
        "  CPU render_scene: p50 {:.3} ms, p99 {:.3} ms ({FRAMES} frames)",
        percentile(&cpu, 0.5),
        percentile(&cpu, 0.99)
    );
    println!(
        "  GPU ({} frames timed): total p50 {:.3} ms (mean {:.3}), opaque p50 {:.3} ms, transparent p50 {:.3} ms",
        gpu_total.len(),
        median(&mut gpu_total.clone()),
        mean(&gpu_total),
        median(&mut gpu_opaque),
        median(&mut gpu_transparent)
    );
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
        "adapter: {} ({:?}, {:?}, {}), instances: {:?}, {WIDTH}x{HEIGHT}",
        caps.adapter_name,
        caps.backend,
        caps.device_type,
        caps.driver,
        InstanceBinding::for_context(&context)
    );
    for (parts, orbit) in [(10_000, false), (10_000, true), (100_000, false), (100_000, true)] {
        run(&context, parts, orbit);
    }
}
