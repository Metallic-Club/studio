//! Renders small scenes on a real (or software) adapter. Skipped when the machine has none.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};

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
