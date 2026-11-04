//! Pipeline compile times (#86, ADR 0008 §9): how long [`Renderer::new`] takes (it returns once the first frame's
//! pipelines are compiled), when the first frame is done on the GPU, and when every background compile is, on the
//! backend `WGPU_BACKEND` picks (`WGPU_DX12_COMPILER` picks DX12's shader compiler).
//!
//! ```text
//! cargo run --release -p metallic_render --example prewarm
//! cargo run --release -p metallic_render --example prewarm -- --cache target/pipeline-cache          # warm (Vulkan)
//! cargo run --release -p metallic_render --example prewarm -- --cache target/pipeline-cache --cold   # cold
//! ```
//!
//! `--cache <dir>` keeps the pipeline cache in `dir` (Vulkan); `--cold` deletes it first. `--msaa <n>` draws the
//! first frame with MSAA. Prints one JSON object.

use std::path::PathBuf;
use std::time::Instant;

use metallic_render::{ContextOptions, OffscreenTarget, RenderContext, Renderer, RendererSettings, wgpu};

fn main() {
    let mut options = ContextOptions::default();
    let mut cold = false;
    let mut msaa = 1;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cache" => options.pipeline_cache_dir = args.next().map(PathBuf::from),
            "--cold" => cold = true,
            "--msaa" => msaa = args.next().and_then(|n| n.parse().ok()).expect("--msaa needs a number"),
            other => panic!("unknown argument {other:?}"),
        }
    }
    if cold && let Some(dir) = options.pipeline_cache_dir.as_ref() {
        let _ = std::fs::remove_dir_all(dir);
    }

    let started = Instant::now();
    let context = RenderContext::new_offscreen(&options).expect("a graphics adapter");
    let setup_ms = ms(started);
    let caps = context.capabilities();
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let (width, height) = (1280, 720);
    let target = OffscreenTarget::new(&context, width, height, format).expect("an offscreen target");

    let start = Instant::now();
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            msaa_samples: msaa,
            ..RendererSettings::new(format, width, height)
        },
    )
    .expect("a renderer");
    let new_ms = ms(start);
    let submission = renderer.render(target.view());
    // Outside the renderer: waiting here only measures when the first frame is done.
    let _ = context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    });
    let first_frame_ms = ms(start);
    let at_first = renderer.pipeline_stats();
    renderer.wait_for_pipelines();
    let stats = renderer.pipeline_stats();
    println!(
        "{{\"backend\":\"{:?}\",\"adapter\":\"{}\",\"driver\":\"{}\",\"dx12_compiler\":\"{}\",\"msaa\":{msaa},\
         \"setup_ms\":{setup_ms:.1},\"new_ms\":{new_ms:.1},\"first_frame_ms\":{first_frame_ms:.1},\
         \"pending_at_first_frame\":{},\"all_ms\":{:.1},\"compiled\":{},\"failed\":{},\"compile_cpu_ms\":{:.1},\
         \"workers\":{},\"cache_loaded\":{}}}",
        caps.backend,
        caps.adapter_name,
        caps.driver,
        std::env::var("WGPU_DX12_COMPILER").unwrap_or_else(|_| "fxc".into()),
        at_first.pending,
        stats.all_ms.unwrap_or(f32::NAN),
        stats.compiled,
        stats.failed,
        stats.compile_ms,
        stats.workers,
        stats
            .cache_loaded
            .map_or_else(|| "null".to_owned(), |bytes| bytes.to_string()),
    );
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}
