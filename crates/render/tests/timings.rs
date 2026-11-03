//! Per-pass GPU timings arrive asynchronously, a few frames late, without the renderer waiting for the GPU.

mod support;

use std::time::{Duration, Instant};

use metallic_render::{OffscreenTarget, PassId, PassSet, Renderer, RendererSettings, wgpu};
use support::gpu;

#[test]
fn timings_arrive_after_some_frames() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let passes = PassSet::default().with(PassId::DepthPrepass);
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            passes,
            ..RendererSettings::new(format, 256, 256)
        },
    )
    .unwrap();
    if !context.capabilities().timestamp_queries {
        assert!(!renderer.gpu_timing_enabled());
        eprintln!(
            "skipped: {} has no timestamp queries",
            context.capabilities().adapter_name
        );
        return;
    }
    assert!(renderer.gpu_timing_enabled());
    assert!(renderer.gpu_timings().is_none(), "no timings before the first frame");
    let target = OffscreenTarget::new(&context, 256, 256, format).unwrap();

    // The renderer never blocks, so on a slow (software) adapter the readback can take a while to come back; give it
    // a few seconds of frames.
    let deadline = Instant::now() + Duration::from_secs(10);
    while renderer.gpu_timings().is_none() {
        assert!(
            Instant::now() < deadline,
            "no GPU timings after {} frames",
            renderer.frame_index()
        );
        renderer.render(target.view());
        std::thread::sleep(Duration::from_millis(1));
    }
    let timings = *renderer.gpu_timings().unwrap();
    eprintln!(
        "timings of frame {} arrived by frame {}: {:?}",
        timings.frame(),
        renderer.frame_index(),
        timings.passes()
    );
    assert!(timings.frame() < renderer.frame_index());
    let names: Vec<_> = timings.passes().iter().map(|timing| timing.pass).collect();
    assert_eq!(names, ["depth_prepass", "opaque", "sky", "transparent", "ui"]);
    for timing in timings.passes() {
        assert!(timing.gpu_ms.is_finite() && timing.gpu_ms >= 0.0, "{timing:?}");
        // An empty pass on a 256x256 target cannot take a second, whatever the adapter.
        assert!(timing.gpu_ms < 1000.0, "{timing:?}");
    }

    // Timings keep coming, newer each time.
    let first = timings.frame();
    let deadline = Instant::now() + Duration::from_secs(10);
    while renderer.gpu_timings().unwrap().frame() == first {
        assert!(Instant::now() < deadline, "timings stopped after frame {first}");
        renderer.render(target.view());
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn timing_can_be_turned_off() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(format, 16, 16)
        },
    )
    .unwrap();
    let target = OffscreenTarget::new(&context, 16, 16, format).unwrap();
    for _ in 0..10 {
        renderer.render(target.view());
    }
    assert!(!renderer.gpu_timing_enabled());
    assert!(renderer.gpu_timings().is_none());
}
