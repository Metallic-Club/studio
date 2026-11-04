//! Per-pass GPU timings arrive asynchronously, a few frames late, without the renderer waiting for the GPU.

mod support;

use std::time::{Duration, Instant};

use metallic_render::{
    FrameTimings, OffscreenTarget, PassId, PassSet, RenderContext, Renderer, RendererSettings, wgpu,
};
use support::gpu;

/// A renderer of `passes` into a 256 × 256 target, or `None` (saying why) when the adapter has no timestamp queries.
fn timed_renderer(context: &RenderContext, passes: PassSet) -> Option<(Renderer, OffscreenTarget)> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes,
            ..RendererSettings::new(format, 256, 256)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    if !context.capabilities().timestamp_queries {
        assert!(!renderer.gpu_timing_enabled());
        eprintln!(
            "skipped: {} has no timestamp queries",
            context.capabilities().adapter_name
        );
        return None;
    }
    assert!(renderer.gpu_timing_enabled());
    assert!(renderer.gpu_timings().is_none(), "no timings before the first frame");
    let target = OffscreenTarget::new(context, 256, 256, format).expect("a target");
    Some((renderer, target))
}

/// Renders until the first timings arrive and returns them.
fn first_timings(renderer: &mut Renderer, target: &OffscreenTarget) -> FrameTimings {
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
    let timings = *renderer.gpu_timings().expect("timings arrived");
    eprintln!(
        "timings of frame {} arrived by frame {}: {:?}",
        timings.frame(),
        renderer.frame_index(),
        timings.passes()
    );
    assert!(timings.frame() < renderer.frame_index());
    for timing in timings.passes() {
        assert!(timing.gpu_ms.is_finite() && timing.gpu_ms >= 0.0, "{timing:?}");
        // An empty pass on a 256x256 target cannot take a second, whatever the adapter.
        assert!(timing.gpu_ms < 1000.0, "{timing:?}");
    }
    timings
}

fn names(timings: &FrameTimings) -> Vec<&'static str> {
    timings.passes().iter().map(|timing| timing.pass).collect()
}

/// Whether the adapter times the passes a merged render pass runs one by one: it writes timestamps inside passes and
/// is not tile-based.
fn times_inside_passes(context: &RenderContext) -> bool {
    let caps = context.capabilities();
    caps.timestamp_queries_inside_passes && !caps.transient_attachments
}

#[test]
fn timings_arrive_after_some_frames() {
    let Some(context) = gpu() else { return };
    let passes = PassSet::default().with(PassId::DepthPrepass);
    let Some((mut renderer, target)) = timed_renderer(&context, passes) else {
        return;
    };
    let timings = first_timings(&mut renderer, &target);
    // Bloom's chain of passes is timed as one. UI is drawn in the finish pass (#97): timed apart where timestamps
    // can be written inside a pass, with it otherwise.
    let mut expected = vec!["depth_prepass", "opaque", "sky", "transparent", "bloom", "post"];
    if times_inside_passes(&context) {
        expected.push("ui");
    }
    assert_eq!(names(&timings), expected);

    // Timings keep coming, newer each time.
    let first = timings.frame();
    let deadline = Instant::now() + Duration::from_secs(10);
    while renderer.gpu_timings().unwrap().frame() == first {
        assert!(Instant::now() < deadline, "timings stopped after frame {first}");
        renderer.render(target.view());
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Merged scene passes (#97) keep their logical pass list: each is timed apart where timestamps can be written inside
/// a pass, and the merged pass is reported as one otherwise.
#[test]
fn merged_passes_are_timed_apart_where_the_device_can() {
    let Some(context) = gpu() else { return };
    let passes = PassSet::default().with(PassId::DepthPrepass).with(PassId::Scene);
    let Some((mut renderer, target)) = timed_renderer(&context, passes) else {
        return;
    };
    let timings = first_timings(&mut renderer, &target);
    let inside = times_inside_passes(&context);
    eprintln!(
        "{} ({:?}): timed inside passes: {inside}",
        context.capabilities().adapter_name,
        context.capabilities().backend
    );
    if inside {
        assert_eq!(
            names(&timings),
            ["depth_prepass", "opaque", "sky", "transparent", "bloom", "post", "ui"]
        );
        assert_eq!(timings.pass(PassId::Scene), None);
    } else {
        // The prepass is never merged.
        assert_eq!(names(&timings), ["depth_prepass", "scene", "bloom", "post"]);
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
    renderer.wait_for_pipelines();
    let target = OffscreenTarget::new(&context, 16, 16, format).unwrap();
    for _ in 0..10 {
        renderer.render(target.view());
    }
    assert!(!renderer.gpu_timing_enabled());
    assert!(renderer.gpu_timings().is_none());
}
