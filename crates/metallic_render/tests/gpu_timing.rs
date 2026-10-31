//! #23: while the profiler records, the renderer times its stages on the GPU and its CPU work
//! in scopes; while it does not, it records nothing. Skipped without an adapter.
use metallic_core::{Value, Vec3 as V3, World};
use metallic_profiler::{Profiler, Subsystem};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, RenderContext, Renderer};

fn scene() -> World {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("workspace");
    let part = w.create("Part", Some(ws)).expect("part");
    w.set(part, "Size", Value::Vector3(V3::new(8.0, 1.0, 8.0)))
        .expect("size");
    w
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(10.0, 8.0, 10.0),
        look_at: Vec3::ZERO,
        fov_deg: 60.0,
    }
}

#[test]
fn profiled_frames_get_render_scopes_and_gpu_stage_times() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut r = Renderer::new(&ctx, wgpu::TextureFormat::Rgba8UnormSrgb);
    r.sync(&scene());

    // Off: frames are drawn but nothing is recorded.
    Profiler::begin_frame();
    r.capture(&camera(), (64, 64)).expect("capture");
    Profiler::end_frame();
    assert!(Profiler::with_frames(|f| f.is_empty()));

    Profiler::set_enabled(true);
    for _ in 0..6 {
        Profiler::begin_frame();
        r.capture(&camera(), (64, 64)).expect("capture");
        Profiler::end_frame();
    }
    Profiler::set_enabled(false);
    let first = Profiler::with_frames(|f| *f.get(0).expect("frames"));
    let names: Vec<_> = first.scopes().iter().map(|s| s.name).collect();
    for name in ["render", "render.prepare", "render.encode", "render.submit"] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    assert!(first.subsystem_ns(Subsystem::Render) > 0);

    let has_timestamps = ctx
        .device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY);
    let timed = Profiler::with_frames(|frames| frames.iter().filter_map(|f| f.gpu_ns()).count());
    if !has_timestamps {
        assert_eq!(timed, 0, "GPU time without timestamp queries");
        eprintln!(
            "no timestamp queries on {}; GPU timing not checked",
            ctx.adapter_name
        );
        return;
    }
    // Results arrive a frame or more late; the newest frames may still be waiting.
    assert!(timed >= 3, "only {timed} of 6 frames got GPU time");
    let gpu = Profiler::with_frames(|frames| {
        *frames
            .iter()
            .find(|f| f.gpu().is_some())
            .expect("a timed frame")
    });
    let passes: Vec<_> = gpu.gpu().expect("timed").iter().map(|p| p.name).collect();
    assert_eq!(passes, ["shadows", "scene", "exposure", "post"]);
    assert!(gpu.gpu_ns().expect("timed") > 0);
}
