//! Renders small scenes on a real (or software) adapter. Skipped when the machine has none.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderError, RenderSettings, Renderer};

const SIZE: (u32, u32) = (320, 200);

fn scene() -> World {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("workspace");
    let ground = w.create("Part", Some(ws)).expect("part");
    w.set(ground, "Size", Value::Vector3(V3::new(200.0, 1.0, 200.0)))
        .expect("size");
    w.set(ground, "Position", Value::Vector3(V3::new(0.0, -0.5, 0.0)))
        .expect("pos");
    let cube = w.create("Part", Some(ws)).expect("part");
    w.set(cube, "Size", Value::Vector3(V3::new(6.0, 6.0, 6.0)))
        .expect("size");
    w.set(cube, "Position", Value::Vector3(V3::new(0.0, 3.0, 0.0)))
        .expect("pos");
    w.set(
        cube,
        "Color",
        Value::Color3(Color3::new(139.0, 108.0, 214.0)),
    )
    .expect("color");
    let glass = w.create("Part", Some(ws)).expect("part");
    w.set(glass, "Size", Value::Vector3(V3::new(8.0, 8.0, 1.0)))
        .expect("size");
    w.set(glass, "Position", Value::Vector3(V3::new(0.0, 4.0, 8.0)))
        .expect("pos");
    w.set(glass, "Transparency", Value::Number(0.5))
        .expect("transparency");
    w
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(14.0, 10.0, 22.0),
        look_at: Vec3::new(0.0, 3.0, 0.0),
        fov_deg: 60.0,
    }
}

fn render(quality: Quality) -> Option<Vec<u8>> {
    let ctx = RenderContext::new_headless().ok()?;
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings { quality },
    );
    r.sync(&scene());
    assert_eq!(r.part_count(), 3);
    Some(r.capture(&camera(), SIZE).expect("capture"))
}

fn distinct_colours(px: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in px.as_chunks::<4>().0 {
        seen.insert([p[0], p[1], p[2]]);
    }
    seen.len()
}

#[test]
fn high_quality_frame_has_sky_ground_and_objects() {
    let Some(px) = render(Quality::High) else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    assert_eq!(px.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    assert!(distinct_colours(&px) > 200, "frame looks flat");
    // Opaque alpha everywhere.
    assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    // Top row is sky, bottom row is ground: they must differ.
    let top = &px[0..3];
    let bottom = &px[px.len() - 4..px.len() - 1];
    assert_ne!(top, bottom);
}

#[test]
fn low_quality_renders_and_resizing_is_safe() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    r.sync(&scene());
    for size in [(320, 200), (97, 61), (640, 480), (320, 200)] {
        let px = r.capture(&camera(), size).expect("capture");
        assert_eq!(px.len(), (size.0 * size.1 * 4) as usize);
        assert!(distinct_colours(&px) > 50);
    }
    // A zero-sized target (minimised window) draws nothing and does not panic.
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    r.render(&tex.create_view(&Default::default()), &camera(), (0, 0));
    // Switching quality at run time keeps working.
    r.set_settings(RenderSettings {
        quality: Quality::High,
    });
    let px = r.capture(&camera(), SIZE).expect("capture");
    assert!(distinct_colours(&px) > 200);
}

/// The default avatar drawn skinned in its rest pose: it covers the middle of the frame, the
/// close view uses LOD0 and a far view LOD2; clearing the instances removes it again.
#[test]
fn skinned_avatar_draws_and_picks_its_level_of_detail() {
    use metallic_render::SkinnedInstance;
    use metallic_render::glam::Mat4;
    use metallic_render::metallic_asset::{Pose, SkinnedModel};
    use std::sync::Arc;

    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = Arc::new(SkinnedModel::from_glb(glb).expect("avatar"));
    // Metres to studs; the avatar stands at the origin facing the camera (+Z).
    let scale = 1.0 / 0.28;
    let mut joints = Vec::new();
    Pose::rest(&model.skeleton).skin_matrices(
        &model.skeleton,
        Mat4::from_scale(Vec3::splat(scale)),
        &mut joints,
    );
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    let empty = World::new_with_services();
    r.sync(&empty);
    let close = Camera {
        position: Vec3::new(0.0, 2.75, 4.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 60.0,
    };
    let before = r.capture(&close, SIZE).expect("capture");
    let instance = SkinnedInstance {
        model: &model,
        joints: &joints,
        slot_colors: &[],
        center: Vec3::new(0.0, 2.75, 0.0),
        height: 5.5,
        detail_scale: scale,
    };
    r.set_skinned(&[instance]);
    let with = r.capture(&close, SIZE).expect("capture");
    assert_eq!(r.skinned_lods(), [0]);
    let centre = |px: &[u8]| {
        let i = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    assert_ne!(centre(&before), centre(&with), "nothing drawn at the torso");
    let changed = before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(with.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    let share = changed as f32 / (SIZE.0 * SIZE.1) as f32;
    assert!(
        (0.05..0.6).contains(&share),
        "avatar covers {share} of the frame"
    );

    let far = Camera {
        position: Vec3::new(0.0, 2.75, 200.0),
        ..close
    };
    r.capture(&far, SIZE).expect("capture");
    assert_eq!(r.skinned_lods(), [2]);

    r.set_skinned(&[]);
    let cleared = r.capture(&close, SIZE).expect("capture");
    assert_eq!(centre(&before), centre(&cleared));
}

/// Runs `f` inside a validation error scope and fails the test on any wgpu validation error.
/// (The context logs uncaptured errors instead of panicking, so tests must catch them here.)
fn no_validation_errors(ctx: &RenderContext, f: impl FnOnce()) {
    let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    f();
    if let Some(e) = pollster::block_on(scope.pop()) {
        panic!("wgpu validation error: {e}");
    }
}

fn target(ctx: &RenderContext, width: u32, height: u32) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

#[test]
fn render_with_a_size_other_than_the_target_uses_the_target() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    for quality in [Quality::Low, Quality::High] {
        let mut r = Renderer::with_settings(
            &ctx,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            RenderSettings { quality },
        );
        r.sync(&scene());
        let tex = target(&ctx, 8, 8);
        let view = tex.create_view(&Default::default());
        no_validation_errors(&ctx, || {
            // Stale sizes, as during a window resize: smaller, larger, then the right one.
            r.render(&view, &camera(), (4, 4));
            r.render(&view, &camera(), (16, 9));
            r.render(&view, &camera(), (8, 8));
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        });
    }
}

#[test]
fn capture_wider_than_8192_does_not_panic() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = Renderer::new(&ctx, wgpu::TextureFormat::Rgba8UnormSrgb);
    r.sync(&scene());
    let max = ctx.device.limits().max_texture_dimension_2d;
    no_validation_errors(&ctx, || {
        let result = r.capture(&camera(), (8193, 2));
        if max >= 8193 {
            let px = result.expect("adapter allows 8193 px, capture must work");
            assert_eq!(px.len(), 8193 * 2 * 4);
        } else {
            assert!(matches!(result, Err(RenderError::InvalidSize)));
        }
    });
}

#[test]
fn capture_above_the_device_limit_is_invalid_size() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = Renderer::new(&ctx, wgpu::TextureFormat::Rgba8UnormSrgb);
    r.sync(&scene());
    let max = ctx.device.limits().max_texture_dimension_2d;
    no_validation_errors(&ctx, || {
        for size in [(max + 1, 2), (2, max + 1)] {
            assert!(matches!(
                r.capture(&camera(), size),
                Err(RenderError::InvalidSize)
            ));
        }
        // Prewarm clamps instead of failing.
        r.prewarm((max + 1, 2));
    });
}
