//! Offscreen clear: sets up a device with no window, renders frames that only clear, reads one back to check the
//! colour, and prints the adapter, the CPU cost of a frame and the per-pass GPU timings.
//!
//! ```text
//! cargo run -p metallic_render --example clear [frames]
//! ```

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use metallic_render::{
    ContextOptions, OffscreenTarget, PassId, PassSet, RenderContext, Renderer, RendererSettings, TIMING_RING_SIZE, wgpu,
};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const WARM_UP: usize = 30;

fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .map(|arg| arg.parse().expect("the frame count is a number"))
        .unwrap_or(600);

    let started = Instant::now();
    let context = RenderContext::new_offscreen(&ContextOptions::default()).expect("a graphics adapter");
    let setup = started.elapsed();
    let caps = context.capabilities();
    println!(
        "adapter:   {} ({:?}, {:?})",
        caps.adapter_name, caps.backend, caps.device_type
    );
    println!("driver:    {}", caps.driver);
    println!("setup:     {:.1} ms (instance, adapter, device)", ms(setup));
    println!(
        "features:  compute {}, indirect {}, native multi-draw-indirect {}, timestamps {}, vertex storage {}, \
         max texture {}, MSAA {:?}, transient attachments {}",
        caps.compute,
        caps.indirect_draw,
        caps.multi_draw_indirect_native,
        caps.timestamp_queries,
        caps.vertex_storage,
        caps.max_texture_size,
        caps.msaa.iter().collect::<Vec<_>>(),
        caps.transient_attachments
    );

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&context, WIDTH, HEIGHT, format).expect("an offscreen target");
    let clear = wgpu::Color {
        r: 0.1,
        g: 0.3,
        b: 0.6,
        a: 1.0,
    };
    let passes = PassSet::default().with(PassId::DepthPrepass).with(PassId::Post);
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            passes,
            clear_color: clear,
            ..RendererSettings::new(format, WIDTH, HEIGHT)
        },
    )
    .expect("a renderer");
    println!(
        "frame:     {WIDTH}x{HEIGHT} {format:?}, passes {passes:?}, GPU timing {}",
        renderer.gpu_timing_enabled()
    );

    // Like a swapchain with two frames of latency: wait, outside the frame, for the frame from two frames back.
    let mut in_flight: [Option<wgpu::SubmissionIndex>; 2] = [None, None];
    let mut cpu = Vec::with_capacity(frames);
    let mut gpu_sum = [0.0f64; PassId::COUNT];
    let (mut gpu_frames, mut last_timed) = (0u32, None);
    for frame in 0..WARM_UP + frames {
        let start = Instant::now();
        let submission = renderer.render(target.view());
        let elapsed = start.elapsed();
        if let Some(oldest) = in_flight[frame % 2].replace(submission) {
            let _ = context.device().poll(wgpu::PollType::Wait {
                submission_index: Some(oldest),
                timeout: None,
            });
        }
        if frame < WARM_UP {
            continue;
        }
        cpu.push(elapsed);
        if let Some(timings) = renderer.gpu_timings()
            && last_timed != Some(timings.frame())
        {
            last_timed = Some(timings.frame());
            gpu_frames += 1;
            for (sum, pass) in gpu_sum.iter_mut().zip(timings.passes()) {
                *sum += f64::from(pass.gpu_ms);
            }
        }
    }

    cpu.sort_unstable();
    let percentile = |p: f64| ms(cpu[((cpu.len() - 1) as f64 * p) as usize]);
    println!(
        "CPU:       render() (encode + submit) over {frames} frames: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
        percentile(0.5),
        percentile(0.99),
        percentile(1.0)
    );
    match renderer.gpu_timings() {
        Some(latest) => {
            println!(
                "GPU:       {gpu_frames} frames timed (readback ring of {TIMING_RING_SIZE}); latest is frame {} of {}",
                latest.frame(),
                renderer.frame_index()
            );
            for (pass, sum) in latest.passes().iter().zip(gpu_sum) {
                println!(
                    "           {:<14} mean {:.4} ms, latest {:.4} ms",
                    pass.pass,
                    sum / f64::from(gpu_frames.max(1)),
                    pass.gpu_ms
                );
            }
            println!("           total          latest {:.4} ms", latest.total_ms());
        }
        None => println!("GPU:       no timings (the adapter has no timestamp queries)"),
    }

    let pixels = support::read_back_rgba8(&context, target.texture());
    let centre = pixels[(HEIGHT / 2 * WIDTH + WIDTH / 2) as usize];
    // The clear colour, sRGB-encoded.
    let expected = [89, 149, 203, 255];
    println!("readback:  centre pixel {centre:?} (expected {expected:?})");
    support::assert_all_pixels(&pixels, expected, 1);
    println!("ok");
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
